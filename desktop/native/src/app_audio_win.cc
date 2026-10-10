// Application sound on Windows: WASAPI process loopback
// (ActivateAudioInterfaceAsync with AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
// Windows 10 2004 and later), behind the same interface as the PipeWire
// capture (app_audio.h) and mixed by the same code (app_audio_mix.h).
//
// "" (every application) is one loopback client that leaves out this
// process's tree (PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE): the
// call's playout and the webview's processes, which this process started,
// stay out. An application is chosen by its binary ("firefox.exe"): one
// client per process tree of that binary that has an audio session
// (PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE), looked for again
// every second, so processes that start playing later are picked up. The
// list comes from the audio sessions of the active playback devices
// (IAudioSessionManager2), without this process's tree and without the
// system sounds.
//
// Windows has none of PipeWire's virtual devices that play applications'
// sound on: no such streams to leave out here. Programs on one binary are
// one entry, which chooses them all; the entry is called by the session's
// display name, or else by the binary's file description.
//
// Threads: one capture thread per capture in the multithreaded apartment
// owns the loopback clients and reads them, the mixer thread hands 10 ms
// blocks on at the pace of the sound (app_audio_mix.h). The list is made on
// a thread of its own in the multithreaded apartment, whatever apartment the
// caller is in.

#include "app_audio.h"
#include "app_audio_mix.h"

#include <rtc_base/logging.h>

#include <windows.h>
#include <unknwn.h>
#include <audioclient.h>
#include <audioclientactivationparams.h>
#include <audiopolicy.h>
#include <mmdeviceapi.h>
#include <tlhelp32.h>

// With _WIN32_WINNT at Windows 10, NTDDI_VERSION is the SDK's newest; the
// process loopback declarations need one of 10.0.20348 or later.
#if !defined(NTDDI_WIN10_FE) || NTDDI_VERSION < NTDDI_WIN10_FE
#  error "application sound needs a Windows SDK of 10.0.20348 or later (process loopback)"
#endif

#include <winrt/base.h>

#include <algorithm>
#include <atomic>
#include <chrono>
#include <cmath>
#include <cstdio>
#include <cwchar>
#include <exception>
#include <future>
#include <iterator>
#include <map>
#include <memory>
#include <mutex>
#include <optional>
#include <set>
#include <stdexcept>
#include <string>
#include <thread>
#include <vector>

namespace gelabber
{
	namespace
	{
		static_assert(MixInput::kBlockFrames == AppAudioCapture::kFrames);
		static_assert(MixInput::kChannels == static_cast<size_t>(AppAudioCapture::kChannels));

		std::string Hex(int32_t code)
		{
			char text[16];
			std::snprintf(text, sizeof text, "0x%08X", static_cast<unsigned>(code));
			return text;
		}

		std::string Describe(const winrt::hresult_error& error)
		{
			return winrt::to_string(error.message()) + " (" + Hex(error.code().value) + ")";
		}

		// What an older Windows answers a process loopback with.
		std::string SupportHint(winrt::hresult code)
		{
			const int32_t value = code.value;
			return value == E_NOTIMPL || value == E_NOINTERFACE || value == E_INVALIDARG
			         ? " (process loopback needs Windows 10 version 2004 or later)"
			         : "";
		}

		std::string Utf8(const std::wstring& text)
		{
			if (text.empty())
				return {};
			const int size = WideCharToMultiByte(
			  CP_UTF8, 0, text.data(), static_cast<int>(text.size()), nullptr, 0, nullptr, nullptr);
			std::string out(static_cast<size_t>(std::max(size, 0)), '\0');
			if (size > 0)
				WideCharToMultiByte(
				  CP_UTF8, 0, text.data(), static_cast<int>(text.size()), out.data(), size, nullptr, nullptr);
			return out;
		}

		std::wstring Lower(std::wstring text)
		{
			if (!text.empty())
				CharLowerBuffW(text.data(), static_cast<DWORD>(text.size()));
			return text;
		}

		// Runs `body` on a thread of its own in the multithreaded apartment
		// and hands on what it returns or throws; Windows' errors as
		// std::runtime_error, which the C ABI reports.
		template <class F>
		auto InMta(F body) -> decltype(body())
		{
			using Result = decltype(body());
			std::optional<Result> result;
			std::exception_ptr failure;
			std::thread([&] {
				try
				{
					winrt::init_apartment(winrt::apartment_type::multi_threaded);
					try
					{
						result.emplace(body());
					}
					catch (...)
					{
						failure = std::current_exception();
					}
					winrt::uninit_apartment();
				}
				catch (...)
				{
					failure = std::current_exception();
				}
			}).join();
			if (failure)
			{
				try
				{
					std::rethrow_exception(failure);
				}
				catch (const winrt::hresult_error& error)
				{
					throw std::runtime_error("application sound: " + Describe(error));
				}
			}
			return std::move(*result);
		}

		// Process id -> parent's, of every process now.
		std::map<DWORD, DWORD> Parents()
		{
			std::map<DWORD, DWORD> parents;
			HANDLE snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
			if (snapshot == INVALID_HANDLE_VALUE)
				return parents;
			PROCESSENTRY32W entry{};
			entry.dwSize = sizeof entry;
			for (BOOL more = Process32FirstW(snapshot, &entry); more; more = Process32NextW(snapshot, &entry))
				parents[entry.th32ProcessID] = entry.th32ParentProcessID;
			CloseHandle(snapshot);
			return parents;
		}

		// Whether `process` is `root` or was started by it, or by a process
		// it started, and so on.
		bool InTreeOf(DWORD process, DWORD root, const std::map<DWORD, DWORD>& parents)
		{
			for (int depth = 0; process != 0 && depth < 64; ++depth)
			{
				if (process == root)
					return true;
				const auto parent = parents.find(process);
				if (parent == parents.end() || parent->second == process)
					return false;
				process = parent->second;
			}
			return false;
		}

		std::wstring ImagePath(DWORD process)
		{
			HANDLE handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, process);
			if (!handle)
				return {};
			wchar_t path[1024];
			DWORD size  = static_cast<DWORD>(std::size(path));
			const BOOL ok = QueryFullProcessImageNameW(handle, 0, path, &size);
			CloseHandle(handle);
			return ok ? std::wstring(path, size) : std::wstring();
		}

		std::wstring FileName(const std::wstring& path)
		{
			const auto slash = path.find_last_of(L"\\/");
			return slash == std::wstring::npos ? path : path.substr(slash + 1);
		}

		// The binary's own name for itself (its FileDescription), if it has one.
		std::wstring Description(const std::wstring& path)
		{
			DWORD ignored    = 0;
			const DWORD size = GetFileVersionInfoSizeW(path.c_str(), &ignored);
			if (size == 0)
				return {};
			std::vector<BYTE> data(size);
			if (!GetFileVersionInfoW(path.c_str(), 0, size, data.data()))
				return {};
			struct Translation
			{
				WORD language;
				WORD codePage;
			};
			Translation* translations = nullptr;
			UINT length               = 0;
			if (!VerQueryValueW(
			      data.data(), L"\\VarFileInfo\\Translation", reinterpret_cast<void**>(&translations), &length) ||
			    !translations || length < sizeof(Translation))
				return {};
			wchar_t key[64];
			std::swprintf(
			  key,
			  std::size(key),
			  L"\\StringFileInfo\\%04x%04x\\FileDescription",
			  translations[0].language,
			  translations[0].codePage);
			wchar_t* text = nullptr;
			UINT chars    = 0;
			if (!VerQueryValueW(data.data(), key, reinterpret_cast<void**>(&text), &chars) || !text || chars == 0)
				return {};
			return text;
		}

		struct Session
		{
			DWORD process{ 0 };
			// What the application called the session, if anything.
			std::wstring name;
		};

		// The audio sessions of the active playback devices that belong to
		// one process, without the system sounds and those that are over.
		// None where Windows' audio is not there to ask (a server with the
		// audio service off): nothing plays there.
		std::vector<Session> Sessions()
		{
			std::vector<Session> out;
			winrt::com_ptr<IMMDeviceCollection> devices;
			UINT count = 0;
			try
			{
				auto enumerator = winrt::create_instance<IMMDeviceEnumerator>(__uuidof(MMDeviceEnumerator));
				winrt::check_hresult(enumerator->EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE, devices.put()));
				winrt::check_hresult(devices->GetCount(&count));
			}
			catch (const winrt::hresult_error& error)
			{
				RTC_LOG(LS_WARNING) << "application sound: no playback devices to ask: " << Describe(error);
				return out;
			}
			for (UINT i = 0; i < count; ++i)
			{
				winrt::com_ptr<IMMDevice> device;
				if (FAILED(devices->Item(i, device.put())))
					continue;
				winrt::com_ptr<IAudioSessionManager2> manager;
				if (FAILED(device->Activate(__uuidof(IAudioSessionManager2), CLSCTX_ALL, nullptr, manager.put_void())))
					continue;
				winrt::com_ptr<IAudioSessionEnumerator> sessions;
				if (FAILED(manager->GetSessionEnumerator(sessions.put())))
					continue;
				int total = 0;
				if (FAILED(sessions->GetCount(&total)))
					continue;
				for (int j = 0; j < total; ++j)
				{
					winrt::com_ptr<IAudioSessionControl> control;
					if (FAILED(sessions->GetSession(j, control.put())))
						continue;
					auto control2 = control.try_as<IAudioSessionControl2>();
					if (!control2 || control2->IsSystemSoundsSession() == S_OK)
						continue;
					AudioSessionState state = AudioSessionStateExpired;
					if (FAILED(control->GetState(&state)) || state == AudioSessionStateExpired)
						continue;
					DWORD process    = 0;
					const HRESULT hr = control2->GetProcessId(&process);
					// AUDCLNT_S_NO_SINGLE_PROCESS: a session of several.
					if (hr != S_OK || process == 0)
						continue;
					Session session;
					session.process = process;
					LPWSTR name     = nullptr;
					if (SUCCEEDED(control->GetDisplayName(&name)) && name)
					{
						// "@..." points into a resource; not worth resolving.
						if (name[0] != L'\0' && name[0] != L'@')
							session.name = name;
						CoTaskMemFree(name);
					}
					out.push_back(std::move(session));
				}
			}
			return out;
		}

		// A session's process with its binary's id, lowercased
		// ("firefox.exe"), and path; none for this process's own tree.
		struct Player
		{
			DWORD process{ 0 };
			std::wstring id;
			std::wstring path;
			std::wstring name;
		};

		std::vector<Player> Players()
		{
			const auto parents = Parents();
			const DWORD self   = GetCurrentProcessId();
			std::vector<Player> out;
			for (auto& session : Sessions())
			{
				if (InTreeOf(session.process, self, parents))
					continue;
				Player player;
				player.process = session.process;
				player.path    = ImagePath(session.process);
				if (player.path.empty())
					continue;
				player.id   = Lower(FileName(player.path));
				player.name = std::move(session.name);
				out.push_back(std::move(player));
			}
			return out;
		}

		// A Win32 event that outlives whoever waits for it: a completion
		// may come after the waiting is over.
		struct Event
		{
			HANDLE handle{ CreateEventW(nullptr, TRUE, FALSE, nullptr) };
			Event()                        = default;
			Event(const Event&)            = delete;
			Event& operator=(const Event&) = delete;
			~Event()
			{
				if (handle)
					CloseHandle(handle);
			}
		};

		// ActivateAudioInterfaceAsync wants an agile handler; C++/WinRT's
		// implements<> is (IAgileObject, free-threaded marshaler).
		struct ActivationDone : winrt::implements<ActivationDone, ::IActivateAudioInterfaceCompletionHandler>
		{
			explicit ActivationDone(std::shared_ptr<Event> done) : done(std::move(done))
			{
			}

			HRESULT STDMETHODCALLTYPE ActivateCompleted(::IActivateAudioInterfaceAsyncOperation*) noexcept override
			{
				SetEvent(done->handle);
				return S_OK;
			}

			std::shared_ptr<Event> done;
		};

		// 48 kHz 16-bit stereo, what the mix takes; Windows converts
		// (AUTOCONVERTPCM), as in Microsoft's ApplicationLoopback sample.
		WAVEFORMATEX MixFormat()
		{
			WAVEFORMATEX format{};
			format.wFormatTag      = WAVE_FORMAT_PCM;
			format.nChannels       = static_cast<WORD>(AppAudioCapture::kChannels);
			format.nSamplesPerSec  = AppAudioCapture::kSampleRate;
			format.wBitsPerSample  = 16;
			format.nBlockAlign     = static_cast<WORD>(format.nChannels * format.wBitsPerSample / 8);
			format.nAvgBytesPerSec = format.nSamplesPerSec * format.nBlockAlign;
			format.cbSize          = 0;
			return format;
		}

		// One process-loopback client and what it has captured for the mix.
		struct Loopback
		{
			DWORD process{ 0 };
			// The process, to see that it is gone (include mode only).
			HANDLE alive{ nullptr };
			winrt::com_ptr<IAudioClient> client;
			winrt::com_ptr<IAudioCaptureClient> capture;
			Event ready;
			bool failed{ false };
			// Its samples waiting for the mixer (the capture's mutex).
			MixInput input;

			Loopback()                           = default;
			Loopback(const Loopback&)            = delete;
			Loopback& operator=(const Loopback&) = delete;
			~Loopback()
			{
				if (client)
					client->Stop();
				if (alive)
					CloseHandle(alive);
			}
		};

		// Activates and starts a client for `process`'s tree (include) or for
		// everything but it (exclude). Throws with Windows' reason.
		std::unique_ptr<Loopback> OpenLoopback(DWORD process, PROCESS_LOOPBACK_MODE mode)
		{
			auto loopback     = std::make_unique<Loopback>();
			loopback->process = process;
			if (!loopback->ready.handle)
				throw std::runtime_error("application sound: CreateEvent failed");
			if (mode == PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE)
				loopback->alive = OpenProcess(SYNCHRONIZE, FALSE, process);

			AUDIOCLIENT_ACTIVATION_PARAMS params{};
			params.ActivationType                                = AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK;
			params.ProcessLoopbackParams.TargetProcessId     = process;
			params.ProcessLoopbackParams.ProcessLoopbackMode = mode;
			PROPVARIANT activation{};
			activation.vt             = VT_BLOB;
			activation.blob.cbSize    = sizeof params;
			activation.blob.pBlobData = reinterpret_cast<BYTE*>(&params);

			auto done    = std::make_shared<Event>();
			auto handler = winrt::make_self<ActivationDone>(done);
			winrt::com_ptr<IActivateAudioInterfaceAsyncOperation> operation;
			winrt::check_hresult(ActivateAudioInterfaceAsync(
			  VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, __uuidof(IAudioClient), &activation, handler.get(), operation.put()));
			if (WaitForSingleObject(done->handle, 10'000) != WAIT_OBJECT_0)
				throw std::runtime_error("application sound: Windows did not activate the process loopback");
			HRESULT activated = E_FAIL;
			winrt::com_ptr<::IUnknown> unknown;
			winrt::check_hresult(operation->GetActivateResult(&activated, unknown.put()));
			winrt::check_hresult(activated);
			loopback->client = unknown.as<IAudioClient>();

			WAVEFORMATEX format = MixFormat();
			// 200 ms of buffer; the client is read about every 10 ms.
			winrt::check_hresult(loopback->client->Initialize(
			  AUDCLNT_SHAREMODE_SHARED,
			  AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
			  2'000'000,
			  0,
			  &format,
			  nullptr));
			winrt::check_hresult(loopback->client->SetEventHandle(loopback->ready.handle));
			winrt::check_hresult(
			  loopback->client->GetService(__uuidof(IAudioCaptureClient), loopback->capture.put_void()));
			winrt::check_hresult(loopback->client->Start());
			return loopback;
		}

		class WasapiAppAudio : public AppAudioCapture
		{
		public:
			WasapiAppAudio(std::string app, Sink sink) : app(std::move(app)), sink(std::move(sink))
			{
				if (!stop.handle)
					throw std::runtime_error("application sound: CreateEvent failed");
			}

			// Returns once the first clients run; throws when Windows cannot
			// capture applications (before Windows 10 2004) or no client
			// could be opened.
			void Start()
			{
				std::promise<std::string> ready;
				auto started = ready.get_future();
				running      = true;
				capturer     = std::thread([this, ready = std::move(ready)]() mutable { Capture(ready); });
				const std::string error = started.get();
				if (!error.empty())
				{
					Halt();
					throw std::runtime_error(error);
				}
				mixer = std::thread([this] { Mix(); });
			}

			~WasapiAppAudio() override
			{
				Halt();
			}

			std::string StateJson() const override
			{
				std::lock_guard lock(mutex);
				uint64_t underruns = endedUnderruns;
				uint64_t overruns  = endedOverruns;
				for (const auto& loopback : loopbacks)
				{
					underruns += loopback->input.underruns;
					overruns += loopback->input.overruns;
				}
				return R"({"state":"live","streams":)" + std::to_string(loopbacks.size()) + R"(,"frames":)" +
				       std::to_string(delivered) + R"(,"underruns":)" + std::to_string(underruns) +
				       R"(,"overruns":)" + std::to_string(overruns) + "}";
			}

		private:
			void Halt()
			{
				running = false;
				SetEvent(stop.handle);
				if (mixer.joinable())
					mixer.join();
				if (capturer.joinable())
					capturer.join();
			}

			// The capture thread: opens the clients, reads them, and in
			// include mode looks for processes of the binary every second.
			void Capture(std::promise<std::string>& ready)
			{
				bool started = false;
				try
				{
					winrt::init_apartment(winrt::apartment_type::multi_threaded);
				}
				catch (const winrt::hresult_error& error)
				{
					ready.set_value("application sound: no COM apartment: " + Describe(error));
					return;
				}
				try
				{
					if (app.empty())
					{
						auto loopback =
						  OpenLoopback(GetCurrentProcessId(), PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE);
						std::lock_guard lock(mutex);
						loopbacks.push_back(std::move(loopback));
					}
					else
					{
						// Processes of the binary that cannot be captured are
						// left out; only when none of them could be, the
						// capture fails. None playing yet is no error.
						const std::string error = Rescan();
						std::lock_guard lock(mutex);
						if (loopbacks.empty() && !error.empty())
							throw std::runtime_error(error);
					}
					started = true;
					ready.set_value(std::string());
					Read();
				}
				catch (const winrt::hresult_error& error)
				{
					const std::string message = "application sound: " + Describe(error) + SupportHint(error.code());
					if (!started)
						ready.set_value(message);
					else
						RTC_LOG(LS_ERROR) << message;
				}
				catch (const std::exception& error)
				{
					if (!started)
						ready.set_value(error.what());
					else
						RTC_LOG(LS_ERROR) << "application sound: " << error.what();
				}
				catch (...)
				{
					if (!started)
						ready.set_value("application sound: unknown error");
				}
				{
					// Every client goes before the apartment does.
					std::lock_guard lock(mutex);
					loopbacks.clear();
				}
				winrt::uninit_apartment();
			}

			// Include mode: a client for each process tree of the binary that
			// has a session and is not in a tree captured already. Returns
			// why the last process that could not be captured could not.
			std::string Rescan()
			{
				std::string failure;
				const auto parents = Parents();
				std::set<DWORD> candidates;
				for (const auto& player : Players())
					if (Utf8(player.id) == app)
						candidates.insert(player.process);
				std::set<DWORD> captured;
				{
					std::lock_guard lock(mutex);
					for (const auto& loopback : loopbacks)
						captured.insert(loopback->process);
				}
				auto coveredBy = [&parents](DWORD process, const std::set<DWORD>& roots) {
					for (DWORD root : roots)
						if (root != process && InTreeOf(process, root, parents))
							return true;
					return false;
				};
				for (DWORD process : candidates)
				{
					if (captured.count(process) || coveredBy(process, captured) || coveredBy(process, candidates) ||
					    refused.count(process))
						continue;
					try
					{
						auto loopback = OpenLoopback(process, PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE);
						std::lock_guard lock(mutex);
						loopbacks.push_back(std::move(loopback));
						captured.insert(process);
					}
					catch (const winrt::hresult_error& error)
					{
						failure = "application sound: " + Describe(error) + SupportHint(error.code());
					}
					catch (const std::exception& error)
					{
						failure = error.what();
					}
					if (!failure.empty() && !captured.count(process))
					{
						// Not asked again every second.
						RTC_LOG(LS_WARNING) << "application sound of process " << process << ": " << failure;
						refused.insert(process);
					}
				}
				return failure;
			}

			void Read()
			{
				auto lastScan = std::chrono::steady_clock::now();
				std::vector<float> samples;
				while (running)
				{
					std::vector<HANDLE> handles{ stop.handle };
					{
						std::lock_guard lock(mutex);
						for (const auto& loopback : loopbacks)
							if (handles.size() < MAXIMUM_WAIT_OBJECTS)
								handles.push_back(loopback->ready.handle);
					}
					// Also wakes without an event: a client that does not
					// signal while nothing plays is read all the same.
					const DWORD woke =
					  WaitForMultipleObjects(static_cast<DWORD>(handles.size()), handles.data(), FALSE, 20);
					if (woke == WAIT_OBJECT_0)
						return;
					{
						std::lock_guard lock(mutex);
						for (auto& loopback : loopbacks)
							Drain(*loopback, samples);
						// Gone: the process ended, or Windows took the client
						// away. What it left plays out.
						for (auto it = loopbacks.begin(); it != loopbacks.end();)
						{
							auto& loopback = **it;
							const bool exited =
							  loopback.alive && WaitForSingleObject(loopback.alive, 0) == WAIT_OBJECT_0;
							if (!exited && !loopback.failed)
							{
								++it;
								continue;
							}
							endedUnderruns += loopback.input.underruns;
							endedOverruns += loopback.input.overruns;
							loopback.input.End();
							ending.push_back(std::move(loopback.input));
							it = loopbacks.erase(it);
						}
					}
					const auto now = std::chrono::steady_clock::now();
					if (!app.empty() && now - lastScan >= std::chrono::seconds(1))
					{
						lastScan = now;
						try
						{
							Rescan();
						}
						catch (const winrt::hresult_error& error)
						{
							RTC_LOG(LS_WARNING) << "application sound: " << Describe(error);
						}
						catch (const std::exception& error)
						{
							RTC_LOG(LS_WARNING) << "application sound: " << error.what();
						}
					}
				}
			}

			// With the mutex held: whatever the client has, into its input.
			static void Drain(Loopback& loopback, std::vector<float>& samples)
			{
				if (loopback.failed)
					return;
				while (true)
				{
					UINT32 packet = 0;
					HRESULT hr    = loopback.capture->GetNextPacketSize(&packet);
					if (FAILED(hr))
					{
						RTC_LOG(LS_WARNING) << "application sound: a loopback client failed: " << Hex(hr);
						loopback.failed = true;
						return;
					}
					if (packet == 0)
						return;
					BYTE* data   = nullptr;
					UINT32 count = 0;
					DWORD flags  = 0;
					hr           = loopback.capture->GetBuffer(&data, &count, &flags, nullptr, nullptr);
					if (FAILED(hr))
					{
						RTC_LOG(LS_WARNING) << "application sound: a loopback client failed: " << Hex(hr);
						loopback.failed = true;
						return;
					}
					const size_t values = static_cast<size_t>(count) * kChannels;
					samples.assign(values, 0.0f);
					if (!(flags & AUDCLNT_BUFFERFLAGS_SILENT) && data)
					{
						const auto* pcm = reinterpret_cast<const int16_t*>(data);
						for (size_t i = 0; i < values; ++i)
							samples[i] = static_cast<float>(pcm[i]) / 32768.0f;
					}
					loopback.capture->ReleaseBuffer(count);
					if (values > 0)
						loopback.input.Push(samples.data(), values);
				}
			}

			// A block from each playing client, summed, about every 10 ms, as
			// on Linux (app_audio_pipewire.cc).
			void Mix()
			{
				constexpr size_t block = kFrames * kChannels;
				std::vector<float> mix(block);
				std::vector<int16_t> pcm(block);
				auto next = std::chrono::steady_clock::now();
				while (running)
				{
					std::fill(mix.begin(), mix.end(), 0.0f);
					std::optional<double> lead;
					{
						std::lock_guard lock(mutex);
						for (auto& loopback : loopbacks)
							if (const auto ahead = loopback->input.Mix(mix.data()))
								lead = lead ? std::min(*lead, *ahead) : *ahead;
						for (auto& input : ending)
							input.Mix(mix.data());
						std::erase_if(ending, [](const MixInput& input) { return input.Done(); });
						++delivered;
					}
					for (size_t i = 0; i < block; ++i)
					{
						const float v = std::clamp(mix[i], -1.0f, 1.0f);
						pcm[i]        = static_cast<int16_t>(std::lrint(v * 32767.0f));
					}
					sink(pcm.data());

					next += MixInterval(lead);
					const auto now = std::chrono::steady_clock::now();
					if (next + std::chrono::milliseconds(50) < now)
						next = now;
					std::this_thread::sleep_until(next);
				}
			}

			const std::string app;
			const Sink sink;
			Event stop;
			// Processes whose capture Windows refused (capture thread).
			std::set<DWORD> refused;
			mutable std::mutex mutex;
			std::vector<std::unique_ptr<Loopback>> loopbacks;
			// What clients that are gone left for the mix (mutex), and what
			// they counted.
			std::vector<MixInput> ending;
			uint64_t endedUnderruns{ 0 };
			uint64_t endedOverruns{ 0 };
			uint64_t delivered{ 0 };
			std::atomic<bool> running{ false };
			std::thread capturer;
			std::thread mixer;
		};
	} // namespace

	std::vector<AudioApp> ListAudioApps()
	{
		return InMta([] {
			struct Entry
			{
				std::wstring name;
				int streams{ 0 };
			};
			std::map<std::wstring, Entry> byId;
			std::map<std::wstring, std::wstring> paths;
			for (auto& player : Players())
			{
				auto& entry = byId[player.id];
				++entry.streams;
				if (entry.name.empty())
					entry.name = player.name;
				paths.emplace(player.id, player.path);
			}
			std::vector<AudioApp> out;
			for (auto& [id, entry] : byId)
			{
				std::wstring name = entry.name;
				if (name.empty())
					name = Description(paths[id]);
				if (name.empty())
				{
					name            = FileName(paths[id]);
					const auto dot  = name.find_last_of(L'.');
					if (dot != std::wstring::npos && dot > 0)
						name.resize(dot);
				}
				AudioApp app;
				app.id      = Utf8(id);
				app.name    = Utf8(name);
				app.streams = entry.streams;
				out.push_back(std::move(app));
			}
			// No two entries under one name: where several are called the
			// same, each says which it is.
			std::map<std::string, int> called;
			for (const auto& app : out)
				++called[app.name];
			for (auto& app : out)
				if (called[app.name] > 1)
					app.name += " (" + app.id + ")";
			return out;
		});
	}

	std::unique_ptr<AppAudioCapture> AppAudioCapture::Start(const std::string& app, Sink sink)
	{
		// Ids are lowercased binaries; what a page stored may not be.
		std::wstring wide(app.size(), L'\0');
		const int size = MultiByteToWideChar(
		  CP_UTF8, 0, app.data(), static_cast<int>(app.size()), wide.data(), static_cast<int>(wide.size()));
		wide.resize(static_cast<size_t>(std::max(size, 0)));
		auto capture = std::make_unique<WasapiAppAudio>(Utf8(Lower(wide)), std::move(sink));
		capture->Start();
		return capture;
	}
} // namespace gelabber

// Screen capture on Windows: the system's own picker
// (Windows.Graphics.Capture.GraphicsCapturePicker) chooses a screen or a
// window, Windows.Graphics.Capture delivers its frames as Direct3D 11
// textures, and this source copies them to the CPU and converts them to I420.
//
// Consent: the app's window loads the server's page, so what the page can ask
// for, server-side XSS can ask for too. Nothing is captured before the user
// has picked something in the system picker; there is no list of screens or
// windows the page could choose from. The one way around the picker is
// GELABBER_TEST_SCREEN_MONITOR=1 in the environment of the process (the
// core's tests in CI), which captures the primary monitor; a page cannot set
// it.
//
// Threads: one thread per source does everything, picker included. It is a
// single-threaded apartment (STA) and pumps its window messages whenever it
// waits. GraphicsCapturePicker is agile and marked for both apartments, and
// its UI runs outside this process; a desktop app has to give it an owner
// window (IInitializeWithWindow) and documents calling it from that window's
// UI thread. The owner here is the app's own top-level window (the Tauri
// window, which belongs to the UI thread of the Rust side), found by the
// core: no handle crosses the C ABI, and so none can come from the page. The
// picker is called from this STA thread rather than from that UI thread:
// the core has no way onto the Tauri event loop, and an STA with its own
// message loop is what the picker gets on a UI thread as well, so the
// completion and any calls the picker makes back into this apartment are
// delivered while the thread waits. The frame pool is free-threaded
// (CreateFreeThreaded) and polled on the same thread at the requested rate,
// so no DispatcherQueue is needed.
//
// States as on Linux: pending (picker open), live (frames flow), cancelled
// (the user closed the picker: no error), ended (the captured window was
// closed or the monitor went away: the item's Closed event), failed (with
// "error").

#include "local_video_source.h"

#include <api/make_ref_counted.h>
#include <api/video/i420_buffer.h>
#include <api/video/video_frame.h>
#include <rtc_base/logging.h>
#include <rtc_base/time_utils.h>

#include <libyuv/convert.h>

// Classic COM before C++/WinRT's base.h: that turns on its interop with
// ::IUnknown (com_ptr, as<> to COM interfaces, guid_of for them).
#include <windows.h>
#include <unknwn.h>
#include <inspectable.h>
#include <d3d11.h>
#include <dxgi.h>
#include <shobjidl_core.h>
#include <timeapi.h>
#include <windows.graphics.capture.interop.h>
#include <windows.graphics.directx.direct3d11.interop.h>

#include <winrt/base.h>
#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.Graphics.h>
#include <winrt/Windows.Graphics.Capture.h>
#include <winrt/Windows.Graphics.DirectX.h>
#include <winrt/Windows.Graphics.DirectX.Direct3D11.h>

#include <algorithm>
#include <atomic>
#include <chrono>
#include <cstdio>
#include <future>
#include <memory>
#include <mutex>
#include <optional>
#include <stdexcept>
#include <string>
#include <thread>

namespace gelabber
{
	namespace
	{
		namespace wf   = winrt::Windows::Foundation;
		namespace wg   = winrt::Windows::Graphics;
		namespace wgc  = winrt::Windows::Graphics::Capture;
		namespace wgd  = winrt::Windows::Graphics::DirectX;
		namespace wgd3 = winrt::Windows::Graphics::DirectX::Direct3D11;

		// BGRA, 4 bytes a pixel: libyuv's "ARGB".
		constexpr auto kPixelFormat = wgd::DirectXPixelFormat::B8G8R8A8UIntNormalized;
		constexpr int32_t kBuffers  = 2;

		// The test hook in the header comment: the variable set to exactly "1".
		bool TestMonitorRequested()
		{
			wchar_t value[4]{};
			const DWORD length = GetEnvironmentVariableW(L"GELABBER_TEST_SCREEN_MONITOR", value, 4);
			return length == 1 && value[0] == L'1';
		}

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

		// A visible top-level window of this process without an owner.
		bool IsOwnTopLevel(HWND window)
		{
			DWORD process = 0;
			GetWindowThreadProcessId(window, &process);
			return process == GetCurrentProcessId() && IsWindowVisible(window) && !GetWindow(window, GW_OWNER);
		}

		BOOL CALLBACK FindOwnTopLevel(HWND window, LPARAM found)
		{
			if (!IsOwnTopLevel(window))
				return TRUE;
			*reinterpret_cast<HWND*>(found) = window;
			return FALSE;
		}

		// The window the picker opens over: the app's own, preferring the one
		// in the foreground, where the user has just clicked. The webview's
		// windows are children of the app's window, not top-level.
		HWND OwnerWindow()
		{
			if (HWND foreground = GetForegroundWindow())
			{
				HWND root = GetAncestor(foreground, GA_ROOTOWNER);
				if (root && IsOwnTopLevel(root))
					return root;
			}
			HWND found = nullptr;
			EnumWindows(FindOwnTopLevel, reinterpret_cast<LPARAM>(&found));
			return found;
		}

		wgc::GraphicsCaptureItem PrimaryMonitorItem()
		{
			HMONITOR monitor = MonitorFromPoint(POINT{ 0, 0 }, MONITOR_DEFAULTTOPRIMARY);
			auto interop =
			  winrt::get_activation_factory<wgc::GraphicsCaptureItem, ::IGraphicsCaptureItemInterop>();
			wgc::GraphicsCaptureItem item{ nullptr };
			winrt::check_hresult(interop->CreateForMonitor(
			  monitor, winrt::guid_of<wgc::GraphicsCaptureItem>(), winrt::put_abi(item)));
			return item;
		}

		winrt::com_ptr<ID3D11Device> CreateDevice()
		{
			winrt::com_ptr<ID3D11Device> device;
			// BGRA support is what Direct3D/WinRT interop requires. WARP
			// where there is no hardware device (a VM, a CI runner).
			HRESULT result = E_FAIL;
			for (auto type : { D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP })
			{
				result = D3D11CreateDevice(
				  nullptr,
				  type,
				  nullptr,
				  D3D11_CREATE_DEVICE_BGRA_SUPPORT,
				  nullptr,
				  0,
				  D3D11_SDK_VERSION,
				  device.put(),
				  nullptr,
				  nullptr);
				if (SUCCEEDED(result))
					return device;
			}
			winrt::throw_hresult(result);
		}

		wgd3::IDirect3DDevice WinRtDevice(const winrt::com_ptr<ID3D11Device>& device)
		{
			auto dxgi = device.as<IDXGIDevice>();
			winrt::com_ptr<::IInspectable> inspectable;
			winrt::check_hresult(CreateDirect3D11DeviceFromDXGIDevice(dxgi.get(), inspectable.put()));
			return inspectable.as<wgd3::IDirect3DDevice>();
		}

		winrt::com_ptr<ID3D11Texture2D> Texture(const wgd3::IDirect3DSurface& surface)
		{
			auto access = surface.as<::Windows::Graphics::DirectX::Direct3D11::IDirect3DDxgiInterfaceAccess>();
			winrt::com_ptr<ID3D11Texture2D> texture;
			winrt::check_hresult(access->GetInterface(winrt::guid_of<ID3D11Texture2D>(), texture.put_void()));
			return texture;
		}

		// A Win32 event that outlives whoever waits for it: the picker's
		// completion may come after the source stopped waiting.
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

		class WgcScreenSource : public LocalVideoSource
		{
		public:
			explicit WgcScreenSource(const ScreenOptions& options) : options(options)
			{
				if (!stop.handle)
					throw std::runtime_error("screen capture: CreateEvent failed");
			}

			~WgcScreenSource() override
			{
				Stop();
			}

			// Returns once the capture thread has found that this Windows can
			// capture and the picker has a window to open over; the picker
			// itself opens afterwards. Throws when either is missing.
			void Start()
			{
				std::promise<std::string> ready;
				auto started = ready.get_future();
				worker       = std::thread([this, ready = std::move(ready)]() mutable { Run(ready); });
				// Run() answers on every path, failures included.
				const std::string error = started.get();
				if (!error.empty())
				{
					worker.join();
					throw std::runtime_error(error);
				}
			}

			void Stop() override
			{
				SetEvent(stop.handle);
				if (worker.joinable())
					worker.join();
			}

			std::string StateJson() const override
			{
				std::lock_guard lock(mutex);
				std::string out = R"({"state":")" + current + R"(")";
				if (width > 0)
					out += R"(,"width":)" + std::to_string(width) + R"(,"height":)" + std::to_string(height);
				out += R"(,"frames":)" + std::to_string(frames);
				if (!error.empty())
				{
					// Messages from Windows; no quotes or backslashes of
					// their own are expected, but they must not break the JSON.
					std::string escaped;
					for (char c : error)
					{
						if (c == '"' || c == '\\')
							escaped += '\\';
						if (static_cast<unsigned char>(c) >= 0x20)
							escaped += c;
					}
					out += R"(,"error":")" + escaped + R"(")";
				}
				return out + "}";
			}

			bool is_screencast() const override
			{
				return true;
			}

		private:
			enum class Waited
			{
				Signalled,
				Stopped,
				TimedOut,
			};

			void Run(std::promise<std::string>& ready)
			{
				bool started = false;
				std::string failure;
				try
				{
					winrt::init_apartment(winrt::apartment_type::single_threaded);
				}
				catch (const winrt::hresult_error& error)
				{
					ready.set_value("screen capture: no COM apartment: " + Describe(error));
					return;
				}
				// Every WinRT object lives inside Capture() and is gone before
				// the apartment is.
				try
				{
					Capture(ready, started);
				}
				catch (const winrt::hresult_error& error)
				{
					failure = Describe(error);
				}
				catch (const std::exception& error)
				{
					failure = error.what();
				}
				catch (...)
				{
					failure = "screen capture: unknown error";
				}
				staging = nullptr;
				if (!failure.empty())
				{
					if (!started)
						ready.set_value(failure);
					else
					{
						RTC_LOG(LS_ERROR) << "screen capture failed: " << failure;
						Fail(failure);
					}
				}
				winrt::uninit_apartment();
			}

			void Capture(std::promise<std::string>& ready, bool& started)
			{
				bool supported = false;
				try
				{
					// Windows 10 1903 on; before that the method itself is missing.
					supported = wgc::GraphicsCaptureSession::IsSupported();
				}
				catch (const winrt::hresult_error& error)
				{
					RTC_LOG(LS_WARNING) << "GraphicsCaptureSession::IsSupported: " << Describe(error);
				}
				if (!supported)
					throw std::runtime_error(
					  "screen capture is not supported on this Windows: Windows.Graphics.Capture "
					  "needs Windows 10 version 1903 or later");

				const bool testMonitor = TestMonitorRequested();
				HWND owner             = nullptr;
				if (!testMonitor)
				{
					owner = OwnerWindow();
					if (!owner)
						throw std::runtime_error("screen capture needs a visible window of the app for the picker");
				}
				started = true;
				ready.set_value(std::string());

				wgc::GraphicsCaptureItem item{ nullptr };
				if (testMonitor)
				{
					RTC_LOG(LS_WARNING) << "GELABBER_TEST_SCREEN_MONITOR=1: capturing the primary monitor without the picker";
					item = PrimaryMonitorItem();
				}
				else
				{
					item = Pick(owner);
					if (!item)
						return;
				}
				Stream(item);
			}

			// The system picker over `owner`. Null when the user closed it
			// (state cancelled) or the source was stopped meanwhile.
			wgc::GraphicsCaptureItem Pick(HWND owner)
			{
				wgc::GraphicsCapturePicker picker;
				winrt::check_hresult(picker.as<::IInitializeWithWindow>()->Initialize(owner));
				auto operation = picker.PickSingleItemAsync();
				auto done      = std::make_shared<Event>();
				if (!done->handle)
					throw std::runtime_error("screen capture: CreateEvent failed");
				operation.Completed([done](auto&&, auto&&) { SetEvent(done->handle); });
				if (Wait(done->handle, std::nullopt) == Waited::Stopped)
				{
					try
					{
						operation.Cancel();
					}
					catch (const winrt::hresult_error& error)
					{
						RTC_LOG(LS_WARNING) << "closing the capture picker: " << Describe(error);
					}
					return nullptr;
				}
				switch (operation.Status())
				{
					case wf::AsyncStatus::Completed:
					{
						auto item = operation.GetResults();
						if (!item)
							SetState("cancelled");
						return item;
					}
					case wf::AsyncStatus::Canceled:
						SetState("cancelled");
						return nullptr;
					default:
						winrt::throw_hresult(operation.ErrorCode());
				}
			}

			void Stream(const wgc::GraphicsCaptureItem& item)
			{
				auto device               = CreateDevice();
				auto winrtDevice          = WinRtDevice(device);
				const wg::SizeInt32 size  = item.Size();
				poolSize                  = { std::max(size.Width, 1), std::max(size.Height, 1) };
				auto pool =
				  wgc::Direct3D11CaptureFramePool::CreateFreeThreaded(winrtDevice, kPixelFormat, kBuffers, poolSize);
				auto session = pool.CreateCaptureSession(item);
				try
				{
					// Windows 10 2004 on (GraphicsCaptureSession2); before it the
					// cursor is always in the picture.
					session.IsCursorCaptureEnabled(options.cursor);
				}
				catch (const winrt::hresult_error& error)
				{
					RTC_LOG(LS_INFO) << "IsCursorCaptureEnabled: " << Describe(error);
				}
				// IsBorderRequired stays as Windows has it: the yellow frame
				// says that something is being captured.
				// The window was closed, the monitor disconnected. The handler
				// runs on whichever thread Windows raises the event on.
				auto closed = std::make_shared<std::atomic<bool>>(false);
				const winrt::event_token token =
				  item.Closed([closed](auto&&, auto&&) { closed->store(true); });
				// The waits below are what paces the frames; Windows' default
				// timer granularity (15.6 ms) would turn 60 fps into 32. Per
				// process since Windows 10 2004.
				timeBeginPeriod(1);
				struct Cleanup
				{
					const wgc::GraphicsCaptureItem& item;
					const winrt::event_token token;
					const wgc::GraphicsCaptureSession& session;
					const wgc::Direct3D11CaptureFramePool& pool;
					~Cleanup()
					{
						timeEndPeriod(1);
						try
						{
							item.Closed(token);
							session.Close();
							pool.Close();
						}
						catch (const winrt::hresult_error& error)
						{
							RTC_LOG(LS_WARNING) << "closing the screen capture: " << Describe(error);
						}
					}
				} cleanup{ item, token, session, pool };

				session.StartCapture();
				SetState("live");

				const auto interval = std::chrono::microseconds(1'000'000 / std::max(1, options.fps));
				auto next           = std::chrono::steady_clock::now();
				webrtc::scoped_refptr<webrtc::I420Buffer> latest;
				while (!closed->load())
				{
					if (auto captured = Grab(device, winrtDevice, pool))
						latest = captured;
					// Windows sends a frame only when the picture changed; the
					// encoders get the last one again in between, as from
					// PipeWire on Linux.
					if (latest)
						Deliver(latest);
					next += interval;
					const auto now = std::chrono::steady_clock::now();
					if (next < now)
						next = now;
					if (Wait(nullptr, next) == Waited::Stopped)
						return;
				}
				RTC_LOG(LS_INFO) << "the captured screen or window is gone";
				SetState("ended");
			}

			// The newest frame in the pool as I420, or null when none came
			// since the last call.
			webrtc::scoped_refptr<webrtc::I420Buffer> Grab(
			  const winrt::com_ptr<ID3D11Device>& device,
			  const wgd3::IDirect3DDevice& winrtDevice,
			  const wgc::Direct3D11CaptureFramePool& pool)
			{
				wgc::Direct3D11CaptureFrame frame{ nullptr };
				for (auto next = pool.TryGetNextFrame(); next; next = pool.TryGetNextFrame())
				{
					if (frame)
						frame.Close();
					frame = next;
				}
				if (!frame)
					return nullptr;

				const wg::SizeInt32 content = frame.ContentSize();
				auto texture                = Texture(frame.Surface());
				D3D11_TEXTURE2D_DESC desc{};
				texture->GetDesc(&desc);
				// A window that grew is larger than the pool's buffers until
				// the pool is recreated below; one that shrank leaves stale
				// pixels beyond its content.
				const int frameWidth  = std::min(content.Width, static_cast<int32_t>(desc.Width));
				const int frameHeight = std::min(content.Height, static_cast<int32_t>(desc.Height));

				webrtc::scoped_refptr<webrtc::I420Buffer> i420;
				if (frameWidth >= 2 && frameHeight >= 2)
				{
					if (!staging || stagingDesc.Width != desc.Width || stagingDesc.Height != desc.Height ||
					    stagingDesc.Format != desc.Format)
					{
						D3D11_TEXTURE2D_DESC wanted{};
						wanted.Width          = desc.Width;
						wanted.Height         = desc.Height;
						wanted.MipLevels      = 1;
						wanted.ArraySize      = 1;
						wanted.Format         = desc.Format;
						wanted.SampleDesc     = { 1, 0 };
						wanted.Usage          = D3D11_USAGE_STAGING;
						wanted.BindFlags      = 0;
						wanted.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
						wanted.MiscFlags      = 0;
						staging               = nullptr;
						winrt::check_hresult(device->CreateTexture2D(&wanted, nullptr, staging.put()));
						stagingDesc = wanted;
					}
					winrt::com_ptr<ID3D11DeviceContext> context;
					device->GetImmediateContext(context.put());
					D3D11_BOX box{};
					box.left   = 0;
					box.top    = 0;
					box.front  = 0;
					box.right  = static_cast<UINT>(frameWidth);
					box.bottom = static_cast<UINT>(frameHeight);
					box.back   = 1;
					context->CopySubresourceRegion(staging.get(), 0, 0, 0, 0, texture.get(), 0, &box);
					D3D11_MAPPED_SUBRESOURCE mapped{};
					winrt::check_hresult(context->Map(staging.get(), 0, D3D11_MAP_READ, 0, &mapped));
					i420 = webrtc::I420Buffer::Create(frameWidth, frameHeight);
					libyuv::ARGBToI420(
					  static_cast<const uint8_t*>(mapped.pData),
					  static_cast<int>(mapped.RowPitch),
					  i420->MutableDataY(),
					  i420->StrideY(),
					  i420->MutableDataU(),
					  i420->StrideU(),
					  i420->MutableDataV(),
					  i420->StrideV(),
					  frameWidth,
					  frameHeight);
					context->Unmap(staging.get(), 0);
				}
				frame.Close();

				if (content.Width != poolSize.Width || content.Height != poolSize.Height)
				{
					poolSize = { std::max(content.Width, 1), std::max(content.Height, 1) };
					pool.Recreate(winrtDevice, kPixelFormat, kBuffers, poolSize);
				}
				return i420;
			}

			void Deliver(const webrtc::scoped_refptr<webrtc::I420Buffer>& i420)
			{
				const int srcWidth  = i420->width();
				const int srcHeight = i420->height();
				{
					// Counts frames handed on, also while nothing consumes the
					// track yet (AdaptFrame then drops them), as on Linux.
					std::lock_guard lock(mutex);
					width  = srcWidth;
					height = srcHeight;
					++frames;
				}
				const int64_t nowUs = webrtc::TimeMicros();
				int outWidth, outHeight, cropWidth, cropHeight, cropX, cropY;
				if (!AdaptFrame(
				      srcWidth,
				      srcHeight,
				      nowUs,
				      &outWidth,
				      &outHeight,
				      &cropWidth,
				      &cropHeight,
				      &cropX,
				      &cropY))
					return;
				webrtc::scoped_refptr<webrtc::VideoFrameBuffer> buffer = i420;
				if (outWidth != srcWidth || outHeight != srcHeight)
				{
					auto scaled = webrtc::I420Buffer::Create(outWidth, outHeight);
					scaled->CropAndScaleFrom(*i420, cropX, cropY, cropWidth, cropHeight);
					buffer = scaled;
				}
				OnFrame(webrtc::VideoFrame::Builder()
				          .set_video_frame_buffer(buffer)
				          .set_timestamp_us(nowUs)
				          .set_rotation(webrtc::kVideoRotation_0)
				          .build());
			}

			// Waits for `event` (may be null), until `deadline` (none: no
			// limit) or until Stop(), handling the thread's window messages
			// meanwhile: this thread is an STA.
			Waited Wait(HANDLE event, std::optional<std::chrono::steady_clock::time_point> deadline)
			{
				HANDLE handles[2] = { stop.handle, event };
				const DWORD count = event ? 2 : 1;
				while (true)
				{
					DWORD timeout = INFINITE;
					if (deadline)
					{
						const auto left = std::chrono::ceil<std::chrono::milliseconds>(
						  *deadline - std::chrono::steady_clock::now());
						timeout = left.count() > 0 ? static_cast<DWORD>(left.count()) : 0;
					}
					const DWORD woke =
					  MsgWaitForMultipleObjectsEx(count, handles, timeout, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
					if (woke == WAIT_OBJECT_0)
						return Waited::Stopped;
					if (count == 2 && woke == WAIT_OBJECT_0 + 1)
						return Waited::Signalled;
					if (woke == WAIT_OBJECT_0 + count)
					{
						MSG message;
						while (PeekMessageW(&message, nullptr, 0, 0, PM_REMOVE))
						{
							TranslateMessage(&message);
							DispatchMessageW(&message);
						}
						if (deadline && std::chrono::steady_clock::now() >= *deadline)
							return Waited::TimedOut;
						continue;
					}
					if (woke == WAIT_TIMEOUT)
						return Waited::TimedOut;
					winrt::throw_last_error();
				}
			}

			void SetState(const std::string& next)
			{
				std::lock_guard lock(mutex);
				// Terminal states stay.
				if (current == "pending" || current == "live")
					current = next;
			}

			void Fail(const std::string& message)
			{
				std::lock_guard lock(mutex);
				if (current == "pending" || current == "live")
				{
					current = "failed";
					error   = message;
				}
			}

			const ScreenOptions options;
			Event stop;
			std::thread worker;
			// The capture thread's own.
			wg::SizeInt32 poolSize{};
			winrt::com_ptr<ID3D11Texture2D> staging;
			D3D11_TEXTURE2D_DESC stagingDesc{};

			mutable std::mutex mutex;
			std::string current{ "pending" };
			std::string error;
			int width{ 0 };
			int height{ 0 };
			uint64_t frames{ 0 };
		};
	} // namespace

	webrtc::scoped_refptr<LocalVideoSource> CreateScreenSource(const ScreenOptions& options)
	{
		// options.type does not narrow the system picker: it always offers
		// screens and windows.
		auto source = webrtc::make_ref_counted<WgcScreenSource>(options);
		source->Start();
		return source;
	}
} // namespace gelabber

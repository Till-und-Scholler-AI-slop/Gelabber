// Application sound on Linux through PipeWire: one capture stream per
// playback stream of the chosen applications (pw-record --target style),
// mixed to 48 kHz stereo in 10 ms blocks. WirePlumber links each capture
// stream to its target; the applications keep playing to their own output.
//
// libpipewire is loaded at runtime like libwebrtc's own PipeWire use, so the
// core has no link-time dependency on it.

#include "app_audio.h"

#include <dlfcn.h>
#include <pipewire/pipewire.h>
#include <spa/param/audio/format-utils.h>
#include <spa/pod/builder.h>
#include <spa/utils/dict.h>
#include <unistd.h>

#include <algorithm>
#include <atomic>
#include <chrono>
#include <cmath>
#include <condition_variable>
#include <cstring>
#include <deque>
#include <map>
#include <mutex>
#include <stdexcept>
#include <thread>

namespace gelabber
{
	namespace
	{
		// The libpipewire functions in use (inline header helpers aside).
		struct PipeWireApi
		{
			decltype(&pw_init) init{};
			decltype(&pw_thread_loop_new) thread_loop_new{};
			decltype(&pw_thread_loop_destroy) thread_loop_destroy{};
			decltype(&pw_thread_loop_start) thread_loop_start{};
			decltype(&pw_thread_loop_stop) thread_loop_stop{};
			decltype(&pw_thread_loop_lock) thread_loop_lock{};
			decltype(&pw_thread_loop_unlock) thread_loop_unlock{};
			decltype(&pw_thread_loop_signal) thread_loop_signal{};
			decltype(&pw_thread_loop_timed_wait) thread_loop_timed_wait{};
			decltype(&pw_thread_loop_get_loop) thread_loop_get_loop{};
			decltype(&pw_context_new) context_new{};
			decltype(&pw_context_destroy) context_destroy{};
			decltype(&pw_context_connect) context_connect{};
			decltype(&pw_core_disconnect) core_disconnect{};
			decltype(&pw_proxy_destroy) proxy_destroy{};
			decltype(&pw_properties_new) properties_new{};
			decltype(&pw_stream_new) stream_new{};
			decltype(&pw_stream_destroy) stream_destroy{};
			decltype(&pw_stream_add_listener) stream_add_listener{};
			decltype(&pw_stream_connect) stream_connect{};
			decltype(&pw_stream_dequeue_buffer) stream_dequeue_buffer{};
			decltype(&pw_stream_queue_buffer) stream_queue_buffer{};
			bool loaded{ false };
		};

		const PipeWireApi& Api()
		{
			static const PipeWireApi api = [] {
				PipeWireApi a;
				void* lib = dlopen("libpipewire-0.3.so.0", RTLD_NOW | RTLD_LOCAL);
				if (!lib)
					return a;
#define GM_PW_LOAD(name)                                                                  \
	a.name = reinterpret_cast<decltype(a.name)>(dlsym(lib, "pw_" #name));                 \
	if (!a.name)                                                                          \
		return a;
				GM_PW_LOAD(init)
				GM_PW_LOAD(thread_loop_new)
				GM_PW_LOAD(thread_loop_destroy)
				GM_PW_LOAD(thread_loop_start)
				GM_PW_LOAD(thread_loop_stop)
				GM_PW_LOAD(thread_loop_lock)
				GM_PW_LOAD(thread_loop_unlock)
				GM_PW_LOAD(thread_loop_signal)
				GM_PW_LOAD(thread_loop_timed_wait)
				GM_PW_LOAD(thread_loop_get_loop)
				GM_PW_LOAD(context_new)
				GM_PW_LOAD(context_destroy)
				GM_PW_LOAD(context_connect)
				GM_PW_LOAD(core_disconnect)
				GM_PW_LOAD(proxy_destroy)
				GM_PW_LOAD(properties_new)
				GM_PW_LOAD(stream_new)
				GM_PW_LOAD(stream_destroy)
				GM_PW_LOAD(stream_add_listener)
				GM_PW_LOAD(stream_connect)
				GM_PW_LOAD(stream_dequeue_buffer)
				GM_PW_LOAD(stream_queue_buffer)
#undef GM_PW_LOAD
				a.init(nullptr, nullptr);
				a.loaded = true;
				return a;
			}();
			if (!api.loaded)
				throw std::runtime_error("application sound needs PipeWire (libpipewire-0.3)");
			return api;
		}

		std::string Lookup(const spa_dict* props, const char* key)
		{
			const char* value = props ? spa_dict_lookup(props, key) : nullptr;
			return value ? value : "";
		}

		struct PlaybackStream
		{
			uint32_t node{ 0 };
			std::string serial;
			AudioApp app;
		};

		// A playback stream of another process, or nothing.
		bool ReadPlaybackStream(uint32_t id, const char* type, const spa_dict* props, PlaybackStream& out)
		{
			if (std::strcmp(type, PW_TYPE_INTERFACE_Node) != 0 ||
			    Lookup(props, PW_KEY_MEDIA_CLASS) != "Stream/Output/Audio")
				return false;
			if (Lookup(props, PW_KEY_APP_PROCESS_ID) == std::to_string(getpid()))
				return false;
			out.node      = id;
			out.serial    = Lookup(props, "object.serial");
			out.app.name  = Lookup(props, PW_KEY_APP_NAME);
			const auto binary = Lookup(props, PW_KEY_APP_PROCESS_BINARY);
			out.app.id    = !binary.empty() ? binary : out.app.name;
			if (out.app.id.empty())
				out.app.id = Lookup(props, PW_KEY_NODE_NAME);
			if (out.app.name.empty())
				out.app.name = out.app.id;
			return !out.app.id.empty();
		}

		// A thread loop with a connected core and a registry.
		class Connection
		{
		public:
			explicit Connection(const char* name) : api(Api())
			{
				loop = api.thread_loop_new(name, nullptr);
				if (!loop)
					throw std::runtime_error("PipeWire thread loop");
				context = api.context_new(api.thread_loop_get_loop(loop), nullptr, 0);
				if (!context || api.thread_loop_start(loop) < 0)
				{
					Close();
					throw std::runtime_error("PipeWire context");
				}
				Lock lock(*this);
				core = api.context_connect(context, nullptr, 0);
				if (!core)
				{
					lock.Release();
					Close();
					throw std::runtime_error("cannot connect to PipeWire");
				}
			}

			// With the lock held; the events arrive on the loop thread.
			void Listen(const pw_registry_events* events, void* data)
			{
				registry = pw_core_get_registry(core, PW_VERSION_REGISTRY, 0);
				spa_zero(registryListener);
				pw_registry_add_listener(registry, &registryListener, events, data);
			}

			~Connection()
			{
				Close();
			}

			// Holds the loop's lock: PipeWire objects are used under it.
			class Lock
			{
			public:
				explicit Lock(Connection& connection) : connection(&connection)
				{
					connection.api.thread_loop_lock(connection.loop);
				}
				~Lock()
				{
					Release();
				}
				void Release()
				{
					if (connection)
						connection->api.thread_loop_unlock(connection->loop);
					connection = nullptr;
				}

			private:
				Connection* connection;
			};

			// Waits (lock held) until the server processed everything sent so far.
			bool Sync(int timeoutSeconds)
			{
				static const pw_core_events events = [] {
					pw_core_events e{};
					e.version = PW_VERSION_CORE_EVENTS;
					e.done    = [](void* data, uint32_t id, int seq) {
						auto* self = static_cast<Connection*>(data);
						if (id == PW_ID_CORE && seq == self->pending)
						{
							self->synced = true;
							self->api.thread_loop_signal(self->loop, false);
						}
					};
					return e;
				}();
				spa_hook listener;
				spa_zero(listener);
				pw_core_add_listener(core, &listener, &events, this);
				synced  = false;
				pending = pw_core_sync(core, PW_ID_CORE, 0);
				while (!synced)
					if (api.thread_loop_timed_wait(loop, timeoutSeconds) != 0)
						break;
				spa_hook_remove(&listener);
				return synced;
			}

			const PipeWireApi& api;
			pw_thread_loop* loop{ nullptr };
			pw_core* core{ nullptr };

		private:
			void Close()
			{
				if (loop)
				{
					api.thread_loop_lock(loop);
					if (registry)
					{
						spa_hook_remove(&registryListener);
						api.proxy_destroy(reinterpret_cast<pw_proxy*>(registry));
						registry = nullptr;
					}
					if (core)
						api.core_disconnect(core);
					core = nullptr;
					api.thread_loop_unlock(loop);
					api.thread_loop_stop(loop);
				}
				if (context)
					api.context_destroy(context);
				context = nullptr;
				if (loop)
					api.thread_loop_destroy(loop);
				loop = nullptr;
			}

			pw_context* context{ nullptr };
			pw_registry* registry{ nullptr };
			spa_hook registryListener{};
			int pending{ 0 };
			bool synced{ false };
		};

		class PipeWireAppAudio : public AppAudioCapture
		{
		public:
			PipeWireAppAudio(std::string app, Sink sink) : app(std::move(app)), sink(std::move(sink))
			{
			}

			void Start()
			{
				static const pw_registry_events events = [] {
					pw_registry_events e{};
					e.version = PW_VERSION_REGISTRY_EVENTS;
					e.global  = [](void* data,
					              uint32_t id,
					              uint32_t /*permissions*/,
					              const char* type,
					              uint32_t /*version*/,
					              const spa_dict* props) {
						static_cast<PipeWireAppAudio*>(data)->OnGlobal(id, type, props);
					};
					e.global_remove = [](void* data, uint32_t id) {
						static_cast<PipeWireAppAudio*>(data)->OnGlobalRemove(id);
					};
					return e;
				}();
				connection = std::make_unique<Connection>("gelabber-app-audio");
				{
					Connection::Lock lock(*connection);
					connection->Listen(&events, this);
				}
				running = true;
				mixer      = std::thread([this] { Mix(); });
			}

			~PipeWireAppAudio() override
			{
				running = false;
				if (mixer.joinable())
					mixer.join();
				if (connection)
				{
					{
						Connection::Lock lock(*connection);
						for (auto& [node, capture] : captures)
							DestroyStream(*capture);
					}
					connection.reset();
				}
			}

			std::string StateJson() const override
			{
				std::lock_guard lock(mutex);
				return R"({"state":"live","streams":)" + std::to_string(captures.size()) + R"(,"frames":)" +
				       std::to_string(delivered) + "}";
			}

		private:
			struct Capture
			{
				PipeWireAppAudio* owner{ nullptr };
				uint32_t node{ 0 };
				pw_stream* stream{ nullptr };
				spa_hook listener{};
				// Interleaved stereo samples waiting for the mixer (mutex).
				std::deque<float> samples;
				bool primed{ false };
			};

			// Registry events run on the loop thread with its lock held.
			void OnGlobal(uint32_t id, const char* type, const spa_dict* props)
			{
				PlaybackStream playback;
				if (!ReadPlaybackStream(id, type, props, playback))
					return;
				if (!app.empty() && playback.app.id != app)
					return;
				auto capture   = std::make_unique<Capture>();
				capture->owner = this;
				capture->node  = id;
				const auto target = playback.serial.empty() ? std::to_string(id) : playback.serial;
				auto* props2      = connection->api.properties_new(
                  PW_KEY_MEDIA_TYPE,
                  "Audio",
                  PW_KEY_MEDIA_CATEGORY,
                  "Capture",
                  PW_KEY_MEDIA_ROLE,
                  "Production",
                  PW_KEY_APP_NAME,
                  "Gelabber",
                  PW_KEY_NODE_NAME,
                  "gelabber-source-audio",
                  "target.object",
                  target.c_str(),
                  PW_KEY_NODE_DONT_RECONNECT,
                  "true",
                  // Capturing must not keep a paused application's output running.
                  "node.passive",
                  "true",
                  nullptr);
				capture->stream =
				  connection->api.stream_new(connection->core, "Gelabber source audio", props2);
				if (!capture->stream)
					return;
				static const pw_stream_events events = [] {
					pw_stream_events e{};
					e.version = PW_VERSION_STREAM_EVENTS;
					e.process = [](void* data) {
						auto* capture = static_cast<Capture*>(data);
						capture->owner->OnProcess(*capture);
					};
					return e;
				}();
				connection->api.stream_add_listener(capture->stream, &capture->listener, &events, capture.get());

				uint8_t buffer[1024];
				spa_pod_builder builder;
				spa_pod_builder_init(&builder, buffer, sizeof(buffer));
				spa_audio_info_raw info;
				spa_zero(info);
				info.format      = SPA_AUDIO_FORMAT_F32;
				info.rate        = kSampleRate;
				info.channels    = kChannels;
				info.position[0] = SPA_AUDIO_CHANNEL_FL;
				info.position[1] = SPA_AUDIO_CHANNEL_FR;
				const spa_pod* params[1] = { spa_format_audio_raw_build(&builder, SPA_PARAM_EnumFormat, &info) };
				const auto flags         = static_cast<pw_stream_flags>(
                  PW_STREAM_FLAG_AUTOCONNECT | PW_STREAM_FLAG_MAP_BUFFERS);
				if (connection->api.stream_connect(capture->stream, PW_DIRECTION_INPUT, PW_ID_ANY, flags, params, 1) <
				    0)
				{
					DestroyStream(*capture);
					return;
				}
				std::lock_guard lock(mutex);
				captures[id] = std::move(capture);
			}

			void OnGlobalRemove(uint32_t id)
			{
				std::unique_ptr<Capture> gone;
				{
					std::lock_guard lock(mutex);
					auto it = captures.find(id);
					if (it == captures.end())
						return;
					gone = std::move(it->second);
					captures.erase(it);
				}
				DestroyStream(*gone);
			}

			void DestroyStream(Capture& capture)
			{
				if (!capture.stream)
					return;
				spa_hook_remove(&capture.listener);
				connection->api.stream_destroy(capture.stream);
				capture.stream = nullptr;
			}

			void OnProcess(Capture& capture)
			{
				auto& api   = connection->api;
				pw_buffer* b = api.stream_dequeue_buffer(capture.stream);
				if (!b)
					return;
				const spa_data& data = b->buffer->datas[0];
				if (data.data && data.chunk)
				{
					const auto* begin = reinterpret_cast<const float*>(
					  static_cast<const uint8_t*>(data.data) + data.chunk->offset);
					const size_t count = std::min<size_t>(data.chunk->size, data.maxsize) / sizeof(float);
					std::lock_guard lock(mutex);
					capture.samples.insert(capture.samples.end(), begin, begin + count);
					// More than 100 ms behind: drop to 40 ms (clock drift).
					constexpr size_t block = kFrames * kChannels;
					if (capture.samples.size() > 10 * block)
						capture.samples.erase(
						  capture.samples.begin(),
						  capture.samples.begin() + static_cast<long>(capture.samples.size() - 4 * block));
				}
				api.stream_queue_buffer(capture.stream, b);
			}

			// Every 10 ms: one block from each stream, summed. A stream plays
			// once 20 ms are buffered, so small timing differences between
			// the sound server and this thread do not click.
			void Mix()
			{
				constexpr size_t block = kFrames * kChannels;
				std::vector<float> mix(block);
				std::vector<int16_t> pcm(block);
				auto next = std::chrono::steady_clock::now();
				while (running)
				{
					std::fill(mix.begin(), mix.end(), 0.0f);
					{
						std::lock_guard lock(mutex);
						for (auto& [node, capture] : captures)
						{
							auto& samples = capture->samples;
							if (!capture->primed && samples.size() >= 2 * block)
								capture->primed = true;
							if (!capture->primed)
								continue;
							const size_t take = std::min(block, samples.size());
							for (size_t i = 0; i < take; ++i)
								mix[i] += samples[i];
							samples.erase(samples.begin(), samples.begin() + static_cast<long>(take));
							if (take < block)
								capture->primed = false;
						}
						++delivered;
					}
					for (size_t i = 0; i < block; ++i)
					{
						const float v = std::clamp(mix[i], -1.0f, 1.0f);
						pcm[i]        = static_cast<int16_t>(std::lrint(v * 32767.0f));
					}
					sink(pcm.data());

					next += std::chrono::milliseconds(10);
					const auto now = std::chrono::steady_clock::now();
					if (next + std::chrono::milliseconds(50) < now)
						next = now;
					std::this_thread::sleep_until(next);
				}
			}

			const std::string app;
			const Sink sink;
			std::unique_ptr<Connection> connection;
			mutable std::mutex mutex;
			std::map<uint32_t, std::unique_ptr<Capture>> captures;
			uint64_t delivered{ 0 };
			std::atomic<bool> running{ false };
			std::thread mixer;
		};
	} // namespace

	std::vector<AudioApp> ListAudioApps()
	{
		std::map<std::string, AudioApp> apps;
		static const pw_registry_events events = [] {
			pw_registry_events e{};
			e.version = PW_VERSION_REGISTRY_EVENTS;
			e.global  = [](void* data,
			              uint32_t id,
			              uint32_t /*permissions*/,
			              const char* type,
			              uint32_t /*version*/,
			              const spa_dict* props) {
				PlaybackStream playback;
				if (!ReadPlaybackStream(id, type, props, playback))
					return;
				auto& apps = *static_cast<std::map<std::string, AudioApp>*>(data);
				auto& app  = apps[playback.app.id];
				app.id     = playback.app.id;
				app.name   = playback.app.name;
				++app.streams;
			};
			return e;
		}();
		Connection connection("gelabber-app-list");
		std::vector<AudioApp> out;
		Connection::Lock lock(connection);
		connection.Listen(&events, &apps);
		connection.Sync(2);
		for (auto& [id, app] : apps)
			out.push_back(app);
		return out;
	}

	std::unique_ptr<AppAudioCapture> AppAudioCapture::Start(const std::string& app, Sink sink)
	{
		auto capture = std::make_unique<PipeWireAppAudio>(app, std::move(sink));
		capture->Start();
		return capture;
	}
} // namespace gelabber

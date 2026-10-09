// Application sound on Linux through PipeWire: one capture stream per
// playback stream of the chosen applications (pw-record --target style),
// mixed to 48 kHz stereo in 10 ms blocks at the pace their sound card sets
// (app_audio_mix.h). WirePlumber links each capture stream to its target;
// the applications keep playing to their own output.
//
// Whose stream it is comes from the stream's client object: the registry
// lists a playback node with application.name and client.id, and only the
// client's info has application.process.id and application.process.binary
// (for native, ALSA plug-in and pipewire-pulse clients alike). Streams of this
// process and of the processes it started (the webview's helpers) are never
// captured: they carry the call itself.
//
// Nor is the playback stream of a virtual device: an echo canceller, a
// loopback, a filter chain (equaliser, virtual surround) or a combined sink
// plays on what was played into it, the call included when it is this
// application's output, and every application's sound a second time. Only
// the node's own info tells (node.virtual, node.link-group).
//
// libpipewire is loaded at runtime like libwebrtc's own PipeWire use, so the
// core has no link-time dependency on it.

#include "app_audio.h"
#include "app_audio_mix.h"

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
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <functional>
#include <iterator>
#include <map>
#include <mutex>
#include <optional>
#include <set>
#include <stdexcept>
#include <thread>
#include <vector>

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
			decltype(&pw_stream_get_node_id) stream_get_node_id{};
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
				GM_PW_LOAD(stream_get_node_id)
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

		pid_t ToPid(const std::string& value)
		{
			return static_cast<pid_t>(std::strtol(value.c_str(), nullptr, 10));
		}

		// Parent of a process, 0 when unknown. /proc/<pid>/stat reads
		// "pid (name) state parent ..." and the name may contain anything.
		pid_t ParentOf(pid_t pid)
		{
			std::ifstream file("/proc/" + std::to_string(pid) + "/stat");
			const std::string stat((std::istreambuf_iterator<char>(file)), std::istreambuf_iterator<char>());
			const auto name = stat.rfind(')');
			// ") S <parent>"
			if (name == std::string::npos || name + 4 >= stat.size())
				return 0;
			return ToPid(stat.substr(name + 4));
		}

		// This process or one it started.
		bool OwnProcess(pid_t pid)
		{
			const pid_t self = getpid();
			for (int depth = 0; pid > 1 && depth < 32; ++depth)
			{
				if (pid == self)
					return true;
				pid = ParentOf(pid);
			}
			return false;
		}

		// Names sound libraries give every application that uses them.
		bool GenericName(const std::string& name)
		{
			return name.empty() || name == "WEBRTC VoiceEngine" || name.rfind("PipeWire ALSA [", 0) == 0 ||
			       name.rfind("ALSA plug-in [", 0) == 0 || name.rfind("Lavf", 0) == 0;
		}

		// The playback half of a virtual device: PipeWire's loopback,
		// filter-chain, echo-cancel and combine-stream modules, which are also
		// what pipewire-pulse loads for module-loopback, -echo-cancel,
		// -combine-sink, -remap-sink and -virtual-sink. They mark both of
		// their streams virtual and put them in a link group, by which the
		// session manager keeps a device from being linked to itself.
		bool VirtualDevice(const spa_dict* props)
		{
			const auto isVirtual = Lookup(props, "node.virtual");
			return isVirtual == "true" || isVirtual == "1" || !Lookup(props, "node.link-group").empty();
		}

		struct PlaybackStream
		{
			uint32_t node{ 0 };
			std::string serial;
			AudioApp app;
			// The id up to 0.5.2 (application.name); clients may have stored it.
			std::string oldId;

			// Whether choosing `id` means this stream.
			bool ChosenBy(const std::string& id) const
			{
				return app.id == id || oldId == id;
			}

			// The application gave itself a name that chooses it and is not
			// its binary's.
			bool Named() const
			{
				return oldId != app.id && oldId == app.name;
			}
		};

		// A thread loop with a connected core.
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
			int pending{ 0 };
			bool synced{ false };
		};

		// Reports the playback streams of other applications, each once its
		// own properties and its owner are known, and their end; and nodes
		// that lost the links into them. Created and destroyed with the
		// loop's lock held; the events arrive on the loop thread.
		class PlaybackWatcher
		{
		public:
			using Added    = std::function<void(const PlaybackStream&)>;
			using Removed  = std::function<void(uint32_t node)>;
			using Unlinked = std::function<void(uint32_t node)>;

			PlaybackWatcher(Connection& connection, Added added, Removed removed, Unlinked unlinked = {})
			  : connection(connection), added(std::move(added)), removed(std::move(removed)),
			    unlinked(std::move(unlinked))
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
						static_cast<PlaybackWatcher*>(data)->OnGlobal(id, type, props);
					};
					e.global_remove = [](void* data, uint32_t id) {
						static_cast<PlaybackWatcher*>(data)->OnGlobalRemove(id);
					};
					return e;
				}();
				static const pw_core_events coreEvents = [] {
					pw_core_events e{};
					e.version = PW_VERSION_CORE_EVENTS;
					e.done    = [](void* data, uint32_t id, int seq) {
						if (id == PW_ID_CORE)
							static_cast<PlaybackWatcher*>(data)->OnDone(seq);
					};
					return e;
				}();
				registry = pw_core_get_registry(connection.core, PW_VERSION_REGISTRY, 0);
				if (!registry)
					throw std::runtime_error("PipeWire registry");
				pw_registry_add_listener(registry, &registryListener, &events, this);
				pw_core_add_listener(connection.core, &coreListener, &coreEvents, this);
			}

			~PlaybackWatcher()
			{
				spa_hook_remove(&coreListener);
				spa_hook_remove(&registryListener);
				for (auto& [id, client] : clients)
					Release(*client);
				for (auto& [id, stream] : streams)
					Release(*stream);
				connection.api.proxy_destroy(reinterpret_cast<pw_proxy*>(registry));
			}

		private:
			struct Client
			{
				PlaybackWatcher* watcher{ nullptr };
				uint32_t id{ 0 };
				pw_proxy* proxy{ nullptr };
				spa_hook listener{};
				// The info arrived; the rest is from it.
				bool known{ false };
				// As the application reports it. Inside a sandbox that is the
				// pid of its own namespace.
				pid_t pid{ 0 };
				// The process on the socket as the server saw it: the
				// application for native clients, pipewire-pulse for
				// PulseAudio ones.
				pid_t peer{ 0 };
				std::string binary;
				std::string name;
			};

			struct Stream
			{
				PlaybackWatcher* watcher{ nullptr };
				uint32_t id{ 0 };
				pw_proxy* proxy{ nullptr };
				spa_hook listener{};
				// The node's info arrived; `device` is from it.
				bool known{ false };
				// A virtual device's playback half, no application's sound.
				bool device{ false };
				uint32_t client{ SPA_ID_INVALID };
				std::string serial;
				std::string appName;
				std::string nodeName;
				enum { Waiting, Skipped, Reported } state{ Waiting };
			};

			void OnGlobal(uint32_t id, const char* type, const spa_dict* props)
			{
				if (std::strcmp(type, PW_TYPE_INTERFACE_Client) == 0)
				{
					const auto [entry, fresh] = clients.try_emplace(id);
					if (!fresh)
						return;
					entry->second  = std::make_unique<Client>();
					auto& client   = *entry->second;
					client.watcher = this;
					client.id      = id;
					client.proxy   =
					  static_cast<pw_proxy*>(pw_registry_bind(registry, id, type, PW_VERSION_CLIENT, 0));
					if (!client.proxy)
						return;
					static const pw_client_events events = [] {
						pw_client_events e{};
						e.version = PW_VERSION_CLIENT_EVENTS;
						e.info    = [](void* data, const pw_client_info* info) {
							auto* client = static_cast<Client*>(data);
							client->watcher->OnClientInfo(*client, info);
						};
						return e;
					}();
					pw_client_add_listener(
					  reinterpret_cast<pw_client*>(client.proxy), &client.listener, &events, &client);
					return;
				}
				if (std::strcmp(type, PW_TYPE_INTERFACE_Link) == 0)
				{
					const auto input = Lookup(props, PW_KEY_LINK_INPUT_NODE);
					if (unlinked && !input.empty())
						links.try_emplace(id, static_cast<uint32_t>(std::strtoul(input.c_str(), nullptr, 10)));
					return;
				}
				if (std::strcmp(type, PW_TYPE_INTERFACE_Node) != 0 ||
				    Lookup(props, PW_KEY_MEDIA_CLASS) != "Stream/Output/Audio")
					return;
				const auto [entry, fresh] = streams.try_emplace(id);
				if (!fresh)
					return;
				entry->second    = std::make_unique<Stream>();
				auto& stream     = *entry->second;
				stream.watcher   = this;
				stream.id        = id;
				stream.serial    = Lookup(props, "object.serial");
				stream.appName   = Lookup(props, PW_KEY_APP_NAME);
				stream.nodeName  = Lookup(props, PW_KEY_NODE_NAME);
				const auto owner = Lookup(props, PW_KEY_CLIENT_ID);
				if (!owner.empty())
					stream.client = static_cast<uint32_t>(std::strtoul(owner.c_str(), nullptr, 10));
				// The listing has a few of the node's properties only; whether
				// it belongs to a virtual device is in its info.
				stream.proxy =
				  static_cast<pw_proxy*>(pw_registry_bind(registry, id, type, PW_VERSION_NODE, 0));
				if (!stream.proxy)
					return;
				static const pw_node_events events = [] {
					pw_node_events e{};
					e.version = PW_VERSION_NODE_EVENTS;
					e.info    = [](void* data, const pw_node_info* info) {
						auto* stream = static_cast<Stream*>(data);
						stream->watcher->OnNodeInfo(*stream, info);
					};
					return e;
				}();
				pw_node_add_listener(
				  reinterpret_cast<pw_node*>(stream.proxy), &stream.listener, &events, &stream);
			}

			void OnGlobalRemove(uint32_t id)
			{
				if (const auto client = clients.find(id); client != clients.end())
				{
					Release(*client->second);
					clients.erase(client);
					return;
				}
				if (const auto link = links.find(id); link != links.end())
				{
					// Told after a round trip: when a playback stream ends, its
					// links go first, and what follows settles the matter.
					const uint32_t node = link->second;
					links.erase(link);
					if (!Linked(node))
					{
						cut.push_back(node);
						cutSeq = pw_core_sync(connection.core, PW_ID_CORE, 0);
					}
					return;
				}
				const auto stream = streams.find(id);
				if (stream == streams.end())
					return;
				const bool reported = stream->second->state == Stream::Reported;
				Release(*stream->second);
				streams.erase(stream);
				if (reported)
					removed(id);
			}

			bool Linked(uint32_t node) const
			{
				return std::any_of(
				  links.begin(), links.end(), [node](const auto& link) { return link.second == node; });
			}

			void OnDone(int seq)
			{
				if (cut.empty() || seq != cutSeq)
					return;
				const auto nodes = std::move(cut);
				cut.clear();
				for (const uint32_t node : nodes)
					if (!Linked(node))
						unlinked(node);
			}

			void OnClientInfo(Client& client, const pw_client_info* info)
			{
				if (!info || !info->props || !(info->change_mask & PW_CLIENT_CHANGE_MASK_PROPS))
					return;
				client.known  = true;
				client.pid    = ToPid(Lookup(info->props, PW_KEY_APP_PROCESS_ID));
				client.peer   = ToPid(Lookup(info->props, PW_KEY_SEC_PID));
				client.binary = Lookup(info->props, PW_KEY_APP_PROCESS_BINARY);
				client.name   = Lookup(info->props, PW_KEY_APP_NAME);
				for (auto& [id, stream] : streams)
					if (stream->client == client.id)
						Decide(*stream);
			}

			// Sent when the node is bound and again whenever it changes (its
			// state, with every start and stop): the first one counts.
			void OnNodeInfo(Stream& stream, const pw_node_info* info)
			{
				if (stream.known || !info || !info->props || !(info->change_mask & PW_NODE_CHANGE_MASK_PROPS))
					return;
				stream.known  = true;
				stream.device = VirtualDevice(info->props);
				Decide(stream);
			}

			// A stream waits for its own info and for its owner's: without
			// them it might be a virtual device's or this application's own.
			// A node or a client that leaves first takes the stream along.
			void Decide(Stream& stream)
			{
				if (stream.state != Stream::Waiting || !stream.known)
					return;
				const Client* owner = nullptr;
				if (stream.client != SPA_ID_INVALID)
				{
					const auto found = clients.find(stream.client);
					if (found == clients.end() || !found->second->known)
						return;
					owner = found->second.get();
				}
				stream.state = Stream::Skipped;
				if (stream.device)
					return;
				if (owner && (OwnProcess(owner->pid) || OwnProcess(owner->peer)))
					return;
				PlaybackStream playback;
				playback.node     = stream.id;
				playback.serial   = stream.serial;
				playback.oldId    = !stream.appName.empty() ? stream.appName : stream.nodeName;
				playback.app.id   = owner && !owner->binary.empty() ? owner->binary : playback.oldId;
				const auto& name  = !stream.appName.empty() || !owner ? stream.appName : owner->name;
				playback.app.name = GenericName(name) ? playback.app.id : name;
				if (playback.app.id.empty())
					return;
				stream.state = Stream::Reported;
				added(playback);
			}

			void Release(Client& client)
			{
				if (!client.proxy)
					return;
				spa_hook_remove(&client.listener);
				connection.api.proxy_destroy(client.proxy);
				client.proxy = nullptr;
			}

			void Release(Stream& stream)
			{
				if (!stream.proxy)
					return;
				spa_hook_remove(&stream.listener);
				connection.api.proxy_destroy(stream.proxy);
				stream.proxy = nullptr;
			}

			Connection& connection;
			const Added added;
			const Removed removed;
			const Unlinked unlinked;
			pw_registry* registry{ nullptr };
			spa_hook registryListener{};
			spa_hook coreListener{};
			std::map<uint32_t, std::unique_ptr<Client>> clients;
			std::map<uint32_t, std::unique_ptr<Stream>> streams;
			// Link -> the node it leads into.
			std::map<uint32_t, uint32_t> links;
			// Nodes whose last link went, until the round trip is back.
			std::vector<uint32_t> cut;
			int cutSeq{ 0 };
		};

		static_assert(MixInput::kBlockFrames == AppAudioCapture::kFrames);
		static_assert(MixInput::kChannels == static_cast<size_t>(AppAudioCapture::kChannels));

		class PipeWireAppAudio : public AppAudioCapture
		{
		public:
			PipeWireAppAudio(std::string app, Sink sink) : app(std::move(app)), sink(std::move(sink))
			{
			}

			void Start()
			{
				connection = std::make_unique<Connection>("gelabber-app-audio");
				{
					Connection::Lock lock(*connection);
					watcher = std::make_unique<PlaybackWatcher>(
					  *connection,
					  [this](const PlaybackStream& playback) { OnStream(playback); },
					  [this](uint32_t node) { OnStreamRemoved(node); },
					  [this](uint32_t node) { OnUnlinked(node); });
				}
				running = true;
				mixer   = std::thread([this] { Mix(); });
			}

			~PipeWireAppAudio() override
			{
				running = false;
				if (mixer.joinable())
					mixer.join();
				if (connection)
				{
					{
						// The watcher first: no stream is added after this.
						Connection::Lock lock(*connection);
						watcher.reset();
						for (auto& [node, capture] : captures)
							DestroyStream(*capture);
					}
					connection.reset();
				}
			}

			std::string StateJson() const override
			{
				std::lock_guard lock(mutex);
				uint64_t underruns = endedUnderruns;
				uint64_t overruns  = endedOverruns;
				for (const auto& [node, capture] : captures)
				{
					underruns += capture->input.underruns;
					overruns += capture->input.overruns;
				}
				return R"({"state":"live","streams":)" + std::to_string(captures.size()) + R"(,"frames":)" +
				       std::to_string(delivered) + R"(,"underruns":)" + std::to_string(underruns) +
				       R"(,"overruns":)" + std::to_string(overruns) + "}";
			}

		private:
			struct Capture
			{
				PipeWireAppAudio* owner{ nullptr };
				uint32_t node{ 0 };
				// The playback stream: its object.serial.
				std::string target;
				pw_stream* stream{ nullptr };
				spa_hook listener{};
				// Its samples waiting for the mixer (mutex).
				MixInput input;
			};

			// On the loop thread with its lock held, like the stream events.
			void OnStream(const PlaybackStream& playback)
			{
				if (!app.empty() && !playback.ChosenBy(app))
					return;
				auto capture    = std::make_unique<Capture>();
				capture->owner  = this;
				capture->node   = playback.node;
				capture->target = playback.serial.empty() ? std::to_string(playback.node) : playback.serial;
				if (!Connect(*capture))
					return;
				std::lock_guard lock(mutex);
				captures[playback.node] = std::move(capture);
			}

			bool Connect(Capture& capture)
			{
				auto* props = connection->api.properties_new(
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
                  capture.target.c_str(),
                  PW_KEY_NODE_DONT_RECONNECT,
                  "true",
                  // This stream or none: a session manager that has not
                  // prepared the playback stream yet would link the default
                  // source instead, the microphone, and leave it there
                  // (WirePlumber 0.5). Lingering, the capture waits.
                  "node.dont-fallback",
                  "true",
                  "node.linger",
                  "true",
                  // Capturing must not keep a paused application's output running.
                  "node.passive",
                  "true",
                  nullptr);
				capture.stream = connection->api.stream_new(connection->core, "Gelabber source audio", props);
				if (!capture.stream)
					return false;
				static const pw_stream_events events = [] {
					pw_stream_events e{};
					e.version = PW_VERSION_STREAM_EVENTS;
					e.process = [](void* data) {
						auto* capture = static_cast<Capture*>(data);
						capture->owner->OnProcess(*capture);
					};
					return e;
				}();
				spa_zero(capture.listener);
				connection->api.stream_add_listener(capture.stream, &capture.listener, &events, &capture);

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
				if (connection->api.stream_connect(capture.stream, PW_DIRECTION_INPUT, PW_ID_ANY, flags, params, 1) <
				    0)
				{
					DestroyStream(capture);
					return false;
				}
				return true;
			}

			// The session manager links a capture once. When the application
			// changes to an output with another channel layout, its ports are
			// replaced and the links with them, for good: a new capture stream
			// is linked again.
			void OnUnlinked(uint32_t node)
			{
				for (auto& [id, capture] : captures)
				{
					if (!capture->stream || connection->api.stream_get_node_id(capture->stream) != node)
						continue;
					DestroyStream(*capture);
					{
						std::lock_guard lock(mutex);
						capture->input.Clear();
					}
					Connect(*capture);
					return;
				}
			}

			void OnStreamRemoved(uint32_t id)
			{
				std::unique_ptr<Capture> gone;
				{
					std::lock_guard lock(mutex);
					auto it = captures.find(id);
					if (it == captures.end())
						return;
					gone = std::move(it->second);
					captures.erase(it);
					endedUnderruns += gone->input.underruns;
					endedOverruns += gone->input.overruns;
					// The end of its sound is still on the way into the mix.
					gone->input.End();
					ending.push_back(std::move(gone->input));
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
					// Whole frames.
					const size_t count =
					  std::min<size_t>(data.chunk->size, data.maxsize) / (sizeof(float) * kChannels) * kChannels;
					std::lock_guard lock(mutex);
					capture.input.Push(begin, count);
				}
				api.stream_queue_buffer(capture.stream, b);
			}

			// A block from each playing stream, summed, about every 10 ms: as
			// much sooner or later as keeps the streams' buffers level against
			// the sound card that fills them (app_audio_mix.h). Silence goes
			// out on the system clock.
			void Mix()
			{
				constexpr size_t block = kFrames * kChannels;
				std::vector<float> mix(block);
				std::vector<int16_t> pcm(block);
				auto next = std::chrono::steady_clock::now();
				while (running)
				{
					std::fill(mix.begin(), mix.end(), 0.0f);
					// What the stream with the least to spare has beyond its
					// cushion.
					std::optional<double> lead;
					{
						std::lock_guard lock(mutex);
						for (auto& [node, capture] : captures)
							if (const auto ahead = capture->input.Mix(mix.data()))
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
			std::unique_ptr<Connection> connection;
			std::unique_ptr<PlaybackWatcher> watcher;
			mutable std::mutex mutex;
			std::map<uint32_t, std::unique_ptr<Capture>> captures;
			// What streams that are gone left for the mix (mutex), and what
			// they counted.
			std::vector<MixInput> ending;
			uint64_t endedUnderruns{ 0 };
			uint64_t endedOverruns{ 0 };
			uint64_t delivered{ 0 };
			std::atomic<bool> running{ false };
			std::thread mixer;
		};
	} // namespace

	std::vector<AudioApp> ListAudioApps()
	{
		std::vector<PlaybackStream> playing;
		Connection connection("gelabber-app-list");
		Connection::Lock lock(connection);
		PlaybackWatcher watcher(
		  connection,
		  [&playing](const PlaybackStream& playback) { playing.push_back(playback); },
		  [&playing](uint32_t node) {
			  std::erase_if(
			    playing, [node](const PlaybackStream& playback) { return playback.node == node; });
		  });
		// One round trip for the registry's listing, one for the info of the
		// clients and nodes in it.
		if (connection.Sync(2))
			connection.Sync(2);

		// An entry for each binary. Programs that share one (Electron, Wine, an
		// interpreter) are told apart by the names they gave themselves: where
		// a binary's streams carry several, each name is an entry as well,
		// next to the binary, which chooses them all. The other way round,
		// a name that programs on several binaries give themselves (their own
		// Electron each, all "Chromium") is an entry next to the binaries and
		// chooses all of them: what 0.5.2 listed, and a client may have
		// stored.
		std::map<std::string, std::set<std::string>> names;
		std::map<std::string, std::set<std::string>> binaries;
		for (const auto& playback : playing)
		{
			auto& ofBinary = names[playback.app.id];
			if (!playback.Named())
				continue;
			ofBinary.insert(playback.oldId);
			binaries[playback.oldId].insert(playback.app.id);
		}
		std::set<std::string> ids;
		for (const auto& [binary, ofBinary] : names)
		{
			ids.insert(binary);
			if (ofBinary.size() > 1)
				ids.insert(ofBinary.begin(), ofBinary.end());
		}
		for (const auto& [name, ofName] : binaries)
			if (ofName.size() > 1)
				ids.insert(name);
		std::vector<AudioApp> out;
		for (const auto& id : ids)
		{
			AudioApp app;
			app.id = id;
			// Called what its streams are called when they agree.
			for (const auto& playback : playing)
			{
				if (!playback.ChosenBy(id))
					continue;
				if (app.streams++ == 0)
					app.name = playback.app.name;
				else if (app.name != playback.app.name)
					app.name = id;
			}
			out.push_back(std::move(app));
		}
		// No two entries under one name: where several are called the same,
		// each says which it is. The one that is the name itself keeps it.
		std::map<std::string, int> called;
		for (const auto& app : out)
			++called[app.name];
		for (auto& app : out)
			if (app.name != app.id && called[app.name] > 1)
				app.name += " (" + app.id + ")";
		return out;
	}

	std::unique_ptr<AppAudioCapture> AppAudioCapture::Start(const std::string& app, Sink sink)
	{
		auto capture = std::make_unique<PipeWireAppAudio>(app, std::move(sink));
		capture->Start();
		return capture;
	}
} // namespace gelabber

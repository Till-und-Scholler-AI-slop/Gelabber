// Gelabber native media core: C ABI over libmediasoupclient + libwebrtc.
// See include/gelabber_media.h for the contract.

#define GM_BUILDING 1
#include "gelabber_media.h"
#include "app_audio.h"
#include "audio_device_switch.h"
#include "audio_devices.h"
#include "audio_pipeline.h"
#include "capture_dsp.h"
#include "local_video_source.h"
#include "playout_limiter.h"
#include "transport_bridge.h"
#include "video_codec_factory.h"

#include "mediasoupclient.hpp"

#include <api/audio/audio_device.h>
#include <api/audio/audio_frame.h>
#include <api/audio/audio_processing.h>
#include <api/audio/builtin_audio_processing_builder.h>
#if __has_include(<api/audio/create_audio_device_module.h>)
#  include <api/audio/create_audio_device_module.h>
#  define GM_CREATE_ADM_WITH_ENVIRONMENT 1
#endif
#include <api/audio_codecs/builtin_audio_decoder_factory.h>
#include <api/audio_codecs/builtin_audio_encoder_factory.h>
#include <api/create_peerconnection_factory.h>
#include <api/environment/environment_factory.h>
#include <api/field_trials_view.h>
#include <api/make_ref_counted.h>
#include <api/media_stream_interface.h>
#include <api/notifier.h>
#include <api/peer_connection_interface.h>
#include <api/rtp_parameters.h>
#include <api/task_queue/default_task_queue_factory.h>
#include <api/video/i420_buffer.h>
#include <api/video/video_frame.h>
#include <api/video/video_sink_interface.h>
#include <audio/utility/audio_frame_operations.h>
#include <media/base/adapted_video_track_source.h>
#include <modules/audio_processing/audio_buffer.h>
#include <modules/audio_processing/include/audio_frame_proxies.h>
#include <modules/audio_device/include/fake_audio_device.h>
#include <rtc_base/logging.h>
#include <rtc_base/thread.h>
#include <rtc_base/time_utils.h>

#if defined(WEBRTC_LINUX)
// gm_video_codecs reports the GStreamer elements the codec factories pick.
#  include "gst_video_decoder.h"
#  include "gst_video_encoder.h"
#endif
#if defined(WEBRTC_WIN)
// gm_video_codecs reports the Media Foundation codecs.
#  include "mf_video.h"
#  include <rtc_base/win32_socket_init.h>
#endif

#include <algorithm>
#include <atomic>
#include <cctype>
#include <cmath>
#include <chrono>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <exception>
#include <future>
#include <map>
#include <memory>
#include <mutex>
#include <optional>
#include <stdexcept>
#include <string>
#include <thread>
#include <utility>
#include <vector>

using json = nlohmann::json;

// Split out of this file (issue #291); the engine below uses them unqualified.
using gelabber::ApmFieldTrials;
using gelabber::AudioLevelSink;
using gelabber::Bridge;
using gelabber::CapturePostProcessor;
using gelabber::CaptureTransport;
using gelabber::FindAudioDevice;
using gelabber::MicrophoneHub;
using gelabber::MicrophoneSource;
using gelabber::MonitoringAudioDevice;
using gelabber::PlayoutDevices;
using gelabber::PreviousAudioDevice;
using gelabber::PushAudioSource;
using gelabber::RecordingDevices;
using gelabber::RecvListener;
using gelabber::SelectPlayoutDevice;
using gelabber::SelectRecordingDevice;
using gelabber::SendListener;

namespace
{
	thread_local std::string lastError;

	void setError(const std::string& message)
	{
		lastError = message;
	}

	char* dupString(const std::string& value)
	{
		auto* out = static_cast<char*>(std::malloc(value.size() + 1));
		if (out)
			std::memcpy(out, value.c_str(), value.size() + 1);
		return out;
	}

	// Every exported entry point runs through this: no C++ exception may cross
	// the C ABI.
	template<typename R, typename F>
	R guarded(R failure, F&& body)
	{
		lastError.clear();
		try
		{
			return body();
		}
		catch (const std::exception& error)
		{
			setError(error.what());
		}
		catch (...)
		{
			setError("unknown native error");
		}
		return failure;
	}

	json parseJson(const char* text, const char* what)
	{
		if (!text)
			throw std::invalid_argument(std::string(what) + " is null");
		return json::parse(text);
	}

	std::string lower(std::string value)
	{
		std::transform(
		  value.begin(), value.end(), value.begin(), [](unsigned char c) { return std::tolower(c); });
		return value;
	}

	webrtc::LoggingSeverity webrtcSeverity(int level)
	{
		switch (level)
		{
			case 1:
				return webrtc::LS_ERROR;
			case 2:
				return webrtc::LS_WARNING;
			case 3:
				return webrtc::LS_INFO;
			case 4:
				return webrtc::LS_VERBOSE;
			default:
				return webrtc::LS_NONE;
		}
	}

	// Moving gradient so encoders produce real, changing frames.
	class TestPatternSource : public gelabber::LocalVideoSource
	{
	public:
		TestPatternSource(int width, int height, int fps) : width(width), height(height), fps(fps)
		{
		}

		~TestPatternSource() override
		{
			Stop();
		}

		void Start()
		{
			running = true;
			worker  = std::thread([this] { Run(); });
		}

		void Stop() override
		{
			running = false;
			if (worker.joinable())
				worker.join();
		}

		bool is_screencast() const override
		{
			return false;
		}

	private:
		void Run()
		{
			const auto interval = std::chrono::microseconds(1'000'000 / std::max(1, fps));
			auto next           = std::chrono::steady_clock::now();
			uint32_t tick       = 0;
			while (running)
			{
				const int64_t nowUs = webrtc::TimeMicros();
				int outWidth, outHeight, cropWidth, cropHeight, cropX, cropY;
				if (AdaptFrame(
				      width,
				      height,
				      nowUs,
				      &outWidth,
				      &outHeight,
				      &cropWidth,
				      &cropHeight,
				      &cropX,
				      &cropY))
				{
					auto buffer = webrtc::I420Buffer::Create(outWidth, outHeight);
					for (int y = 0; y < outHeight; ++y)
					{
						auto* row = buffer->MutableDataY() + y * buffer->StrideY();
						for (int x = 0; x < outWidth; ++x)
							row[x] = static_cast<uint8_t>((x + y + tick * 4) & 0xff);
					}
					const int chromaHeight = (outHeight + 1) / 2;
					std::memset(buffer->MutableDataU(), static_cast<int>((tick * 2) & 0xff), buffer->StrideU() * chromaHeight);
					std::memset(buffer->MutableDataV(), 128, buffer->StrideV() * chromaHeight);
					OnFrame(webrtc::VideoFrame::Builder()
					          .set_video_frame_buffer(buffer)
					          .set_timestamp_us(nowUs)
					          .set_rotation(webrtc::kVideoRotation_0)
					          .build());
				}
				++tick;
				next += interval;
				std::this_thread::sleep_until(next);
			}
		}

		const int width;
		const int height;
		const int fps;
		std::atomic<bool> running{ false };
		std::thread worker;
	};

	// Counts the frames of a video track and hands them, as I420 within the
	// sink's limits, to the sink the app set (its views of the stream).
	class FrameCounter : public webrtc::VideoSinkInterface<webrtc::VideoFrame>
	{
	public:
		void OnFrame(const webrtc::VideoFrame& frame) override
		{
			frames.fetch_add(1, std::memory_order_relaxed);
			width.store(frame.width(), std::memory_order_relaxed);
			height.store(frame.height(), std::memory_order_relaxed);

			std::lock_guard lock(sinkMutex);
			if (!sink || !Due(webrtc::TimeMicros()))
				return;
			auto i420 = frame.video_frame_buffer()->ToI420();
			if (!i420)
				return;
			const int sourceWidth  = i420->width();
			const int sourceHeight = i420->height();
			// The limits are for the picture as displayed.
			const bool sideways =
			  frame.rotation() == webrtc::kVideoRotation_90 || frame.rotation() == webrtc::kVideoRotation_270;
			int outWidth  = sourceWidth;
			int outHeight = sourceHeight;
			if (Fit(
			      sideways ? limits.max_height : limits.max_width,
			      sideways ? limits.max_width : limits.max_height,
			      outWidth,
			      outHeight))
			{
				// libyuv's box filter divides by a rounded-down reciprocal of
				// the box area: exact enough up to 16x16 source pixels per
				// pixel, visibly dark far beyond. Quarter the picture first.
				while (int64_t{ i420->width() } * i420->height() > int64_t{ 256 } * outWidth * outHeight)
				{
					auto quarter = webrtc::I420Buffer::Create((i420->width() + 3) / 4, (i420->height() + 3) / 4);
					quarter->ScaleFrom(*i420);
					i420 = quarter;
				}
				// One buffer for every frame: the sink reads it during the call only.
				if (!scaled || scaled->width() != outWidth || scaled->height() != outHeight)
					scaled = webrtc::I420Buffer::Create(outWidth, outHeight);
				scaled->ScaleFrom(*i420);
				i420 = scaled;
			}
			const gm_video_frame out{
				i420->width(),
				i420->height(),
				i420->DataY(),
				i420->DataU(),
				i420->DataV(),
				i420->StrideY(),
				i420->StrideU(),
				i420->StrideV(),
				static_cast<int>(frame.rotation()),
				frame.timestamp_us(),
				sourceWidth,
				sourceHeight,
			};
			sink(sinkUser, &out);
		}

		// Returns once no call to the previous sink runs any more.
		void SetSink(gm_video_frame_fn fn, void* user)
		{
			std::lock_guard lock(sinkMutex);
			sink     = fn;
			sinkUser = user;
		}

		void SetLimits(const gm_video_sink_limits* wanted)
		{
			const gm_video_sink_limits next = wanted ? *wanted : gm_video_sink_limits{ 0, 0, 0 };
			if (next.max_width < 0 || next.max_height < 0 || next.max_fps < 0)
				throw std::invalid_argument("video sink limits must not be negative");
			std::lock_guard lock(sinkMutex);
			limits = next;
			nextDue.reset();
			scaled = nullptr;
		}

		std::atomic<uint64_t> frames{ 0 };
		std::atomic<int> width{ 0 };
		std::atomic<int> height{ 0 };

	private:
		// Shrinks width x height to fit the limits (0: none), aspect kept, to
		// even dimensions. False when the picture fits as it is.
		static bool Fit(int maxWidth, int maxHeight, int& width, int& height)
		{
			const bool wide = maxWidth > 0 && width > maxWidth;
			const bool tall = maxHeight > 0 && height > maxHeight;
			if (!wide && !tall)
				return false;
			const int64_t w = width;
			const int64_t h = height;
			// The side that has to shrink more decides.
			if (wide && (!tall || int64_t{ maxWidth } * h <= int64_t{ maxHeight } * w))
			{
				height = static_cast<int>(h * maxWidth / w);
				width  = maxWidth;
			}
			else
			{
				width  = static_cast<int>(w * maxHeight / h);
				height = maxHeight;
			}
			width  = std::max(2, width & ~1);
			height = std::max(2, height & ~1);
			return true;
		}

		// Drops frames above max_fps the way libwebrtc's FramerateController
		// does: evenly, and half an interval lenient, so a stream at about the
		// limit loses nothing to jitter.
		bool Due(int64_t nowUs)
		{
			if (limits.max_fps <= 0)
				return true;
			const int64_t interval = 1'000'000 / limits.max_fps;
			if (nextDue)
			{
				const int64_t ahead = *nextDue - nowUs;
				if (ahead > -2 * interval && ahead < 2 * interval)
				{
					if (ahead > 0)
						return false;
					*nextDue += interval;
					return true;
				}
			}
			// The first frame, or one far off the schedule: start over.
			nextDue = nowUs + interval / 2;
			return true;
		}

		std::mutex sinkMutex;
		gm_video_frame_fn sink{ nullptr };
		void* sinkUser{ nullptr };
		gm_video_sink_limits limits{ 0, 0, 0 };
		std::optional<int64_t> nextDue;
		webrtc::scoped_refptr<webrtc::I420Buffer> scaled;
	};

	class NoopProducerListener : public mediasoupclient::Producer::Listener
	{
	public:
		void OnTransportClose(mediasoupclient::Producer*) override
		{
		}
	};

	class NoopConsumerListener : public mediasoupclient::Consumer::Listener
	{
	public:
		void OnTransportClose(mediasoupclient::Consumer*) override
		{
		}
	};

	NoopProducerListener producerListener;
	NoopConsumerListener consumerListener;

	std::once_flag initialized;

#if defined(WEBRTC_WIN)
	// libwebrtc leaves Winsock to its embedder, and the network thread's
	// socket server needs it from its constructor on. Never cleaned up:
	// WSACleanup would run while the library unloads, where it must not.
	void StartWinsock()
	{
		static webrtc::WinsockInitializer* const winsock = new webrtc::WinsockInitializer();
		if (winsock->error() != 0)
			throw std::runtime_error("WSAStartup failed with error " + std::to_string(winsock->error()));
	}
#endif
} // namespace

struct gm_engine
{
	std::unique_ptr<webrtc::Thread> network;
	std::unique_ptr<webrtc::Thread> worker;
	std::unique_ptr<webrtc::Thread> signaling;
	std::unique_ptr<webrtc::FakeAudioDeviceModule> dummyAudio;
	std::unique_ptr<webrtc::TaskQueueFactory> taskQueues;
	// Platform audio device module; null with "dummy" audio. Used on the
	// worker thread only.
	webrtc::scoped_refptr<webrtc::AudioDeviceModule> adm;
	// `adm` as the voice pipeline sees it (microphone test monitoring).
	webrtc::scoped_refptr<MonitoringAudioDevice> monitor;
	std::shared_ptr<MicrophoneHub> microphones{ std::make_shared<MicrophoneHub>() };
	std::unique_ptr<CaptureTransport> capture;
	webrtc::scoped_refptr<webrtc::AudioProcessing> apm;
	std::shared_ptr<gelabber::CaptureDsp> dsp;
	webrtc::scoped_refptr<webrtc::PeerConnectionFactoryInterface> factory;
	std::atomic<uint64_t> nextTrack{ 0 };
	std::mutex audioMutex;
	std::string inputId;
	std::string outputId;
	// The audio consumers and their volumes (ApplyPlaybackVolumes).
	std::mutex playbackMutex;
	std::vector<gm_consumer*> audioConsumers;

	mediasoupclient::PeerConnection::Options Options() const
	{
		mediasoupclient::PeerConnection::Options options;
		options.factory = factory.get();
		// The media server has no ICE-TCP (media/src/sfu), so TCP host
		// candidates can never pair. Gathering them opens a listening
		// socket per interface, which is what makes the Windows firewall
		// ask about the app. TURN over TCP or TLS is a relay port and stays.
		options.config.tcp_candidate_policy = webrtc::PeerConnectionInterface::kTcpCandidatePolicyDisabled;
		return options;
	}

	std::string TrackId(const char* prefix)
	{
		return std::string(prefix) + "-" + std::to_string(++nextTrack);
	}
};

struct gm_device
{
	gm_engine* engine;
	mediasoupclient::Device device;
};

struct gm_transport
{
	gm_device* device{ nullptr };
	gm_direction direction{ GM_SEND };
	std::unique_ptr<Bridge> bridge;
	std::unique_ptr<SendListener> sendListener;
	std::unique_ptr<RecvListener> recvListener;
	mediasoupclient::SendTransport* send{ nullptr };
	mediasoupclient::RecvTransport* recv{ nullptr };
	std::string id;

	mediasoupclient::Transport* Base() const
	{
		return send ? static_cast<mediasoupclient::Transport*>(send) : recv;
	}
};

struct gm_source
{
	gm_engine* engine;
	webrtc::scoped_refptr<webrtc::MediaStreamTrackInterface> track;
	// Null for audio.
	webrtc::scoped_refptr<gelabber::LocalVideoSource> video;
	// Application sound only.
	std::unique_ptr<gelabber::AppAudioCapture> appAudio;
	// The app's sink for a self view. On the track only while a sink is set,
	// so an unwatched source still captures nothing before it is produced.
	std::unique_ptr<FrameCounter> preview;
	bool previewOnTrack{ false };

	// Takes the self view's sink off the track; returns once it is not running.
	void DetachPreview()
	{
		if (!previewOnTrack)
			return;
		static_cast<webrtc::VideoTrackInterface*>(track.get())->RemoveSink(preview.get());
		previewOnTrack = false;
	}
};

struct gm_producer
{
	mediasoupclient::Producer* producer{ nullptr };
	std::string id;
};

struct gm_consumer
{
	gm_engine* engine{ nullptr };
	mediasoupclient::Consumer* consumer{ nullptr };
	std::unique_ptr<FrameCounter> counter;
	std::unique_ptr<AudioLevelSink> audio;
	std::string id;
	// Audio: the volume asked for (0..4), and the one its receive stream
	// plays at (libwebrtc starts it at 1). Under the engine's playbackMutex.
	double volume{ 1.0 };
	double streamVolume{ 1.0 };
};

namespace
{
	// Playback above 100 % (#223). libwebrtc scales a receive stream with
	// saturation, so a loud speaker clips on its own above 1. The largest
	// volume above 1 is therefore the master gain of the playout limiter,
	// after the mix, and every stream plays at its volume over that gain:
	// each still ends at its own volume, and only the sum can reach the
	// limiter. Without the platform device module there is no playout to
	// limit, and streams take their volume as it is. Called with the
	// engine's playbackMutex held.
	void ApplyPlaybackVolumes(gm_engine& engine)
	{
		double master = 1.0;
		if (engine.monitor)
			for (const auto* consumer : engine.audioConsumers)
				master = std::max(master, consumer->volume);
		for (auto* consumer : engine.audioConsumers)
		{
			const double stream = consumer->volume / master;
			if (stream == consumer->streamVolume)
				continue;
			// RemoteAudioSource scales the decoded audio by this factor.
			static_cast<webrtc::AudioTrackInterface*>(consumer->consumer->GetTrack())
			  ->GetSource()
			  ->SetVolume(stream);
			consumer->streamVolume = stream;
		}
		engine.capture->Playout().SetGain(master);
	}

	std::string CaptureMode(const json& options)
	{
		auto mode = options.value("processingMode", std::string("enhanced"));
		if (mode != "enhanced" && mode != "browser" && mode != "original")
			throw std::invalid_argument("processingMode must be enhanced, browser or original");
		return mode;
	}

	// The engine-wide part of a microphone's processing: one capture
	// pipeline serves every microphone source and the microphone test.
	void ApplyCaptureMode(gm_engine& engine, const std::string& mode, const json& options)
	{
		engine.dsp->SetDenoise(mode == "enhanced");
		engine.dsp->SetGain(static_cast<float>(options.value("inputGain", 1.0)));
	}
} // namespace

extern "C" {

uint32_t gm_abi_version(void)
{
	return GM_ABI_VERSION;
}

const char* gm_last_error(void)
{
	return lastError.empty() ? nullptr : lastError.c_str();
}

void gm_string_free(char* s)
{
	std::free(s);
}

void gm_set_log_level(int level)
{
	webrtc::LogMessage::LogToDebug(webrtcSeverity(level));
	mediasoupclient::Logger::SetLogLevel(
	  static_cast<mediasoupclient::Logger::LogLevel>(std::clamp(level, 0, 4)));
	mediasoupclient::Logger::SetDefaultHandler();
}

gm_engine* gm_engine_new(const char* optionsJson)
{
	return guarded<gm_engine*>(nullptr, [&]() -> gm_engine* {
#if defined(WEBRTC_WIN)
		StartWinsock();
#endif
		std::call_once(initialized, [] { mediasoupclient::Initialize(); });
		const auto options = optionsJson ? json::parse(optionsJson) : json::object();
		const auto audio   = options.value("audio", std::string("default"));
		if (audio != "default" && audio != "dummy")
			throw std::invalid_argument("audio must be default or dummy");

		auto engine       = std::make_unique<gm_engine>();
		engine->network   = webrtc::Thread::CreateWithSocketServer();
		engine->worker    = webrtc::Thread::Create();
		engine->signaling = webrtc::Thread::Create();
		engine->network->SetName("gm_network", nullptr);
		engine->worker->SetName("gm_worker", nullptr);
		engine->signaling->SetName("gm_signaling", nullptr);
		if (!engine->network->Start() || !engine->worker->Start() || !engine->signaling->Start())
			throw std::runtime_error("failed to start libwebrtc threads");

		engine->dsp = std::make_shared<gelabber::CaptureDsp>();
		engine->apm = webrtc::BuiltinAudioProcessingBuilder()
		                .SetCapturePostProcessing(std::make_unique<CapturePostProcessor>(engine->dsp))
		                .Build(webrtc::CreateEnvironment(std::make_unique<ApmFieldTrials>()));
		if (!engine->apm)
			throw std::runtime_error("failed to create audio processing");
		engine->capture = std::make_unique<CaptureTransport>(engine->apm.get(), engine->dsp, engine->microphones);

		webrtc::scoped_refptr<webrtc::AudioDeviceModule> adm;
		if (audio == "dummy")
		{
			engine->dummyAudio = std::make_unique<webrtc::FakeAudioDeviceModule>();
			adm                = webrtc::scoped_refptr<webrtc::AudioDeviceModule>(engine->dummyAudio.get());
		}
		else
		{
			// Created here rather than inside the factory so device
			// selection can reach it.
			engine->taskQueues = webrtc::CreateDefaultTaskQueueFactory();
			engine->adm        = engine->worker->BlockingCall([&] {
#if defined(GM_CREATE_ADM_WITH_ENVIRONMENT)
				return webrtc::CreateAudioDeviceModule(
				  webrtc::CreateEnvironment(), webrtc::AudioDeviceModule::kPlatformDefaultAudio);
#else
				return webrtc::AudioDeviceModule::Create(
				  webrtc::AudioDeviceModule::kPlatformDefaultAudio, engine->taskQueues.get());
#endif
			});
			if (!engine->adm)
				throw std::runtime_error("failed to create the audio device module");
			engine->monitor = webrtc::make_ref_counted<MonitoringAudioDevice>(engine->adm, engine->capture.get());
			adm             = engine->monitor;
		}


		engine->factory = webrtc::CreatePeerConnectionFactory(
		  engine->network.get(),
		  engine->worker.get(),
		  engine->signaling.get(),
		  adm,
		  webrtc::CreateBuiltinAudioEncoderFactory(),
		  webrtc::CreateBuiltinAudioDecoderFactory(),
		  gelabber::CreateVideoEncoderFactory(),
		  gelabber::CreateVideoDecoderFactory(),
		  nullptr,
		  engine->apm);
		if (!engine->factory)
			throw std::runtime_error("failed to create PeerConnectionFactory");
		return engine.release();
	});
}

void gm_engine_free(gm_engine* engine)
{
	guarded<int>(0, [&] {
		if (!engine)
			return 0;
		// Capture runs through the APM: stop it before anything goes,
		// including a microphone test still holding it.
		if (engine->adm)
			engine->worker->BlockingCall([&] {
				engine->monitor->SetMonitoring(false);
				engine->adm->StopRecording();
			});
		engine->factory = nullptr;
		engine->apm     = nullptr;
		if (engine->adm)
			engine->worker->BlockingCall([&] {
				engine->monitor = nullptr;
				engine->adm     = nullptr;
			});
		engine->signaling->Stop();
		engine->worker->Stop();
		engine->network->Stop();
		delete engine;
		return 0;
	});
}

char* gm_audio_devices(gm_engine* engine)
{
	return guarded<char*>(nullptr, [&] {
		json out = { { "inputs", json::array() }, { "outputs", json::array() }, { "input", "" }, { "output", "" } };
		if (!engine->adm)
			return dupString(out.dump());
		engine->worker->BlockingCall([&] {
			for (const auto& device : RecordingDevices(*engine->adm))
				out["inputs"].push_back({ { "id", device.id }, { "name", device.name } });
			for (const auto& device : PlayoutDevices(*engine->adm))
				out["outputs"].push_back({ { "id", device.id }, { "name", device.name } });
		});
		std::lock_guard lock(engine->audioMutex);
		out["input"]  = engine->inputId;
		out["output"] = engine->outputId;
		// Device names come from the system and need not be valid UTF-8.
		return dupString(out.dump(-1, ' ', false, json::error_handler_t::replace));
	});
}

int gm_audio_configure(gm_engine* engine, const char* optionsJson)
{
	return guarded<int>(-1, [&] {
		const auto options = parseJson(optionsJson, "audio options");
		if (options.contains("inputGain"))
			engine->dsp->SetGain(options["inputGain"].get<float>());
		const bool input  = options.contains("input");
		const bool output = options.contains("output");
		if (!input && !output)
			return 0;
		if (!engine->adm)
			throw std::runtime_error("no audio devices with dummy audio");

		std::string error;
		engine->worker->BlockingCall([&] {
			// The inner module, not the monitor: the monitor's StopRecording
			// holds back while the microphone test runs, and the module then
			// refuses another device.
			auto& adm = *engine->adm;
			// The selected ids change only here, on the worker.
			std::string inputId, outputId;
			{
				std::lock_guard lock(engine->audioMutex);
				inputId  = engine->inputId;
				outputId = engine->outputId;
			}
			if (input)
			{
				const auto id      = options["input"].get<std::string>();
				const auto devices = RecordingDevices(adm);
				const auto index   = FindAudioDevice(devices, id);
				if (!index)
				{
					error = "unknown input device " + id;
					return;
				}
				// Whether capture should run, not whether the module runs: a
				// restart that failed before is tried again, which the page's
				// fallback to the default device relies on.
				const bool wanted = engine->monitor ? engine->monitor->CaptureWanted() : adm.Recording();
				const gelabber::DeviceOps ops{
					[&] { return adm.StopRecording(); },
					[&] { return adm.RecordingIsInitialized(); },
					[&](int device) { return SelectRecordingDevice(adm, device); },
					[&] { return adm.InitRecording(); },
					[&] { return adm.StartRecording(); },
				};
				// Once the previous device is unplugged, a rollback goes to
				// the default (PreviousAudioDevice).
				const bool previousListed = FindAudioDevice(devices, inputId).has_value();
				switch (gelabber::SwitchDevice(ops, wanted, *index, PreviousAudioDevice(devices, inputId)))
				{
					case gelabber::SwitchResult::Switched:
						break;
					case gelabber::SwitchResult::KeptPrevious:
						if (previousListed)
							error = "cannot use input device " + id + ", kept the previous one";
						else
						{
							std::lock_guard lock(engine->audioMutex);
							engine->inputId = "";
							error = "cannot use input device " + id + ", fell back to the default device";
						}
						return;
					case gelabber::SwitchResult::Stopped:
						error = wanted ? "cannot restart capture on " + id : "cannot select input device " + id;
						return;
				}
				std::lock_guard lock(engine->audioMutex);
				engine->inputId = id;
			}
			if (output)
			{
				const auto id      = options["output"].get<std::string>();
				const auto devices = PlayoutDevices(adm);
				const auto index   = FindAudioDevice(devices, id);
				if (!index)
				{
					error = "unknown output device " + id;
					return;
				}
				// The monitor passes playout through: the voice pipeline sees
				// the module's own state and starts it again for its next
				// stream.
				const bool wanted = adm.Playing();
				const gelabber::DeviceOps ops{
					[&] { return adm.StopPlayout(); },
					[&] { return adm.PlayoutIsInitialized(); },
					[&](int device) { return SelectPlayoutDevice(adm, device); },
					[&] { return adm.InitPlayout(); },
					[&] { return adm.StartPlayout(); },
				};
				const bool previousListed = FindAudioDevice(devices, outputId).has_value();
				switch (gelabber::SwitchDevice(ops, wanted, *index, PreviousAudioDevice(devices, outputId)))
				{
					case gelabber::SwitchResult::Switched:
						break;
					case gelabber::SwitchResult::KeptPrevious:
						if (previousListed)
							error = "cannot use output device " + id + ", kept the previous one";
						else
						{
							std::lock_guard lock(engine->audioMutex);
							engine->outputId = "";
							error = "cannot use output device " + id + ", fell back to the default device";
						}
						return;
					case gelabber::SwitchResult::Stopped:
						error = wanted ? "cannot restart playout on " + id : "cannot select output device " + id;
						return;
				}
				std::lock_guard lock(engine->audioMutex);
				engine->outputId = id;
			}
		});
		if (!error.empty())
			throw std::runtime_error(error);
		return 0;
	});
}

int gm_audio_monitor(gm_engine* engine, const char* optionsJson)
{
	return guarded<int>(-1, [&] {
		if (!engine->monitor)
			throw std::runtime_error("no audio devices with dummy audio");
		const bool on = optionsJson != nullptr;
		if (on)
		{
			const auto options = parseJson(optionsJson, "monitor options");
			ApplyCaptureMode(*engine, CaptureMode(options), options);
		}
		const bool started = engine->worker->BlockingCall([&] { return engine->monitor->SetMonitoring(on); });
		if (!started)
			throw std::runtime_error("cannot start capture for the microphone test");
		return 0;
	});
}

char* gm_audio_levels(gm_engine* engine)
{
	return guarded<char*>(nullptr, [&] {
		const auto levels  = engine->dsp->Levels();
		const auto playout = engine->capture->Playout().Levels();
		return dupString(json{ { "input", levels.raw },
		                       { "processed", levels.processed },
		                       { "clipping", levels.clipping },
		                       { "denoised", levels.denoised },
		                       { "blocks", levels.blocks },
		                       { "channels", levels.channels },
		                       { "playout",
		                         { { "gain", playout.gain },
		                           { "peak", playout.peak },
		                           { "reduction", playout.reduction },
		                           { "windows", playout.windows } } } }
		                   .dump());
	});
}

gm_device* gm_device_new(gm_engine* engine)
{
	return guarded<gm_device*>(nullptr, [&]() -> gm_device* {
		if (!engine)
			throw std::invalid_argument("engine is null");
		return new gm_device{ engine, {} };
	});
}

void gm_device_free(gm_device* device)
{
	delete device;
}

int gm_device_load(gm_device* device, const char* routerRtpCapabilitiesJson)
{
	return guarded<int>(-1, [&] {
		auto caps    = parseJson(routerRtpCapabilitiesJson, "router RTP capabilities");
		auto options = device->engine->Options();
		device->device.Load(caps, &options, /*preferLocalCodecsOrder*/ false);
		return 0;
	});
}

char* gm_device_rtp_capabilities(gm_device* device)
{
	return guarded<char*>(nullptr, [&] { return dupString(device->device.GetRtpCapabilities().dump()); });
}

int gm_device_can_produce(gm_device* device, const char* kind)
{
	return guarded<int>(-1, [&] { return device->device.CanProduce(kind ? kind : "") ? 1 : 0; });
}

gm_transport* gm_device_create_transport(
  gm_device* device, gm_direction direction, const char* transportJson, gm_event_fn onEvent, void* user)
{
	return guarded<gm_transport*>(nullptr, [&]() -> gm_transport* {
		const auto params = parseJson(transportJson, "transport parameters");
		for (const char* key : { "id", "iceParameters", "iceCandidates", "dtlsParameters" })
			if (!params.contains(key))
				throw std::invalid_argument(std::string("transport parameters lack ") + key);

		auto options = device->engine->Options();
		if (params.contains("iceServers"))
		{
			for (const auto& server : params["iceServers"])
			{
				webrtc::PeerConnectionInterface::IceServer ice;
				if (server["urls"].is_string())
					ice.urls.push_back(server["urls"].get<std::string>());
				else
					for (const auto& url : server["urls"])
						ice.urls.push_back(url.get<std::string>());
				ice.username = server.value("username", std::string());
				ice.password = server.value("credential", std::string());
				options.config.servers.push_back(ice);
			}
		}
		if (params.value("iceTransportPolicy", std::string("all")) == "relay")
			options.config.type = webrtc::PeerConnectionInterface::kRelay;

		// mediasoup >= 3.13 (and the 0.29 Rust crate) names the candidate
		// address `address`; libmediasoupclient still reads `ip`.
		auto candidates = params["iceCandidates"];
		for (auto& candidate : candidates)
			if (!candidate.contains("ip") && candidate.contains("address"))
				candidate["ip"] = candidate["address"];

		auto transport       = std::make_unique<gm_transport>();
		transport->device    = device;
		transport->direction = direction;
		transport->bridge    = std::make_unique<Bridge>(onEvent, user);
		const auto id        = params["id"].get<std::string>();
		if (direction == GM_SEND)
		{
			transport->sendListener = std::make_unique<SendListener>(transport->bridge.get());
			transport->send         = device->device.CreateSendTransport(
        transport->sendListener.get(),
        id,
        params["iceParameters"],
        candidates,
        params["dtlsParameters"],
        &options);
		}
		else
		{
			transport->recvListener = std::make_unique<RecvListener>(transport->bridge.get());
			transport->recv         = device->device.CreateRecvTransport(
        transport->recvListener.get(),
        id,
        params["iceParameters"],
        candidates,
        params["dtlsParameters"],
        &options);
		}
		transport->id = transport->Base()->GetId();
		return transport.release();
	});
}

void gm_transport_free(gm_transport* transport)
{
	guarded<int>(0, [&] {
		if (!transport)
			return 0;
		transport->bridge->FailPending();
		if (auto* base = transport->Base(); base && !base->IsClosed())
			base->Close();
		delete transport->send;
		delete transport->recv;
		delete transport;
		return 0;
	});
}

const char* gm_transport_id(gm_transport* transport)
{
	return transport ? transport->id.c_str() : nullptr;
}

int gm_transport_respond(gm_transport* transport, uint64_t request, const char* resultJson, const char* error)
{
	return guarded<int>(-1, [&] {
		if ((resultJson == nullptr) == (error == nullptr))
			throw std::invalid_argument("exactly one of result and error is required");
		if (!transport->bridge->Respond(request, resultJson, error))
			throw std::invalid_argument("unknown request");
		return 0;
	});
}

int gm_transport_restart_ice(gm_transport* transport, const char* iceParametersJson)
{
	return guarded<int>(-1, [&] {
		transport->Base()->RestartIce(parseJson(iceParametersJson, "ICE parameters"));
		return 0;
	});
}

char* gm_transport_stats(gm_transport* transport)
{
	return guarded<char*>(nullptr, [&] { return dupString(transport->Base()->GetStats().dump()); });
}

gm_source* gm_source_new_microphone(gm_engine* engine, const char* optionsJson)
{
	return guarded<gm_source*>(nullptr, [&]() -> gm_source* {
		const auto options = optionsJson ? json::parse(optionsJson) : json::object();
		const auto mode    = CaptureMode(options);
		const bool echoCancellation = options.value("echoCancellation", true);

		// Same switches as the web client's getUserMedia constraints
		// (web/src/voice/settings.ts micConstraints).
		webrtc::AudioOptions audio;
		audio.echo_cancellation = echoCancellation;
		audio.noise_suppression = mode == "browser" && options.value("noiseSuppression", true);
		audio.auto_gain_control = mode != "original" && options.value("autoGainControl", true);
		audio.highpass_filter   = mode != "original";

		ApplyCaptureMode(*engine, mode, options);

		auto source = webrtc::make_ref_counted<MicrophoneSource>(
		  audio, mode == "original" && !echoCancellation ? 2 : 1, engine->microphones);
		auto track  = engine->factory->CreateAudioTrack(engine->TrackId("mic"), source.get());
		if (!track)
			throw std::runtime_error("failed to create audio track");
		return new gm_source{ engine, track, nullptr };
	});
}

gm_source* gm_source_new_test_pattern(gm_engine* engine, int width, int height, int fps)
{
	return guarded<gm_source*>(nullptr, [&]() -> gm_source* {
		if (width < 16 || height < 16 || width > 7680 || height > 4320 || fps < 1 || fps > 120)
			throw std::invalid_argument("test pattern size or rate out of range");
		auto pattern = webrtc::make_ref_counted<TestPatternSource>(width, height, fps);
		auto track   = engine->factory->CreateVideoTrack(pattern, engine->TrackId("pattern"));
		if (!track)
			throw std::runtime_error("failed to create video track");
		pattern->Start();
		return new gm_source{ engine, track, pattern };
	});
}

gm_source* gm_source_new_screen(gm_engine* engine, const char* optionsJson)
{
	return guarded<gm_source*>(nullptr, [&]() -> gm_source* {
		const auto options = optionsJson ? json::parse(optionsJson) : json::object();
		gelabber::ScreenOptions screen;
		const auto type = options.value("type", std::string("any"));
		if (type == "screen")
			screen.type = gelabber::ScreenOptions::Type::Screen;
		else if (type == "window")
			screen.type = gelabber::ScreenOptions::Type::Window;
		else if (type != "any")
			throw std::invalid_argument("type must be any, screen or window");
		screen.fps    = options.value("fps", 30);
		screen.cursor = options.value("cursor", true);
		if (screen.fps < 1 || screen.fps > 120)
			throw std::invalid_argument("fps out of range");
		const auto hint = options.value("contentHint", std::string("detail"));
		webrtc::VideoTrackInterface::ContentHint contentHint;
		if (hint == "detail")
			contentHint = webrtc::VideoTrackInterface::ContentHint::kDetailed;
		else if (hint == "text")
			contentHint = webrtc::VideoTrackInterface::ContentHint::kText;
		else if (hint == "motion")
			contentHint = webrtc::VideoTrackInterface::ContentHint::kFluid;
		else
			throw std::invalid_argument("contentHint must be detail, text or motion");

		auto source = gelabber::CreateScreenSource(screen);
		auto track  = engine->factory->CreateVideoTrack(source, engine->TrackId("screen"));
		if (!track)
		{
			source->Stop();
			throw std::runtime_error("failed to create video track");
		}
		track->set_content_hint(contentHint);
		return new gm_source{ engine, track, source };
	});
}

char* gm_audio_apps(gm_engine*)
{
	return guarded<char*>(nullptr, [&] {
		json out = json::array();
		for (const auto& app : gelabber::ListAudioApps())
			out.push_back({ { "id", app.id }, { "name", app.name }, { "streams", app.streams } });
		return dupString(out.dump(-1, ' ', false, json::error_handler_t::replace));
	});
}

gm_source* gm_source_new_app_audio(gm_engine* engine, const char* optionsJson)
{
	return guarded<gm_source*>(nullptr, [&]() -> gm_source* {
		const auto options = optionsJson ? parseJson(optionsJson, "application sound options") : json::object();
		// No options: the voice engine would apply them to the shared APM.
		auto source = webrtc::make_ref_counted<PushAudioSource>(webrtc::AudioOptions());
		auto capture = gelabber::AppAudioCapture::Start(
		  options.value("app", std::string()), [source](const int16_t* pcm) {
			  source->Deliver(
			    pcm,
			    gelabber::AppAudioCapture::kSampleRate,
			    gelabber::AppAudioCapture::kChannels,
			    gelabber::AppAudioCapture::kFrames);
		  });
		auto track = engine->factory->CreateAudioTrack(engine->TrackId("app-audio"), source.get());
		if (!track)
			throw std::runtime_error("failed to create audio track");
		auto* out     = new gm_source{ engine, track, nullptr };
		out->appAudio = std::move(capture);
		return out;
	});
}

char* gm_video_codecs(void)
{
	return guarded<char*>(nullptr, [&] {
		json out = { { "h264", nullptr }, { "av1", nullptr } };
		const auto entry = [](const std::string& encoder,
		                      bool hardwareEncoder,
		                      const std::string& decoder,
		                      bool hardwareDecoder) {
			return json{ { "encoder", encoder.empty() ? json(nullptr) : json(encoder) },
				         { "hardwareEncoder", hardwareEncoder },
				         { "decoder", decoder.empty() ? json(nullptr) : json(decoder) },
				         { "hardwareDecoder", hardwareDecoder } };
		};
		for (const auto codec : { gelabber::SystemCodec::kH264, gelabber::SystemCodec::kAv1 })
		{
			const auto key = codec == gelabber::SystemCodec::kH264 ? "h264" : "av1";
#if defined(WEBRTC_LINUX)
			// The same choice the engine's factories make; GStreamer loads once.
			const auto encoder = gelabber::FindGstEncoder(codec);
			const auto decoder = gelabber::FindGstDecoder(codec);
			out[key]           = entry(
			  encoder, gelabber::IsHardwareEncoder(encoder), decoder, gelabber::IsHardwareDecoder(decoder));
#elif defined(WEBRTC_WIN)
			const auto& codecs = gelabber::FindMfCodecs(codec);
			out[key] = entry(codecs.encoder, codecs.hardwareEncoder, codecs.decoder, codecs.hardwareDecoder);
#else
			(void)key;
			(void)entry;
#endif
		}
		return dupString(out.dump());
	});
}

char* gm_video_devices(gm_engine*)
{
	return guarded<char*>(nullptr, [&] {
		json out = json::array();
		for (const auto& camera : gelabber::ListCameras())
			out.push_back({ { "id", camera.id }, { "name", camera.name } });
		return dupString(out.dump(-1, ' ', false, json::error_handler_t::replace));
	});
}

gm_source* gm_source_new_camera(gm_engine* engine, const char* optionsJson)
{
	return guarded<gm_source*>(nullptr, [&]() -> gm_source* {
		const auto options = optionsJson ? parseJson(optionsJson, "camera options") : json::object();
		gelabber::CameraOptions camera;
		camera.device = options.value("device", std::string());
		camera.width  = options.value("width", 1280);
		camera.height = options.value("height", 720);
		camera.fps    = options.value("fps", 30);
		if (camera.width < 16 || camera.height < 16 || camera.width > 7680 || camera.height > 4320 ||
		    camera.fps < 1 || camera.fps > 120)
			throw std::invalid_argument("camera size or rate out of range");
		auto source = gelabber::CreateCameraSource(camera);
		auto track  = engine->factory->CreateVideoTrack(source, engine->TrackId("camera"));
		if (!track)
		{
			source->Stop();
			throw std::runtime_error("failed to create video track");
		}
		track->set_content_hint(webrtc::VideoTrackInterface::ContentHint::kFluid);
		return new gm_source{ engine, track, source };
	});
}

char* gm_source_state(gm_source* source)
{
	return guarded<char*>(nullptr, [&] {
		if (source->video)
			return dupString(source->video->StateJson());
		if (source->appAudio)
			return dupString(source->appAudio->StateJson());
		return dupString(R"({"state":"live"})");
	});
}

void gm_source_free(gm_source* source)
{
	guarded<int>(0, [&] {
		if (!source)
			return 0;
		source->DetachPreview();
		if (source->video)
			source->video->Stop();
		// Stops delivering before the track goes.
		source->appAudio.reset();
		delete source;
		return 0;
	});
}

gm_producer* gm_transport_produce(gm_transport* transport, gm_source* source, const char* optionsJson)
{
	return guarded<gm_producer*>(nullptr, [&]() -> gm_producer* {
		if (!transport->send)
			throw std::invalid_argument("produce needs a send transport");
		const auto options = optionsJson ? json::parse(optionsJson) : json::object();
		const auto kind    = source->track->kind();

		// Pick the codec from the device's capabilities. Video prefers H264
		// (hardware encoders), then VP8.
		const auto& caps = transport->device->device.GetRtpCapabilities();
		std::vector<std::string> wanted;
		if (options.contains("codec"))
			wanted.push_back(lower(options["codec"].get<std::string>()));
		else if (kind == webrtc::MediaStreamTrackInterface::kVideoKind)
			wanted = { "video/h264", "video/vp8" };
		else
			wanted = { "audio/opus" };
		std::optional<json> codec;
		for (const auto& mime : wanted)
		{
			for (const auto& candidate : caps["codecs"])
			{
				if (lower(candidate.value("mimeType", std::string())) != mime)
					continue;
				// H264: packetization-mode 1, constrained baseline first (router order).
				if (mime == "video/h264" && candidate.contains("parameters") &&
				    candidate["parameters"].value("packetization-mode", 0) != 1)
					continue;
				codec = candidate;
				break;
			}
			if (codec)
				break;
		}
		if (!codec)
			throw std::runtime_error("no matching codec in device capabilities");

		std::vector<webrtc::RtpEncodingParameters> encodings;
		if (options.contains("encodings"))
		{
			for (const auto& item : options["encodings"])
			{
				webrtc::RtpEncodingParameters encoding;
				if (item.contains("scaleResolutionDownBy"))
					encoding.scale_resolution_down_by = item["scaleResolutionDownBy"].get<double>();
				if (item.contains("maxBitrate"))
					encoding.max_bitrate_bps = item["maxBitrate"].get<int>();
				if (item.contains("maxFramerate"))
					encoding.max_framerate = item["maxFramerate"].get<double>();
				if (item.contains("scalabilityMode"))
					encoding.scalability_mode = item["scalabilityMode"].get<std::string>();
				if (item.contains("active"))
					encoding.active = item["active"].get<bool>();
				encodings.push_back(encoding);
			}
		}
		const json codecOptions = options.value("codecOptions", json::object());
		const json appData      = options.value("appData", json::object());

		auto* producer = transport->send->Produce(
		  &producerListener,
		  source->track.get(),
		  encodings.empty() ? nullptr : &encodings,
		  codecOptions.empty() ? nullptr : &codecOptions,
		  &*codec,
		  appData);
		return new gm_producer{ producer, producer->GetId() };
	});
}

void gm_producer_free(gm_producer* producer)
{
	guarded<int>(0, [&] {
		if (!producer)
			return 0;
		if (!producer->producer->IsClosed())
			producer->producer->Close();
		delete producer->producer;
		delete producer;
		return 0;
	});
}

int gm_producer_free_released(gm_producer* producer)
{
	return guarded<int>(-1, [&] {
		if (!producer)
			throw std::invalid_argument("producer is null");
		// Our own reference outlives the free: dropping it last proves nobody
		// else (transceiver map, PeerConnection) still holds the sender.
		webrtc::scoped_refptr<webrtc::RtpSenderInterface> sender(producer->producer->GetRtpSender());
		gm_producer_free(producer);
		if (!lastError.empty())
			return -1;
		return sender.release()->Release() == webrtc::RefCountReleaseStatus::kDroppedLastRef ? 1 : 0;
	});
}

const char* gm_producer_id(gm_producer* producer)
{
	return producer ? producer->id.c_str() : nullptr;
}

char* gm_producer_rtp_parameters(gm_producer* producer)
{
	return guarded<char*>(
	  nullptr, [&] { return dupString(producer->producer->GetRtpParameters().dump()); });
}

int gm_producer_pause(gm_producer* producer, int paused)
{
	return guarded<int>(-1, [&] {
		if (paused)
			producer->producer->Pause();
		else
			producer->producer->Resume();
		return 0;
	});
}

namespace
{
	// RTCRtpEncodingParameters.priority as the browser maps it to a bitrate
	// weight (very-low 0.5, low 1, medium 2, high 4).
	const std::pair<const char*, double> kPriorities[] = {
		{ "very-low", 0.5 }, { "low", 1.0 }, { "medium", 2.0 }, { "high", 4.0 }
	};
	const std::pair<const char*, webrtc::Priority> kNetworkPriorities[] = {
		{ "very-low", webrtc::Priority::kVeryLow },
		{ "low", webrtc::Priority::kLow },
		{ "medium", webrtc::Priority::kMedium },
		{ "high", webrtc::Priority::kHigh },
	};

	webrtc::RtpSenderInterface* SenderOf(gm_producer* producer)
	{
		auto* sender = producer->producer->GetRtpSender();
		if (!sender)
			throw std::runtime_error("producer has no RTP sender");
		return sender;
	}
} // namespace

int gm_producer_replace_source(gm_producer* producer, gm_source* source)
{
	return guarded<int>(-1, [&] {
		if (!source)
			throw std::invalid_argument("source is null");
		if (source->track->kind() != producer->producer->GetKind())
			throw std::invalid_argument("replacement source is of another kind");
		producer->producer->ReplaceTrack(source->track.get());
		return 0;
	});
}

char* gm_producer_get_parameters(gm_producer* producer)
{
	return guarded<char*>(nullptr, [&] {
		const auto params = SenderOf(producer)->GetParameters();
		json encodings    = json::array();
		for (const auto& encoding : params.encodings)
		{
			json item{ { "active", encoding.active } };
			if (encoding.max_bitrate_bps)
				item["maxBitrate"] = *encoding.max_bitrate_bps;
			if (encoding.max_framerate)
				item["maxFramerate"] = *encoding.max_framerate;
			if (encoding.scale_resolution_down_by)
				item["scaleResolutionDownBy"] = *encoding.scale_resolution_down_by;
			// Priorities are per sender (libwebrtc keeps them on the first
			// encoding); report them on every encoding like the browser.
			const auto& first = params.encodings.front();
			for (const auto& [name, weight] : kPriorities)
				if (first.bitrate_priority == weight)
					item["priority"] = name;
			for (const auto& [name, priority] : kNetworkPriorities)
				if (first.network_priority == priority)
					item["networkPriority"] = name;
			encodings.push_back(item);
		}
		return dupString(json{ { "encodings", encodings } }.dump());
	});
}

int gm_producer_set_parameters(gm_producer* producer, const char* parametersJson)
{
	return guarded<int>(-1, [&] {
		const auto update = json::parse(parametersJson ? parametersJson : "{}");
		auto* sender      = SenderOf(producer);
		auto params       = sender->GetParameters();
		const auto& items = update.value("encodings", json::array());
		if (items.size() > params.encodings.size())
			throw std::invalid_argument("more encodings than the sender has");
		for (size_t i = 0; i < items.size(); ++i)
		{
			const auto& item = items[i];
			auto& encoding   = params.encodings[i];
			// Per-sender values: libwebrtc refuses them on later encodings.
			auto& first = params.encodings.front();
			if (item.contains("maxBitrate"))
				encoding.max_bitrate_bps = item["maxBitrate"].is_null()
				                             ? std::nullopt
				                             : std::optional<int>(item["maxBitrate"].get<int>());
			if (item.contains("maxFramerate"))
				encoding.max_framerate = item["maxFramerate"].is_null()
				                           ? std::nullopt
				                           : std::optional<double>(item["maxFramerate"].get<double>());
			if (item.contains("active"))
				encoding.active = item["active"].get<bool>();
			if (item.contains("priority"))
			{
				const auto name = item["priority"].get<std::string>();
				bool known      = false;
				for (const auto& [candidate, weight] : kPriorities)
					if (name == candidate)
						first.bitrate_priority = weight, known = true;
				if (!known)
					throw std::invalid_argument("unknown priority " + name);
			}
			if (item.contains("networkPriority"))
			{
				const auto name = item["networkPriority"].get<std::string>();
				bool known      = false;
				for (const auto& [candidate, priority] : kNetworkPriorities)
					if (name == candidate)
						first.network_priority = priority, known = true;
				if (!known)
					throw std::invalid_argument("unknown network priority " + name);
			}
		}
		const auto result = sender->SetParameters(params);
		if (!result.ok())
			throw std::runtime_error(std::string("SetParameters: ") + result.message());
		return 0;
	});
}

int gm_source_set_enabled(gm_source* source, int enabled)
{
	return guarded<int>(-1, [&] {
		source->track->set_enabled(enabled != 0);
		return 0;
	});
}

char* gm_producer_stats(gm_producer* producer)
{
	return guarded<char*>(nullptr, [&] { return dupString(producer->producer->GetStats().dump()); });
}

gm_consumer* gm_transport_consume(gm_transport* transport, const char* consumerJson)
{
	return guarded<gm_consumer*>(nullptr, [&]() -> gm_consumer* {
		if (!transport->recv)
			throw std::invalid_argument("consume needs a receive transport");
		auto params = parseJson(consumerJson, "consumer parameters");
		auto rtp    = params.at("rtpParameters");
		auto* consumer = transport->recv->Consume(
		  &consumerListener,
		  params.at("id").get<std::string>(),
		  params.at("producerId").get<std::string>(),
		  params.at("kind").get<std::string>(),
		  &rtp,
		  params.value("appData", json::object()));
		auto out      = std::make_unique<gm_consumer>();
		out->engine   = transport->device->engine;
		out->consumer = consumer;
		out->id       = consumer->GetId();
		if (consumer->GetKind() == "video")
		{
			out->counter = std::make_unique<FrameCounter>();
			auto* track  = static_cast<webrtc::VideoTrackInterface*>(consumer->GetTrack());
			track->AddOrUpdateSink(out->counter.get(), webrtc::VideoSinkWants());
		}
		else
		{
			out->audio  = std::make_unique<AudioLevelSink>();
			auto* track = static_cast<webrtc::AudioTrackInterface*>(consumer->GetTrack());
			track->AddSink(out->audio.get());
			// At 1 like a new stream, under a master gain already above it.
			std::lock_guard lock(out->engine->playbackMutex);
			out->engine->audioConsumers.push_back(out.get());
			ApplyPlaybackVolumes(*out->engine);
		}
		return out.release();
	});
}

void gm_consumer_free(gm_consumer* consumer)
{
	guarded<int>(0, [&] {
		if (!consumer)
			return 0;
		if (consumer->counter)
		{
			auto* track = static_cast<webrtc::VideoTrackInterface*>(consumer->consumer->GetTrack());
			track->RemoveSink(consumer->counter.get());
		}
		if (consumer->audio)
		{
			auto* track = static_cast<webrtc::AudioTrackInterface*>(consumer->consumer->GetTrack());
			track->RemoveSink(consumer->audio.get());
			// The master gain may drop with it.
			std::lock_guard lock(consumer->engine->playbackMutex);
			std::erase(consumer->engine->audioConsumers, consumer);
			ApplyPlaybackVolumes(*consumer->engine);
		}
		if (!consumer->consumer->IsClosed())
			consumer->consumer->Close();
		delete consumer->consumer;
		delete consumer;
		return 0;
	});
}

int gm_consumer_free_released(gm_consumer* consumer)
{
	return guarded<int>(-1, [&] {
		if (!consumer)
			throw std::invalid_argument("consumer is null");
		webrtc::scoped_refptr<webrtc::RtpReceiverInterface> receiver(
		  consumer->consumer->GetRtpReceiver());
		gm_consumer_free(consumer);
		if (!lastError.empty())
			return -1;
		return receiver.release()->Release() == webrtc::RefCountReleaseStatus::kDroppedLastRef ? 1 : 0;
	});
}

const char* gm_consumer_id(gm_consumer* consumer)
{
	return consumer ? consumer->id.c_str() : nullptr;
}

int gm_consumer_pause(gm_consumer* consumer, int paused)
{
	return guarded<int>(-1, [&] {
		if (paused)
			consumer->consumer->Pause();
		else
			consumer->consumer->Resume();
		return 0;
	});
}

int gm_consumer_set_video_sink(gm_consumer* consumer, gm_video_frame_fn fn, void* user)
{
	return guarded<int>(-1, [&] {
		if (!consumer->counter)
			throw std::invalid_argument("a video sink needs a video consumer");
		consumer->counter->SetSink(fn, user);
		return 0;
	});
}

int gm_consumer_set_video_sink_limits(gm_consumer* consumer, const gm_video_sink_limits* limits)
{
	return guarded<int>(-1, [&] {
		if (!consumer->counter)
			throw std::invalid_argument("a video sink needs a video consumer");
		consumer->counter->SetLimits(limits);
		return 0;
	});
}

int gm_source_set_video_sink(gm_source* source, gm_video_frame_fn fn, void* user)
{
	return guarded<int>(-1, [&] {
		if (!fn)
		{
			// Also fine for a source that never had one (audio included).
			source->DetachPreview();
			if (source->preview)
				source->preview->SetSink(nullptr, nullptr);
			return 0;
		}
		if (!source->video)
			throw std::invalid_argument("a video sink needs a video source");
		if (!source->preview)
			source->preview = std::make_unique<FrameCounter>();
		source->preview->SetSink(fn, user);
		if (!source->previewOnTrack)
		{
			// No wants: the self view takes what the encoders get and never
			// limits them (the broadcaster keeps the smallest request).
			static_cast<webrtc::VideoTrackInterface*>(source->track.get())
			  ->AddOrUpdateSink(source->preview.get(), webrtc::VideoSinkWants());
			source->previewOnTrack = true;
		}
		return 0;
	});
}

int gm_source_set_video_sink_limits(gm_source* source, const gm_video_sink_limits* limits)
{
	return guarded<int>(-1, [&] {
		if (!source->video)
			throw std::invalid_argument("a video sink needs a video source");
		if (!source->preview)
			source->preview = std::make_unique<FrameCounter>();
		source->preview->SetLimits(limits);
		return 0;
	});
}

int gm_consumer_set_volume(gm_consumer* consumer, double volume)
{
	return guarded<int>(-1, [&] {
		if (!consumer->audio)
			throw std::invalid_argument("volume needs an audio consumer");
		if (!std::isfinite(volume))
			throw std::invalid_argument("volume must be a number");
		std::lock_guard lock(consumer->engine->playbackMutex);
		consumer->volume = std::clamp(volume, 0.0, gelabber::PlayoutLimiter::kMaxGain);
		ApplyPlaybackVolumes(*consumer->engine);
		return 0;
	});
}

char* gm_consumer_stats(gm_consumer* consumer)
{
	return guarded<char*>(nullptr, [&] {
		json out = json::object();
		if (consumer->counter)
		{
			out["framesReceived"] = consumer->counter->frames.load();
			out["width"]          = consumer->counter->width.load();
			out["height"]         = consumer->counter->height.load();
		}
		if (consumer->audio)
		{
			out["audioLevel"]    = consumer->audio->level.load();
			out["samplesPlayed"] = consumer->audio->samples.load();
		}
		out["rtc"] = consumer->consumer->GetStats();
		return dupString(out.dump());
	});
}

} // extern "C"

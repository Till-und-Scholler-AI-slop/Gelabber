// Gelabber native media core: C ABI over libmediasoupclient + libwebrtc.
// See include/gelabber_media.h for the contract.

#define GM_BUILDING 1
#include "gelabber_media.h"
#include "app_audio.h"
#include "capture_dsp.h"
#include "local_video_source.h"

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
#include <api/video_codecs/video_decoder_factory_template.h>
#include <api/video_codecs/video_decoder_factory_template_dav1d_adapter.h>
#include <api/video_codecs/video_decoder_factory_template_libvpx_vp8_adapter.h>
#include <api/video_codecs/video_decoder_factory_template_libvpx_vp9_adapter.h>
#include <api/video_codecs/video_decoder_factory_template_open_h264_adapter.h>
#include <api/video_codecs/video_encoder.h>
#include <api/video_codecs/video_encoder_factory_template.h>
#include <api/video_codecs/video_encoder_factory_template_libaom_av1_adapter.h>
#include <api/video_codecs/video_encoder_factory_template_libvpx_vp8_adapter.h>
#include <api/video_codecs/video_encoder_factory_template_libvpx_vp9_adapter.h>
#include <api/video_codecs/video_encoder_factory_template_open_h264_adapter.h>
#include <audio/utility/audio_frame_operations.h>
#include <media/base/adapted_video_track_source.h>
#include <modules/audio_processing/audio_buffer.h>
#include <modules/audio_processing/include/audio_frame_proxies.h>
#include <modules/audio_device/include/fake_audio_device.h>
#include <rtc_base/logging.h>
#include <rtc_base/thread.h>
#include <rtc_base/time_utils.h>

#if defined(WEBRTC_LINUX)
// Hardware H264 through GStreamer, behind libwebrtc's simulcast adapter. The
// Windows libwebrtc package does not carry the adapter.
#  include "gst_h264_encoder.h"
#  include <media/engine/simulcast_encoder_adapter.h>
#endif
#if defined(WEBRTC_WIN)
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

	// The APM's capture post-processor: RNNoise, gain and meters after
	// libwebrtc's own echo cancellation, noise suppression and AGC.
	class CapturePostProcessor : public webrtc::CustomProcessing
	{
	public:
		explicit CapturePostProcessor(std::shared_ptr<gelabber::CaptureDsp> dsp) : dsp(std::move(dsp))
		{
		}

		void Initialize(int sampleRateHz, int numChannels) override
		{
			RTC_LOG(LS_INFO) << "Capture post-processing: " << sampleRateHz << " Hz, " << numChannels
			                 << " channel(s)";
			dsp->Initialize(sampleRateHz, numChannels);
		}

		void Process(webrtc::AudioBuffer* audio) override
		{
			dsp->Process(
			  audio->channels(), static_cast<int>(audio->num_channels()), static_cast<int>(audio->num_frames()));
		}

		std::string ToString() const override
		{
			return "GelabberCapture";
		}

	private:
		const std::shared_ptr<gelabber::CaptureDsp> dsp;
	};

	// The APM's field trials. Each m-section has its own voice channel,
	// and a channel whose only stream lost its track (a closed producer)
	// reports "all muted" to the engine-wide APM, which then skips the
	// capture post-processing (RNNoise, gain, meters) and its noise
	// suppression for every other microphone stream too. The kill switch
	// keeps the APM processing whatever the channels report.
	class ApmFieldTrials : public webrtc::FieldTrialsView
	{
	public:
		std::string Lookup(absl::string_view key) const override
		{
			return key == "WebRTC-MutedStateKillSwitch" ? "Enabled" : "";
		}
	};

	// Decoded remote audio as it is played out: level (0..100, 80 ms
	// windows like the microphone meter) and a sample count.
	class AudioLevelSink : public webrtc::AudioTrackSinkInterface
	{
	public:
		void OnData(
		  const void* audioData,
		  int bitsPerSample,
		  int sampleRate,
		  size_t numberOfChannels,
		  size_t numberOfFrames) override
		{
			if (bitsPerSample != 16 || !audioData)
				return;
			const auto* pcm     = static_cast<const int16_t*>(audioData);
			const size_t values = numberOfChannels * numberOfFrames;
			for (size_t i = 0; i < values; ++i)
				sum += static_cast<double>(pcm[i]) * pcm[i];
			count += values;
			window += numberOfFrames;
			samples.fetch_add(numberOfFrames, std::memory_order_relaxed);
			if (sampleRate > 0 && window >= static_cast<size_t>(sampleRate) * 80 / 1000)
			{
				const double rms = std::sqrt(sum / static_cast<double>(std::max<uint64_t>(count, 1))) / 32768.0;
				level.store(static_cast<int>(std::min(100.0, std::round(rms * 350.0))));
				sum    = 0;
				count  = 0;
				window = 0;
			}
		}

		std::atomic<uint64_t> samples{ 0 };
		std::atomic<int> level{ 0 };

	private:
		double sum{ 0 };
		uint64_t count{ 0 };
		size_t window{ 0 };
	};

	// Stands for the system default where the module has no index for it.
	constexpr int kDefaultAudioDevice = -1;
#if defined(WEBRTC_WIN)
	// Core Audio lists the endpoints only; the default is a role that is
	// selected on its own (SelectRecordingDevice, SelectPlayoutDevice).
	constexpr bool kDefaultAudioDeviceIsListed = false;
#else
	// Index 0 is the system default (PulseAudio).
	constexpr bool kDefaultAudioDeviceIsListed = true;
#endif

	struct AudioDeviceEntry
	{
		// The module's index, or kDefaultAudioDevice.
		int index;
		std::string id;
		std::string name;
	};

	// Devices in module order, the system default first with id "". Other
	// ids are the module's GUID where it has one (Windows endpoint ids);
	// libwebrtc's PulseAudio module reports none, so there the display
	// name is the id, numbered when names repeat.
	template<typename Count, typename NameOf>
	std::vector<AudioDeviceEntry> ListAudioDevices(Count count, NameOf nameOf)
	{
		std::vector<AudioDeviceEntry> out;
		if (!kDefaultAudioDeviceIsListed)
			out.push_back({ kDefaultAudioDevice, "", "Default" });
		std::map<std::string, int> seen;
		char name[webrtc::kAdmMaxDeviceNameSize];
		char guid[webrtc::kAdmMaxGuidSize];
		for (int i = 0, n = count(); i < n; ++i)
		{
			name[0] = guid[0] = '\0';
			if (nameOf(static_cast<uint16_t>(i), name, guid) != 0)
				continue;
			std::string id;
			if (i > 0 || !kDefaultAudioDeviceIsListed)
			{
				id = guid[0] != '\0' ? guid : name;
				if (const int repeat = ++seen[id]; repeat > 1)
					id += " (" + std::to_string(repeat) + ")";
			}
			out.push_back({ i, id, name });
		}
		return out;
	}

	std::vector<AudioDeviceEntry> RecordingDevices(webrtc::AudioDeviceModule& adm)
	{
		return ListAudioDevices(
		  [&] { return adm.RecordingDevices(); },
		  [&](uint16_t i, char* name, char* guid) { return adm.RecordingDeviceName(i, name, guid); });
	}

	std::vector<AudioDeviceEntry> PlayoutDevices(webrtc::AudioDeviceModule& adm)
	{
		return ListAudioDevices(
		  [&] { return adm.PlayoutDevices(); },
		  [&](uint16_t i, char* name, char* guid) { return adm.PlayoutDeviceName(i, name, guid); });
	}

	// The index of the device with this id, if there is one.
	std::optional<int> FindAudioDevice(const std::vector<AudioDeviceEntry>& devices, const std::string& id)
	{
		for (const auto& device : devices)
			if (device.id == id)
				return device.index;
		return std::nullopt;
	}

	// Windows has two defaults. The system default is the console role:
	// streams on the communications role make Windows turn every other
	// application down for as long as they run.
	int32_t SelectRecordingDevice(webrtc::AudioDeviceModule& adm, int index)
	{
		return index == kDefaultAudioDevice ? adm.SetRecordingDevice(webrtc::AudioDeviceModule::kDefaultDevice)
		                                    : adm.SetRecordingDevice(static_cast<uint16_t>(index));
	}

	int32_t SelectPlayoutDevice(webrtc::AudioDeviceModule& adm, int index)
	{
		return index == kDefaultAudioDevice ? adm.SetPlayoutDevice(webrtc::AudioDeviceModule::kDefaultDevice)
		                                    : adm.SetPlayoutDevice(static_cast<uint16_t>(index));
	}

	// Shared by send and receive listeners: turns libmediasoupclient's
	// blocking std::future callbacks into numbered events answered through
	// gm_transport_respond.
	class Bridge
	{
	public:
		Bridge(gm_event_fn onEvent, void* user) : onEvent(onEvent), user(user)
		{
		}

		std::future<void> Connect(const json& dtlsParameters)
		{
			std::promise<void> promise;
			auto future = promise.get_future();
			uint64_t id;
			{
				std::lock_guard<std::mutex> lock(mutex);
				id = ++nextRequest;
				connects.emplace(id, std::move(promise));
			}
			Emit(GM_EVENT_CONNECT, id, dtlsParameters);
			return future;
		}

		std::future<std::string> Produce(const std::string& kind, const json& rtpParameters, const json& appData)
		{
			std::promise<std::string> promise;
			auto future = promise.get_future();
			uint64_t id;
			{
				std::lock_guard<std::mutex> lock(mutex);
				id = ++nextRequest;
				produces.emplace(id, std::move(promise));
			}
			Emit(
			  GM_EVENT_PRODUCE,
			  id,
			  json{ { "kind", kind }, { "rtpParameters", rtpParameters }, { "appData", appData } });
			return future;
		}

		void State(const std::string& state)
		{
			Emit(GM_EVENT_CONNECTION_STATE, 0, json{ { "state", state } });
		}

		bool Respond(uint64_t request, const char* resultJson, const char* error)
		{
			std::lock_guard<std::mutex> lock(mutex);
			if (auto it = connects.find(request); it != connects.end())
			{
				if (error)
					it->second.set_exception(std::make_exception_ptr(std::runtime_error(error)));
				else
					it->second.set_value();
				connects.erase(it);
				return true;
			}
			if (auto it = produces.find(request); it != produces.end())
			{
				if (error)
				{
					it->second.set_exception(std::make_exception_ptr(std::runtime_error(error)));
				}
				else
				{
					auto result = json::parse(resultJson);
					if (!result.contains("id") || !result["id"].is_string())
						it->second.set_exception(
						  std::make_exception_ptr(std::runtime_error("produce result without id")));
					else
						it->second.set_value(result["id"].get<std::string>());
				}
				produces.erase(it);
				return true;
			}
			return false;
		}

		// Unblocks producers/consumers still waiting when the transport goes away.
		void FailPending()
		{
			std::lock_guard<std::mutex> lock(mutex);
			for (auto& [id, promise] : connects)
				promise.set_exception(std::make_exception_ptr(std::runtime_error("transport closed")));
			for (auto& [id, promise] : produces)
				promise.set_exception(std::make_exception_ptr(std::runtime_error("transport closed")));
			connects.clear();
			produces.clear();
		}

	private:
		void Emit(gm_event_kind kind, uint64_t request, const json& payload)
		{
			if (onEvent)
			{
				const auto text = payload.dump();
				onEvent(user, kind, request, text.c_str());
			}
		}

		gm_event_fn onEvent;
		void* user;
		std::mutex mutex;
		uint64_t nextRequest{ 0 };
		std::map<uint64_t, std::promise<void>> connects;
		std::map<uint64_t, std::promise<std::string>> produces;
	};

	class SendListener : public mediasoupclient::SendTransport::Listener
	{
	public:
		explicit SendListener(Bridge* bridge) : bridge(bridge)
		{
		}
		std::future<void> OnConnect(mediasoupclient::Transport*, const json& dtlsParameters) override
		{
			return bridge->Connect(dtlsParameters);
		}
		void OnConnectionStateChange(mediasoupclient::Transport*, const std::string& state) override
		{
			bridge->State(state);
		}
		std::future<std::string> OnProduce(
		  mediasoupclient::SendTransport*, const std::string& kind, json rtpParameters, const json& appData) override
		{
			return bridge->Produce(kind, rtpParameters, appData);
		}
		std::future<std::string> OnProduceData(
		  mediasoupclient::SendTransport*, const json&, const std::string&, const std::string&, const json&) override
		{
			std::promise<std::string> promise;
			promise.set_exception(std::make_exception_ptr(std::runtime_error("data channels unsupported")));
			return promise.get_future();
		}

	private:
		Bridge* bridge;
	};

	class RecvListener : public mediasoupclient::RecvTransport::Listener
	{
	public:
		explicit RecvListener(Bridge* bridge) : bridge(bridge)
		{
		}
		std::future<void> OnConnect(mediasoupclient::Transport*, const json& dtlsParameters) override
		{
			return bridge->Connect(dtlsParameters);
		}
		void OnConnectionStateChange(mediasoupclient::Transport*, const std::string& state) override
		{
			bridge->State(state);
		}

	private:
		Bridge* bridge;
	};

	// Source of a local audio track that the core feeds itself. libwebrtc's
	// own capture path hands the microphone to every audio send stream; the
	// core instead routes each track's audio from its source, so a
	// microphone and an application's sound stay separate tracks.
	class PushAudioSource : public webrtc::Notifier<webrtc::AudioSourceInterface>
	{
	public:
		explicit PushAudioSource(webrtc::AudioOptions options) : audioOptions(std::move(options))
		{
		}

		SourceState state() const override
		{
			return kLive;
		}
		bool remote() const override
		{
			return false;
		}
		// The voice engine applies these to the APM, as for its own sources.
		const webrtc::AudioOptions options() const override
		{
			return audioOptions;
		}
		void AddSink(webrtc::AudioTrackSinkInterface* sink) override
		{
			std::lock_guard lock(sinkMutex);
			sinks.push_back(sink);
		}
		void RemoveSink(webrtc::AudioTrackSinkInterface* sink) override
		{
			std::lock_guard lock(sinkMutex);
			sinks.erase(std::remove(sinks.begin(), sinks.end(), sink), sinks.end());
		}

		bool HasSinks()
		{
			std::lock_guard lock(sinkMutex);
			return !sinks.empty();
		}

		// 16-bit interleaved PCM, 10 ms; from one thread at a time.
		void Deliver(const int16_t* pcm, int sampleRate, size_t channels, size_t frames)
		{
			std::lock_guard lock(sinkMutex);
			for (auto* sink : sinks)
				sink->OnData(pcm, 16, sampleRate, channels, frames, std::nullopt);
		}

	private:
		const webrtc::AudioOptions audioOptions;
		std::mutex sinkMutex;
		std::vector<webrtc::AudioTrackSinkInterface*> sinks;
	};

	class MicrophoneHub;

	// A microphone track's source. Stereo only in original mode without
	// echo cancellation: it takes the device's stereo past the APM.
	class MicrophoneSource : public PushAudioSource
	{
	public:
		MicrophoneSource(webrtc::AudioOptions options, size_t channels, std::shared_ptr<MicrophoneHub> hub);
		~MicrophoneSource() override;

		const size_t channels;

	private:
		const std::shared_ptr<MicrophoneHub> hub;
	};

	// The microphone sources of an engine; all get the processed capture.
	class MicrophoneHub
	{
	public:
		void Add(MicrophoneSource* source)
		{
			std::lock_guard lock(mutex);
			sources.push_back(source);
		}
		void Remove(MicrophoneSource* source)
		{
			std::lock_guard lock(mutex);
			sources.erase(std::remove(sources.begin(), sources.end(), source), sources.end());
		}
		bool WantsStereo()
		{
			std::lock_guard lock(mutex);
			for (auto* source : sources)
				if (source->channels == 2 && source->HasSinks())
					return true;
			return false;
		}
		// `mono`: the APM's output. `stereo`: the device's own stereo for
		// stereo sources, or null to give them the processed mono too.
		void Deliver(const webrtc::AudioFrame& mono, const webrtc::AudioFrame* stereo)
		{
			std::lock_guard lock(mutex);
			for (auto* source : sources)
			{
				const auto& frame = source->channels == 2 && stereo ? *stereo : mono;
				source->Deliver(
				  frame.data(), frame.sample_rate_hz(), frame.num_channels(), frame.samples_per_channel());
			}
		}

	private:
		std::mutex mutex;
		std::vector<MicrophoneSource*> sources;
	};

	MicrophoneSource::MicrophoneSource(
	  webrtc::AudioOptions options, size_t channels, std::shared_ptr<MicrophoneHub> hub)
	  : PushAudioSource(std::move(options)), channels(channels), hub(std::move(hub))
	{
		this->hub->Add(this);
	}

	MicrophoneSource::~MicrophoneSource()
	{
		hub->Remove(this);
	}

	// Between the audio device module and the voice engine. Capture runs
	// through the APM (echo cancellation against the playout, noise
	// suppression, the post-processor) here and goes to the microphone
	// sources only, instead of to every send stream. Playout passes through
	// to the engine's own transport, which also feeds the APM's echo
	// reference.
	class CaptureTransport : public webrtc::AudioTransport
	{
	public:
		CaptureTransport(
		  webrtc::AudioProcessing* apm,
		  std::shared_ptr<gelabber::CaptureDsp> dsp,
		  std::shared_ptr<MicrophoneHub> hub)
		  : apm(apm), dsp(std::move(dsp)), hub(std::move(hub))
		{
		}

		void SetRender(webrtc::AudioTransport* transport)
		{
			render.store(transport);
		}

		int32_t RecordedDataIsAvailable(
		  const void* samples,
		  size_t samplesPerChannel,
		  size_t bytesPerSample,
		  size_t channels,
		  uint32_t sampleRate,
		  uint32_t totalDelayMs,
		  int32_t clockDrift,
		  uint32_t currentMicLevel,
		  bool keyPressed,
		  uint32_t& newMicLevel) override
		{
			return RecordedDataIsAvailable(
			  samples,
			  samplesPerChannel,
			  bytesPerSample,
			  channels,
			  sampleRate,
			  totalDelayMs,
			  clockDrift,
			  currentMicLevel,
			  keyPressed,
			  newMicLevel,
			  std::nullopt);
		}

		int32_t RecordedDataIsAvailable(
		  const void* samples,
		  size_t samplesPerChannel,
		  size_t bytesPerSample,
		  size_t channels,
		  uint32_t sampleRate,
		  uint32_t totalDelayMs,
		  int32_t /*clockDrift*/,
		  uint32_t /*currentMicLevel*/,
		  bool keyPressed,
		  uint32_t& newMicLevel,
		  std::optional<int64_t> /*estimatedCaptureTimeNs*/) override
		{
			// The OS microphone volume stays where the person set it.
			newMicLevel = 0;
			if (bytesPerSample != channels * sizeof(int16_t) || !samples)
				return -1;
			// The APM always runs mono, like every non-original stream in the
			// engine's own path. A changing capture channel count leaves
			// parts of the APM sized for the old one (an out-of-bounds abort
			// in its post filter).
			frame.UpdateFrame(
			  0,
			  static_cast<const int16_t*>(samples),
			  samplesPerChannel,
			  static_cast<int>(sampleRate),
			  webrtc::AudioFrame::kNormalSpeech,
			  webrtc::AudioFrame::kVadUnknown,
			  channels);
			const bool stereo = channels == 2 && hub->WantsStereo();
			if (stereo)
			{
				stereoFrame.CopyFrom(frame);
				ApplyGain(stereoFrame, dsp->Gain());
			}
			if (frame.num_channels() > 1)
				webrtc::AudioFrameOperations::DownmixChannels(1, &frame);
			apm->set_stream_delay_ms(static_cast<int>(totalDelayMs));
			apm->set_stream_key_pressed(keyPressed);
			webrtc::ProcessAudioFrame(apm, &frame);
			hub->Deliver(frame, stereo ? &stereoFrame : nullptr);
			return 0;
		}

		int32_t NeedMorePlayData(
		  size_t samplesPerChannel,
		  size_t bytesPerSample,
		  size_t channels,
		  uint32_t sampleRate,
		  void* audio,
		  size_t& samplesOut,
		  int64_t* elapsedTimeMs,
		  int64_t* ntpTimeMs) override
		{
			if (auto* transport = render.load())
				return transport->NeedMorePlayData(
				  samplesPerChannel,
				  bytesPerSample,
				  channels,
				  sampleRate,
				  audio,
				  samplesOut,
				  elapsedTimeMs,
				  ntpTimeMs);
			std::memset(audio, 0, samplesPerChannel * bytesPerSample);
			samplesOut = samplesPerChannel;
			return 0;
		}

		void PullRenderData(
		  int bitsPerSample,
		  int sampleRate,
		  size_t channels,
		  size_t frames,
		  void* audio,
		  int64_t* elapsedTimeMs,
		  int64_t* ntpTimeMs) override
		{
			if (auto* transport = render.load())
				transport->PullRenderData(
				  bitsPerSample, sampleRate, channels, frames, audio, elapsedTimeMs, ntpTimeMs);
		}

	private:
		static void ApplyGain(webrtc::AudioFrame& target, float gain)
		{
			if (gain == 1.0f || target.muted())
				return;
			int16_t* data = target.mutable_data();
			for (size_t i = 0, n = target.samples_per_channel() * target.num_channels(); i < n; ++i)
				data[i] = static_cast<int16_t>(std::clamp(std::lround(data[i] * gain), -32768L, 32767L));
		}

		webrtc::AudioProcessing* const apm;
		const std::shared_ptr<gelabber::CaptureDsp> dsp;
		const std::shared_ptr<MicrophoneHub> hub;
		std::atomic<webrtc::AudioTransport*> render{ nullptr };
		// Capture thread only.
		webrtc::AudioFrame frame;
		webrtc::AudioFrame stereoFrame;
	};

	// The platform audio device module as the engine's voice pipeline sees
	// it. libwebrtc records only while a microphone stream sends and stops
	// the module when the last one goes; the microphone test needs meters
	// without a call, so the module records while either wants it.
	// Everything else is passed through. Called on the worker thread only.
	class MonitoringAudioDevice : public webrtc::AudioDeviceModule
	{
	public:
		MonitoringAudioDevice(webrtc::scoped_refptr<webrtc::AudioDeviceModule> inner, CaptureTransport* capture)
		  : inner(std::move(inner)), capture(capture)
		{
		}

		// The test's side; returns false if capture cannot start.
		bool SetMonitoring(bool on)
		{
			monitoring = on;
			if (on)
			{
				if (inner->Recording())
					return true;
				if (!inner->RecordingIsInitialized() && inner->InitRecording() != 0)
					return false;
				return inner->StartRecording() == 0;
			}
			if (!streamsRecording && inner->Recording())
				inner->StopRecording();
			return true;
		}

		// The voice pipeline sees its own recording state, so a stream
		// starting during the test still registers.
		bool Recording() const override
		{
			return streamsRecording;
		}
		int32_t InitRecording() override
		{
			return inner->Recording() ? 0 : inner->InitRecording();
		}
		int32_t StartRecording() override
		{
			const int32_t result = inner->Recording() ? 0 : inner->StartRecording();
			streamsRecording     = result == 0;
			return result;
		}
		int32_t StopRecording() override
		{
			streamsRecording = false;
			return monitoring ? 0 : inner->StopRecording();
		}

		int32_t ActiveAudioLayer(AudioLayer* layer) const override
		{
			return inner->ActiveAudioLayer(layer);
		}
		// The module talks to the capture transport; the engine's own
		// transport only plays out.
		int32_t RegisterAudioCallback(webrtc::AudioTransport* callback) override
		{
			capture->SetRender(callback);
			return inner->RegisterAudioCallback(callback ? capture : nullptr);
		}
		int32_t Init() override
		{
			return inner->Init();
		}
		int32_t Terminate() override
		{
			return inner->Terminate();
		}
		bool Initialized() const override
		{
			return inner->Initialized();
		}
		int16_t PlayoutDevices() override
		{
			return inner->PlayoutDevices();
		}
		int16_t RecordingDevices() override
		{
			return inner->RecordingDevices();
		}
		int32_t PlayoutDeviceName(
		  uint16_t index, char name[webrtc::kAdmMaxDeviceNameSize], char guid[webrtc::kAdmMaxGuidSize]) override
		{
			return inner->PlayoutDeviceName(index, name, guid);
		}
		int32_t RecordingDeviceName(
		  uint16_t index, char name[webrtc::kAdmMaxDeviceNameSize], char guid[webrtc::kAdmMaxGuidSize]) override
		{
			return inner->RecordingDeviceName(index, name, guid);
		}
		int32_t SetPlayoutDevice(uint16_t index) override
		{
			return inner->SetPlayoutDevice(index);
		}
		// Only called on Windows, where the voice engine starts on the
		// default communications device. It gets the system default
		// instead (see SelectPlayoutDevice), the device of the id "".
		int32_t SetPlayoutDevice(WindowsDeviceType) override
		{
			return inner->SetPlayoutDevice(kDefaultDevice);
		}
		int32_t SetRecordingDevice(uint16_t index) override
		{
			return inner->SetRecordingDevice(index);
		}
		int32_t SetRecordingDevice(WindowsDeviceType) override
		{
			return inner->SetRecordingDevice(kDefaultDevice);
		}
		int32_t PlayoutIsAvailable(bool* available) override
		{
			return inner->PlayoutIsAvailable(available);
		}
		int32_t InitPlayout() override
		{
			return inner->InitPlayout();
		}
		bool PlayoutIsInitialized() const override
		{
			return inner->PlayoutIsInitialized();
		}
		int32_t RecordingIsAvailable(bool* available) override
		{
			return inner->RecordingIsAvailable(available);
		}
		bool RecordingIsInitialized() const override
		{
			return inner->RecordingIsInitialized();
		}
		int32_t StartPlayout() override
		{
			return inner->StartPlayout();
		}
		int32_t StopPlayout() override
		{
			return inner->StopPlayout();
		}
		bool Playing() const override
		{
			return inner->Playing();
		}
		int32_t InitSpeaker() override
		{
			return inner->InitSpeaker();
		}
		bool SpeakerIsInitialized() const override
		{
			return inner->SpeakerIsInitialized();
		}
		int32_t InitMicrophone() override
		{
			return inner->InitMicrophone();
		}
		bool MicrophoneIsInitialized() const override
		{
			return inner->MicrophoneIsInitialized();
		}
		int32_t SpeakerVolumeIsAvailable(bool* available) override
		{
			return inner->SpeakerVolumeIsAvailable(available);
		}
		int32_t SetSpeakerVolume(uint32_t volume) override
		{
			return inner->SetSpeakerVolume(volume);
		}
		int32_t SpeakerVolume(uint32_t* volume) const override
		{
			return inner->SpeakerVolume(volume);
		}
		int32_t MaxSpeakerVolume(uint32_t* volume) const override
		{
			return inner->MaxSpeakerVolume(volume);
		}
		int32_t MinSpeakerVolume(uint32_t* volume) const override
		{
			return inner->MinSpeakerVolume(volume);
		}
		int32_t MicrophoneVolumeIsAvailable(bool* available) override
		{
			return inner->MicrophoneVolumeIsAvailable(available);
		}
		int32_t SetMicrophoneVolume(uint32_t volume) override
		{
			return inner->SetMicrophoneVolume(volume);
		}
		int32_t MicrophoneVolume(uint32_t* volume) const override
		{
			return inner->MicrophoneVolume(volume);
		}
		int32_t MaxMicrophoneVolume(uint32_t* volume) const override
		{
			return inner->MaxMicrophoneVolume(volume);
		}
		int32_t MinMicrophoneVolume(uint32_t* volume) const override
		{
			return inner->MinMicrophoneVolume(volume);
		}
		int32_t SpeakerMuteIsAvailable(bool* available) override
		{
			return inner->SpeakerMuteIsAvailable(available);
		}
		int32_t SetSpeakerMute(bool enable) override
		{
			return inner->SetSpeakerMute(enable);
		}
		int32_t SpeakerMute(bool* enabled) const override
		{
			return inner->SpeakerMute(enabled);
		}
		int32_t MicrophoneMuteIsAvailable(bool* available) override
		{
			return inner->MicrophoneMuteIsAvailable(available);
		}
		int32_t SetMicrophoneMute(bool enable) override
		{
			return inner->SetMicrophoneMute(enable);
		}
		int32_t MicrophoneMute(bool* enabled) const override
		{
			return inner->MicrophoneMute(enabled);
		}
		int32_t StereoPlayoutIsAvailable(bool* available) const override
		{
			return inner->StereoPlayoutIsAvailable(available);
		}
		int32_t SetStereoPlayout(bool enable) override
		{
			return inner->SetStereoPlayout(enable);
		}
		int32_t StereoPlayout(bool* enabled) const override
		{
			return inner->StereoPlayout(enabled);
		}
		int32_t StereoRecordingIsAvailable(bool* available) const override
		{
			return inner->StereoRecordingIsAvailable(available);
		}
		int32_t SetStereoRecording(bool enable) override
		{
			return inner->SetStereoRecording(enable);
		}
		int32_t StereoRecording(bool* enabled) const override
		{
			return inner->StereoRecording(enabled);
		}
		int32_t PlayoutDelay(uint16_t* delayMs) const override
		{
			return inner->PlayoutDelay(delayMs);
		}
		// No processing in the device: the voice engine would switch the
		// APM's own off for it. Windows offers an echo canceller that
		// records 16 kHz mono, which RNNoise (48 kHz) and stereo cannot
		// use. libwebrtc's PulseAudio module has none to begin with.
		bool BuiltInAECIsAvailable() const override
		{
			return false;
		}
		bool BuiltInAGCIsAvailable() const override
		{
			return false;
		}
		bool BuiltInNSIsAvailable() const override
		{
			return false;
		}
		int32_t EnableBuiltInAEC(bool /*enable*/) override
		{
			return -1;
		}
		int32_t EnableBuiltInAGC(bool /*enable*/) override
		{
			return -1;
		}
		int32_t EnableBuiltInNS(bool /*enable*/) override
		{
			return -1;
		}
		int32_t GetPlayoutUnderrunCount() const override
		{
			return inner->GetPlayoutUnderrunCount();
		}
		std::optional<Stats> GetStats() const override
		{
			return inner->GetStats();
		}

	private:
		const webrtc::scoped_refptr<webrtc::AudioDeviceModule> inner;
		CaptureTransport* const capture;
		bool monitoring{ false };
		bool streamsRecording{ false };
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

	using SoftwareEncoderFactory = webrtc::VideoEncoderFactoryTemplate<
	  webrtc::OpenH264EncoderTemplateAdapter,
	  webrtc::LibvpxVp8EncoderTemplateAdapter,
	  webrtc::LibvpxVp9EncoderTemplateAdapter,
	  webrtc::LibaomAv1EncoderTemplateAdapter>;

	// A software encoder that holds libwebrtc to its simulcast layers. libvpx
	// (VP8) and OpenH264 encode the layers of a producer in one encoder and
	// refuse a set in which a layer has not exactly the top layer's aspect
	// (WEBRTC_VIDEO_CODEC_ERR_SIMULCAST_PARAMETERS_NOT_SUPPORTED): the frame
	// has to divide by every layer's scaleResolutionDownBy. libwebrtc's own
	// factory puts them behind SimulcastEncoderAdapter, which then gives each
	// layer an encoder; the Windows package does not carry it. Told that the
	// encoder's alignment holds for every layer, libwebrtc works out what the
	// layers need (AlignmentAdjuster) and asks the source for such frames.
	// The sources crop to LocalVideoSource::kResolutionAlignment on their
	// own, so the web client's layers never wait for that.
	class SimulcastAlignedEncoder : public webrtc::VideoEncoder
	{
	public:
		explicit SimulcastAlignedEncoder(std::unique_ptr<webrtc::VideoEncoder> encoder)
		  : encoder(std::move(encoder))
		{
		}

		void SetFecControllerOverride(webrtc::FecControllerOverride* fecControllerOverride) override
		{
			encoder->SetFecControllerOverride(fecControllerOverride);
		}

		int InitEncode(const webrtc::VideoCodec* codec, const Settings& settings) override
		{
			return encoder->InitEncode(codec, settings);
		}

		int32_t RegisterEncodeCompleteCallback(webrtc::EncodedImageCallback* callback) override
		{
			return encoder->RegisterEncodeCompleteCallback(callback);
		}

		int32_t Release() override
		{
			return encoder->Release();
		}

		int32_t Encode(
		  const webrtc::VideoFrame& frame, const std::vector<webrtc::VideoFrameType>* types) override
		{
			return encoder->Encode(frame, types);
		}

		void SetRates(const RateControlParameters& parameters) override
		{
			encoder->SetRates(parameters);
		}

		void OnPacketLossRateUpdate(float packetLossRate) override
		{
			encoder->OnPacketLossRateUpdate(packetLossRate);
		}

		void OnRttUpdate(int64_t rttMs) override
		{
			encoder->OnRttUpdate(rttMs);
		}

		void OnLossNotification(const LossNotification& lossNotification) override
		{
			encoder->OnLossNotification(lossNotification);
		}

		EncoderInfo GetEncoderInfo() const override
		{
			auto info = encoder->GetEncoderInfo();
			info.apply_alignment_to_all_simulcast_layers = true;
			return info;
		}

	private:
		const std::unique_ptr<webrtc::VideoEncoder> encoder;
	};

	// libwebrtc's software encoders. On Linux H264 moves to a hardware
	// encoder (GStreamer: VA-API/NVENC) when the system has one. Hardware
	// H264 runs per simulcast layer behind SimulcastEncoderAdapter, which
	// also falls back to OpenH264 when the hardware encoder fails.
	class EncoderFactory : public webrtc::VideoEncoderFactory
	{
	public:
		EncoderFactory()
		{
#if defined(WEBRTC_LINUX)
			const auto element = gelabber::FindGstH264Encoder();
			if (!element.empty())
			{
				std::vector<webrtc::SdpVideoFormat> h264;
				for (const auto& format : software.GetSupportedFormats())
					if (lower(format.name) == "h264")
						h264.push_back(format);
				hardware = gelabber::CreateGstH264EncoderFactory(element, std::move(h264));
				RTC_LOG(LS_INFO) << "H264 encoder: GStreamer " << element;
			}
#endif
		}

		std::vector<webrtc::SdpVideoFormat> GetSupportedFormats() const override
		{
			return software.GetSupportedFormats();
		}

		CodecSupport QueryCodecSupport(
		  const webrtc::SdpVideoFormat& format, std::optional<std::string> scalabilityMode) const override
		{
			return software.QueryCodecSupport(format, scalabilityMode);
		}

		std::unique_ptr<webrtc::VideoEncoder> Create(
		  const webrtc::Environment& env, const webrtc::SdpVideoFormat& format) override
		{
#if defined(WEBRTC_LINUX)
			if (hardware && lower(format.name) == "h264")
				return std::make_unique<webrtc::SimulcastEncoderAdapter>(env, hardware.get(), &software, format);
#endif
			auto encoder = software.Create(env, format);
			if (!encoder)
				return nullptr;
			return std::make_unique<SimulcastAlignedEncoder>(std::move(encoder));
		}

	private:
		SoftwareEncoderFactory software;
#if defined(WEBRTC_LINUX)
		std::unique_ptr<webrtc::VideoEncoderFactory> hardware;
#endif
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

	mediasoupclient::PeerConnection::Options Options() const
	{
		mediasoupclient::PeerConnection::Options options;
		options.factory = factory.get();
		// The media server has no ICE-TCP (media/src/sfu.rs), so TCP host
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
	mediasoupclient::Consumer* consumer{ nullptr };
	std::unique_ptr<FrameCounter> counter;
	std::unique_ptr<AudioLevelSink> audio;
	std::string id;
};

namespace
{
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


		// TODO(desktop): hardware H264 decoding.
		engine->factory = webrtc::CreatePeerConnectionFactory(
		  engine->network.get(),
		  engine->worker.get(),
		  engine->signaling.get(),
		  adm,
		  webrtc::CreateBuiltinAudioEncoderFactory(),
		  webrtc::CreateBuiltinAudioDecoderFactory(),
		  std::make_unique<EncoderFactory>(),
		  std::make_unique<webrtc::VideoDecoderFactoryTemplate<
		    webrtc::OpenH264DecoderTemplateAdapter,
		    webrtc::LibvpxVp8DecoderTemplateAdapter,
		    webrtc::LibvpxVp9DecoderTemplateAdapter,
		    webrtc::Dav1dDecoderTemplateAdapter>>(),
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
			auto& adm = *engine->adm;
			if (input)
			{
				const auto id    = options["input"].get<std::string>();
				const auto index = FindAudioDevice(RecordingDevices(adm), id);
				if (!index)
				{
					error = "unknown input device " + id;
					return;
				}
				const bool running = adm.Recording();
				if (running)
					adm.StopRecording();
				if (SelectRecordingDevice(adm, *index) != 0)
					error = "cannot select input device " + id;
				if (running && (adm.InitRecording() != 0 || adm.StartRecording() != 0))
					error = "cannot restart capture on " + id;
				if (!error.empty())
					return;
				std::lock_guard lock(engine->audioMutex);
				engine->inputId = id;
			}
			if (output)
			{
				const auto id    = options["output"].get<std::string>();
				const auto index = FindAudioDevice(PlayoutDevices(adm), id);
				if (!index)
				{
					error = "unknown output device " + id;
					return;
				}
				const bool running = adm.Playing();
				if (running)
					adm.StopPlayout();
				if (SelectPlayoutDevice(adm, *index) != 0)
					error = "cannot select output device " + id;
				if (running && (adm.InitPlayout() != 0 || adm.StartPlayout() != 0))
					error = "cannot restart playout on " + id;
				if (!error.empty())
					return;
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
		const auto levels = engine->dsp->Levels();
		return dupString(json{ { "input", levels.raw },
		                       { "processed", levels.processed },
		                       { "clipping", levels.clipping },
		                       { "denoised", levels.denoised },
		                       { "blocks", levels.blocks },
		                       { "channels", levels.channels } }
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
		audio.auto_gain_control = mode == "browser" && options.value("autoGainControl", true);
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
		}
		if (!consumer->consumer->IsClosed())
			consumer->consumer->Close();
		delete consumer->consumer;
		delete consumer;
		return 0;
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
		auto* track = static_cast<webrtc::AudioTrackInterface*>(consumer->consumer->GetTrack());
		// RemoteAudioSource scales the decoded audio by this factor (0..10).
		track->GetSource()->SetVolume(std::clamp(volume, 0.0, 2.0));
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

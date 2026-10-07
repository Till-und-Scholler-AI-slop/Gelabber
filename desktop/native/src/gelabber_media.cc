// Gelabber native media core: C ABI over libmediasoupclient + libwebrtc.
// See include/gelabber_media.h for the contract.

#define GM_BUILDING 1
#include "gelabber_media.h"
#include "capture_dsp.h"
#include "gst_h264_encoder.h"
#include "local_video_source.h"

#include "mediasoupclient.hpp"

#include <api/audio/audio_device.h>
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
#include <api/video_codecs/video_encoder_factory_template.h>
#include <api/video_codecs/video_encoder_factory_template_libaom_av1_adapter.h>
#include <api/video_codecs/video_encoder_factory_template_libvpx_vp8_adapter.h>
#include <api/video_codecs/video_encoder_factory_template_libvpx_vp9_adapter.h>
#include <api/video_codecs/video_encoder_factory_template_open_h264_adapter.h>
#include <media/base/adapted_video_track_source.h>
#include <media/engine/simulcast_encoder_adapter.h>
#include <modules/audio_processing/audio_buffer.h>
#include <modules/audio_device/include/fake_audio_device.h>
#include <rtc_base/logging.h>
#include <rtc_base/thread.h>
#include <rtc_base/time_utils.h>

#include <algorithm>
#include <atomic>
#include <cctype>
#include <cmath>
#include <chrono>
#include <cstring>
#include <future>
#include <map>
#include <memory>
#include <mutex>
#include <optional>
#include <string>
#include <thread>
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

	class FrameCounter : public webrtc::VideoSinkInterface<webrtc::VideoFrame>
	{
	public:
		void OnFrame(const webrtc::VideoFrame& frame) override
		{
			frames.fetch_add(1, std::memory_order_relaxed);
			width.store(frame.width(), std::memory_order_relaxed);
			height.store(frame.height(), std::memory_order_relaxed);
		}

		std::atomic<uint64_t> frames{ 0 };
		std::atomic<int> width{ 0 };
		std::atomic<int> height{ 0 };
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

	struct AudioDeviceEntry
	{
		uint16_t index;
		std::string id;
		std::string name;
	};

	// Devices in module order. Index 0 is the system default with id "".
	// Other ids are the module's GUID where it has one (Windows endpoint
	// ids); libwebrtc's PulseAudio module reports none, so there the
	// display name is the id, numbered when names repeat.
	template<typename Count, typename NameOf>
	std::vector<AudioDeviceEntry> ListAudioDevices(Count count, NameOf nameOf)
	{
		std::vector<AudioDeviceEntry> out;
		std::map<std::string, int> seen;
		char name[webrtc::kAdmMaxDeviceNameSize];
		char guid[webrtc::kAdmMaxGuidSize];
		for (int i = 0, n = count(); i < n; ++i)
		{
			name[0] = guid[0] = '\0';
			if (nameOf(static_cast<uint16_t>(i), name, guid) != 0)
				continue;
			std::string id;
			if (i > 0)
			{
				id = guid[0] != '\0' ? guid : name;
				if (const int repeat = ++seen[id]; repeat > 1)
					id += " (" + std::to_string(repeat) + ")";
			}
			out.push_back({ static_cast<uint16_t>(i), id, name });
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

	int FindAudioDevice(const std::vector<AudioDeviceEntry>& devices, const std::string& id)
	{
		for (const auto& device : devices)
			if (device.id == id)
				return device.index;
		return -1;
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

	// The platform audio device module as the engine's voice pipeline sees
	// it. libwebrtc records only while a microphone stream sends and stops
	// the module when the last one goes; the microphone test needs meters
	// without a call, so the module records while either wants it.
	// Everything else is passed through. Called on the worker thread only.
	class MonitoringAudioDevice : public webrtc::AudioDeviceModule
	{
	public:
		explicit MonitoringAudioDevice(webrtc::scoped_refptr<webrtc::AudioDeviceModule> inner)
		  : inner(std::move(inner))
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
		int32_t RegisterAudioCallback(webrtc::AudioTransport* callback) override
		{
			return inner->RegisterAudioCallback(callback);
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
		int32_t SetPlayoutDevice(WindowsDeviceType device) override
		{
			return inner->SetPlayoutDevice(device);
		}
		int32_t SetRecordingDevice(uint16_t index) override
		{
			return inner->SetRecordingDevice(index);
		}
		int32_t SetRecordingDevice(WindowsDeviceType device) override
		{
			return inner->SetRecordingDevice(device);
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
		bool BuiltInAECIsAvailable() const override
		{
			return inner->BuiltInAECIsAvailable();
		}
		bool BuiltInAGCIsAvailable() const override
		{
			return inner->BuiltInAGCIsAvailable();
		}
		bool BuiltInNSIsAvailable() const override
		{
			return inner->BuiltInNSIsAvailable();
		}
		int32_t EnableBuiltInAEC(bool enable) override
		{
			return inner->EnableBuiltInAEC(enable);
		}
		int32_t EnableBuiltInAGC(bool enable) override
		{
			return inner->EnableBuiltInAGC(enable);
		}
		int32_t EnableBuiltInNS(bool enable) override
		{
			return inner->EnableBuiltInNS(enable);
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

	// libwebrtc's software encoders, with H264 moved to a hardware encoder
	// (GStreamer: VA-API/NVENC) when the system has one. Hardware H264 runs
	// per simulcast layer behind SimulcastEncoderAdapter, which also falls
	// back to OpenH264 when the hardware encoder fails.
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
			if (hardware && lower(format.name) == "h264")
				return std::make_unique<webrtc::SimulcastEncoderAdapter>(env, hardware.get(), &software, format);
			return software.Create(env, format);
		}

	private:
		SoftwareEncoderFactory software;
		std::unique_ptr<webrtc::VideoEncoderFactory> hardware;
	};

	NoopProducerListener producerListener;
	NoopConsumerListener consumerListener;

	std::once_flag initialized;
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
	// Null for the microphone.
	webrtc::scoped_refptr<gelabber::LocalVideoSource> video;
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
		// Stereo capture only in original mode; the module records stereo
		// when the device can, and the APM downmixes unless told otherwise.
		auto config                            = engine.apm->GetConfig();
		config.pipeline.multi_channel_capture = mode == "original";
		engine.apm->ApplyConfig(config);
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
			engine->monitor = webrtc::make_ref_counted<MonitoringAudioDevice>(engine->adm);
			adm             = engine->monitor;
		}

		engine->dsp = std::make_shared<gelabber::CaptureDsp>();
		engine->apm = webrtc::BuiltinAudioProcessingBuilder()
		                .SetCapturePostProcessing(std::make_unique<CapturePostProcessor>(engine->dsp))
		                .Build(webrtc::CreateEnvironment(std::make_unique<ApmFieldTrials>()));
		if (!engine->apm)
			throw std::runtime_error("failed to create audio processing");

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
				const auto id   = options["input"].get<std::string>();
				const int index = FindAudioDevice(RecordingDevices(adm), id);
				if (index < 0)
				{
					error = "unknown input device " + id;
					return;
				}
				const bool running = adm.Recording();
				if (running)
					adm.StopRecording();
				if (adm.SetRecordingDevice(static_cast<uint16_t>(index)) != 0)
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
				const auto id   = options["output"].get<std::string>();
				const int index = FindAudioDevice(PlayoutDevices(adm), id);
				if (index < 0)
				{
					error = "unknown output device " + id;
					return;
				}
				const bool running = adm.Playing();
				if (running)
					adm.StopPlayout();
				if (adm.SetPlayoutDevice(static_cast<uint16_t>(index)) != 0)
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

		auto source = engine->factory->CreateAudioSource(audio);
		if (!source)
			throw std::runtime_error("failed to create audio source");
		auto track = engine->factory->CreateAudioTrack(engine->TrackId("mic"), source.get());
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

char* gm_source_state(gm_source* source)
{
	return guarded<char*>(nullptr, [&] {
		return dupString(source->video ? source->video->StateJson() : std::string(R"({"state":"live"})"));
	});
}

void gm_source_free(gm_source* source)
{
	guarded<int>(0, [&] {
		if (!source)
			return 0;
		if (source->video)
			source->video->Stop();
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

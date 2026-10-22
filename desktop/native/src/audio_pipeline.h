// The engine's audio path between the device module and the voice engine:
// the APM post-processor and field trials, playout level meters, the
// microphone sources and their hub, the capture transport and the device
// module wrapper that keeps capture running for the microphone test.
// Header-only, included by gelabber_media.cc.
#ifndef GELABBER_AUDIO_PIPELINE_H
#define GELABBER_AUDIO_PIPELINE_H

#include "audio_devices.h"
#include "capture_dsp.h"
#include "playout_limiter.h"

#include <api/audio/audio_device.h>
#include <api/audio/audio_frame.h>
#include <api/audio/audio_processing.h>
#include <api/field_trials_view.h>
#include <api/media_stream_interface.h>
#include <api/notifier.h>
#include <audio/utility/audio_frame_operations.h>
#include <modules/audio_processing/audio_buffer.h>
#include <modules/audio_processing/include/audio_frame_proxies.h>
#include <rtc_base/logging.h>

#include <algorithm>
#include <atomic>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <memory>
#include <mutex>
#include <optional>
#include <string>
#include <vector>

namespace gelabber
{
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

	inline MicrophoneSource::MicrophoneSource(
	  webrtc::AudioOptions options, size_t channels, std::shared_ptr<MicrophoneHub> hub)
	  : PushAudioSource(std::move(options)), channels(channels), hub(std::move(hub))
	{
		this->hub->Add(this);
	}

	inline MicrophoneSource::~MicrophoneSource()
	{
		hub->Remove(this);
	}

	// Between the audio device module and the voice engine. Capture runs
	// through the APM (echo cancellation against the playout, noise
	// suppression, the post-processor) here and goes to the microphone
	// sources only, instead of to every send stream. Playout comes from the
	// engine's own transport, which also feeds the APM's echo reference, and
	// leaves through the playout limiter: the master gain of volumes above
	// 100 % applies there, after the echo reference was taken.
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
			{
				const auto result = transport->NeedMorePlayData(
				  samplesPerChannel,
				  bytesPerSample,
				  channels,
				  sampleRate,
				  audio,
				  samplesOut,
				  elapsedTimeMs,
				  ntpTimeMs);
				// bytesPerSample is per frame, as for capture.
				if (result == 0 && bytesPerSample == channels * sizeof(int16_t))
					playout.Process(
					  static_cast<int16_t*>(audio), std::min(samplesOut, samplesPerChannel), channels, sampleRate);
				return result;
			}
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

		gelabber::PlayoutLimiter& Playout()
		{
			return playout;
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
		gelabber::PlayoutLimiter playout;
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

		// Whether capture should run, for a stream or for the test. The
		// module itself may be stopped after a restart that failed.
		bool CaptureWanted() const
		{
			return streamsRecording || monitoring;
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
} // namespace gelabber

#endif

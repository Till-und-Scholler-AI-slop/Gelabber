// Microphone processing after libwebrtc's APM: RNNoise ("enhanced" mode),
// input gain and level meters. Runs on the audio capture thread as the
// APM's capture post-processor; settings and levels are atomics so other
// threads can change and read them.
#pragma once

#include <atomic>
#include <cstdint>
#include <vector>

struct DenoiseState;

namespace gelabber
{
	struct CaptureLevels
	{
		// 0..100 like the web client's meter: min(100, round(RMS * 350)).
		int raw{ 0 };
		int processed{ 0 };
		bool clipping{ false };
		// RNNoise ran on the last block (needs 48 kHz, 10 ms blocks).
		bool denoised{ false };
		// Blocks processed since the engine started, and the channel count
		// of the last one: lets callers tell live meters from stale ones.
		uint64_t blocks{ 0 };
		int channels{ 0 };
	};

	class CaptureDsp
	{
	public:
		CaptureDsp();
		~CaptureDsp();
		CaptureDsp(const CaptureDsp&)            = delete;
		CaptureDsp& operator=(const CaptureDsp&) = delete;

		void SetDenoise(bool enabled);
		bool Denoise() const;
		// Linear, clamped to 0..2 (the web client's "Mic-Gain").
		void SetGain(float gain);

		void Initialize(int sampleRateHz, int numChannels);
		// One block per call; samples in libwebrtc's FloatS16 scale
		// (-32768..32767), processed in place.
		void Process(float* const* channels, int numChannels, int numFrames);

		CaptureLevels Levels() const;

	private:
		void Publish();

		std::atomic<bool> denoise{ false };
		std::atomic<float> gain{ 1.0f };
		std::atomic<int> sampleRate{ 0 };

		// Capture thread only.
		std::vector<DenoiseState*> states;
		std::vector<float> scratch;
		double rawSum{ 0 };
		double processedSum{ 0 };
		uint64_t samples{ 0 };
		int windowFrames{ 0 };
		int windowTarget{ 0 };
		bool windowClipping{ false };

		std::atomic<int> rawLevel{ 0 };
		std::atomic<int> processedLevel{ 0 };
		std::atomic<bool> clipping{ false };
		std::atomic<bool> denoised{ false };
		std::atomic<uint64_t> blockCount{ 0 };
		std::atomic<int> lastChannels{ 0 };
	};
} // namespace gelabber

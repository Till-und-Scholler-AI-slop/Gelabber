// Microphone processing after libwebrtc's APM: RNNoise ("enhanced" mode),
// DeepFilterNet3 beside it ("strong" mode, strong_filter.h), input gain,
// the input sensitivity gate (input_gate.h) and level meters. Runs on the audio capture thread as the
// APM's capture post-processor; settings and levels are atomics so other
// threads can change and read them.
#pragma once

#include "input_gate.h"
#include "strong_filter.h"

#include <atomic>
#include <cstdint>
#include <string>
#include <vector>

struct DenoiseState;

namespace gelabber
{
	// The microphone's noise filter: none, RNNoise ("enhanced"), or
	// DeepFilterNet3 with RNNoise as its fallback and the gate's speech
	// detector ("strong").
	enum class NoiseFilter
	{
		Off,
		Standard,
		Strong,
	};

	inline const char* NoiseFilterName(NoiseFilter filter)
	{
		switch (filter)
		{
			case NoiseFilter::Standard:
				return "standard";
			case NoiseFilter::Strong:
				return "strong";
			default:
				return "off";
		}
	}

	struct CaptureLevels
	{
		// 0..100 like the web client's meter: min(100, round(RMS * 350)).
		int raw{ 0 };
		int processed{ 0 };
		bool clipping{ false };
		// RNNoise ran on the last block (needs 48 kHz, 10 ms blocks).
		bool denoised{ false };
		// Whose output the last block carries: RNNoise's ("standard") also
		// while the strong filter loads or after it fell back.
		NoiseFilter filter{ NoiseFilter::Off };
		// The strong filter's slot, its error and its time per block.
		StrongState strong{ StrongState::Off };
		std::string strongError;
		float strongMs{ 0.0f };
		float strongMaxMs{ 0.0f };
		// Blocks processed since the engine started, and the channel count
		// of the last one: lets callers tell live meters from stale ones.
		uint64_t blocks{ 0 };
		int channels{ 0 };
		// The gate's last block.
		GateReport gate;
	};

	class CaptureDsp
	{
	public:
		// `strongApi`: the strong filter's functions, or null without one.
		explicit CaptureDsp(const StrongFilterApi* strongApi = nullptr);
		~CaptureDsp();
		CaptureDsp(const CaptureDsp&)            = delete;
		CaptureDsp& operator=(const CaptureDsp&) = delete;

		// Strong starts building DeepFilterNet3 off this thread; until it
		// is ready the capture gets RNNoise's output.
		void SetNoiseFilter(NoiseFilter filter);
		// Linear, clamped to 0..2 (the web client's "Mic-Gain").
		void SetGain(float gain);
		float Gain() const;
		// Input sensitivity; off until set.
		void SetGate(GateMode mode, float thresholdDb);

		void Initialize(int sampleRateHz, int numChannels);
		// One block per call; samples in libwebrtc's FloatS16 scale
		// (-32768..32767), processed in place.
		void Process(float* const* channels, int numChannels, int numFrames);

		CaptureLevels Levels() const;

	private:
		void Publish();

		std::atomic<NoiseFilter> noiseFilter{ NoiseFilter::Off };
		std::atomic<float> gain{ 1.0f };
		std::atomic<int> sampleRate{ 0 };

		// Capture thread only.
		std::vector<DenoiseState*> states;
		std::vector<float> scratch;
		std::vector<float> unfiltered;
		// RNNoise's speech probability of the block before: DeepFilterNet3's
		// output is one block later than RNNoise's.
		float previousSpeech{ 0.0f };
		InputGate gate;
		StrongFilter strong;
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
		std::atomic<NoiseFilter> filterUsed{ NoiseFilter::Off };
		std::atomic<uint64_t> blockCount{ 0 };
		std::atomic<int> lastChannels{ 0 };
	};
} // namespace gelabber

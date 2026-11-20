// Input sensitivity (GEL-27): a noise gate after RNNoise and the input gain,
// kept free of libwebrtc so that it also runs in a plain test
// (../tests/input_gate_test.cc).
//
// The web client runs the same gate in web/public/audio/rnnoise-worklet.js
// (`InputGate`). Keep the numbers and the steps of `Process` the same in
// both: a 10 ms block's RMS in dB of full scale is compared with a
// threshold, the gate opens above it and closes after a hold time below it,
// and fades in or out so that nothing clicks. Closed, it sends digital
// silence, which Opus DTX then leaves out.
//
// The threshold is the user's (manual), or found from the room's noise floor
// (automatic). With RNNoise running, automatic also needs RNNoise's speech
// probability for the block, which keeps key clicks and other short noise
// out even when they are loud.
//
// The gate opens only after kOpenBlocks blocks in a row speak (or are above
// the threshold): a key click lasts one or two. So that the first of those
// blocks is not lost, the output runs kLookaheadBlocks behind (20 ms); off,
// it runs without delay.
#ifndef GELABBER_INPUT_GATE_H
#define GELABBER_INPUT_GATE_H

#include <algorithm>
#include <atomic>
#include <cmath>
#include <cstddef>
#include <string>
#include <vector>

namespace gelabber
{
	enum class GateMode
	{
		Off,
		Auto,
		Manual,
	};

	inline const char* GateModeName(GateMode mode)
	{
		switch (mode)
		{
			case GateMode::Auto:
				return "auto";
			case GateMode::Manual:
				return "manual";
			default:
				return "off";
		}
	}

	// "auto" or "manual"; anything else is off.
	inline GateMode ParseGateMode(const std::string& name)
	{
		if (name == "auto")
			return GateMode::Auto;
		if (name == "manual")
			return GateMode::Manual;
		return GateMode::Off;
	}

	struct GateReport
	{
		GateMode mode{ GateMode::Off };
		float levelDb{ -100.0f };
		float thresholdDb{ -60.0f };
		bool open{ true };
	};

	class InputGate
	{
	public:
		static constexpr int kOpenBlocks        = 3;
		static constexpr int kLookaheadBlocks   = 2;
		static constexpr float kHoldMs          = 250.0f;
		static constexpr float kHysteresisDb    = 3.0f;
		static constexpr float kAttackMs        = 5.0f;
		static constexpr float kReleaseMs       = 40.0f;
		static constexpr float kSilenceDb       = -100.0f;
		static constexpr float kFloorMinDb      = -90.0f;
		static constexpr float kFloorMaxDb      = -30.0f;
		static constexpr float kFloorRiseDbPerS = 1.5f;
		// Automatic, from the level alone ("Browser filters").
		static constexpr float kLevelMarginDb = 14.0f;
		static constexpr float kLevelMinDb    = -60.0f;
		static constexpr float kLevelMaxDb    = -30.0f;
		// Automatic with RNNoise's speech probability ("Enhanced voice").
		static constexpr float kSpeechMarginDb = 6.0f;
		static constexpr float kSpeechMinDb    = -70.0f;
		static constexpr float kSpeechMaxDb    = -40.0f;
		static constexpr float kSpeechOpen     = 0.6f;
		// The manual threshold's range.
		static constexpr float kManualMinDb = -80.0f;
		static constexpr float kManualMaxDb = 0.0f;

		// Any thread.
		void Configure(GateMode next, float thresholdDb)
		{
			manualDb.store(std::clamp(std::isfinite(thresholdDb) ? thresholdDb : -50.0f, kManualMinDb,
			                          kManualMaxDb));
			mode.store(next);
		}

		GateReport Report() const
		{
			GateReport report;
			report.mode        = mode.load();
			report.levelDb     = reportLevel.load();
			report.thresholdDb = reportThreshold.load();
			report.open        = reportOpen.load();
			return report;
		}

		// Capture thread: a new stream starts from scratch.
		void Reset(int sampleRateHz)
		{
			rate    = std::max(sampleRateHz, 1);
			floorDb = kFloorMaxDb;
			open    = true;
			holdMs  = 0.0f;
			amount  = 1.0f;
			run     = 0;
			delay.clear();
			slot   = 0;
			bypass = false;
		}

		// Capture thread: one block in, the one kLookaheadBlocks before it
		// out (off: the same block, unchanged), every channel in place. `fullScale` is the samples' full scale
		// (32768 for libwebrtc's FloatS16), `speech` RNNoise's probability
		// for the block or a negative value without it.
		void Process(float* const* channels, int numChannels, int numFrames, float fullScale, float speech)
		{
			if (numChannels <= 0 || numFrames <= 0)
				return;
			const GateMode now   = mode.load();
			const float blockMs  = 1000.0f * static_cast<float>(numFrames) / static_cast<float>(rate);
			const float attack   = 1000.0f / (kAttackMs * static_cast<float>(rate));
			const float release  = 1000.0f / (kReleaseMs * static_cast<float>(rate));
			double sum           = 0.0;
			for (int c = 0; c < numChannels; ++c)
				for (int i = 0; i < numFrames; ++i)
				{
					const double sample = channels[c][i] / fullScale;
					sum += sample * sample;
				}
			const double rms = std::sqrt(sum / (static_cast<double>(numFrames) * numChannels));
			const float level =
			  rms > 0 ? std::max(kSilenceDb, static_cast<float>(20.0 * std::log10(rms))) : kSilenceDb;
			// The floor falls to any quieter block at once and rises slowly,
			// so pauses between words keep it at the room's noise.
			floorDb = level < floorDb ? std::max(level, kFloorMinDb)
			                          : std::min(floorDb + kFloorRiseDbPerS * blockMs / 1000.0f, kFloorMaxDb);
			const bool useSpeech = now == GateMode::Auto && speech >= 0.0f;
			const float threshold =
			  now == GateMode::Manual ? manualDb.load()
			  : useSpeech             ? std::clamp(floorDb + kSpeechMarginDb, kSpeechMinDb, kSpeechMaxDb)
			                          : std::clamp(floorDb + kLevelMarginDb, kLevelMinDb, kLevelMaxDb);
			if (now == GateMode::Off)
			{
				// Off (Original, push-to-talk) costs no look-ahead: the block
				// leaves as it came. Switched on, the gate starts from an
				// empty delay.
				open   = true;
				run    = 0;
				amount = 1.0f;
				if (!bypass)
				{
					bypass = true;
					std::fill(delay.begin(), delay.end(), 0.0f);
				}
				reportLevel.store(level);
				reportThreshold.store(threshold);
				reportOpen.store(true);
				return;
			}
			bypass = false;
			const bool opens = level >= threshold && (!useSpeech || speech >= kSpeechOpen);
			// With speech probabilities, only speech keeps it open: a key
			// click in a pause must not stretch the hold.
			const bool stays = useSpeech ? opens : level >= threshold - kHysteresisDb;
			run              = opens ? run + 1 : 0;
			if (run >= kOpenBlocks || (open && stays))
			{
				open   = true;
				holdMs = kHoldMs;
			}
			else if (open)
			{
				holdMs -= blockMs;
				if (holdMs <= 0.0f)
					open = false;
			}
			// The newest block waits; the oldest one leaves, gated. A new
			// block size or channel count starts the delay over (silence).
			const size_t slots = static_cast<size_t>(kLookaheadBlocks) + 1;
			const size_t size  = static_cast<size_t>(numChannels) * static_cast<size_t>(numFrames);
			if (delay.size() != slots * size)
			{
				delay.assign(slots * size, 0.0f);
				slot = 0;
			}
			for (int c = 0; c < numChannels; ++c)
				std::copy(channels[c], channels[c] + numFrames,
				          delay.begin() + static_cast<std::ptrdiff_t>(slot * size + c * static_cast<size_t>(numFrames)));
			slot = (slot + 1) % slots;
			for (int c = 0; c < numChannels; ++c)
			{
				const auto from = delay.begin() + static_cast<std::ptrdiff_t>(slot * size + c * static_cast<size_t>(numFrames));
				std::copy(from, from + numFrames, channels[c]);
			}
			const float target = open ? 1.0f : 0.0f;
			if (target != 1.0f || amount != 1.0f)
				for (int i = 0; i < numFrames; ++i)
				{
					amount = target > amount ? std::min(1.0f, amount + attack) : std::max(0.0f, amount - release);
					for (int c = 0; c < numChannels; ++c)
						channels[c][i] = amount == 0.0f ? 0.0f : channels[c][i] * amount;
				}
			reportLevel.store(level);
			reportThreshold.store(threshold);
			reportOpen.store(open);
		}

	private:
		std::atomic<GateMode> mode{ GateMode::Off };
		std::atomic<float> manualDb{ -50.0f };

		// Capture thread only.
		int rate{ 48000 };
		float floorDb{ kFloorMaxDb };
		bool open{ true };
		float holdMs{ 0.0f };
		float amount{ 1.0f };
		int run{ 0 };
		std::vector<float> delay;
		size_t slot{ 0 };
		bool bypass{ false };

		std::atomic<float> reportLevel{ kSilenceDb };
		std::atomic<float> reportThreshold{ kLevelMinDb };
		std::atomic<bool> reportOpen{ true };
	};
} // namespace gelabber

#endif

#include "capture_dsp.h"

#include <rnnoise.h>

#include <algorithm>
#include <cmath>

namespace gelabber
{
	namespace
	{
		constexpr int kRnnoiseRate = 48000;
		constexpr float kFullScale = 32768.0f;
		// Same window as the web client's meter update (80 ms).
		constexpr int kWindowMs = 80;

		int MeterLevel(double sumSquares, uint64_t count)
		{
			if (count == 0)
				return 0;
			const double rms = std::sqrt(sumSquares / static_cast<double>(count)) / kFullScale;
			return static_cast<int>(std::min(100.0, std::round(rms * 350.0)));
		}
	} // namespace

	CaptureDsp::CaptureDsp(const StrongFilterApi* strongApi) : strong(strongApi)
	{
	}

	CaptureDsp::~CaptureDsp()
	{
		for (auto* state : states)
			rnnoise_destroy(state);
	}

	void CaptureDsp::SetNoiseFilter(NoiseFilter filter)
	{
		noiseFilter.store(filter);
		strong.Select(filter == NoiseFilter::Strong);
	}

	void CaptureDsp::SetGain(float value)
	{
		gain.store(std::clamp(std::isfinite(value) ? value : 1.0f, 0.0f, 2.0f));
	}

	float CaptureDsp::Gain() const
	{
		return gain.load();
	}

	void CaptureDsp::SetGate(GateMode mode, float thresholdDb)
	{
		gate.Configure(mode, thresholdDb);
	}

	void CaptureDsp::Initialize(int sampleRateHz, int numChannels)
	{
		sampleRate.store(sampleRateHz);
		// Fresh RNNoise state per stream: its history belongs to the old one.
		for (auto* state : states)
			rnnoise_destroy(state);
		states.clear();
		states.resize(static_cast<size_t>(std::max(numChannels, 1)), nullptr);
		rawSum = processedSum = 0;
		samples               = 0;
		windowFrames          = 0;
		windowTarget          = std::max(1, sampleRateHz * kWindowMs / 1000);
		windowClipping        = false;
		previousSpeech        = 0.0f;
		gate.Reset(sampleRateHz);
		strong.Restart();
	}

	void CaptureDsp::Process(float* const* channels, int numChannels, int numFrames)
	{
		if (numChannels <= 0 || numFrames <= 0)
			return;
		if (static_cast<int>(states.size()) < numChannels)
			states.resize(static_cast<size_t>(numChannels), nullptr);

		for (int c = 0; c < numChannels; ++c)
			for (int i = 0; i < numFrames; ++i)
				rawSum += static_cast<double>(channels[c][i]) * channels[c][i];

		// RNNoise takes 480-sample frames at 48 kHz, which is exactly one
		// APM block; other rates pass through (the APM resamples mics to
		// 48 kHz unless the device itself is slower).
		const auto mode       = noiseFilter.load();
		const bool canDenoise = mode != NoiseFilter::Off && sampleRate.load() == kRnnoiseRate &&
		                        numFrames == rnnoise_get_frame_size();
		// DeepFilterNet3 on the APM's mono output, from the same input as
		// RNNoise, which keeps running for the gate and as the fallback.
		const bool tryStrong = canDenoise && mode == NoiseFilter::Strong && numChannels == 1;
		if (tryStrong)
			unfiltered.assign(channels[0], channels[0] + numFrames);
		// RNNoise's speech probability for the gate: the most likely channel.
		float speech = -1.0f;
		if (canDenoise)
		{
			scratch.resize(static_cast<size_t>(numFrames));
			for (int c = 0; c < numChannels; ++c)
			{
				if (!states[c])
					states[c] = rnnoise_create(nullptr);
				if (!states[c])
					continue;
				std::copy(channels[c], channels[c] + numFrames, scratch.begin());
				speech = std::max(speech, rnnoise_process_frame(states[c], channels[c], scratch.data()));
			}
		}
		const bool strongUsed = tryStrong && strong.Process(unfiltered.data(), numFrames, sampleRate.load());
		if (strongUsed)
			std::copy(unfiltered.begin(), unfiltered.end(), channels[0]);
		// The gate gets the probability of the block it gates: with
		// DeepFilterNet3's output RNNoise's from the block before, as
		// docs/v0.13-deepfilternet/gate.mjs measured it.
		if (canDenoise)
		{
			const float current = speech;
			if (strongUsed)
				speech = previousSpeech;
			previousSpeech = current;
		}
		filterUsed.store(strongUsed ? NoiseFilter::Strong : canDenoise ? NoiseFilter::Standard : NoiseFilter::Off);
		denoised.store(canDenoise);
		lastChannels.store(numChannels);
		blockCount.fetch_add(1);

		const float g = gain.load();
		if (g != 1.0f)
			for (int c = 0; c < numChannels; ++c)
				for (int i = 0; i < numFrames; ++i)
					channels[c][i] = std::clamp(channels[c][i] * g, -kFullScale, kFullScale - 1.0f);
		// Clipping is the gain's, before the gate fades it away.
		for (int c = 0; c < numChannels; ++c)
			for (int i = 0; i < numFrames; ++i)
				if (std::fabs(channels[c][i]) >= 0.99f * kFullScale)
					windowClipping = true;
		gate.Process(channels, numChannels, numFrames, kFullScale, speech);
		for (int c = 0; c < numChannels; ++c)
			for (int i = 0; i < numFrames; ++i)
				processedSum += static_cast<double>(channels[c][i]) * channels[c][i];

		samples += static_cast<uint64_t>(numFrames) * numChannels;
		windowFrames += numFrames;
		if (windowFrames >= windowTarget)
			Publish();
	}

	void CaptureDsp::Publish()
	{
		rawLevel.store(MeterLevel(rawSum, samples));
		processedLevel.store(MeterLevel(processedSum, samples));
		clipping.store(windowClipping);
		rawSum = processedSum = 0;
		samples               = 0;
		windowFrames          = 0;
		windowClipping        = false;
	}

	CaptureLevels CaptureDsp::Levels() const
	{
		CaptureLevels levels;
		levels.raw         = rawLevel.load();
		levels.processed   = processedLevel.load();
		levels.clipping    = clipping.load();
		levels.denoised    = denoised.load();
		levels.filter      = filterUsed.load();
		levels.strong      = strong.State();
		levels.strongError = strong.Error();
		levels.strongMs    = strong.BusyMs();
		levels.blocks      = blockCount.load();
		levels.channels    = lastChannels.load();
		levels.gate        = gate.Report();
		return levels;
	}
} // namespace gelabber

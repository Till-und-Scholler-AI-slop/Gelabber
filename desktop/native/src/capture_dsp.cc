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

	CaptureDsp::CaptureDsp() = default;

	CaptureDsp::~CaptureDsp()
	{
		for (auto* state : states)
			rnnoise_destroy(state);
	}

	void CaptureDsp::SetDenoise(bool enabled)
	{
		denoise.store(enabled);
	}

	bool CaptureDsp::Denoise() const
	{
		return denoise.load();
	}

	void CaptureDsp::SetGain(float value)
	{
		gain.store(std::clamp(std::isfinite(value) ? value : 1.0f, 0.0f, 2.0f));
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
		const bool canDenoise = denoise.load() && sampleRate.load() == kRnnoiseRate &&
		                        numFrames == rnnoise_get_frame_size();
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
				rnnoise_process_frame(states[c], channels[c], scratch.data());
			}
		}
		denoised.store(canDenoise);
		channels.store(numChannels);
		blocks.fetch_add(1);

		const float g = gain.load();
		for (int c = 0; c < numChannels; ++c)
		{
			float* data = channels[c];
			for (int i = 0; i < numFrames; ++i)
			{
				float sample = data[i];
				if (g != 1.0f)
				{
					sample  = std::clamp(sample * g, -kFullScale, kFullScale - 1.0f);
					data[i] = sample;
				}
				processedSum += static_cast<double>(sample) * sample;
				if (std::fabs(sample) >= 0.99f * kFullScale)
					windowClipping = true;
			}
		}

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
		levels.raw       = rawLevel.load();
		levels.processed = processedLevel.load();
		levels.clipping  = clipping.load();
		levels.denoised  = denoised.load();
		levels.blocks    = blocks.load();
		levels.channels  = channels.load();
		return levels;
	}
} // namespace gelabber

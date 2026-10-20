// Playback above 100 % (issue #223), kept free of libwebrtc so that it also
// runs in a plain test (../tests/playout_limiter_test.cc).
//
// libwebrtc scales each receive stream by its volume with saturation, so a
// loud speaker at 200 % clips hard on its own. Instead, no stream is scaled
// above 1: the largest volume above 1 becomes a master gain that applies to
// the mix as it leaves for the sound card, followed by this limiter. The mix
// is linear, so every consumer still plays at its own volume and only the
// sum can reach the limiter.
//
// The limiter looks ahead by a millisecond and a half: the output is that
// much behind the mix, at every volume, so turning the master gain on or off
// never skips or repeats samples. Each frame needs the gain that keeps its
// loudest channel at the ceiling; the smallest of those over the look-ahead
// window, smoothed over the same window, is in place by the time that frame
// leaves. Recovery is slower (kReleaseSeconds). At a master gain of 1 the
// ceiling is full scale itself, where an int16 mix cannot go beyond: the
// samples pass unchanged, only later.
#ifndef GELABBER_PLAYOUT_LIMITER_H
#define GELABBER_PLAYOUT_LIMITER_H

#include <algorithm>
#include <atomic>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <vector>

namespace gelabber
{
	struct PlayoutLevels
	{
		// The master gain asked for (1 at or below 100 %).
		float gain{ 1.0f };
		// Of the last window (100 ms): the loudest output sample as a
		// fraction of full scale, and the limiter's smallest gain (1 when it
		// did nothing).
		float peak{ 0.0f };
		float reduction{ 1.0f };
		// Windows measured so far: lets callers tell fresh levels from stale.
		uint64_t windows{ 0 };
	};

	class PlayoutLimiter
	{
	public:
		// -0.3 dBFS: room for resampling after the mix.
		static constexpr float kCeiling         = 0.966f;
		static constexpr double kLookAheadSeconds = 0.0015;
		static constexpr double kReleaseSeconds   = 0.1;
		// The largest layout the state is reserved for when it is made, so
		// a format change on the render thread allocates nothing. Beyond it
		// the state grows there once.
		static constexpr uint32_t kMaxReservedRate   = 192000;
		static constexpr size_t kMaxReservedChannels = 8;

		// Allocates; not on the render thread.
		PlayoutLimiter()
		{
			const size_t frames = WindowFrames(kMaxReservedRate);
			delay.reserve(frames * kMaxReservedChannels);
			needed.reserve(frames);
			smoothed.reserve(frames);
		}

		// Any thread. Clamped to 1..2. Takes effect in one step at the next
		// block, like the streams' new volumes over it, which are set
		// before it: a ramp here would let the other streams swell or dip
		// while it runs.
		void SetGain(double gain)
		{
			target.store(static_cast<float>(std::clamp(gain, 1.0, 2.0)));
		}

		float Gain() const
		{
			return target.load();
		}

		// The render thread: `frames` frames of interleaved int16 samples, in
		// place.
		void Process(int16_t* samples, size_t frames, size_t channels, uint32_t sampleRate)
		{
			if (!samples || channels == 0 || sampleRate == 0)
				return;
			if (channels != layoutChannels || sampleRate != layoutRate)
				Reset(channels, sampleRate);
			const float gain    = target.load();
			const float ceiling = gain > 1.0f ? kCeiling : 1.0f;
			for (size_t frame = 0; frame < frames; ++frame)
			{
				int16_t* values = samples + frame * channels;

				// In: the frame at the master gain, and the limiter gain it
				// needs.
				float* in     = delay.data() + head * channels;
				float loudest = 0.0f;
				for (size_t c = 0; c < channels; ++c)
				{
					in[c]   = values[c] / 32768.0f * gain;
					loudest = std::max(loudest, std::fabs(in[c]));
				}
				needed[head] = loudest > ceiling ? ceiling / loudest : 1.0f;
				float least  = 1.0f;
				for (float value : needed)
					least = std::min(least, value);
				// Recovers towards `least`, never above it, and arrives.
				held = least < held ? least : held + (least - held) * releaseStep;
				if (least - held < 1e-4)
					held = least;
				const auto kept = static_cast<float>(held);
				sum += kept - smoothed[head];
				smoothed[head] = kept;
				// The oldest frame leaves now; its slot takes the next one.
				// Without look-ahead that is the frame just in.
				head = (head + 1) % window;
				if (head == 0)
				{
					// Against rounding drift; exact again at a gain of 1.
					sum = 0.0;
					for (float value : smoothed)
						sum += value;
				}
				const float applied =
				  std::min(1.0f, static_cast<float>(sum / static_cast<double>(window)));

				// Out: the frame from a look-ahead ago, at the smallest gain
				// any frame since needed.
				const float* leaving = delay.data() + head * channels;
				for (size_t c = 0; c < channels; ++c)
				{
					const float value = leaving[c] * applied;
					values[c]         = static_cast<int16_t>(
					  std::clamp(std::lround(value * 32768.0f), -32768L, 32767L));
					windowPeak = std::max(windowPeak, std::fabs(value));
				}
				windowReduction = std::min(windowReduction, applied);
				if (++windowFrames >= windowTarget)
					Publish();
			}
		}

		// Any thread.
		PlayoutLevels Levels() const
		{
			PlayoutLevels levels;
			levels.gain      = target.load();
			levels.peak      = peak.load();
			levels.reduction = reduction.load();
			levels.windows   = windows.load();
			return levels;
		}

		// Frames between a frame's arrival and its departure.
		size_t DelayFrames() const
		{
			return window - 1;
		}

		// For tests: where the look-ahead's samples are kept.
		const float* DelayStorage() const
		{
			return delay.data();
		}

	private:
		// Frames in the look-ahead window at `rate`, the newest included.
		static size_t WindowFrames(uint32_t rate)
		{
			return static_cast<size_t>(std::lround(rate * kLookAheadSeconds)) + 1;
		}

		// Within the reserved capacity: no allocation.
		void Reset(size_t newChannels, uint32_t newRate)
		{
			layoutChannels  = newChannels;
			layoutRate      = newRate;
			window          = WindowFrames(newRate);
			delay.assign(window * newChannels, 0.0f);
			needed.assign(window, 1.0f);
			smoothed.assign(window, 1.0f);
			sum             = static_cast<double>(window);
			held            = 1.0;
			head            = 0;
			releaseStep     = 1.0 - std::exp(-1.0 / (kReleaseSeconds * newRate));
			windowTarget    = std::max<size_t>(1, newRate / 10);
			windowFrames    = 0;
			windowPeak      = 0.0f;
			windowReduction = 1.0f;
		}

		void Publish()
		{
			peak.store(windowPeak);
			reduction.store(windowReduction);
			windows.fetch_add(1);
			windowFrames    = 0;
			windowPeak      = 0.0f;
			windowReduction = 1.0f;
		}

		std::atomic<float> target{ 1.0f };

		// Render thread only.
		size_t layoutChannels{ 0 };
		uint32_t layoutRate{ 0 };
		// Frames in the look-ahead window, the newest included.
		size_t window{ 1 };
		// The window's frames at the master gain, and the gain each needs.
		std::vector<float> delay;
		std::vector<float> needed;
		// The held gains of the window, and their sum.
		std::vector<float> smoothed;
		double sum{ 1.0 };
		// Double: release steps this small stop short of 1 in float.
		double held{ 1.0 };
		size_t head{ 0 };
		double releaseStep{ 0.0 };
		size_t windowTarget{ 1 };
		size_t windowFrames{ 0 };
		float windowPeak{ 0.0f };
		float windowReduction{ 1.0f };

		std::atomic<float> peak{ 0.0f };
		std::atomic<float> reduction{ 1.0f };
		std::atomic<uint64_t> windows{ 0 };
	};
} // namespace gelabber

#endif

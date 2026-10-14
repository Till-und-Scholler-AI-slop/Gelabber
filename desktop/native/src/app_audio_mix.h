// Buffering and pacing of the application-sound mix (app_audio_pipewire.cc),
// kept free of the sound server so that it also runs against a simulated
// sound card (../tests/app_audio_mix_test.cc).
//
// The samples of a playback stream arrive at the pace of the sound card that
// drives its graph, a burst (the graph's quantum) at a time. The mix leaves in
// 10 ms blocks at a pace of its own. With a clock of its own as well, the two
// drift apart: a card 50 ppm slower than the system clock delivers 2.4 frames
// a second less than a mix on the system clock takes, a stream's buffer runs
// dry every few seconds, and each time a block goes out part silence. So the
// mix follows the card instead. Every stream keeps a cushion below its
// bursts; where the lowest point of the stream that has least to spare moves
// off that cushion, the blocks come a little later or sooner, by 0.3 % at
// most. Nothing is done to the samples.
//
// Streams on a second card with another clock cannot be followed as well.
// The mix keeps to the slowest; a faster one runs ahead and is cut back.
#ifndef GELABBER_APP_AUDIO_MIX_H
#define GELABBER_APP_AUDIO_MIX_H

#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <deque>
#include <optional>

namespace gelabber
{
	// The samples of one playback stream on their way into the mix:
	// interleaved, at the mix's rate. Not thread-safe.
	class MixInput
	{
	public:
		static constexpr size_t kChannels = 2;
		// 10 ms at 48 kHz.
		static constexpr size_t kBlockFrames = 480;
		// What a playing stream keeps at its lowest, just before a burst:
		// 20 ms for bursts that come late.
		static constexpr size_t kCushionFrames = 2 * kBlockFrames;

		// Samples as they arrive, `count` values.
		void Push(const float* values, size_t count)
		{
			samples.insert(samples.end(), values, values + count);
			burst = std::max(burst, count / kChannels);
			quiet = 0;
			// Far ahead: its card is faster than the one the mix follows, or
			// the mix was held up. Back to where a burst normally leaves it.
			const size_t limit = (StartFrames() + burst + 5 * kBlockFrames) * kChannels;
			if (samples.size() > limit)
			{
				const auto surplus = samples.size() - StartFrames() * kChannels;
				samples.erase(samples.begin(), samples.begin() + static_cast<std::ptrdiff_t>(surplus));
				++overruns;
			}
		}

		// The stream is gone: what it left plays out, however little.
		void End()
		{
			ended = true;
		}

		// An ended stream has played out.
		bool Done() const
		{
			return ended && samples.empty();
		}

		// Adds the stream's next block to `mix` (kBlockFrames * kChannels
		// values) if it plays. Returns how many frames its lowest point is
		// above the cushion (negative: below), by which the mix is paced;
		// nothing while it does not play (before a burst and the cushion are
		// there, and after it ran dry) and for a stream that is playing out.
		std::optional<double> Mix(float* mix)
		{
			const size_t frames = samples.size() / kChannels;
			if (!playing)
			{
				if (frames == 0)
					return std::nullopt;
				// A sound too short to fill the cushion plays once nothing
				// has come for longer than its bursts are apart.
				if (!ended && frames < StartFrames() && ++quiet <= burst / kBlockFrames + 3)
					return std::nullopt;
				playing = true;
				lows.clear();
			}
			const size_t take = std::min(kBlockFrames, frames);
			for (size_t i = 0; i < take * kChannels; ++i)
				mix[i] += samples[i];
			samples.erase(samples.begin(), samples.begin() + static_cast<std::ptrdiff_t>(take * kChannels));
			if (take < kBlockFrames)
			{
				// The application paused or ended, or a burst is later than
				// the cushion allows for: what there was is its last sound
				// until it starts over.
				playing = false;
				if (!ended)
					++underruns;
				return std::nullopt;
			}
			if (ended)
				return std::nullopt;
			// The level right after a block is taken is lowest for the block
			// before a burst: the lowest of the last blocks, over a few
			// bursts, is what the cushion is held against. (The front of
			// `lows` is that minimum; the levels behind it only grow.)
			const size_t left     = frames - take;
			const uint64_t window = std::clamp<uint64_t>(4 * (burst / kBlockFrames + 1), 16, 128);
			while (!lows.empty() && lows.back().frames >= left)
				lows.pop_back();
			lows.push_back({ blocks, left });
			while (lows.front().block + window <= blocks)
				lows.pop_front();
			++blocks;
			return static_cast<double>(lows.front().frames) - static_cast<double>(kCushionFrames);
		}

		// For a stream that starts over (its capture was replaced).
		void Clear()
		{
			samples.clear();
			lows.clear();
			playing = false;
			burst   = 0;
		}

		// Times it ran dry while it played (its application paused, or a
		// burst came too late), and times it was cut back.
		uint64_t underruns{ 0 };
		uint64_t overruns{ 0 };

	private:
		// The level a stream starts playing at, and normally has right after
		// a burst: the burst, the cushion, and the block that may be taken
		// just before the next burst.
		size_t StartFrames() const
		{
			return burst + kCushionFrames + kBlockFrames;
		}

		// The level after a block of the mix.
		struct Low
		{
			uint64_t block;
			size_t frames;
		};

		std::deque<float> samples;
		bool playing{ false };
		bool ended{ false };
		// The largest burst so far, in frames.
		size_t burst{ 0 };
		// Blocks of the mix since the last burst, while it waits to play.
		size_t quiet{ 0 };
		// Blocks taken, and the candidates for the lowest level of the last
		// ones (see Mix).
		uint64_t blocks{ 0 };
		std::deque<Low> lows;
	};

	// The time from one block of the mix to the next: 10 ms, stretched or
	// shortened by what the playing stream with the least to spare is off its
	// cushion (the smallest value MixInput::Mix returned for the block; none
	// playing: exactly 10 ms). 5 ppm a frame brings a card that is 100 ppm
	// off to rest 20 frames from the cushion, within a few seconds.
	inline std::chrono::nanoseconds MixInterval(std::optional<double> lead)
	{
		constexpr double kBlockNs = 10'000'000.0;
		constexpr double kGain    = 5e-6;
		constexpr double kMaxSkew = 3e-3;
		const double skew         = std::clamp(lead.value_or(0.0) * kGain, -kMaxSkew, kMaxSkew);
		// Ahead of the cushion: sooner.
		return std::chrono::nanoseconds(std::llround(kBlockNs * (1.0 - skew)));
	}
} // namespace gelabber

#endif

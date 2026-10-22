// The playout limiter (../src/playout_limiter.h) on generated mixes: a master
// gain of 1 leaves every sample as it was, a loud mix at 2 stays below the
// ceiling without a sample at full scale, and a quiet one plays at exactly
// twice its level. No libwebrtc, no sound card.
//
// desktop/core/tests/voice.rs builds and runs this with the host's C++
// compiler. By hand, in desktop/native:
//   c++ -std=c++20 -O2 -I src -o limiter_test tests/playout_limiter_test.cc && ./limiter_test
#include "playout_limiter.h"

#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

using gelabber::PlayoutLimiter;

namespace
{
	constexpr uint32_t kRate     = 48000;
	constexpr size_t kChannels   = 2;
	constexpr size_t kBlock      = kRate / 100;
	constexpr double kPi         = 3.14159265358979323846;
	constexpr int kCeilingSample = static_cast<int>(PlayoutLimiter::kCeiling * 32768.0f) + 1;

	int failures = 0;

	void Expect(bool holds, const std::string& what)
	{
		if (holds)
			return;
		std::printf("FAILED: %s\n", what.c_str());
		++failures;
	}

	// The same numbers with every compiler and library.
	class Random
	{
	public:
		explicit Random(uint64_t seed) : state(seed * 2 + 1)
		{
		}

		int16_t Sample()
		{
			state = state * 6364136223846793005ULL + 1442695040888963407ULL;
			return static_cast<int16_t>(state >> 48);
		}

	private:
		uint64_t state;
	};

	// Interleaved stereo: a sine of `amplitude` (of full scale) at `hz` on
	// the left, the same a quarter period on in the right.
	std::vector<int16_t> Sine(double amplitude, double hz, size_t frames, size_t from = 0)
	{
		std::vector<int16_t> out(frames * kChannels);
		for (size_t i = 0; i < frames; ++i)
		{
			const double t = static_cast<double>(from + i) / kRate;
			out[i * 2]     = static_cast<int16_t>(std::lround(amplitude * 32767.0 * std::sin(2 * kPi * hz * t)));
			out[i * 2 + 1] = static_cast<int16_t>(std::lround(amplitude * 32767.0 * std::cos(2 * kPi * hz * t)));
		}
		return out;
	}

	// Runs `in` through `limiter` in 10 ms blocks, as the device module asks.
	std::vector<int16_t> Play(PlayoutLimiter& limiter, std::vector<int16_t> in)
	{
		for (size_t at = 0; at < in.size(); at += kBlock * kChannels)
		{
			const size_t frames = std::min(kBlock, (in.size() - at) / kChannels);
			limiter.Process(in.data() + at, frames, kChannels, kRate);
		}
		return in;
	}

	void UnityPassesUnchanged()
	{
		PlayoutLimiter limiter;
		std::vector<int16_t> in(kRate * kChannels);
		Random random(1);
		for (auto& sample : in)
			sample = random.Sample();
		// Full scale both ways, which a gain of 1 must not touch either.
		in[1000] = -32768;
		in[1001] = 32767;
		const auto out     = Play(limiter, in);
		const size_t delay = limiter.DelayFrames();
		Expect(delay > 0 && delay < kRate / 100, "the look-ahead is under a block: " + std::to_string(delay));
		size_t changed = 0;
		for (size_t i = delay * kChannels; i < out.size(); ++i)
			changed += out[i] != in[i - delay * kChannels];
		Expect(changed == 0, "gain 1: " + std::to_string(changed) + " samples changed");
		for (size_t i = 0; i < delay * kChannels; ++i)
			changed += out[i] != 0;
		Expect(changed == 0, "gain 1: the look-ahead starts silent");
		const auto levels = limiter.Levels();
		Expect(levels.gain == 1.0f && levels.reduction == 1.0f, "gain 1: the limiter did nothing");
		Expect(levels.windows == 10, "gain 1: ten windows of 100 ms: " + std::to_string(levels.windows));
	}

	void LoudMixStaysBelowTheCeiling()
	{
		PlayoutLimiter limiter;
		limiter.SetGain(2.0);
		// A sine at 90 % of full scale: at twice that, every crest would clip.
		const auto out = Play(limiter, Sine(0.9, 440.0, kRate));
		int loudest    = 0;
		size_t atFull  = 0;
		for (int16_t sample : out)
		{
			loudest = std::max(loudest, std::abs(static_cast<int>(sample)));
			atFull += sample == 32767 || sample == -32768;
		}
		Expect(loudest <= kCeilingSample, "gain 2: loudest sample " + std::to_string(loudest));
		Expect(atFull == 0, "gain 2: " + std::to_string(atFull) + " samples at full scale");
		// Limited, not silenced: the crests of the last half second reach
		// close to the ceiling.
		int lateLoudest = 0;
		for (size_t i = out.size() / 2; i < out.size(); ++i)
			lateLoudest = std::max(lateLoudest, std::abs(static_cast<int>(out[i])));
		Expect(lateLoudest >= kCeilingSample * 9 / 10, "gain 2: crests reach " + std::to_string(lateLoudest));
		const auto levels = limiter.Levels();
		Expect(levels.reduction < 0.6f, "gain 2: the limiter works: " + std::to_string(levels.reduction));
		Expect(levels.peak <= PlayoutLimiter::kCeiling + 1e-4f, "gain 2: peak " + std::to_string(levels.peak));
	}

	void LoudNoiseStaysBelowTheCeiling()
	{
		PlayoutLimiter limiter;
		limiter.SetGain(2.0);
		// Full-scale noise: a new crest every few samples, the look-ahead's
		// worst case.
		std::vector<int16_t> in(kRate * kChannels);
		Random random(3);
		for (auto& sample : in)
			sample = random.Sample();
		int loudest = 0;
		for (int16_t sample : Play(limiter, in))
			loudest = std::max(loudest, std::abs(static_cast<int>(sample)));
		Expect(loudest <= kCeilingSample, "gain 2, noise: loudest sample " + std::to_string(loudest));
	}

	void QuietMixPlaysAtTheGain()
	{
		PlayoutLimiter limiter;
		limiter.SetGain(2.0);
		// A third of full scale: twice that is still below the ceiling.
		const auto in      = Sine(0.3, 440.0, kRate);
		const auto out     = Play(limiter, in);
		const size_t delay = limiter.DelayFrames();
		int worst = 0;
		for (size_t i = delay * kChannels; i < out.size(); ++i)
			worst = std::max(worst, std::abs(out[i] - 2 * in[i - delay * kChannels]));
		Expect(worst <= 1, "gain 2, quiet: off by " + std::to_string(worst));
		Expect(limiter.Levels().reduction == 1.0f, "gain 2, quiet: the limiter did nothing");
	}

	// The highest gain (400 %): a loud mix and full-scale noise still end
	// below the ceiling, and a quiet voice plays at four times its level.
	void HighestGain()
	{
		{
			PlayoutLimiter limiter;
			limiter.SetGain(PlayoutLimiter::kMaxGain);
			int loudest = 0;
			for (int16_t sample : Play(limiter, Sine(0.9, 440.0, kRate)))
				loudest = std::max(loudest, std::abs(static_cast<int>(sample)));
			Expect(loudest <= kCeilingSample, "gain 4: loudest sample " + std::to_string(loudest));
		}
		{
			PlayoutLimiter limiter;
			limiter.SetGain(PlayoutLimiter::kMaxGain);
			std::vector<int16_t> in(kRate * kChannels);
			Random random(5);
			for (auto& sample : in)
				sample = random.Sample();
			int loudest = 0;
			for (int16_t sample : Play(limiter, in))
				loudest = std::max(loudest, std::abs(static_cast<int>(sample)));
			Expect(loudest <= kCeilingSample, "gain 4, noise: loudest sample " + std::to_string(loudest));
		}
		{
			PlayoutLimiter limiter;
			limiter.SetGain(PlayoutLimiter::kMaxGain);
			// A fifth of full scale: four times that is still below the
			// ceiling.
			const auto in      = Sine(0.2, 440.0, kRate);
			const auto out     = Play(limiter, in);
			const size_t delay = limiter.DelayFrames();
			int worst          = 0;
			for (size_t i = delay * kChannels; i < out.size(); ++i)
				worst = std::max(worst, std::abs(out[i] - 4 * in[i - delay * kChannels]));
			Expect(worst <= 1, "gain 4, quiet: off by " + std::to_string(worst));
			Expect(limiter.Levels().reduction == 1.0f, "gain 4, quiet: the limiter did nothing");
		}
	}

	// Two streams, A at 200 % and B at 100 %, then A back to 100 %. The
	// mix carries each at its volume over the master gain (B at 1/2, then
	// at 1); the streams' new volumes and the new gain take effect in the
	// same block. B must not swell or dip in between.
	void VolumeChangeLeavesTheOtherStreamAlone()
	{
		PlayoutLimiter limiter;
		const auto a = Sine(0.2, 500.0, kRate);
		const auto b = Sine(0.2, 300.0, kRate);
		std::vector<int16_t> mix(a.size());
		std::vector<double> expected(a.size());
		const size_t change = kRate / 2 / kBlock * kBlock * kChannels;
		for (size_t i = 0; i < mix.size(); ++i)
		{
			const double volumeA = i < change ? 2.0 : 1.0;
			const double master  = std::max(1.0, volumeA);
			mix[i]      = static_cast<int16_t>(std::lround(a[i] * (volumeA / master) + b[i] / master));
			expected[i] = a[i] * volumeA + b[i];
		}
		for (size_t at = 0; at < mix.size(); at += kBlock * kChannels)
		{
			limiter.SetGain(at < change ? 2.0 : 1.0);
			limiter.Process(mix.data() + at, kBlock, kChannels, kRate);
		}
		const size_t delay = limiter.DelayFrames();
		double worst       = 0;
		for (size_t i = delay * kChannels; i < mix.size(); ++i)
			worst = std::max(worst, std::fabs(mix[i] - expected[i - delay * kChannels]));
		Expect(worst <= 2.5, "volume change: another stream off by " + std::to_string(worst));
	}

	void BackToUnity()
	{
		PlayoutLimiter limiter;
		limiter.SetGain(2.0);
		Play(limiter, Sine(0.9, 440.0, kRate / 2));
		limiter.SetGain(1.0);
		// The release is long over after a second.
		Play(limiter, Sine(0.9, 440.0, kRate, kRate / 2));
		std::vector<int16_t> in(kRate * kChannels);
		Random random(2);
		for (auto& sample : in)
			sample = random.Sample();
		const auto out     = Play(limiter, in);
		const size_t delay = limiter.DelayFrames();
		size_t changed     = 0;
		for (size_t i = delay * kChannels; i < out.size(); ++i)
			changed += out[i] != in[i - delay * kChannels];
		Expect(changed == 0, "back at gain 1: " + std::to_string(changed) + " samples changed");
		Expect(limiter.Levels().reduction == 1.0f, "back at gain 1: the limiter rests");
	}

	void GainIsClamped()
	{
		PlayoutLimiter limiter;
		limiter.SetGain(9.0);
		Expect(limiter.Gain() == 4.0f, "gain clamped to 4");
		limiter.SetGain(0.5);
		Expect(limiter.Gain() == 1.0f, "gain clamped to 1");
	}

	void FormatChange()
	{
		PlayoutLimiter limiter;
		limiter.SetGain(2.0);
		Play(limiter, Sine(0.9, 440.0, kRate / 2));
		// A mono device at 44.1 kHz: the state starts over and still limits.
		std::vector<int16_t> mono(44100);
		for (size_t i = 0; i < mono.size(); ++i)
			mono[i] = static_cast<int16_t>(std::lround(0.9 * 32767.0 * std::sin(2 * kPi * 440.0 * i / 44100.0)));
		for (size_t at = 0; at < mono.size(); at += 441)
			limiter.Process(mono.data() + at, 441, 1, 44100);
		int loudest = 0;
		for (int16_t sample : mono)
			loudest = std::max(loudest, std::abs(static_cast<int>(sample)));
		Expect(loudest <= kCeilingSample, "mono 44.1 kHz: loudest sample " + std::to_string(loudest));
	}

	// The render thread allocates nothing on a format change up to the
	// reserved layout: the look-ahead stays where it was made.
	void FormatChangeAllocatesNothing()
	{
		PlayoutLimiter limiter;
		const float* reserved = limiter.DelayStorage();
		std::vector<int16_t> samples(PlayoutLimiter::kMaxReservedRate / 100 * PlayoutLimiter::kMaxReservedChannels);
		const struct
		{
			size_t channels;
			uint32_t rate;
		} layouts[] = { { 2, 48000 }, { 1, 44100 }, { 8, 192000 }, { 6, 96000 }, { 2, 16000 } };
		for (const auto& layout : layouts)
		{
			limiter.Process(samples.data(), layout.rate / 100, layout.channels, layout.rate);
			Expect(
			  limiter.DelayStorage() == reserved,
			  "no allocation for " + std::to_string(layout.channels) + " channels at " + std::to_string(layout.rate));
		}
	}
} // namespace

int main()
{
	UnityPassesUnchanged();
	LoudMixStaysBelowTheCeiling();
	LoudNoiseStaysBelowTheCeiling();
	QuietMixPlaysAtTheGain();
	HighestGain();
	VolumeChangeLeavesTheOtherStreamAlone();
	BackToUnity();
	GainIsClamped();
	FormatChange();
	FormatChangeAllocatesNothing();
	if (failures > 0)
	{
		std::printf("%d check(s) failed\n", failures);
		return 1;
	}
	std::printf("playout limiter: all checks passed\n");
	return 0;
}

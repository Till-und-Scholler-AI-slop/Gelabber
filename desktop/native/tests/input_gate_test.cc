// The input sensitivity gate (../src/input_gate.h) on generated blocks, the
// same cases as the web client's web/src/voice/inputGate.test.mjs: off
// changes nothing and adds no look-ahead, on it runs 20 ms behind, a
// manual threshold lets a voice through and sends silence below it after
// the hold time, a short pause keeps it open, a 20 ms burst does not open it while the start of speech
// is sent whole, automatic finds the room's noise by itself, and RNNoise's
// speech probability keeps loud clicks out. No libwebrtc, no sound card.
//
// desktop/core/tests/voice.rs builds and runs this with the host's C++
// compiler. By hand, in desktop/native:
//   c++ -std=c++20 -O2 -I src -o gate_test tests/input_gate_test.cc && ./gate_test
#include "input_gate.h"

#include <cmath>
#include <cstdint>
#include <cstdio>
#include <string>
#include <vector>

using gelabber::GateMode;
using gelabber::InputGate;

namespace
{
	constexpr int kRate       = 48000;
	constexpr int kBlock      = kRate / 100;
	constexpr float kFull     = 32768.0f;
	constexpr double kPi      = 3.14159265358979323846;
	constexpr float kNoSpeech = -1.0f;

	int failures = 0;

	void Expect(bool holds, const std::string& what)
	{
		if (holds)
			return;
		std::printf("FAILED: %s\n", what.c_str());
		++failures;
	}

	// libwebrtc's FloatS16 samples of a tone at `db` dBFS RMS.
	std::vector<float> Tone(float db, int ms, double hz = 220.0)
	{
		const double amplitude = std::sqrt(2.0) * std::pow(10.0, db / 20.0) * kFull;
		std::vector<float> out(static_cast<size_t>(kRate) * ms / 1000);
		for (size_t n = 0; n < out.size(); ++n)
			out[n] = static_cast<float>(amplitude * std::sin(2.0 * kPi * hz * static_cast<double>(n) / kRate));
		return out;
	}

	// White noise at about `db` dBFS RMS; the same numbers everywhere.
	std::vector<float> Noise(float db, int ms, uint32_t seed = 7)
	{
		const double amplitude = std::sqrt(3.0) * std::pow(10.0, db / 20.0) * kFull;
		std::vector<float> out(static_cast<size_t>(kRate) * ms / 1000);
		uint32_t state = seed;
		for (auto& sample : out)
		{
			state  = state * 1664525u + 1013904223u;
			sample = static_cast<float>(amplitude * (2.0 * (state / 4294967295.0) - 1.0));
		}
		return out;
	}

	std::vector<float> Join(std::initializer_list<std::vector<float>> parts)
	{
		std::vector<float> out;
		for (const auto& part : parts)
			out.insert(out.end(), part.begin(), part.end());
		return out;
	}

	// Runs `input` block by block, one mono channel; `speech` per block.
	std::vector<float> Run(InputGate& gate, std::vector<float> input, float speech = kNoSpeech,
	                       std::vector<bool>* open = nullptr)
	{
		for (size_t at = 0; at + kBlock <= input.size(); at += kBlock)
		{
			float* channels[] = { input.data() + at };
			gate.Process(channels, 1, kBlock, kFull, speech);
			if (open)
				open->push_back(gate.Report().open);
		}
		return input;
	}

	double LevelDb(const std::vector<float>& samples, int fromMs, int toMs)
	{
		double sum = 0.0;
		const size_t from = static_cast<size_t>(kRate) * fromMs / 1000;
		const size_t to   = static_cast<size_t>(kRate) * toMs / 1000;
		for (size_t n = from; n < to; ++n)
			sum += (samples[n] / kFull) * (samples[n] / kFull);
		const double rms = std::sqrt(sum / static_cast<double>(to - from));
		return rms > 0 ? 20.0 * std::log10(rms) : -INFINITY;
	}

	bool Silent(const std::vector<float>& samples, int fromMs, int toMs)
	{
		for (size_t n = static_cast<size_t>(kRate) * fromMs / 1000; n < static_cast<size_t>(kRate) * toMs / 1000; ++n)
			if (samples[n] != 0.0f)
				return false;
		return true;
	}

	bool Near(double value, double expected, double within = 0.5)
	{
		return std::fabs(value - expected) <= within;
	}

	void Setup(InputGate& gate, GateMode mode, float thresholdDb = -50.0f)
	{
		gate.Reset(kRate);
		gate.Configure(mode, thresholdDb);
	}
} // namespace

int main()
{
	{
		InputGate gate;
		Setup(gate, GateMode::Off);
		const auto input  = Join({ Noise(-60, 200), Tone(-20, 200) });
		const auto output = Run(gate, input);
		Expect(output == input, "off: every sample as it was, without look-ahead");
		Expect(gate.Report().open, "off: open");
		// Switched on, the output runs kLookaheadBlocks behind.
		gate.Configure(GateMode::Manual, -80);
		const auto later  = Tone(-20, 300, 330);
		const auto gated  = Run(gate, later);
		const size_t late = static_cast<size_t>(InputGate::kLookaheadBlocks) * kBlock;
		bool behind       = true;
		for (size_t n = 0; n < gated.size(); ++n)
			behind = behind && gated[n] == (n < late ? 0.0f : later[n - late]);
		Expect(behind, "on: 20 ms behind, after an empty look-ahead");
	}
	{
		InputGate gate;
		Setup(gate, GateMode::Manual, -40);
		const auto output = Run(gate, Join({ Tone(-20, 400), Noise(-60, 800), Tone(-20, 400) }));
		Expect(Near(LevelDb(output, 50, 400), -20), "manual: the voice passes");
		Expect(LevelDb(output, 410, 630) > -61, "manual: held 250 ms after the voice");
		Expect(Silent(output, 710, 1200), "manual: silence below the threshold");
		Expect(Near(LevelDb(output, 1250, 1600), -20), "manual: the voice again");
	}
	{
		InputGate gate;
		Setup(gate, GateMode::Manual, -45);
		std::vector<bool> open;
		const auto output = Run(gate,
		                        Join({ Tone(-25, 300), std::vector<float>(kRate * 150 / 1000),
		                               Tone(-25, 300), std::vector<float>(kRate * 600 / 1000) }),
		                        kNoSpeech, &open);
		bool keptOpen = true;
		for (size_t block = 0; block < 75; ++block)
			keptOpen = keptOpen && open[block];
		Expect(keptOpen, "a 150 ms pause keeps the gate open");
		Expect(!open.back(), "a 600 ms pause closes it");
		Expect(Near(LevelDb(output, 460, 750), -25), "the second phrase passes whole");
	}
	{
		// A fan at -55 dBFS, a voice at -25 dBFS, the fan again.
		InputGate gate;
		Setup(gate, GateMode::Auto);
		const auto output = Run(gate, Join({ Noise(-55, 3000), Tone(-25, 500), Noise(-55, 1500, 9) }));
		Expect(Silent(output, 2000, 3000), "automatic: the fan is gated");
		Expect(Near(LevelDb(output, 3040, 3500), -25), "automatic: the voice passes");
		Expect(Silent(output, 3900, 5000), "automatic: the fan after the voice is gated");
		const auto report = gate.Report();
		Expect(report.thresholdDb > -55.0f && report.thresholdDb <= -30.0f, "automatic: threshold above the fan");
	}
	{
		// Loud clicks RNNoise calls no speech stay out; with speech they pass.
		auto clicks = Noise(-70, 2000, 3);
		for (int at = 500; at < 2000; at += 180)
			for (int n = 0; n < 96; ++n)
				clicks[static_cast<size_t>(kRate) * at / 1000 + n] +=
				  static_cast<float>(0.3 * std::exp(-n / 18.0) * (n % 2 ? 1 : -1) * kFull);
		InputGate quiet;
		Setup(quiet, GateMode::Auto);
		Expect(Silent(Run(quiet, clicks, 0.05f), 1000, 2000), "automatic: clicks without speech stay out");
		InputGate voice;
		Setup(voice, GateMode::Auto);
		Expect(!Silent(Run(voice, Tone(-30, 800), 0.9f), 100, 800), "automatic: speech at -30 dB passes");
	}
	{
		// 20 ms above the threshold is a click, 30 ms is speech; thanks to
		// the look-ahead the speech's first block is sent too.
		InputGate gate;
		Setup(gate, GateMode::Manual, -40);
		const auto output = Run(gate, Join({ std::vector<float>(kRate / 2), Tone(-20, 20),
		                                     std::vector<float>(kRate / 2), Tone(-20, 300) }));
		Expect(Silent(output, 0, 1040), "a 20 ms burst stays out");
		Expect(LevelDb(output, 1040, 1050) > -23.0, "the first 10 ms of speech are sent");
		Expect(Near(LevelDb(output, 1050, 1320), -20), "the rest of it too");
	}
	{
		// A change of settings applies to the next block.
		InputGate gate;
		Setup(gate, GateMode::Manual, -10);
		Expect(Silent(Run(gate, Tone(-30, 500)), 300, 500), "a -10 dB threshold silences -30 dB");
		gate.Configure(GateMode::Off, -10);
		Expect(Near(LevelDb(Run(gate, Tone(-30, 300)), 20, 300), -30), "switched off, it passes at once");
		gate.Configure(GateMode::Manual, 25);
		Expect(gate.Report().mode == GateMode::Manual, "the mode is reported");
		Run(gate, Tone(-20, 50));
		Expect(gate.Report().thresholdDb == 0.0f, "the threshold stays within -80 and 0 dB");
	}
	{
		// Stereo: one decision, the same fade on both channels.
		InputGate gate;
		Setup(gate, GateMode::Manual, -40);
		auto left = Tone(-60, 600), right = Tone(-60, 600, 330);
		for (size_t at = 0; at + kBlock <= left.size(); at += kBlock)
		{
			float* channels[] = { left.data() + at, right.data() + at };
			gate.Process(channels, 2, kBlock, kFull, kNoSpeech);
		}
		Expect(Silent(left, 400, 600) && Silent(right, 400, 600), "stereo: both channels gated");
	}
	if (failures)
	{
		std::printf("%d failed\n", failures);
		return 1;
	}
	std::printf("input gate: all passed\n");
	return 0;
}

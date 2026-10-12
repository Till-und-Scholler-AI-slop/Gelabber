// The application-sound mix (../src/app_audio_mix.h) against simulated sound
// cards whose clocks are off the system clock: bursts of the graph's quantum
// at the card's pace, some of them late, taken by a mix thread that wakes a
// little late itself. No sound server, no libwebrtc, no real time: five
// minutes of a card take a fraction of a second.
//
// desktop/core/tests/voice.rs builds and runs this with the host's C++
// compiler. By hand, in desktop/native:
//   c++ -std=c++20 -O2 -I src -o mix_test tests/app_audio_mix_test.cc && ./mix_test
#include "app_audio_mix.h"

#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

using gelabber::MixInput;
using gelabber::MixInterval;

namespace
{
	constexpr double kRate = 48000.0;
	constexpr double kNsPerS = 1e9;

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

		// 0 <= value < 1
		double Next()
		{
			state = state * 6364136223846793005ULL + 1442695040888963407ULL;
			return static_cast<double>(state >> 11) / static_cast<double>(1ULL << 53);
		}

	private:
		uint64_t state;
	};

	struct Card
	{
		// How far its clock is off: at -50 it delivers 50 ppm fewer frames a
		// second than the system clock counts.
		double ppm{ 0 };
		// Frames a burst.
		size_t quantum{ 1024 };
		// The stream plays from here, pauses in between, and is gone from
		// `until` on (seconds).
		double from{ 0 };
		double pauseFrom{ -1 };
		double pauseTo{ -1 };
		double until{ 1e9 };
	};

	struct Stream
	{
		uint64_t underruns{ 0 };
		uint64_t overruns{ 0 };
		// Frames pushed and frames found in the mix in their order.
		uint64_t pushed{ 0 };
		uint64_t mixed{ 0 };
		// Blocks in which the mix did not carry the frames next in order.
		uint64_t broken{ 0 };
		// Everything pushed and everything the mix carried, added up.
		double pushedSum{ 0 };
		double mixedSum{ 0 };
		// The stream ended and played out.
		bool done{ false };

		// Frames that went in and have not come out.
		uint64_t Behind() const
		{
			return pushed > mixed ? pushed - mixed : 0;
		}
	};

	struct Outcome
	{
		uint64_t blocks{ 0 };
		std::vector<Stream> streams;
	};

	// The value of a stream's frame `index` on both channels: exact in a
	// float, and no two neighbours alike.
	float Sample(uint64_t index)
	{
		return static_cast<float>(index % 4096);
	}

	// `seconds` of the cards' streams through one mix. One burst in twenty
	// is up to `late` ms late (a busy loop thread), the others up to a tenth
	// of that; the mix wakes up to 1 ms late.
	Outcome Run(const std::vector<Card>& cards, double seconds, double late, uint64_t seed)
	{
		struct State
		{
			Card card;
			MixInput input;
			double burstAt;
			double lateness;
			Stream result;
		};
		Random random(seed);
		const auto lateness = [&] {
			return (random.Next() < 0.05 ? random.Next() : random.Next() * 0.1) * late * 1e6;
		};
		std::vector<State> states;
		for (const auto& card : cards)
			states.push_back({ card, MixInput(), card.from * kNsPerS, lateness(), Stream() });

		Outcome outcome;
		std::vector<float> burst;
		std::vector<float> mix(MixInput::kBlockFrames * MixInput::kChannels);
		double next = 0;
		while (next < seconds * kNsPerS)
		{
			const double now = next + random.Next() * 1e6;
			std::optional<double> lead;
			// A single stream's frames can be told in the mix.
			const bool alone = states.size() == 1;
			for (auto& state : states)
			{
				const auto& card    = state.card;
				const double period = static_cast<double>(card.quantum) / (kRate * (1 + card.ppm * 1e-6)) * kNsPerS;
				while (state.burstAt + state.lateness <= now && state.burstAt < card.until * kNsPerS)
				{
					const double at = state.burstAt / kNsPerS;
					if (at < card.pauseFrom || at >= card.pauseTo)
					{
						burst.clear();
						for (size_t i = 0; i < card.quantum; ++i, ++state.result.pushed)
						{
							burst.insert(burst.end(), MixInput::kChannels, Sample(state.result.pushed));
							state.result.pushedSum += MixInput::kChannels * Sample(state.result.pushed);
						}
						state.input.Push(burst.data(), burst.size());
					}
					state.burstAt += period;
					state.lateness = lateness();
				}
				if (now >= card.until * kNsPerS)
					state.input.End();
				std::fill(mix.begin(), mix.end(), 0.0f);
				const auto ahead = state.input.Mix(mix.data());
				for (const float value : mix)
					state.result.mixedSum += value;
				if (ahead)
					lead = lead ? std::min(*lead, *ahead) : *ahead;
				if (ahead && alone)
				{
					bool inOrder = true;
					for (size_t i = 0; i < MixInput::kBlockFrames; ++i, ++state.result.mixed)
						inOrder = inOrder && mix[i * MixInput::kChannels] == Sample(state.result.mixed) &&
						          mix[i * MixInput::kChannels + 1] == Sample(state.result.mixed);
					if (!inOrder)
						++state.result.broken;
				}
			}
			next += static_cast<double>(MixInterval(lead).count());
			++outcome.blocks;
		}
		for (auto& state : states)
		{
			state.result.underruns = state.input.underruns;
			state.result.overruns  = state.input.overruns;
			state.result.done      = state.input.Done();
			outcome.streams.push_back(state.result);
		}
		return outcome;
	}

	std::string Name(const Card& card)
	{
		char text[96];
		std::snprintf(text, sizeof(text), "a card %+.0f ppm off, bursts of %zu frames", card.ppm, card.quantum);
		return text;
	}
} // namespace

int main()
{
	constexpr double kSeconds = 300;

	// One card, off either way, at the quanta PipeWire runs with: its
	// default 1024, 256 next to a low-latency client, 1115 for a 44.1 kHz
	// graph resampled to 48 kHz, 8192 where the quantum is left to grow. A
	// mix on the system clock ran dry every 13 s at -50 ppm and 1024.
	uint64_t seed = 0;
	for (const size_t quantum : { size_t{ 256 }, size_t{ 1024 }, size_t{ 1115 }, size_t{ 2048 }, size_t{ 8192 } })
	{
		for (const double ppm : { -500.0, -50.0, 0.0, 50.0, 500.0 })
		{
			const Card card{ ppm, quantum };
			const auto outcome = Run({ card }, kSeconds, 15, ++seed);
			const auto& stream = outcome.streams[0];
			Expect(stream.underruns == 0, Name(card) + ": ran dry " + std::to_string(stream.underruns) + " times");
			Expect(stream.overruns == 0, Name(card) + ": cut back " + std::to_string(stream.overruns) + " times");
			Expect(stream.broken == 0, Name(card) + ": " + std::to_string(stream.broken) + " blocks out of order");
			// The mix goes at the card's pace: what it took is what the
			// card delivered, less what waits in the buffer.
			const double blocks = kSeconds * 100 * (1 + ppm * 1e-6);
			const double slack  = static_cast<double>(quantum) / MixInput::kBlockFrames + 6;
			Expect(
			  std::abs(static_cast<double>(outcome.blocks) - blocks) <= slack,
			  Name(card) + ": " + std::to_string(outcome.blocks) + " blocks, the card's pace makes " +
			    std::to_string(blocks));
			Expect(
			  stream.Behind() <= 2 * quantum + 4 * MixInput::kBlockFrames,
			  Name(card) + ": " + std::to_string(stream.Behind()) + " frames behind at the end");
		}
	}

	// An application that pauses: its buffer runs dry once, and it plays
	// again when it goes on.
	{
		Card card{ -50, 1024 };
		card.pauseFrom     = 100;
		card.pauseTo       = 105;
		const auto outcome = Run({ card }, kSeconds, 15, ++seed);
		const auto& stream = outcome.streams[0];
		Expect(stream.underruns == 1, "a pause: ran dry " + std::to_string(stream.underruns) + " times");
		Expect(stream.overruns == 0, "a pause: cut back " + std::to_string(stream.overruns) + " times");
		Expect(
		  stream.Behind() <= 2 * card.quantum + 4 * MixInput::kBlockFrames,
		  "a pause: " + std::to_string(stream.Behind()) + " frames behind at the end");
	}

	// A sound shorter than the cushion is deep, in a stream that then says
	// nothing more: two bursts and no third. It plays all the same.
	{
		Card card{ 0, 1024 };
		card.from          = 10;
		card.pauseFrom     = 10.04;
		card.pauseTo       = 1e9;
		const auto outcome = Run({ card }, 20, 15, ++seed);
		const auto& stream = outcome.streams[0];
		Expect(stream.pushed == 2 * card.quantum, "a short sound: " + std::to_string(stream.pushed) + " frames");
		Expect(stream.mixedSum == stream.pushedSum, "a short sound: not all of it was in the mix");
	}

	// A stream that goes away in the middle of its sound, as its last
	// samples always are: what is on its way into the mix plays out.
	{
		Card card{ 50, 1024 };
		card.until         = 60;
		const auto outcome = Run({ card }, 70, 15, ++seed);
		const auto& stream = outcome.streams[0];
		Expect(stream.done, "a stream that ends: it never played out");
		Expect(stream.mixedSum == stream.pushedSum, "a stream that ends: not all of it was in the mix");
		Expect(stream.underruns == 0, "a stream that ends: its end counted as running dry");
	}

	// A second stream that starts later on the same card disturbs neither.
	{
		Card later{ -50, 1024 };
		later.from         = 30.0031;
		const auto outcome = Run({ Card{ -50, 1024 }, later }, kSeconds, 15, ++seed);
		for (const auto& stream : outcome.streams)
		{
			Expect(stream.underruns == 0, "two streams of one card: one ran dry");
			Expect(stream.overruns == 0, "two streams of one card: one was cut back");
		}
	}

	// Two cards, one slow and one fast: the mix keeps to the slow one, which
	// never runs dry. The fast one runs ahead by 9.6 frames a second and is
	// cut back when it is too far.
	{
		const auto outcome = Run({ Card{ -100, 1024 }, Card{ 100, 1024 } }, 4 * kSeconds, 15, ++seed);
		const auto& slow    = outcome.streams[0];
		const auto& fast    = outcome.streams[1];
		Expect(slow.underruns == 0 && slow.overruns == 0, "two cards: the slow one ran dry or was cut back");
		Expect(fast.underruns == 0, "two cards: the fast one ran dry");
		Expect(
		  fast.overruns >= 1 && fast.overruns <= 5,
		  "two cards: the fast one was cut back " + std::to_string(fast.overruns) + " times");
	}

	// Silence goes out on the system clock, and the pace changes by 0.3 %
	// at most.
	Expect(MixInterval(std::nullopt).count() == 10'000'000, "the interval without a stream");
	Expect(MixInterval(0.0).count() == 10'000'000, "the interval on the cushion");
	Expect(MixInterval(1e9).count() == 9'970'000, "the shortest interval");
	Expect(MixInterval(-1e9).count() == 10'030'000, "the longest interval");

	if (failures == 0)
		std::puts("the mix follows its sound cards");
	return failures == 0 ? 0 : 1;
}

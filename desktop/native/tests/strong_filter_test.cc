// The "strong" microphone filter's slot (../src/strong_filter.h, GEL-28)
// against a fake filter: it loads off the capture thread and leaves the
// block alone until the filter is ready, warms a filter up that joins a
// running stream, scales FloatS16 to the filter's -1..1 and back, restarts
// it with a new stream, falls back when the filter cannot be built, fails
// or takes more than half of the real time, tries again when it is chosen
// again, and lets go of it when another mode is chosen. No libwebrtc, no
// DeepFilterNet.
//
// desktop/core/tests/voice.rs builds and runs this with the host's C++
// compiler. By hand, in desktop/native:
//   c++ -std=c++20 -O2 -I src -o strong_test tests/strong_filter_test.cc && ./strong_test
#include "strong_filter.h"

#include <atomic>
#include <chrono>
#include <cstdio>
#include <cstring>
#include <string>
#include <thread>
#include <vector>

using gelabber::StrongFilter;
using gelabber::StrongFilterApi;
using gelabber::StrongState;

namespace
{
	constexpr int kRate   = StrongFilter::kRate;
	constexpr int kBlock  = StrongFilter::kBlockFrames;
	constexpr float kFull = 32768.0f;

	int failures = 0;

	void Expect(bool holds, const std::string& what)
	{
		if (holds)
			return;
		std::printf("FAILED: %s\n", what.c_str());
		++failures;
	}

	// What the fake filter was asked to do; any thread.
	struct Fake
	{
		std::atomic<int> createMs{ 0 };
		std::atomic<bool> createFails{ false };
		std::atomic<int> processMs{ 0 };
		std::atomic<bool> processFails{ false };
		std::atomic<int> created{ 0 };
		std::atomic<int> destroyed{ 0 };
		std::atomic<int> resets{ 0 };
		std::atomic<int> processed{ 0 };
		std::atomic<float> lastInput{ 0.0f };
	} fake;

	struct FakeFilter
	{
		int id;
	};

	void* Create(char* error, size_t errorSize)
	{
		std::this_thread::sleep_for(std::chrono::milliseconds(fake.createMs.load()));
		if (fake.createFails.load())
		{
			std::snprintf(error, errorSize, "%s", "no model");
			return nullptr;
		}
		return new FakeFilter{ ++fake.created };
	}

	void Reset(void*)
	{
		++fake.resets;
	}

	// Halves the block, as a filter that takes some noise out.
	int Process(void*, float* samples, size_t count)
	{
		++fake.processed;
		fake.lastInput.store(samples[0]);
		if (fake.processMs.load())
			std::this_thread::sleep_for(std::chrono::milliseconds(fake.processMs.load()));
		if (fake.processFails.load())
			return -1;
		for (size_t i = 0; i < count; ++i)
			samples[i] *= 0.5f;
		return 0;
	}

	void Destroy(void* filter)
	{
		++fake.destroyed;
		delete static_cast<FakeFilter*>(filter);
	}

	const StrongFilterApi kApi{ Create, Reset, Process, Destroy };

	void Clear()
	{
		fake.createMs     = 0;
		fake.createFails  = false;
		fake.processMs    = 0;
		fake.processFails = false;
		fake.created      = 0;
		fake.destroyed    = 0;
		fake.resets       = 0;
		fake.processed    = 0;
	}

	// One block of a constant `value` (FloatS16); whether the filter's
	// output replaced it, and the first sample afterwards.
	struct Block
	{
		bool used;
		float sample;
	};

	Block Run(StrongFilter& filter, float value = 16384.0f, int frames = kBlock, int rate = kRate)
	{
		std::vector<float> samples(static_cast<size_t>(frames), value);
		const bool used = filter.Process(samples.data(), frames, rate);
		return { used, samples[0] };
	}

	bool WaitFor(StrongFilter& filter, StrongState state)
	{
		for (int i = 0; i < 500 && filter.State() != state; ++i)
			std::this_thread::sleep_for(std::chrono::milliseconds(2));
		return filter.State() == state;
	}

	// A ready filter in a stream that started after it was loaded.
	void Ready(StrongFilter& filter)
	{
		filter.Select(true);
		WaitFor(filter, StrongState::Active);
		filter.Restart();
	}
} // namespace

int main()
{
	{
		Clear();
		StrongFilter filter(nullptr);
		Expect(filter.State() == StrongState::Off, "not chosen, it is off");
		filter.Select(true);
		Expect(filter.State() == StrongState::Unavailable, "without a filter it is unavailable");
		const auto block = Run(filter);
		Expect(!block.used && block.sample == 16384.0f, "unavailable, the block stays as it was");
	}
	{
		Clear();
		StrongFilter filter(&kApi);
		Expect(!Run(filter).used && fake.created == 0, "not chosen, nothing is loaded or run");
	}
	{
		Clear();
		fake.createMs = 100;
		StrongFilter filter(&kApi);
		filter.Select(true);
		Expect(filter.State() == StrongState::Loading, "chosen, it loads");
		Expect(!Run(filter).used && fake.processed == 0, "while it loads, the block stays as it was");
		Expect(WaitFor(filter, StrongState::Active), "loaded, it is active");
		Expect(fake.created == 1, "one filter was built");
		for (int i = 0; i < StrongFilter::kWarmupBlocks; ++i)
			Expect(!Run(filter).used, "a filter that joins a running stream warms up first");
		Expect(fake.processed == StrongFilter::kWarmupBlocks, "it runs while it warms up");
		Expect(fake.lastInput.load() == 0.5f, "it gets -1..1");
		const auto block = Run(filter);
		Expect(block.used && block.sample == 8192.0f, "warm, its output replaces the block, in FloatS16");
		filter.Select(true);
		Expect(fake.created == 1 && filter.State() == StrongState::Active, "chosen again, it is kept");
	}
	{
		Clear();
		StrongFilter filter(&kApi);
		Ready(filter);
		Expect(Run(filter).used && fake.resets == 0, "ready when a stream starts, it runs at once");
		filter.Restart();
		Expect(Run(filter).used && fake.resets == 1, "a new stream restarts it, without a warm-up");
		Expect(!Run(filter, 1.0f, 441, 44100).used, "other rates pass through");
		Expect(!Run(filter, 1.0f, 160).used, "other block sizes pass through");
		Expect(filter.State() == StrongState::Active, "and it stays active");
	}
	{
		Clear();
		fake.createFails = true;
		StrongFilter filter(&kApi);
		filter.Select(true);
		Expect(WaitFor(filter, StrongState::Failed), "a filter that cannot be built fails");
		Expect(filter.Error() == "no model", "with the filter's error");
		Expect(!Run(filter).used, "failed, the block stays as it was");
		fake.createFails = false;
		filter.Select(true);
		Expect(WaitFor(filter, StrongState::Active), "chosen again, it loads again");
		Expect(filter.Error().empty(), "and the error is gone");
	}
	{
		Clear();
		StrongFilter filter(&kApi);
		Ready(filter);
		fake.processFails = true;
		Expect(!Run(filter).used, "a block it fails on stays as it was");
		Expect(filter.State() == StrongState::Failed, "and it has failed");
		Expect(filter.Error() == "the filter failed on a block", "saying why");
		const int processed = fake.processed;
		Run(filter);
		Expect(fake.processed == processed, "failed, it no longer runs");
		fake.processFails = false;
		filter.Select(true);
		Expect(WaitFor(filter, StrongState::Active) && fake.created == 2 && fake.destroyed == 1,
		       "chosen again, a new filter replaces the failed one");
		Expect(filter.Error().empty(), "and the failure is forgotten");
	}
	{
		Clear();
		fake.processMs = 6;
		StrongFilter filter(&kApi);
		Ready(filter);
		int used = 0;
		for (int i = 0; i < StrongFilter::kWindowBlocks; ++i)
			used += Run(filter).used;
		Expect(used == StrongFilter::kWindowBlocks, "until a window is over, a slow filter runs");
		Expect(filter.State() == StrongState::Slow, "6 ms per 10 ms block is too slow");
		Expect(filter.BusyMs() >= 6.0f, "the time per block is reported");
		const int processed = fake.processed;
		Expect(!Run(filter).used && fake.processed == processed, "too slow, it no longer runs");
		fake.processMs = 0;
		filter.Select(true);
		Expect(filter.State() == StrongState::Active && fake.created == 1, "chosen again, it tries again");
		const int resets = fake.resets;
		for (int i = 0; i < StrongFilter::kWarmupBlocks; ++i)
			Run(filter);
		Expect(fake.resets == resets + 1, "restarted, as its history is old");
		Expect(Run(filter).used, "and its output is used after the warm-up");
		for (int i = 0; i < StrongFilter::kWindowBlocks; ++i)
			Run(filter);
		Expect(filter.State() == StrongState::Active && filter.BusyMs() < 5.0f, "fast enough, it stays");
	}
	{
		Clear();
		StrongFilter filter(&kApi);
		Ready(filter);
		filter.Select(false);
		Expect(filter.State() == StrongState::Off && fake.destroyed == 1, "another mode lets go of it");
		Expect(!Run(filter).used, "and the block stays as it was");
	}
	{
		Clear();
		fake.createMs = 100;
		StrongFilter filter(&kApi);
		filter.Select(true);
		filter.Select(false);
		std::this_thread::sleep_for(std::chrono::milliseconds(200));
		Expect(filter.State() == StrongState::Off, "chosen away while it loads, it stays off");
		Expect(fake.created == 1 && fake.destroyed == 1, "and the loaded filter goes");
		Expect(!Run(filter).used, "unused");
	}
	{
		Clear();
		fake.createMs = 100;
		{
			StrongFilter filter(&kApi);
			filter.Select(true);
		}
		Expect(fake.created == 1 && fake.destroyed == 1, "gone while it loads, the load is waited for");
		{
			StrongFilter filter(&kApi);
			fake.createMs = 0;
			Ready(filter);
		}
		Expect(fake.destroyed == 2, "gone, it lets go of its filter");
	}
	Expect(std::strcmp(gelabber::StrongStateName(StrongState::Slow), "slow") == 0, "states have names");
	if (failures)
	{
		std::printf("%d failed\n", failures);
		return 1;
	}
	std::printf("strong filter: all passed\n");
	return 0;
}

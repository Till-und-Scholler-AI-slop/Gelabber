// The "strong" microphone filter (GEL-28): DeepFilterNet3 in place of
// RNNoise's output, after the APM. The model itself is Rust (libDF with
// tract, desktop/core/src/strong_filter.rs), which hands the core four
// functions (gm_set_strong_filter); this is the slot that holds such a
// filter for the capture post-processor, kept free of libwebrtc so that it
// also runs in a plain test (../tests/strong_filter_test.cc).
//
// - Building the model takes about half a second, so it happens on a thread
//   of its own when the mode is chosen; until it is ready the capture keeps
//   RNNoise's output ("standard").
// - A filter that joins a running stream gets kWarmupBlocks blocks before
//   its output is used: it starts from silence and is 10 ms later than
//   RNNoise.
// - It falls back to RNNoise's output for good when it cannot be built or
//   fails on a block ("failed"), and when it takes more than kMaxBusyShare
//   of the real time over a window of kWindowBlocks blocks ("slow"). Choosing
//   the mode again tries again.
// - The capture thread never waits: it only tries the lock, which other
//   threads take briefly when the mode changes or a filter was built.
#ifndef GELABBER_STRONG_FILTER_H
#define GELABBER_STRONG_FILTER_H

#include <algorithm>
#include <array>
#include <atomic>
#include <chrono>
#include <cstddef>
#include <cstdint>
#include <mutex>
#include <string>
#include <thread>

namespace gelabber
{
	// gm_strong_filter's functions (gelabber_media.h).
	struct StrongFilterApi
	{
		void* (*create)(char* error, size_t errorSize);
		void (*reset)(void* filter);
		int (*process)(void* filter, float* samples, size_t count);
		void (*destroy)(void* filter);
	};

	enum class StrongState
	{
		// Another mode.
		Off,
		// Chosen, but the core has no filter (no gm_set_strong_filter).
		Unavailable,
		Loading,
		Active,
		Failed,
		Slow,
	};

	inline const char* StrongStateName(StrongState state)
	{
		switch (state)
		{
			case StrongState::Unavailable:
				return "unavailable";
			case StrongState::Loading:
				return "loading";
			case StrongState::Active:
				return "active";
			case StrongState::Failed:
				return "failed";
			case StrongState::Slow:
				return "slow";
			default:
				return "off";
		}
	}

	class StrongFilter
	{
	public:
		// DeepFilterNet3 works on 10 ms at 48 kHz, one APM block.
		static constexpr int kRate          = 48000;
		static constexpr int kBlockFrames   = 480;
		static constexpr int kWarmupBlocks  = 3;
		static constexpr int kWindowBlocks  = 100;
		static constexpr float kMaxBusyShare = 0.5f;
		static constexpr float kFullScale   = 32768.0f;

		// `api` may be null: then the filter is unavailable.
		explicit StrongFilter(const StrongFilterApi* api) : hasApi(api != nullptr)
		{
			if (api)
				this->api = *api;
		}

		~StrongFilter()
		{
			std::lock_guard control(controlMutex);
			{
				std::lock_guard lock(mutex);
				chosen = false;
			}
			if (loader.joinable())
				loader.join();
			if (filter)
				api.destroy(filter);
		}

		StrongFilter(const StrongFilter&)            = delete;
		StrongFilter& operator=(const StrongFilter&) = delete;

		// Any thread but the capture thread: the mode is "strong" or not.
		void Select(bool on)
		{
			std::lock_guard control(controlMutex);
			void* drop = nullptr;
			bool load  = false;
			{
				std::lock_guard lock(mutex);
				chosen = on;
				if (!on)
				{
					drop   = filter;
					filter = nullptr;
					state.store(StrongState::Off);
					busyMs.store(0.0f);
					maxMs.store(0.0f);
				}
				else if (!hasApi)
					state.store(StrongState::Unavailable);
				else if (loading)
					state.store(StrongState::Loading);
				else if (filter && state.load() == StrongState::Slow)
				{
					// The same filter again, from a clean history.
					retry = true;
					state.store(StrongState::Active);
				}
				else if (!filter || state.load() == StrongState::Failed)
				{
					drop    = filter;
					filter  = nullptr;
					load    = true;
					loading = true;
					state.store(StrongState::Loading);
				}
			}
			if (drop)
				api.destroy(drop);
			if (on)
			{
				SetError("");
				blockFailed.store(false);
			}
			if (load)
			{
				if (loader.joinable())
					loader.join();
				loader = std::thread([this] { Load(); });
			}
		}

		// Capture thread: a new stream; the filter's history belongs to the
		// old one.
		void Restart()
		{
			streamStarted = true;
		}

		// Capture thread: one mono block in FloatS16 (-32768..32767), in
		// place. True if the filter's output replaced it.
		bool Process(float* samples, int frames, int rate)
		{
			if (frames != kBlockFrames || rate != kRate)
				return false;
			std::unique_lock lock(mutex, std::try_to_lock);
			if (!lock)
				return false;
			const bool started = streamStarted;
			streamStarted      = false;
			if (!filter || state.load() != StrongState::Active)
				return false;
			if (generation != seenGeneration)
			{
				// Built from nothing: no reset, but a warm-up mid-stream.
				seenGeneration = generation;
				warm           = started ? kWarmupBlocks : 0;
				StartWindow();
			}
			else if (started)
			{
				api.reset(filter);
				warm = kWarmupBlocks;
				StartWindow();
			}
			else if (retry)
			{
				api.reset(filter);
				warm = 0;
				StartWindow();
			}
			retry = false;

			for (int i = 0; i < frames; ++i)
				buffer[static_cast<size_t>(i)] = samples[i] / kFullScale;
			const auto start  = std::chrono::steady_clock::now();
			const int result  = api.process(filter, buffer.data(), static_cast<size_t>(frames));
			const float spent = std::chrono::duration<float, std::milli>(std::chrono::steady_clock::now() - start).count();
			if (result < 0)
			{
				// The audio thread only sets a flag; Error() words it.
				blockFailed.store(true);
				state.store(StrongState::Failed);
				return false;
			}
			windowMs += spent;
			windowMaxMs = std::max(windowMaxMs, spent);
			if (++windowBlocks == kWindowBlocks)
			{
				busyMs.store(windowMs / kWindowBlocks);
				maxMs.store(windowMaxMs);
				const float blockMs = 1000.0f * kBlockFrames / kRate;
				if (windowMs > kMaxBusyShare * blockMs * kWindowBlocks)
					state.store(StrongState::Slow);
				StartWindow();
			}
			if (warm < kWarmupBlocks)
			{
				++warm;
				return false;
			}
			for (int i = 0; i < frames; ++i)
				samples[i] = buffer[static_cast<size_t>(i)] * kFullScale;
			return true;
		}

		// Any thread.
		StrongState State() const
		{
			return state.load();
		}
		// Why it failed, else empty.
		std::string Error() const
		{
			if (blockFailed.load())
				return "the filter failed on a block";
			std::lock_guard lock(errorMutex);
			return error;
		}
		// Mean time per block over the last full window, in ms.
		float BusyMs() const
		{
			return busyMs.load();
		}
		// The slowest block of that window, in ms.
		float MaxMs() const
		{
			return maxMs.load();
		}

	private:
		void Load()
		{
			char message[256] = {};
			void* made        = api.create(message, sizeof message);
			void* drop        = nullptr;
			{
				std::lock_guard lock(mutex);
				loading = false;
				if (!made)
				{
					if (chosen)
					{
						state.store(StrongState::Failed);
						SetError(message[0] ? message : "the filter could not be built");
					}
				}
				else if (chosen && !filter)
				{
					filter = made;
					++generation;
					state.store(StrongState::Active);
				}
				else
					drop = made;
			}
			if (drop)
				api.destroy(drop);
		}

		void SetError(const std::string& text)
		{
			std::lock_guard lock(errorMutex);
			error = text;
		}

		void StartWindow()
		{
			windowMs     = 0.0f;
			windowMaxMs  = 0.0f;
			windowBlocks = 0;
		}

		StrongFilterApi api{};
		const bool hasApi;

		// Select and the destructor, one at a time.
		std::mutex controlMutex;
		std::thread loader;

		// Guards what the capture thread shares with Select and the loader.
		std::mutex mutex;
		void* filter{ nullptr };
		bool chosen{ false };
		bool loading{ false };
		bool retry{ false };
		uint64_t generation{ 0 };

		std::atomic<StrongState> state{ StrongState::Off };
		std::atomic<float> busyMs{ 0.0f };
		std::atomic<float> maxMs{ 0.0f };
		// The loader's reason, and a failed block's flag (the capture thread
		// takes no lock for it).
		mutable std::mutex errorMutex;
		std::string error;
		std::atomic<bool> blockFailed{ false };

		// Capture thread only.
		uint64_t seenGeneration{ 0 };
		bool streamStarted{ false };
		int warm{ 0 };
		float windowMs{ 0.0f };
		float windowMaxMs{ 0.0f };
		int windowBlocks{ 0 };
		std::array<float, kBlockFrames> buffer{};
	};
} // namespace gelabber

#endif

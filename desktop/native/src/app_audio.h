// Sound of other applications for Go Live/screen share ("source audio"),
// captured from their playback streams. Platform-specific implementations;
// no libwebrtc types here.
#ifndef GELABBER_APP_AUDIO_H
#define GELABBER_APP_AUDIO_H

#include <cstddef>
#include <cstdint>
#include <functional>
#include <memory>
#include <string>
#include <vector>

namespace gelabber
{
	struct AudioApp
	{
		// Its binary; an application that reports none goes by its name.
		std::string id;
		std::string name;
		// Playback streams the id chooses right now.
		int streams{ 0 };
	};

	// Applications playing sound, without this process and those it started,
	// and without the playback streams of virtual devices (a loopback, a
	// filter chain, an echo canceller, a combined sink), which play on what
	// applications played into them.
	// Applications that share a binary and call themselves differently are
	// listed by name as well, next to the binary, which chooses them all.
	std::vector<AudioApp> ListAudioApps();

	class AppAudioCapture
	{
	public:
		static constexpr int kSampleRate = 48000;
		static constexpr int kChannels   = 2;
		static constexpr size_t kFrames  = kSampleRate / 100;

		// 10 ms of 16-bit interleaved stereo at 48 kHz, from one thread. Silence
		// while the captured applications play nothing.
		using Sink = std::function<void(const int16_t* pcm)>;

		// `app`: an AudioApp id, or "" for every application. This process and
		// those it started are never captured, nor is what a virtual device
		// plays on. The application's name, the id up to 0.5.2, selects it as
		// well. Streams the applications open later are picked up. Throws
		// when the platform's sound server is unavailable.
		static std::unique_ptr<AppAudioCapture> Start(const std::string& app, Sink sink);

		virtual ~AppAudioCapture() = default;
		// {"state":"live","streams":n,"frames":n}
		virtual std::string StateJson() const = 0;
	};
} // namespace gelabber

#endif

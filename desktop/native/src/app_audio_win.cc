// Application sound on Windows. Not implemented yet: no applications are
// listed and starting a capture fails. The implementation belongs in this
// file, behind the same interface as the PipeWire capture.
// desktop/core/tests/mediasoup_loopback.rs holds both to that.

#include "app_audio.h"

#include <stdexcept>

namespace gelabber
{
	std::vector<AudioApp> ListAudioApps()
	{
		return {};
	}

	std::unique_ptr<AppAudioCapture> AppAudioCapture::Start(const std::string& /*app*/, Sink /*sink*/)
	{
		throw std::runtime_error("application sound is not available on Windows yet");
	}
} // namespace gelabber

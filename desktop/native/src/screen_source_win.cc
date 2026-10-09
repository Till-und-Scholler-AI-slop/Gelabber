// Screen capture on Windows. Not implemented yet: creating a source fails, and
// the app reports the reason. The implementation belongs in this file, behind
// the same CreateScreenSource as the PipeWire source.

#include "local_video_source.h"

#include <stdexcept>

namespace gelabber
{
	webrtc::scoped_refptr<LocalVideoSource> CreateScreenSource(const ScreenOptions& /*options*/)
	{
		throw std::runtime_error("screen capture is not available on Windows yet");
	}
} // namespace gelabber

// The video codecs the core takes from the system beside libwebrtc's own
// (GStreamer on Linux, Media Foundation on Windows). No libwebrtc types here.
//
// H264: libwebrtc carries none (rtc_use_h264=false), so H264 is the
// system's alone, on the GPU where it can and else on the CPU.
// AV1: libwebrtc's libaom and dav1d run on the CPU; the system's codecs are
// used only where they run on the GPU, in front of them (issue #318).
#ifndef GELABBER_SYSTEM_CODEC_H
#define GELABBER_SYSTEM_CODEC_H

namespace gelabber
{
	enum class SystemCodec
	{
		kH264,
		kAv1,
	};

	inline const char* CodecName(SystemCodec codec)
	{
		return codec == SystemCodec::kH264 ? "H264" : "AV1";
	}
} // namespace gelabber

#endif

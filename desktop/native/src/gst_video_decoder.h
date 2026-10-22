// Video decoding through the system's GStreamer.
//
// H264: NVDEC (nvh264dec) on NVIDIA, VA-API (vah264dec, vaapih264dec) on
// Intel/AMD, else a software element of the system (avdec_h264,
// openh264dec). The core carries no H264 decoder of its own: without
// GStreamer or an H264 element it has no H264. A hardware decoder that fails
// at runtime hands over to the software element where there is one
// (VideoDecoderSoftwareFallbackWrapper).
//
// AV1: NVDEC (nvav1dec), VA-API (vaav1dec, vaapiav1dec). Hardware only: on
// the CPU the core has dav1d, which takes over from a hardware decoder that
// fails (HardwareFirstDecoder).
#ifndef GELABBER_GST_VIDEO_DECODER_H
#define GELABBER_GST_VIDEO_DECODER_H

#include "system_codec.h"

#include <api/video_codecs/video_decoder.h>

#include <memory>
#include <string>

namespace gelabber
{
	// Element used to decode `codec`, or "" when there is none.
	// GELABBER_H264_DECODER and GELABBER_AV1_DECODER override the choice: an
	// element name (also software ones such as avdec_h264 or av1dec, for
	// tests) or "none".
	std::string FindGstDecoder(SystemCodec codec);

	// A software element to fall back to (H264: avdec_h264, openh264dec), or
	// ""; always "" for AV1, which falls back to dav1d.
	std::string FindGstSoftwareDecoder(SystemCodec codec);

	// Whether `element` decodes on a GPU, not on the CPU.
	bool IsHardwareDecoder(const std::string& element);

	// A decoder on `element`.
	// `behindFallback`: wrapped in a software fallback, so a failure hands
	// over to the wrapper's software decoder instead of restarting the
	// pipeline.
	std::unique_ptr<webrtc::VideoDecoder> CreateGstDecoder(
	  SystemCodec codec, const std::string& element, bool behindFallback);
} // namespace gelabber

#endif

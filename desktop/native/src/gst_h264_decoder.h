// H264 decoding through the system's GStreamer: NVDEC (nvh264dec) on
// NVIDIA, VA-API (vah264dec, vaapih264dec) on Intel/AMD, else a software
// element of the system (avdec_h264, openh264dec).
//
// The core carries no H264 decoder of its own: without GStreamer or an H264
// element it has no H264. A hardware decoder that fails at runtime hands
// over to the software element where there is one
// (VideoDecoderSoftwareFallbackWrapper).
#ifndef GELABBER_GST_H264_DECODER_H
#define GELABBER_GST_H264_DECODER_H

#include <api/video_codecs/video_decoder.h>

#include <memory>
#include <string>

namespace gelabber
{
	// Element used to decode H264, or "" when there is none.
	// GELABBER_H264_DECODER overrides the choice: an element name (also
	// software ones such as avdec_h264, for tests) or "none".
	std::string FindGstH264Decoder();

	// A software element (avdec_h264, openh264dec) to fall back to, or "".
	std::string FindGstH264SoftwareDecoder();

	// Whether `element` decodes on a GPU, not on the CPU.
	bool IsHardwareH264Decoder(const std::string& element);

	// A decoder on `element`; wrap it for the software fallback.
	std::unique_ptr<webrtc::VideoDecoder> CreateGstH264Decoder(const std::string& element);
} // namespace gelabber

#endif

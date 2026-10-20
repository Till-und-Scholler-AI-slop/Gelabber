// Hardware H264 decoding through the system's GStreamer: NVDEC (nvh264dec)
// on NVIDIA, VA-API (vah264dec, vaapih264dec) on Intel/AMD.
//
// Without GStreamer or a hardware H264 element the core keeps libwebrtc's
// software decoder. A hardware decoder that fails at runtime hands over to
// it as well (VideoDecoderSoftwareFallbackWrapper).
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

	// A decoder on `element`; wrap it for the software fallback.
	std::unique_ptr<webrtc::VideoDecoder> CreateGstH264Decoder(const std::string& element);
} // namespace gelabber

#endif

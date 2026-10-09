// Hardware H264 through the system's GStreamer: VA-API (vah264enc,
// vah264lpenc, vaapih264enc) on Intel/AMD, NVENC (nvh264enc) on NVIDIA.
//
// GStreamer is loaded with dlopen when the engine starts. Without it, or
// without a hardware H264 element, the core keeps libwebrtc's software
// encoders. A hardware encoder that fails at runtime hands over to OpenH264
// (VideoEncoderSoftwareFallbackWrapper).
#ifndef GELABBER_GST_H264_ENCODER_H
#define GELABBER_GST_H264_ENCODER_H

#include <api/video_codecs/video_encoder_factory.h>

#include <memory>
#include <string>
#include <vector>

namespace gelabber
{
	// Element used for H264, or "" when there is none. GELABBER_H264_ENCODER
	// overrides the choice: an element name (also software ones such as
	// x264enc, for tests) or "none".
	std::string FindGstH264Encoder();

	// Creates single-stream encoders on `element`; wrap in
	// SimulcastEncoderAdapter for simulcast and software fallback.
	std::unique_ptr<webrtc::VideoEncoderFactory> CreateGstH264EncoderFactory(
	  const std::string& element, std::vector<webrtc::SdpVideoFormat> formats);
} // namespace gelabber

#endif

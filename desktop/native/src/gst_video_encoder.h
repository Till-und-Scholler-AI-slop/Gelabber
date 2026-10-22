// Video encoding through the system's GStreamer.
//
// H264: VA-API (vah264enc, vah264lpenc, vaapih264enc) on Intel/AMD, NVENC
// (nvh264enc) on NVIDIA, else a software element of the system
// (openh264enc, x264enc). The core carries no H264 encoder of its own:
// without GStreamer or an H264 element it has no H264. A hardware encoder
// that fails at runtime hands over to the software element where there is
// one (SimulcastEncoderAdapter).
//
// AV1: NVENC (nvav1enc) on NVIDIA from RTX 40, VA-API (vaav1enc,
// vaav1lpenc) on Intel Arc/Core Ultra and AMD from RX 7000. Hardware only:
// on the CPU the core has libaom, which takes over from a hardware encoder
// that fails (HardwareFirstEncoder).
//
// GStreamer is loaded with dlopen when the engine starts.
#ifndef GELABBER_GST_VIDEO_ENCODER_H
#define GELABBER_GST_VIDEO_ENCODER_H

#include "system_codec.h"

#include <api/video_codecs/video_encoder.h>
#include <api/video_codecs/video_encoder_factory.h>

#include <memory>
#include <string>
#include <vector>

namespace gelabber
{
	// Element used for `codec`, or "" when there is none.
	// GELABBER_H264_ENCODER and GELABBER_AV1_ENCODER override the choice: an
	// element name (also software ones such as x264enc or av1enc, for tests)
	// or "none".
	std::string FindGstEncoder(SystemCodec codec);

	// A software element to fall back to (H264: openh264enc, x264enc), or "";
	// always "" for AV1, which falls back to libaom.
	std::string FindGstSoftwareEncoder(SystemCodec codec);

	// Whether `element` encodes on a GPU or media engine, not on the CPU.
	bool IsHardwareEncoder(const std::string& element);

	// A single-stream encoder on `element`.
	std::unique_ptr<webrtc::VideoEncoder> CreateGstEncoder(SystemCodec codec, const std::string& element);

	// Creates single-stream encoders on `element`; wrap in
	// SimulcastEncoderAdapter for simulcast and software fallback.
	std::unique_ptr<webrtc::VideoEncoderFactory> CreateGstEncoderFactory(
	  SystemCodec codec, const std::string& element, std::vector<webrtc::SdpVideoFormat> formats);
} // namespace gelabber

#endif

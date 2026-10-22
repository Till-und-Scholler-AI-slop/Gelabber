// The engine's video codecs: libwebrtc's software VP8, VP9 and AV1, and
// H264 through the system where it has an encoder or decoder (GStreamer
// on Linux, Media Foundation on Windows). The core carries no H264 of its own.
#ifndef GELABBER_VIDEO_CODEC_FACTORY_H
#define GELABBER_VIDEO_CODEC_FACTORY_H

#include <api/video_codecs/video_decoder_factory.h>
#include <api/video_codecs/video_encoder_factory.h>

#include <memory>

namespace gelabber
{
	std::unique_ptr<webrtc::VideoEncoderFactory> CreateVideoEncoderFactory();
	std::unique_ptr<webrtc::VideoDecoderFactory> CreateVideoDecoderFactory();
} // namespace gelabber

#endif

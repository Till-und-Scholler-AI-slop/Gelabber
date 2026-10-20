// H264 through Windows Media Foundation: the GPU's encoder MFT (NVENC, AMF,
// Quick Sync) where there is one, else Microsoft's software encoder, and
// Microsoft's decoder, which decodes on the GPU through DXVA where it can.
//
// libwebrtc's own H264 (OpenH264, FFmpeg) is not part of the Windows core;
// these MFTs ship with Windows and the GPU drivers. An encoder covers one
// picture; `CreateMfH264Encoder` lays one per simulcast layer.
#ifndef GELABBER_MF_H264_H
#define GELABBER_MF_H264_H

#include <api/video_codecs/video_decoder.h>
#include <api/video_codecs/video_encoder.h>

#include <memory>
#include <string>

namespace gelabber
{
	struct MfH264Codecs
	{
		// Friendly name of the encoder MFT, "" when there is none.
		std::string encoder;
		bool hardwareEncoder{ false };
		// Friendly name of the decoder MFT, "" when there is none.
		std::string decoder;
	};

	// What the system has, looked up once.
	const MfH264Codecs& FindMfH264Codecs();

	// A simulcast-capable encoder on the best encoder MFT, or null without one.
	std::unique_ptr<webrtc::VideoEncoder> CreateMfH264Encoder();

	// A decoder on the decoder MFT, or null without one.
	std::unique_ptr<webrtc::VideoDecoder> CreateMfH264Decoder();
} // namespace gelabber

#endif

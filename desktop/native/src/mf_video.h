// Video codecs through Windows Media Foundation.
//
// H264: the GPU's encoder MFT (NVENC, AMF, Quick Sync) where there is one,
// else Microsoft's software encoder, and Microsoft's decoder, which decodes
// on the GPU through DXVA where it can. The core carries no H264 codec of
// its own (rtc_use_h264=false); these MFTs ship with Windows and the GPU
// drivers. An encoder covers one picture; `CreateMfEncoder` lays one per
// simulcast layer.
//
// AV1: the GPU's encoder MFT (NVIDIA from RTX 40, AMD from RX 7000, Intel
// Arc/Core Ultra) and Microsoft's AV1 decoder where it decodes through
// DXVA. Hardware only, in front of libaom and dav1d (HardwareFirstEncoder,
// HardwareFirstDecoder), and one stream, never simulcast layers.
#ifndef GELABBER_MF_VIDEO_H
#define GELABBER_MF_VIDEO_H

#include "system_codec.h"

#include <api/video_codecs/video_decoder.h>
#include <api/video_codecs/video_encoder.h>

#include <memory>
#include <string>

namespace gelabber
{
	struct MfCodecs
	{
		// Friendly name of the encoder MFT, "" when there is none.
		std::string encoder;
		bool hardwareEncoder{ false };
		// Friendly name of the decoder MFT, "" when there is none.
		std::string decoder;
	};

	// What the system has, looked up once.
	const MfCodecs& FindMfCodecs(SystemCodec codec);

	// An encoder on the best encoder MFT, or null without one. H264: one per
	// simulcast layer.
	std::unique_ptr<webrtc::VideoEncoder> CreateMfEncoder(SystemCodec codec);

	// A decoder on the decoder MFT, or null without one.
	std::unique_ptr<webrtc::VideoDecoder> CreateMfDecoder(SystemCodec codec);
} // namespace gelabber

#endif

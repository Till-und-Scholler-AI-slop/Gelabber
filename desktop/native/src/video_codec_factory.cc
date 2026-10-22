// libwebrtc's software video codecs plus H264 and AV1 through the system's
// codecs (GStreamer on Linux, Media Foundation on Windows), as the engine's
// video encoder and decoder factories.
#include "video_codec_factory.h"

// AV1 on the GPU in front of libaom and dav1d (both systems).
#include "codec_fallback.h"

#include <api/video_codecs/video_decoder_factory_template.h>
#include <api/video_codecs/video_decoder_factory_template_dav1d_adapter.h>
#include <api/video_codecs/video_decoder_factory_template_libvpx_vp8_adapter.h>
#include <api/video_codecs/video_decoder_factory_template_libvpx_vp9_adapter.h>
#include <api/video_codecs/video_encoder.h>
#include <api/video_codecs/video_encoder_factory_template.h>
#include <api/video_codecs/video_encoder_factory_template_libaom_av1_adapter.h>
#include <api/video_codecs/video_encoder_factory_template_libvpx_vp8_adapter.h>
#include <api/video_codecs/video_encoder_factory_template_libvpx_vp9_adapter.h>
#include <rtc_base/logging.h>

#if defined(WEBRTC_LINUX)
// H264 and AV1 through GStreamer: H264 encoders behind libwebrtc's
// simulcast adapter (which the Windows libwebrtc package does not carry),
// hardware H264 decoders behind its software fallback.
#  include "gst_video_decoder.h"
#  include "gst_video_encoder.h"
#  include <api/video_codecs/video_decoder_software_fallback_wrapper.h>
#  include <media/engine/simulcast_encoder_adapter.h>
#endif
#if defined(WEBRTC_WIN)
// H264 and AV1 through Media Foundation.
#  include "mf_video.h"
#endif

#include <algorithm>
#include <cctype>
#include <memory>
#include <optional>
#include <string>
#include <vector>

namespace gelabber
{
	namespace
	{
		std::string Lower(std::string name)
		{
			std::transform(
			  name.begin(), name.end(), name.begin(), [](unsigned char c) { return std::tolower(c); });
			return name;
		}

		// libwebrtc's own encoders. Neither libwebrtc build carries H264
		// (rtc_use_h264=false): H264 is the system's alone, see EncoderFactory.
		using SoftwareEncoderFactory = webrtc::VideoEncoderFactoryTemplate<
		  webrtc::LibvpxVp8EncoderTemplateAdapter,
		  webrtc::LibvpxVp9EncoderTemplateAdapter,
		  webrtc::LibaomAv1EncoderTemplateAdapter>;

		// A software encoder that holds libwebrtc to its simulcast layers. libvpx
		// (VP8) encodes the layers of a producer in one encoder and
		// refuse a set in which a layer has not exactly the top layer's aspect
		// (WEBRTC_VIDEO_CODEC_ERR_SIMULCAST_PARAMETERS_NOT_SUPPORTED): the frame
		// has to divide by every layer's scaleResolutionDownBy. libwebrtc's own
		// factory puts them behind SimulcastEncoderAdapter, which then gives each
		// layer an encoder; the Windows package does not carry it. Told that the
		// encoder's alignment holds for every layer, libwebrtc works out what the
		// layers need (AlignmentAdjuster) and asks the source for such frames.
		// The sources crop to LocalVideoSource::kResolutionAlignment on their
		// own, so the web client's layers never wait for that.
		class SimulcastAlignedEncoder : public webrtc::VideoEncoder
		{
		public:
			explicit SimulcastAlignedEncoder(std::unique_ptr<webrtc::VideoEncoder> encoder)
			  : encoder(std::move(encoder))
			{
			}

			void SetFecControllerOverride(webrtc::FecControllerOverride* fecControllerOverride) override
			{
				encoder->SetFecControllerOverride(fecControllerOverride);
			}

			int InitEncode(const webrtc::VideoCodec* codec, const Settings& settings) override
			{
				return encoder->InitEncode(codec, settings);
			}

			int32_t RegisterEncodeCompleteCallback(webrtc::EncodedImageCallback* callback) override
			{
				return encoder->RegisterEncodeCompleteCallback(callback);
			}

			int32_t Release() override
			{
				return encoder->Release();
			}

			int32_t Encode(
			  const webrtc::VideoFrame& frame, const std::vector<webrtc::VideoFrameType>* types) override
			{
				return encoder->Encode(frame, types);
			}

			void SetRates(const RateControlParameters& parameters) override
			{
				encoder->SetRates(parameters);
			}

			void OnPacketLossRateUpdate(float packetLossRate) override
			{
				encoder->OnPacketLossRateUpdate(packetLossRate);
			}

			void OnRttUpdate(int64_t rttMs) override
			{
				encoder->OnRttUpdate(rttMs);
			}

			void OnLossNotification(const LossNotification& lossNotification) override
			{
				encoder->OnLossNotification(lossNotification);
			}

			EncoderInfo GetEncoderInfo() const override
			{
				auto info = encoder->GetEncoderInfo();
				info.apply_alignment_to_all_simulcast_layers = true;
				return info;
			}

		private:
			const std::unique_ptr<webrtc::VideoEncoder> encoder;
		};

		// The H264 formats the system's codecs take part in: Constrained
		// Baseline and Baseline in packetization mode 1, which the router and
		// every receiver have.
		std::vector<webrtc::SdpVideoFormat> SystemH264Formats()
		{
			std::vector<webrtc::SdpVideoFormat> formats;
			for (const char* profile : { "42e01f", "42001f" })
				formats.emplace_back(
				  "H264",
				  webrtc::CodecParameterMap{ { "level-asymmetry-allowed", "1" },
				                             { "packetization-mode", "1" },
				                             { "profile-level-id", profile } });
			return formats;
		}

		// libwebrtc's software encoders for VP8, VP9 and AV1, H264 through the
		// system where it has an encoder, and AV1 through the system where it
		// has a hardware encoder, in front of libaom (HardwareFirstEncoder),
		// which takes over when it fails. Linux: GStreamer; for H264 a hardware
		// element (VA-API/NVENC) else a software one, per simulcast layer behind
		// SimulcastEncoderAdapter, which hands a failing hardware encoder over
		// to the software element. Windows: Media Foundation, for H264 the GPU's
		// encoder else Microsoft's software one.
		class EncoderFactory : public webrtc::VideoEncoderFactory
		{
		public:
			EncoderFactory()
			{
	#if defined(WEBRTC_WIN)
				const auto& codecs = gelabber::FindMfCodecs(gelabber::SystemCodec::kH264);
				if (!codecs.encoder.empty())
					h264 = SystemH264Formats();
				h264Hardware = codecs.hardwareEncoder;
				const auto& av1Codecs = gelabber::FindMfCodecs(gelabber::SystemCodec::kAv1);
				av1System             = !av1Codecs.encoder.empty();
				av1Hardware           = av1Codecs.hardwareEncoder;
	#endif
	#if defined(WEBRTC_LINUX)
				const auto element = gelabber::FindGstEncoder(gelabber::SystemCodec::kH264);
				if (!element.empty())
				{
					h264         = SystemH264Formats();
					h264Hardware = gelabber::IsHardwareEncoder(element);
					primary      = gelabber::CreateGstEncoderFactory(gelabber::SystemCodec::kH264, element, h264);
					if (const auto software = gelabber::FindGstSoftwareEncoder(gelabber::SystemCodec::kH264);
					    h264Hardware && !software.empty())
						fallback = gelabber::CreateGstEncoderFactory(gelabber::SystemCodec::kH264, software, h264);
					RTC_LOG(LS_INFO) << "H264 encoder: GStreamer " << element;
				}
				av1Element  = gelabber::FindGstEncoder(gelabber::SystemCodec::kAv1);
				av1System   = !av1Element.empty();
				av1Hardware = gelabber::IsHardwareEncoder(av1Element);
				if (av1System)
					RTC_LOG(LS_INFO) << "AV1 encoder: GStreamer " << av1Element << " in front of libaom";
	#endif
			}

			std::vector<webrtc::SdpVideoFormat> GetSupportedFormats() const override
			{
				auto formats = software.GetSupportedFormats();
				formats.insert(formats.end(), h264.begin(), h264.end());
				return formats;
			}

			CodecSupport QueryCodecSupport(
			  const webrtc::SdpVideoFormat& format, std::optional<std::string> scalabilityMode) const override
			{
				const auto name = Lower(format.name);
				if (name == "av1")
				{
					auto support = software.QueryCodecSupport(format, scalabilityMode);
					support.is_power_efficient =
					  support.is_supported && av1Hardware && (!scalabilityMode || *scalabilityMode == "L1T1");
					return support;
				}
				if (name != "h264")
					return software.QueryCodecSupport(format, scalabilityMode);
				CodecSupport support;
				support.is_supported = !h264.empty() && format.IsCodecInList(h264) &&
				                       (!scalabilityMode || *scalabilityMode == "L1T1");
				support.is_power_efficient = support.is_supported && h264Hardware;
				return support;
			}

			std::unique_ptr<webrtc::VideoEncoder> Create(
			  const webrtc::Environment& env, const webrtc::SdpVideoFormat& format) override
			{
				const auto name = Lower(format.name);
				if (name == "h264")
				{
					if (h264.empty())
						return nullptr;
	#if defined(WEBRTC_WIN)
					return gelabber::CreateMfEncoder(gelabber::SystemCodec::kH264);
	#elif defined(WEBRTC_LINUX)
					return std::make_unique<webrtc::SimulcastEncoderAdapter>(
					  env, primary.get(), fallback.get(), format);
	#else
					return nullptr;
	#endif
				}
				auto encoder = software.Create(env, format);
				if (!encoder)
					return nullptr;
				if (name == "av1" && av1System)
				{
	#if defined(WEBRTC_WIN)
					auto system = gelabber::CreateMfEncoder(gelabber::SystemCodec::kAv1);
	#elif defined(WEBRTC_LINUX)
					auto system = gelabber::CreateGstEncoder(gelabber::SystemCodec::kAv1, av1Element);
	#else
					std::unique_ptr<webrtc::VideoEncoder> system;
	#endif
					if (system)
						return std::make_unique<gelabber::HardwareFirstEncoder>(std::move(system), std::move(encoder));
				}
				return std::make_unique<SimulcastAlignedEncoder>(std::move(encoder));
			}

		private:
			SoftwareEncoderFactory software;
			std::vector<webrtc::SdpVideoFormat> h264;
			bool h264Hardware{ false };
			// The system has an AV1 encoder; on the GPU (else only a forced
			// software element, for tests).
			bool av1System{ false };
			bool av1Hardware{ false };
	#if defined(WEBRTC_LINUX)
			std::unique_ptr<webrtc::VideoEncoderFactory> primary;
			std::unique_ptr<webrtc::VideoEncoderFactory> fallback;
			std::string av1Element;
	#endif
		};

		using SoftwareDecoderFactory = webrtc::VideoDecoderFactoryTemplate<
		  webrtc::LibvpxVp8DecoderTemplateAdapter,
		  webrtc::LibvpxVp9DecoderTemplateAdapter,
		  webrtc::Dav1dDecoderTemplateAdapter>;

		// libwebrtc's software decoders for VP8, VP9 and AV1, H264 through the
		// system where it has a decoder, and AV1 through the system where it
		// decodes on the GPU, in front of dav1d (HardwareFirstDecoder). Linux:
		// GStreamer; for H264 a hardware element (NVDEC/VA-API) behind a
		// software element it hands over to when it fails, else the software
		// element alone. Windows: Microsoft's decoders, on the GPU through DXVA
		// where they can.
		class DecoderFactory : public webrtc::VideoDecoderFactory
		{
		public:
			DecoderFactory()
			{
	#if defined(WEBRTC_WIN)
				if (!gelabber::FindMfCodecs(gelabber::SystemCodec::kH264).decoder.empty())
					h264 = SystemH264Formats();
				av1System = !gelabber::FindMfCodecs(gelabber::SystemCodec::kAv1).decoder.empty();
	#endif
	#if defined(WEBRTC_LINUX)
				element = gelabber::FindGstDecoder(gelabber::SystemCodec::kH264);
				if (!element.empty())
				{
					h264 = SystemH264Formats();
					if (gelabber::IsHardwareDecoder(element))
						fallback = gelabber::FindGstSoftwareDecoder(gelabber::SystemCodec::kH264);
					RTC_LOG(LS_INFO) << "H264 decoder: GStreamer " << element;
				}
				av1Element = gelabber::FindGstDecoder(gelabber::SystemCodec::kAv1);
				av1System  = !av1Element.empty();
				if (av1System)
					RTC_LOG(LS_INFO) << "AV1 decoder: GStreamer " << av1Element << " in front of dav1d";
	#endif
			}

			std::vector<webrtc::SdpVideoFormat> GetSupportedFormats() const override
			{
				auto formats = software.GetSupportedFormats();
				formats.insert(formats.end(), h264.begin(), h264.end());
				return formats;
			}

			std::unique_ptr<webrtc::VideoDecoder> Create(
			  const webrtc::Environment& env, const webrtc::SdpVideoFormat& format) override
			{
				const auto name = Lower(format.name);
				if (name == "av1" && av1System)
				{
					auto dav1d = software.Create(env, format);
	#if defined(WEBRTC_WIN)
					auto system = gelabber::CreateMfDecoder(gelabber::SystemCodec::kAv1);
	#elif defined(WEBRTC_LINUX)
					auto system = gelabber::CreateGstDecoder(gelabber::SystemCodec::kAv1, av1Element, true);
	#else
					std::unique_ptr<webrtc::VideoDecoder> system;
	#endif
					if (!dav1d || !system)
						return dav1d;
					return std::make_unique<gelabber::HardwareFirstDecoder>(std::move(system), std::move(dav1d));
				}
				if (name != "h264")
					return software.Create(env, format);
				if (h264.empty())
					return nullptr;
	#if defined(WEBRTC_WIN)
				return gelabber::CreateMfDecoder(gelabber::SystemCodec::kH264);
	#elif defined(WEBRTC_LINUX)
				if (fallback.empty())
					return gelabber::CreateGstDecoder(gelabber::SystemCodec::kH264, element, false);
				return webrtc::CreateVideoDecoderSoftwareFallbackWrapper(
				  env,
				  gelabber::CreateGstDecoder(gelabber::SystemCodec::kH264, fallback, false),
				  gelabber::CreateGstDecoder(gelabber::SystemCodec::kH264, element, true));
	#else
				return nullptr;
	#endif
			}

		private:
			SoftwareDecoderFactory software;
			std::vector<webrtc::SdpVideoFormat> h264;
			bool av1System{ false };
	#if defined(WEBRTC_LINUX)
			std::string element;
			std::string fallback;
			std::string av1Element;
	#endif
		};
	} // namespace

	std::unique_ptr<webrtc::VideoEncoderFactory> CreateVideoEncoderFactory()
	{
		return std::make_unique<EncoderFactory>();
	}

	std::unique_ptr<webrtc::VideoDecoderFactory> CreateVideoDecoderFactory()
	{
		return std::make_unique<DecoderFactory>();
	}
} // namespace gelabber

// A codec on the GPU with libwebrtc's own on the CPU behind it, for AV1
// (issue #318): the system's hardware encoder or decoder first, libaom or
// dav1d once it cannot start or fails while it runs. For good: a hardware
// codec that failed once is not tried again for that stream.
//
// libwebrtc's VideoEncoderSoftwareFallbackWrapper does the same, but the
// Windows package does not carry it (nor SimulcastEncoderAdapter), so both
// systems use these. AV1 goes out as one stream, never in simulcast layers.
#ifndef GELABBER_CODEC_FALLBACK_H
#define GELABBER_CODEC_FALLBACK_H

#include <api/video/encoded_image.h>
#include <api/video/video_frame.h>
#include <api/video_codecs/video_codec.h>
#include <api/video_codecs/video_decoder.h>
#include <api/video_codecs/video_encoder.h>
#include <modules/video_coding/include/video_error_codes.h>
#include <rtc_base/logging.h>

#include <memory>
#include <optional>
#include <utility>
#include <vector>

namespace gelabber
{
	class HardwareFirstEncoder : public webrtc::VideoEncoder
	{
	public:
		HardwareFirstEncoder(
		  std::unique_ptr<webrtc::VideoEncoder> hardware, std::unique_ptr<webrtc::VideoEncoder> software)
		  : hardware(std::move(hardware)), software(std::move(software))
		{
		}

		~HardwareFirstEncoder() override
		{
			Release();
		}

		void SetFecControllerOverride(webrtc::FecControllerOverride* fecControllerOverride) override
		{
			hardware->SetFecControllerOverride(fecControllerOverride);
			software->SetFecControllerOverride(fecControllerOverride);
		}

		int InitEncode(const webrtc::VideoCodec* codec, const Settings& settings) override
		{
			Release();
			if (!codec)
				return WEBRTC_VIDEO_CODEC_ERR_PARAMETER;
			this->codec    = *codec;
			this->settings = settings;
			rates.reset();
			if (!failed)
			{
				const int result = hardware->InitEncode(codec, settings);
				if (result == WEBRTC_VIDEO_CODEC_OK)
				{
					active = hardware.get();
					return result;
				}
				RTC_LOG(LS_WARNING) << hardware->GetEncoderInfo().implementation_name << " did not start ("
				                    << result << "), " << software->GetEncoderInfo().implementation_name
				                    << " takes over";
				hardware->Release();
				failed = true;
			}
			return StartSoftware();
		}

		int32_t RegisterEncodeCompleteCallback(webrtc::EncodedImageCallback* callback) override
		{
			this->callback = callback;
			hardware->RegisterEncodeCompleteCallback(callback);
			return software->RegisterEncodeCompleteCallback(callback);
		}

		int32_t Release() override
		{
			if (active)
				active->Release();
			active = nullptr;
			return WEBRTC_VIDEO_CODEC_OK;
		}

		int32_t Encode(const webrtc::VideoFrame& frame, const std::vector<webrtc::VideoFrameType>* types) override
		{
			if (!active)
				return WEBRTC_VIDEO_CODEC_UNINITIALIZED;
			const int32_t result = active->Encode(frame, types);
			if (result != WEBRTC_VIDEO_CODEC_FALLBACK_SOFTWARE || active != hardware.get())
				return result;
			RTC_LOG(LS_WARNING) << hardware->GetEncoderInfo().implementation_name << " failed, "
			                    << software->GetEncoderInfo().implementation_name << " takes over";
			hardware->Release();
			active = nullptr;
			failed = true;
			if (StartSoftware() != WEBRTC_VIDEO_CODEC_OK)
				return WEBRTC_VIDEO_CODEC_ERROR;
			// The receivers need a key frame from the new encoder.
			const std::vector<webrtc::VideoFrameType> key(
			  types && !types->empty() ? types->size() : 1, webrtc::VideoFrameType::kVideoFrameKey);
			return active->Encode(frame, &key);
		}

		void SetRates(const RateControlParameters& parameters) override
		{
			rates = parameters;
			if (active)
				active->SetRates(parameters);
		}

		void OnPacketLossRateUpdate(float packetLossRate) override
		{
			if (active)
				active->OnPacketLossRateUpdate(packetLossRate);
		}

		void OnRttUpdate(int64_t rttMs) override
		{
			if (active)
				active->OnRttUpdate(rttMs);
		}

		void OnLossNotification(const LossNotification& lossNotification) override
		{
			if (active)
				active->OnLossNotification(lossNotification);
		}

		EncoderInfo GetEncoderInfo() const override
		{
			if (active)
				return active->GetEncoderInfo();
			return (failed ? software : hardware)->GetEncoderInfo();
		}

	private:
		int StartSoftware()
		{
			if (callback)
				software->RegisterEncodeCompleteCallback(callback);
			const int result = software->InitEncode(&codec, settings);
			if (result != WEBRTC_VIDEO_CODEC_OK)
				return result;
			active = software.get();
			if (rates)
				active->SetRates(*rates);
			return result;
		}

		const std::unique_ptr<webrtc::VideoEncoder> hardware;
		const std::unique_ptr<webrtc::VideoEncoder> software;
		webrtc::VideoEncoder* active{ nullptr };
		bool failed{ false };
		webrtc::VideoCodec codec;
		Settings settings{ webrtc::VideoEncoder::Capabilities(false), 1, 0 };
		std::optional<RateControlParameters> rates;
		webrtc::EncodedImageCallback* callback{ nullptr };
	};

	class HardwareFirstDecoder : public webrtc::VideoDecoder
	{
	public:
		HardwareFirstDecoder(
		  std::unique_ptr<webrtc::VideoDecoder> hardware, std::unique_ptr<webrtc::VideoDecoder> software)
		  : hardware(std::move(hardware)), software(std::move(software))
		{
		}

		~HardwareFirstDecoder() override
		{
			Release();
		}

		bool Configure(const Settings& settings) override
		{
			Release();
			this->settings = settings;
			if (!failed)
			{
				if (hardware->Configure(settings))
				{
					active = hardware.get();
					return true;
				}
				RTC_LOG(LS_WARNING) << hardware->GetDecoderInfo().implementation_name << " did not start, "
				                    << software->GetDecoderInfo().implementation_name << " takes over";
				hardware->Release();
				failed = true;
			}
			return StartSoftware();
		}

		int32_t Decode(const webrtc::EncodedImage& input, int64_t renderTimeMs) override
		{
			if (!active)
				return WEBRTC_VIDEO_CODEC_UNINITIALIZED;
			const int32_t result = active->Decode(input, renderTimeMs);
			if (result != WEBRTC_VIDEO_CODEC_FALLBACK_SOFTWARE || active != hardware.get())
				return result;
			RTC_LOG(LS_WARNING) << hardware->GetDecoderInfo().implementation_name << " failed, "
			                    << software->GetDecoderInfo().implementation_name << " takes over";
			hardware->Release();
			active = nullptr;
			failed = true;
			if (!StartSoftware())
				return WEBRTC_VIDEO_CODEC_ERROR;
			// From a delta frame the new decoder has nothing to go on; the
			// error makes libwebrtc ask the sender for a key frame.
			if (input.FrameType() != webrtc::VideoFrameType::kVideoFrameKey)
				return WEBRTC_VIDEO_CODEC_ERROR;
			return active->Decode(input, renderTimeMs);
		}

		int32_t RegisterDecodeCompleteCallback(webrtc::DecodedImageCallback* callback) override
		{
			this->callback = callback;
			hardware->RegisterDecodeCompleteCallback(callback);
			return software->RegisterDecodeCompleteCallback(callback);
		}

		int32_t Release() override
		{
			if (active)
				active->Release();
			active = nullptr;
			return WEBRTC_VIDEO_CODEC_OK;
		}

		DecoderInfo GetDecoderInfo() const override
		{
			if (active)
				return active->GetDecoderInfo();
			return (failed ? software : hardware)->GetDecoderInfo();
		}

		const char* ImplementationName() const override
		{
			// The info's name is a temporary; this one has to outlive the call.
			name = GetDecoderInfo().implementation_name;
			return name.c_str();
		}

	private:
		bool StartSoftware()
		{
			if (callback)
				software->RegisterDecodeCompleteCallback(callback);
			if (!software->Configure(settings))
				return false;
			active = software.get();
			return true;
		}

		const std::unique_ptr<webrtc::VideoDecoder> hardware;
		const std::unique_ptr<webrtc::VideoDecoder> software;
		webrtc::VideoDecoder* active{ nullptr };
		bool failed{ false };
		Settings settings;
		webrtc::DecodedImageCallback* callback{ nullptr };
		mutable std::string name;
	};
} // namespace gelabber

#endif

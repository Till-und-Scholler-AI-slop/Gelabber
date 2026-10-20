#include "gst_h264_encoder.h"

#include "gst_common.h"

#include <api/environment/environment.h>
#include <api/video/encoded_image.h>
#include <api/video/i420_buffer.h>
#include <api/video/video_frame.h>
#include <api/video_codecs/video_codec.h>
#include <api/video_codecs/video_encoder.h>
#include <common_video/h264/h264_bitstream_parser.h>
#include <modules/video_coding/include/video_codec_interface.h>
#include <modules/video_coding/include/video_error_codes.h>
#include <rtc_base/logging.h>

#include <glib-object.h>
#include <libyuv/convert_from.h>

#include <algorithm>
#include <atomic>
#include <cstdlib>
#include <cstring>
#include <deque>
#include <initializer_list>
#include <mutex>
#include <optional>
#include <thread>

namespace gelabber
{
	namespace
	{
		using namespace gst;

		bool IsSoftware(const std::string& element)
		{
			return element == "x264enc" || element == "openh264enc";
		}

		// Bitrate property unit per element (kbit/s except openh264enc).
		void SetBitrate(GstElement* encoder, const std::string& element, uint32_t bps)
		{
			const uint32_t value = element == "openh264enc" ? bps : std::max<uint32_t>(1, bps / 1000);
			SetArg(encoder, "bitrate", std::to_string(value).c_str());
		}

		// Low-latency, IDR-on-request, no B-frames, CBR.
		void ConfigureEncoder(GstElement* encoder, const std::string& element)
		{
			if (element == "nvh264enc")
			{
				SetFirst(encoder, "preset", { "p3", "low-latency-hq" });
				SetArg(encoder, "tune", "ultra-low-latency");
				SetFirst(encoder, "rc-mode", { "cbr", "cbr-ld-hq" });
				SetArg(encoder, "bframes", "0");
				SetArg(encoder, "zerolatency", "true");
				SetArg(encoder, "gop-size", "-1");
				SetArg(encoder, "aud", "false");
			}
			else if (element == "vah264enc" || element == "vah264lpenc")
			{
				SetArg(encoder, "rate-control", "cbr");
				SetArg(encoder, "b-frames", "0");
				SetArg(encoder, "ref-frames", "1");
				SetArg(encoder, "key-int-max", "1024");
				SetArg(encoder, "target-usage", "6");
				SetArg(encoder, "aud", "false");
			}
			else if (element == "vaapih264enc")
			{
				SetArg(encoder, "rate-control", "cbr");
				SetArg(encoder, "max-bframes", "0");
				SetArg(encoder, "keyframe-period", "1024");
				SetArg(encoder, "aud", "false");
			}
			else if (element == "x264enc")
			{
				SetArg(encoder, "tune", "zerolatency");
				SetArg(encoder, "speed-preset", "ultrafast");
				SetArg(encoder, "bframes", "0");
				SetArg(encoder, "key-int-max", "1024");
				SetArg(encoder, "pass", "cbr");
			}
			else if (element == "openh264enc")
			{
				SetArg(encoder, "rate-control", "bitrate");
				SetArg(encoder, "gop-size", "1024");
				SetArg(encoder, "complexity", "low");
			}
		}

		bool IsIdr(const uint8_t* data, size_t size)
		{
			// Annex B: 00 00 01 or 00 00 00 01 start codes.
			for (size_t i = 0; i + 3 < size; ++i)
			{
				if (data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1)
				{
					if ((data[i + 3] & 0x1f) == 5)
						return true;
					i += 2;
				}
			}
			return false;
		}

		class GstH264Encoder : public webrtc::VideoEncoder
		{
		public:
			explicit GstH264Encoder(std::string element) : element(std::move(element))
			{
			}

			~GstH264Encoder() override
			{
				Release();
			}

			int InitEncode(const webrtc::VideoCodec* codec, const Settings&) override
			{
				Release();
				if (!codec || codec->codecType != webrtc::kVideoCodecH264 || codec->width < 2 ||
				    codec->height < 2 || codec->numberOfSimulcastStreams > 1)
					return WEBRTC_VIDEO_CODEC_ERR_PARAMETER;
				width  = codec->width & ~1;
				height = codec->height & ~1;
				fps    = std::max<int>(1, codec->maxFramerate);
				bitrate.store(std::max<uint32_t>(codec->startBitrate, 100) * 1000);

				// h264parse re-sends SPS/PPS with every IDR and splits access
				// units; the caps keep the stream decodable as constrained
				// baseline (the router's preferred H264 profile).
				const std::string description =
				  "appsrc name=src is-live=true format=time block=false max-buffers=3 "
				  "caps=video/x-raw,format=NV12,width=" +
				  std::to_string(width) + ",height=" + std::to_string(height) + ",framerate=" +
				  std::to_string(fps) + "/1 ! queue max-size-buffers=2 leaky=downstream ! " + element +
				  " name=enc ! video/x-h264,profile=(string){constrained-baseline,baseline} ! "
				  "h264parse config-interval=-1 ! "
				  "video/x-h264,stream-format=byte-stream,alignment=au ! "
				  "appsink name=sink sync=false max-buffers=8 drop=false";
				const auto& gst = Gst();
				GError* error   = nullptr;
				pipeline        = gst.parse_launch(description.c_str(), &error);
				if (!pipeline || error)
				{
					RTC_LOG(LS_WARNING) << "GStreamer " << element
					                    << " pipeline: " << (error ? error->message : "?");
					if (error)
						g_error_free(error);
					Release();
					return WEBRTC_VIDEO_CODEC_FALLBACK_SOFTWARE;
				}
				src     = gst.bin_get_by_name(pipeline, "src");
				encoder = gst.bin_get_by_name(pipeline, "enc");
				sink    = gst.bin_get_by_name(pipeline, "sink");
				bus     = gst.element_get_bus(pipeline);
				if (!src || !encoder || !sink || !bus)
				{
					Release();
					return WEBRTC_VIDEO_CODEC_FALLBACK_SOFTWARE;
				}
				ConfigureEncoder(encoder, element);
				SetBitrate(encoder, element, bitrate.load());
				appliedBitrate = bitrate.load();
				if (gst.element_set_state(pipeline, kStatePlaying) == kStateChangeFailure || CheckBus())
				{
					RTC_LOG(LS_WARNING) << "GStreamer " << element << " did not start";
					Release();
					return WEBRTC_VIDEO_CODEC_FALLBACK_SOFTWARE;
				}
				failed  = false;
				running = true;
				puller  = std::thread([this] { Pull(); });
				RTC_LOG(LS_INFO) << "GStreamer H264 " << element << " " << width << "x" << height << "@"
				                 << fps;
				return WEBRTC_VIDEO_CODEC_OK;
			}

			int32_t RegisterEncodeCompleteCallback(webrtc::EncodedImageCallback* callback) override
			{
				std::lock_guard lock(mutex);
				this->callback = callback;
				return WEBRTC_VIDEO_CODEC_OK;
			}

			int32_t Release() override
			{
				running = false;
				if (puller.joinable())
					puller.join();
				const auto& gst = Gst();
				if (pipeline)
					gst.element_set_state(pipeline, kStateNull);
				for (GstElement** element : { &src, &encoder, &sink })
				{
					if (*element)
						gst.object_unref(*element);
					*element = nullptr;
				}
				if (bus)
					gst.object_unref(bus);
				bus = nullptr;
				if (pipeline)
					gst.object_unref(pipeline);
				pipeline = nullptr;
				std::lock_guard lock(mutex);
				pending.clear();
				firstUs.reset();
				return WEBRTC_VIDEO_CODEC_OK;
			}

			int32_t Encode(
			  const webrtc::VideoFrame& frame, const std::vector<webrtc::VideoFrameType>* types) override
			{
				if (!pipeline)
					return WEBRTC_VIDEO_CODEC_UNINITIALIZED;
				if (failed)
					return WEBRTC_VIDEO_CODEC_FALLBACK_SOFTWARE;
				if (bitrate.load() == 0)
					return WEBRTC_VIDEO_CODEC_OK;
				const auto& gst = Gst();

				const uint32_t target = bitrate.load();
				if (target != appliedBitrate)
				{
					SetBitrate(encoder, element, target);
					appliedBitrate = target;
				}

				const bool key = types && std::any_of(types->begin(), types->end(), [](auto type) {
					                 return type == webrtc::VideoFrameType::kVideoFrameKey;
				                 });
				if (key)
					gst.element_send_event(
					  encoder, gst.video_event_new_upstream_force_key_unit(kClockTimeNone, TRUE, ++keyCount));

				auto i420 = frame.video_frame_buffer()->ToI420();
				if (!i420)
					return WEBRTC_VIDEO_CODEC_ERROR;
				if (i420->width() != width || i420->height() != height)
				{
					auto scaled = webrtc::I420Buffer::Create(width, height);
					scaled->ScaleFrom(*i420);
					i420 = scaled;
				}
				const size_t ySize = static_cast<size_t>(width) * height;
				GstBuffer* buffer  = gst.buffer_new_allocate(nullptr, ySize + ySize / 2, nullptr);
				GstMapInfo map{};
				if (!buffer || !gst.buffer_map(buffer, &map, kMapWrite))
				{
					if (buffer)
						gst.mini_object_unref(buffer);
					return WEBRTC_VIDEO_CODEC_ERROR;
				}
				libyuv::I420ToNV12(
				  i420->DataY(),
				  i420->StrideY(),
				  i420->DataU(),
				  i420->StrideU(),
				  i420->DataV(),
				  i420->StrideV(),
				  map.data,
				  width,
				  map.data + ySize,
				  width,
				  width,
				  height);
				gst.buffer_unmap(buffer, &map);

				const int64_t nowUs = frame.timestamp_us();
				GstClockTime pts;
				{
					std::lock_guard lock(mutex);
					if (!firstUs)
						firstUs = nowUs;
					pts = kPtsBase + static_cast<GstClockTime>(std::max<int64_t>(0, nowUs - *firstUs)) * 1000;
					if (!pending.empty() && pts <= pending.back().pts)
						pts = pending.back().pts + 1;
					pending.push_back({ pts,
					                    frame.rtp_timestamp(),
					                    frame.ntp_time_ms(),
					                    frame.render_time_ms(),
					                    frame.rotation(),
					                    frame.color_space() });
					// Bounded: the encoder may drop frames.
					while (pending.size() > 64)
						pending.pop_front();
				}
				buffer->pts      = pts;
				buffer->dts      = kClockTimeNone;
				buffer->duration = kSecond / fps;
				// Takes the buffer.
				if (gst.app_src_push_buffer(src, buffer) != 0)
					return WEBRTC_VIDEO_CODEC_ERROR;
				return WEBRTC_VIDEO_CODEC_OK;
			}

			void SetRates(const RateControlParameters& parameters) override
			{
				bitrate.store(parameters.bitrate.get_sum_bps());
			}

			EncoderInfo GetEncoderInfo() const override
			{
				EncoderInfo info;
				info.supports_native_handle         = false;
				info.implementation_name            = "GStreamer " + element;
				info.is_hardware_accelerated        = !IsSoftware(element);
				info.supports_simulcast             = false;
				info.requested_resolution_alignment = 2;
				info.scaling_settings               = ScalingSettings(24, 37);
				info.preferred_pixel_formats        = { webrtc::VideoFrameBuffer::Type::kI420,
					                                    webrtc::VideoFrameBuffer::Type::kNV12 };
				return info;
			}

		private:
			struct Pending
			{
				GstClockTime pts;
				uint32_t rtpTimestamp;
				int64_t ntpMs;
				int64_t renderMs;
				webrtc::VideoRotation rotation;
				std::optional<webrtc::ColorSpace> colorSpace;
			};

			// True when the pipeline reported an error.
			bool CheckBus()
			{
				return PopErrors(bus, element);
			}

			void Pull()
			{
				const auto& gst = Gst();
				webrtc::H264BitstreamParser parser;
				while (running)
				{
					if (CheckBus())
					{
						// Encode() hands over to the software encoder.
						failed = true;
						return;
					}
					GstSample* sample = gst.app_sink_try_pull_sample(sink, kSecond / 20);
					if (!sample)
						continue;
					GstBuffer* buffer = gst.sample_get_buffer(sample);
					GstMapInfo map{};
					if (buffer && gst.buffer_map(buffer, &map, kMapRead))
					{
						Deliver(parser, buffer->pts, map.data, map.size);
						gst.buffer_unmap(buffer, &map);
					}
					gst.mini_object_unref(sample);
				}
			}

			void Deliver(webrtc::H264BitstreamParser& parser, GstClockTime pts, const uint8_t* data, size_t size)
			{
				std::lock_guard lock(mutex);
				// Frames the encoder dropped stay in front of the match.
				while (!pending.empty() && pending.front().pts < pts)
					pending.pop_front();
				if (pending.empty() || pending.front().pts != pts || !callback || size == 0)
					return;
				const Pending meta = pending.front();
				pending.pop_front();

				webrtc::EncodedImage image;
				image.SetEncodedData(webrtc::EncodedImageBuffer::Create(data, size));
				image._encodedWidth  = width;
				image._encodedHeight = height;
				image.SetRtpTimestamp(meta.rtpTimestamp);
				image.ntp_time_ms_     = meta.ntpMs;
				image.capture_time_ms_ = meta.renderMs;
				image.rotation_        = meta.rotation;
				image.SetColorSpace(meta.colorSpace);
				const bool idr = IsIdr(data, size);
				image._frameType =
				  idr ? webrtc::VideoFrameType::kVideoFrameKey : webrtc::VideoFrameType::kVideoFrameDelta;
				parser.ParseBitstream(image);
				image.qp_ = parser.GetLastSliceQp().value_or(-1);

				webrtc::CodecSpecificInfo info;
				info.codecType                                = webrtc::kVideoCodecH264;
				info.codecSpecific.H264.packetization_mode    = webrtc::H264PacketizationMode::NonInterleaved;
				info.codecSpecific.H264.temporal_idx          = webrtc::kNoTemporalIdx;
				info.codecSpecific.H264.idr_frame             = idr;
				info.codecSpecific.H264.base_layer_sync       = false;
				callback->OnEncodedImage(image, &info);
			}

			const std::string element;
			GstElement* pipeline{ nullptr };
			GstElement* src{ nullptr };
			GstElement* encoder{ nullptr };
			GstElement* sink{ nullptr };
			GstBus* bus{ nullptr };
			int width{ 0 };
			int height{ 0 };
			int fps{ 30 };
			std::atomic<uint32_t> bitrate{ 0 };
			uint32_t appliedBitrate{ 0 };
			guint keyCount{ 0 };
			std::atomic<bool> running{ false };
			std::atomic<bool> failed{ false };
			std::thread puller;
			std::mutex mutex;
			webrtc::EncodedImageCallback* callback{ nullptr };
			std::deque<Pending> pending;
			std::optional<int64_t> firstUs;
		};

		class GstH264EncoderFactory : public webrtc::VideoEncoderFactory
		{
		public:
			GstH264EncoderFactory(std::string element, std::vector<webrtc::SdpVideoFormat> formats)
			  : element(std::move(element)), formats(std::move(formats))
			{
			}

			std::vector<webrtc::SdpVideoFormat> GetSupportedFormats() const override
			{
				return formats;
			}

			std::unique_ptr<webrtc::VideoEncoder> Create(
			  const webrtc::Environment&, const webrtc::SdpVideoFormat&) override
			{
				return std::make_unique<GstH264Encoder>(element);
			}

		private:
			const std::string element;
			const std::vector<webrtc::SdpVideoFormat> formats;
		};
	} // namespace

	std::string FindGstH264Encoder()
	{
		const char* forced = std::getenv("GELABBER_H264_ENCODER");
		if (forced && std::strcmp(forced, "none") == 0)
			return {};
		if (!Gst().ok)
			return {};
		// h264parse (gst-plugins-bad) splits the encoder output into access units.
		if (!HasElement("appsrc") || !HasElement("appsink") || !HasElement("h264parse"))
		{
			RTC_LOG(LS_INFO) << "GStreamer lacks appsrc/appsink/h264parse; software H264 only";
			return {};
		}
		if (forced && *forced)
		{
			if (HasElement(forced))
				return forced;
			RTC_LOG(LS_WARNING) << "GELABBER_H264_ENCODER=" << forced << " not found";
			return {};
		}
		for (const char* candidate : { "nvh264enc", "vah264enc", "vah264lpenc", "vaapih264enc" })
			if (HasElement(candidate))
				return candidate;
		RTC_LOG(LS_INFO) << "no hardware H264 encoder in GStreamer; software H264 only";
		return {};
	}

	bool IsHardwareH264Encoder(const std::string& element)
	{
		return !element.empty() && !IsSoftware(element);
	}

	std::unique_ptr<webrtc::VideoEncoderFactory> CreateGstH264EncoderFactory(
	  const std::string& element, std::vector<webrtc::SdpVideoFormat> formats)
	{
		return std::make_unique<GstH264EncoderFactory>(element, std::move(formats));
	}
} // namespace gelabber

#include "gst_h264_decoder.h"

#include "gst_common.h"

#include <api/video/encoded_image.h>
#include <api/video/i420_buffer.h>
#include <api/video/video_frame.h>
#include <api/video_codecs/video_decoder.h>
#include <modules/video_coding/include/video_error_codes.h>
#include <rtc_base/logging.h>

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
			return element == "avdec_h264" || element == "openh264dec";
		}

		// Inputs without a decoded frame after which a decoder counts as
		// stuck (two seconds at 30 fps): hardware decoders hold back a
		// frame or two, not dozens.
		constexpr int kStuckAfter = 60;

		// No frame delay: a call shows each frame as soon as it is decoded.
		void ConfigureDecoder(GstElement* decoder, const std::string& element)
		{
			if (element == "nvh264dec")
				SetArg(decoder, "max-display-delay", "0");
			else if (element == "vaapih264dec")
				SetArg(decoder, "low-latency", "true");
			else if (element == "avdec_h264")
			{
				// Frame threads hold back as many frames as there are threads.
				SetArg(decoder, "thread-type", "slice");
				SetArg(decoder, "output-corrupt", "false");
			}
		}

		// The decoder runs in a GStreamer pipeline and hands frames back from
		// its own thread, as asynchronous hardware decoders do; libwebrtc
		// matches them to their input by RTP timestamp.
		class GstH264Decoder : public webrtc::VideoDecoder
		{
		public:
			GstH264Decoder(std::string element, bool behindFallback)
			  : element(std::move(element)), behindFallback(behindFallback)
			{
			}

			~GstH264Decoder() override
			{
				Release();
			}

			bool Configure(const Settings& settings) override
			{
				Release();
				this->settings = settings;
				// h264parse turns libwebrtc's access units (Annex B) into what
				// the decoders take; videoconvert brings frames from GPU memory
				// to I420 in system memory.
				const std::string description =
				  "appsrc name=src is-live=true format=time block=false max-buffers=8 "
				  "caps=video/x-h264,stream-format=byte-stream,alignment=au ! h264parse ! " +
				  element +
				  " name=dec ! videoconvert ! video/x-raw,format=I420 ! "
				  "appsink name=sink sync=false max-buffers=4 drop=true";
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
					return false;
				}
				src     = gst.bin_get_by_name(pipeline, "src");
				decoder = gst.bin_get_by_name(pipeline, "dec");
				sink    = gst.bin_get_by_name(pipeline, "sink");
				bus     = gst.element_get_bus(pipeline);
				if (!src || !decoder || !sink || !bus)
				{
					Release();
					return false;
				}
				ConfigureDecoder(decoder, element);
				if (gst.element_set_state(pipeline, kStatePlaying) == kStateChangeFailure || PopErrors(bus, element))
				{
					RTC_LOG(LS_WARNING) << "GStreamer " << element << " did not start";
					Release();
					return false;
				}
				failed  = false;
				running = true;
				puller  = std::thread([this] { Pull(); });
				RTC_LOG(LS_INFO) << "GStreamer H264 decoder " << element;
				return true;
			}

			int32_t Decode(const webrtc::EncodedImage& input, int64_t renderTimeMs) override
			{
				if (!pipeline)
					return WEBRTC_VIDEO_CODEC_UNINITIALIZED;
				if (failed)
				{
					// Behind a fallback wrapper, which moves to the software
					// element and asks for a key frame; otherwise the pipeline
					// starts afresh.
					if (behindFallback)
						return WEBRTC_VIDEO_CODEC_FALLBACK_SOFTWARE;
					if (!Configure(settings))
						return WEBRTC_VIDEO_CODEC_ERROR;
				}
				if (!input.data() || input.size() == 0)
					return WEBRTC_VIDEO_CODEC_ERR_PARAMETER;
				const auto& gst   = Gst();
				GstBuffer* buffer = gst.buffer_new_allocate(nullptr, input.size(), nullptr);
				GstMapInfo map{};
				if (!buffer || !gst.buffer_map(buffer, &map, kMapWrite))
				{
					if (buffer)
						gst.mini_object_unref(buffer);
					return WEBRTC_VIDEO_CODEC_ERROR;
				}
				std::memcpy(map.data, input.data(), input.size());
				gst.buffer_unmap(buffer, &map);

				GstClockTime pts;
				{
					std::lock_guard lock(mutex);
					// Only the order counts: one millisecond per frame.
					pts = kPtsBase + (++count) * (kSecond / 1000);
					const auto* colorSpace = input.ColorSpace();
					pending.push_back({ pts,
					                    input.RtpTimestamp(),
					                    input.NtpTimeMs(),
					                    renderTimeMs,
					                    input.rotation(),
					                    colorSpace ? std::optional(*colorSpace) : std::nullopt });
					// Bounded: the decoder drops what it cannot decode.
					while (pending.size() > 64)
						pending.pop_front();
					// A decoder that stops putting out frames without an error
					// on the bus hands over like a failed one.
					if (++sinceOutput > kStuckAfter && !IsSoftware(element))
					{
						RTC_LOG(LS_WARNING) << "GStreamer " << element << " puts out no frames";
						failed = true;
					}
				}
				buffer->pts      = pts;
				buffer->dts      = pts;
				buffer->duration = kClockTimeNone;
				// Takes the buffer.
				if (gst.app_src_push_buffer(src, buffer) != 0)
					return WEBRTC_VIDEO_CODEC_ERROR;
				return WEBRTC_VIDEO_CODEC_OK;
			}

			int32_t RegisterDecodeCompleteCallback(webrtc::DecodedImageCallback* callback) override
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
				for (GstElement** element : { &src, &decoder, &sink })
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
				sinceOutput = 0;
				return WEBRTC_VIDEO_CODEC_OK;
			}

			DecoderInfo GetDecoderInfo() const override
			{
				DecoderInfo info;
				info.implementation_name     = "GStreamer " + element;
				info.is_hardware_accelerated = !IsSoftware(element);
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

			void Pull()
			{
				const auto& gst = Gst();
				while (running)
				{
					if (PopErrors(bus, element))
					{
						// Decode() hands over to the software decoder.
						failed = true;
						return;
					}
					GstSample* sample = gst.app_sink_try_pull_sample(sink, kSecond / 20);
					if (!sample)
						continue;
					Deliver(sample);
					gst.mini_object_unref(sample);
				}
			}

			void Deliver(GstSample* sample)
			{
				const auto& gst = Gst();
				GstBuffer* buffer = gst.sample_get_buffer(sample);
				GstCaps* caps     = gst.sample_get_caps(sample);
				const GstStructure* structure = caps ? gst.caps_get_structure(caps, 0) : nullptr;
				gint width = 0, height = 0;
				if (!buffer || !structure || !gst.structure_get_int(structure, "width", &width) ||
				    !gst.structure_get_int(structure, "height", &height) || width < 1 || height < 1)
					return;
				// The planes as the decoder laid them out, else GStreamer's
				// default I420 layout (rows of four bytes, even heights).
				const auto round4 = [](int value) { return (value + 3) & ~3; };
				const int chromaHeight = (height + 1) / 2;
				gsize offset[3]        = { 0, 0, 0 };
				gint stride[3]         = { round4(width), round4((width + 1) / 2), round4((width + 1) / 2) };
				offset[1]              = static_cast<gsize>(stride[0]) * ((height + 1) & ~1);
				offset[2]              = offset[1] + static_cast<gsize>(stride[1]) * chromaHeight;
				if (const GstVideoMeta* meta = gst.buffer_get_video_meta(buffer); meta && meta->n_planes >= 3)
					for (int plane = 0; plane < 3; ++plane)
					{
						offset[plane] = meta->offset[plane];
						stride[plane] = meta->stride[plane];
					}
				GstMapInfo map{};
				if (!gst.buffer_map(buffer, &map, kMapRead))
					return;
				const gsize needed = std::max({ offset[0] + static_cast<gsize>(stride[0]) * height,
				                                offset[1] + static_cast<gsize>(stride[1]) * chromaHeight,
				                                offset[2] + static_cast<gsize>(stride[2]) * chromaHeight });
				if (map.size < needed || stride[0] < width || stride[1] < (width + 1) / 2)
				{
					gst.buffer_unmap(buffer, &map);
					return;
				}
				auto i420 = webrtc::I420Buffer::Copy(
				  width,
				  height,
				  map.data + offset[0],
				  stride[0],
				  map.data + offset[1],
				  stride[1],
				  map.data + offset[2],
				  stride[2]);
				const GstClockTime pts = buffer->pts;
				gst.buffer_unmap(buffer, &map);

				Pending meta;
				webrtc::DecodedImageCallback* target;
				{
					std::lock_guard lock(mutex);
					// Frames the decoder dropped stay in front of the match.
					while (!pending.empty() && pending.front().pts < pts)
						pending.pop_front();
					if (pending.empty() || pending.front().pts != pts || !callback)
						return;
					meta = pending.front();
					pending.pop_front();
					sinceOutput = 0;
					target      = callback;
				}
				// Outside the lock: Decode() on libwebrtc's decoder thread
				// does not wait for the renderer.
				auto frame = webrtc::VideoFrame::Builder()
				               .set_video_frame_buffer(i420)
				               .set_rtp_timestamp(meta.rtpTimestamp)
				               .set_timestamp_ms(meta.renderMs)
				               .set_ntp_time_ms(meta.ntpMs)
				               .set_rotation(meta.rotation)
				               .set_color_space(meta.colorSpace)
				               .build();
				target->Decoded(frame, std::nullopt, std::nullopt);
			}

			const std::string element;
			const bool behindFallback;
			Settings settings;
			GstElement* pipeline{ nullptr };
			GstElement* src{ nullptr };
			GstElement* decoder{ nullptr };
			GstElement* sink{ nullptr };
			GstBus* bus{ nullptr };
			uint64_t count{ 0 };
			int sinceOutput{ 0 };
			std::atomic<bool> running{ false };
			std::atomic<bool> failed{ false };
			std::thread puller;
			std::mutex mutex;
			webrtc::DecodedImageCallback* callback{ nullptr };
			std::deque<Pending> pending;
		};
	} // namespace

	namespace
	{
		bool CanDecode()
		{
			if (!Gst().ok)
				return false;
			if (!HasElement("appsrc") || !HasElement("appsink") || !HasElement("h264parse") ||
			    !HasElement("videoconvert"))
			{
				RTC_LOG(LS_INFO) << "GStreamer lacks appsrc/appsink/h264parse/videoconvert; no H264 decoder";
				return false;
			}
			return true;
		}

		std::string FirstElement(std::initializer_list<const char*> candidates)
		{
			for (const char* candidate : candidates)
				if (HasElement(candidate))
					return candidate;
			return {};
		}
	} // namespace

	std::string FindGstH264Decoder()
	{
		const char* forced = std::getenv("GELABBER_H264_DECODER");
		if (forced && std::strcmp(forced, "none") == 0)
			return {};
		if (!CanDecode())
			return {};
		if (forced && *forced)
		{
			if (HasElement(forced))
				return forced;
			RTC_LOG(LS_WARNING) << "GELABBER_H264_DECODER=" << forced << " not found";
			return {};
		}
		if (auto hardware = FirstElement({ "nvh264dec", "vah264dec", "vaapih264dec" }); !hardware.empty())
			return hardware;
		RTC_LOG(LS_INFO) << "no hardware H264 decoder in GStreamer";
		return FindGstH264SoftwareDecoder();
	}

	std::string FindGstH264SoftwareDecoder()
	{
		const char* forced = std::getenv("GELABBER_H264_DECODER");
		if (forced && std::strcmp(forced, "none") == 0)
			return {};
		if (!CanDecode())
			return {};
		return FirstElement({ "avdec_h264", "openh264dec" });
	}

	bool IsHardwareH264Decoder(const std::string& element)
	{
		return !element.empty() && !IsSoftware(element);
	}

	std::unique_ptr<webrtc::VideoDecoder> CreateGstH264Decoder(const std::string& element, bool behindFallback)
	{
		return std::make_unique<GstH264Decoder>(element, behindFallback);
	}
} // namespace gelabber

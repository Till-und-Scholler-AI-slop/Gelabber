// Cameras through libwebrtc's video capture module (V4L2 on Linux,
// DirectShow on Windows). The module captures on its own thread and hands
// over I420 frames; they are adapted to the sinks' wants here.

#include "local_video_source.h"

#include <api/make_ref_counted.h>
#include <api/video/i420_buffer.h>
#include <api/video/video_frame.h>
#include <api/video/video_sink_interface.h>
#include <modules/video_capture/video_capture.h>
#include <modules/video_capture/video_capture_defines.h>
#include <modules/video_capture/video_capture_factory.h>
#include <rtc_base/logging.h>
#include <rtc_base/time_utils.h>

#include <memory>
#include <mutex>
#include <stdexcept>

namespace gelabber
{
	namespace
	{
		class CameraSource : public LocalVideoSource
		{
		public:
			void Start(const CameraOptions& options)
			{
				std::unique_ptr<webrtc::VideoCaptureModule::DeviceInfo> info(
				  webrtc::VideoCaptureFactory::CreateDeviceInfo());
				if (!info)
					throw std::runtime_error("no camera support on this system");
				std::string id = options.device;
				if (id.empty())
				{
					const auto cameras = ListCameras();
					if (cameras.empty())
						throw std::runtime_error("no camera found");
					id = cameras.front().id;
				}
				webrtc::VideoCaptureCapability wanted;
				wanted.width     = options.width;
				wanted.height    = options.height;
				wanted.maxFPS    = options.fps;
				wanted.videoType = webrtc::VideoType::kI420;
				webrtc::VideoCaptureCapability chosen;
				if (info->GetBestMatchedCapability(id.c_str(), wanted, chosen) < 0)
					chosen = wanted;

				module = webrtc::VideoCaptureFactory::Create(id.c_str());
				if (!module)
					throw std::runtime_error("cannot open camera " + id);
				module->SetApplyRotation(true);
				module->RegisterCaptureDataCallback(&sink);
				if (module->StartCapture(chosen) != 0)
				{
					module->DeRegisterCaptureDataCallback();
					module = nullptr;
					throw std::runtime_error("camera " + id + " is busy or failed to start");
				}
				RTC_LOG(LS_INFO) << "Camera " << id << ": " << chosen.width << "x" << chosen.height << " @ "
				                 << chosen.maxFPS;
			}

			void Stop() override
			{
				webrtc::scoped_refptr<webrtc::VideoCaptureModule> running;
				{
					std::lock_guard lock(mutex);
					running.swap(module);
					stopped = true;
				}
				// Outside the lock: stopping joins the capture thread, which
				// may be waiting for it in Deliver.
				if (running)
				{
					running->StopCapture();
					running->DeRegisterCaptureDataCallback();
				}
			}

			std::string StateJson() const override
			{
				std::lock_guard lock(mutex);
				return std::string(R"({"state":")") + (stopped ? "ended" : "live") + R"(","width":)" +
				       std::to_string(width) + R"(,"height":)" + std::to_string(height) +
				       R"(,"frames":)" + std::to_string(frames) + "}";
			}

			~CameraSource() override
			{
				Stop();
			}

		private:
			void Deliver(const webrtc::VideoFrame& frame)
			{
				const int srcWidth  = frame.width();
				const int srcHeight = frame.height();
				{
					std::lock_guard lock(mutex);
					width  = srcWidth;
					height = srcHeight;
					++frames;
				}
				const int64_t nowUs = webrtc::TimeMicros();
				int outWidth, outHeight, cropWidth, cropHeight, cropX, cropY;
				if (!AdaptFrame(
				      srcWidth,
				      srcHeight,
				      nowUs,
				      &outWidth,
				      &outHeight,
				      &cropWidth,
				      &cropHeight,
				      &cropX,
				      &cropY))
					return;
				webrtc::scoped_refptr<webrtc::VideoFrameBuffer> buffer = frame.video_frame_buffer();
				if (outWidth != srcWidth || outHeight != srcHeight)
				{
					auto scaled = webrtc::I420Buffer::Create(outWidth, outHeight);
					scaled->CropAndScaleFrom(*buffer->ToI420(), cropX, cropY, cropWidth, cropHeight);
					buffer = scaled;
				}
				OnFrame(webrtc::VideoFrame::Builder()
				          .set_video_frame_buffer(buffer)
				          .set_timestamp_us(nowUs)
				          .set_rotation(webrtc::kVideoRotation_0)
				          .build());
			}

			// The module's callback; a member so its OnFrame does not clash
			// with the track source's.
			struct Sink : webrtc::VideoSinkInterface<webrtc::VideoFrame>
			{
				explicit Sink(CameraSource* owner) : owner(owner)
				{
				}
				void OnFrame(const webrtc::VideoFrame& frame) override
				{
					owner->Deliver(frame);
				}
				CameraSource* owner;
			};

			Sink sink{ this };
			mutable std::mutex mutex;
			webrtc::scoped_refptr<webrtc::VideoCaptureModule> module;
			bool stopped{ false };
			int width{ 0 };
			int height{ 0 };
			uint64_t frames{ 0 };
		};
	} // namespace

	std::vector<CameraInfo> ListCameras()
	{
		std::vector<CameraInfo> out;
		std::unique_ptr<webrtc::VideoCaptureModule::DeviceInfo> info(
		  webrtc::VideoCaptureFactory::CreateDeviceInfo());
		if (!info)
			return out;
		char name[256];
		char id[256];
		for (uint32_t i = 0, n = info->NumberOfDevices(); i < n; ++i)
		{
			name[0] = id[0] = '\0';
			if (info->GetDeviceName(i, name, sizeof(name), id, sizeof(id)) == 0 && id[0] != '\0')
				out.push_back({ id, name[0] != '\0' ? name : id });
		}
		return out;
	}

	webrtc::scoped_refptr<LocalVideoSource> CreateCameraSource(const CameraOptions& options)
	{
		auto source = webrtc::make_ref_counted<CameraSource>();
		source->Start(options);
		return source;
	}
} // namespace gelabber

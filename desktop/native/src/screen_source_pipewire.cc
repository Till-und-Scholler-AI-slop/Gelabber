// Screen capture on Linux/Wayland: xdg-desktop-portal ScreenCast picks the
// source, PipeWire delivers the frames (libwebrtc's BaseCapturerPipeWire).
//
// The portal talks D-Bus through GLib and dispatches its replies on the
// thread-default GMainContext of the thread that started the request. The
// capture thread owns such a context and iterates it between frames, so the
// picker works without a GLib main loop in the host (Tauri's GTK loop or none
// at all in tests).

#include "local_video_source.h"

#include <api/make_ref_counted.h>
#include <api/video/i420_buffer.h>
#include <api/video/video_frame.h>
#include <modules/desktop_capture/delegated_source_list_controller.h>
#include <modules/desktop_capture/desktop_capture_options.h>
#include <modules/desktop_capture/desktop_capture_types.h>
#include <modules/desktop_capture/desktop_capturer.h>
#include <modules/desktop_capture/desktop_frame.h>
#include <modules/desktop_capture/linux/wayland/base_capturer_pipewire.h>
#include <rtc_base/logging.h>
#include <rtc_base/time_utils.h>

#include <glib.h>
#include <libyuv/convert.h>

#include <algorithm>
#include <atomic>
#include <chrono>
#include <memory>
#include <mutex>
#include <stdexcept>
#include <thread>

namespace gelabber
{
	namespace
	{
		webrtc::CaptureType captureType(ScreenOptions::Type type)
		{
			switch (type)
			{
				case ScreenOptions::Type::Screen:
					return webrtc::CaptureType::kScreen;
				case ScreenOptions::Type::Window:
					return webrtc::CaptureType::kWindow;
				default:
					return webrtc::CaptureType::kAnyScreenContent;
			}
		}

		class PipeWireScreenSource : public LocalVideoSource,
		                                   public webrtc::DesktopCapturer::Callback,
		                                   public webrtc::DelegatedSourceListController::Observer
		{
		public:
			explicit PipeWireScreenSource(const ScreenOptions& options) : options(options)
			{
			}

			~PipeWireScreenSource() override
			{
				Stop();
			}

			void Start()
			{
				running = true;
				worker  = std::thread([this] { Run(); });
			}

			void Stop() override
			{
				running = false;
				if (worker.joinable())
					worker.join();
			}

			// pending: picker open; live: frames flow; cancelled: the user
			// closed the picker; ended: the desktop stopped the stream;
			// failed: portal or PipeWire error.
			std::string StateJson() const override
			{
				std::lock_guard lock(mutex);
				std::string out = R"({"state":")" + current + R"(")";
				if (width > 0)
					out += R"(,"width":)" + std::to_string(width) + R"(,"height":)" + std::to_string(height);
				out += R"(,"frames":)" + std::to_string(frames) + "}";
				return out;
			}

			bool is_screencast() const override
			{
				return true;
			}

		private:
			void Run()
			{
				GMainContext* context = g_main_context_new();
				g_main_context_push_thread_default(context);
				{
					auto capture = webrtc::DesktopCaptureOptions::CreateDefault();
					capture.set_allow_pipewire(true);
					capture.set_prefer_cursor_embedded(options.cursor);
					auto capturer = std::make_unique<webrtc::BaseCapturerPipeWire>(
					  capture, captureType(options.type));
					capturer->GetDelegatedSourceListController()->Observe(this);
					capturer->Start(this);
					// Like Chromium: before the stream exists. The capture loop
					// below paces delivery to `fps` either way.
					capturer->SetMaxFrameRate(static_cast<uint32_t>(options.fps));

					const auto interval = std::chrono::microseconds(1'000'000 / std::max(1, options.fps));
					auto next           = std::chrono::steady_clock::now();
					while (running)
					{
						while (g_main_context_iteration(context, FALSE))
						{
						}
						if (State() == "live")
							capturer->CaptureFrame();
						next += interval;
						const auto now = std::chrono::steady_clock::now();
						if (next < now)
							next = now;
						std::this_thread::sleep_until(next);
					}
					capturer->GetDelegatedSourceListController()->Observe(nullptr);
					// Closes the portal session and the PipeWire stream.
					capturer.reset();
					while (g_main_context_iteration(context, FALSE))
					{
					}
				}
				g_main_context_pop_thread_default(context);
				g_main_context_unref(context);
			}

			std::string State() const
			{
				std::lock_guard lock(mutex);
				return current;
			}

			void SetState(const std::string& next)
			{
				std::lock_guard lock(mutex);
				// Terminal states stay.
				if (current == "pending" || current == "live")
					current = next;
			}

			// DelegatedSourceListController::Observer
			void OnSelection() override
			{
				SetState("live");
			}
			void OnCancelled() override
			{
				SetState("cancelled");
			}
			void OnError() override
			{
				SetState("failed");
			}

			// DesktopCapturer::Callback
			void OnCaptureResult(
			  webrtc::DesktopCapturer::Result result, std::unique_ptr<webrtc::DesktopFrame> frame) override
			{
				if (result == webrtc::DesktopCapturer::Result::ERROR_PERMANENT)
				{
					SetState(State() == "live" ? "ended" : "failed");
					return;
				}
				// ERROR_TEMPORARY: no frame from PipeWire yet.
				if (result != webrtc::DesktopCapturer::Result::SUCCESS || !frame || !frame->data())
					return;

				const int srcWidth  = frame->size().width();
				const int srcHeight = frame->size().height();
				if (srcWidth < 2 || srcHeight < 2)
					return;
				{
					// Counts captured frames, also while nothing consumes the
					// track yet (AdaptFrame then drops them).
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

				// DesktopFrame is BGRA in memory, libyuv's "ARGB".
				auto i420 = webrtc::I420Buffer::Create(srcWidth, srcHeight);
				libyuv::ARGBToI420(
				  frame->data(),
				  frame->stride(),
				  i420->MutableDataY(),
				  i420->StrideY(),
				  i420->MutableDataU(),
				  i420->StrideU(),
				  i420->MutableDataV(),
				  i420->StrideV(),
				  srcWidth,
				  srcHeight);
				webrtc::scoped_refptr<webrtc::VideoFrameBuffer> buffer = i420;
				if (outWidth != srcWidth || outHeight != srcHeight)
				{
					auto scaled = webrtc::I420Buffer::Create(outWidth, outHeight);
					scaled->CropAndScaleFrom(*i420, cropX, cropY, cropWidth, cropHeight);
					buffer = scaled;
				}
				OnFrame(webrtc::VideoFrame::Builder()
				          .set_video_frame_buffer(buffer)
				          .set_timestamp_us(nowUs)
				          .set_rotation(webrtc::kVideoRotation_0)
				          .build());
			}

			const ScreenOptions options;
			std::atomic<bool> running{ false };
			std::thread worker;
			mutable std::mutex mutex;
			std::string current{ "pending" };
			int width{ 0 };
			int height{ 0 };
			uint64_t frames{ 0 };
		};
	} // namespace

	webrtc::scoped_refptr<LocalVideoSource> CreateScreenSource(const ScreenOptions& options)
	{
		if (!webrtc::BaseCapturerPipeWire::IsSupported())
			throw std::runtime_error("screen capture needs a Wayland session with PipeWire");
		auto source = webrtc::make_ref_counted<PipeWireScreenSource>(options);
		source->Start();
		return source;
	}
} // namespace gelabber

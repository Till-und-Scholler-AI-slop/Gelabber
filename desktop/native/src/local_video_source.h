// Video sources the core drives itself (test pattern, screen, camera).
#ifndef GELABBER_LOCAL_VIDEO_SOURCE_H
#define GELABBER_LOCAL_VIDEO_SOURCE_H

#include <api/scoped_refptr.h>
#include <media/base/adapted_video_track_source.h>

#include <cstdint>
#include <optional>
#include <string>
#include <vector>

namespace gelabber
{
	class LocalVideoSource : public webrtc::AdaptedVideoTrackSource
	{
	public:
		// Both sides of every frame are a multiple of this: up to three
		// columns and rows are cropped away. A simulcast layer is the frame
		// divided by its scaleResolutionDownBy, and libvpx (VP8) and OpenH264
		// encode a producer's layers only when each has exactly the top
		// layer's aspect; a 1366x768 screen over layers of 4 and 1 would be
		// 342x192 below 1366x768, which they refuse, and nothing is sent. 4
		// is the largest factor the web client asks for
		// (web/src/voice/mediasoupConnection.ts), so its layers fit from the
		// first frame on. Other factors reach the source as an encoder's
		// wants a frame later (SimulcastAlignedEncoder in gelabber_media.cc).
		static constexpr int kResolutionAlignment = 4;

		LocalVideoSource() : webrtc::AdaptedVideoTrackSource(kResolutionAlignment)
		{
		}

		// Stops producing frames; idempotent, safe before destruction.
		virtual void Stop() = 0;
		// JSON object with at least {"state": "..."}; see gm_source_state.
		virtual std::string StateJson() const
		{
			return R"({"state":"live"})";
		}

		SourceState state() const override
		{
			return kLive;
		}
		bool remote() const override
		{
			return false;
		}
		std::optional<bool> needs_denoising() const override
		{
			return false;
		}

	protected:
		// The track source's own, which this hides, and false as well where
		// the alignment leaves nothing of the frame: a window a few pixels
		// high comes out with no rows at all.
		bool AdaptFrame(
		  int width,
		  int height,
		  int64_t timeUs,
		  int* outWidth,
		  int* outHeight,
		  int* cropWidth,
		  int* cropHeight,
		  int* cropX,
		  int* cropY)
		{
			return webrtc::AdaptedVideoTrackSource::AdaptFrame(
			         width, height, timeUs, outWidth, outHeight, cropWidth, cropHeight, cropX, cropY) &&
			       *outWidth > 0 && *outHeight > 0;
		}
	};

	struct ScreenOptions
	{
		enum class Type
		{
			Any,
			Screen,
			Window,
		};
		Type type{ Type::Any };
		int fps{ 30 };
		bool cursor{ true };
	};

	struct CameraInfo
	{
		std::string id;
		std::string name;
	};

	struct CameraOptions
	{
		// Empty: the first camera.
		std::string device;
		int width{ 1280 };
		int height{ 720 };
		int fps{ 30 };
	};

	// Cameras the platform's capture module sees.
	std::vector<CameraInfo> ListCameras();
	// Starts capturing from a camera at the closest supported format.
	// Throws when the camera is missing or busy.
	webrtc::scoped_refptr<LocalVideoSource> CreateCameraSource(const CameraOptions& options);

	// Screen or window chosen in the desktop's own picker. Starts capturing
	// right away; the picker opens asynchronously. Throws when the platform
	// has no supported capture path.
	webrtc::scoped_refptr<LocalVideoSource> CreateScreenSource(const ScreenOptions& options);
} // namespace gelabber

#endif

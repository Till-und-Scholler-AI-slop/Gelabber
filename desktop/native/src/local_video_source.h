// Video sources the core drives itself (test pattern, screen, camera).
#ifndef GELABBER_LOCAL_VIDEO_SOURCE_H
#define GELABBER_LOCAL_VIDEO_SOURCE_H

#include <api/scoped_refptr.h>
#include <media/base/adapted_video_track_source.h>

#include <optional>
#include <string>
#include <vector>

namespace gelabber
{
	class LocalVideoSource : public webrtc::AdaptedVideoTrackSource
	{
	public:
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

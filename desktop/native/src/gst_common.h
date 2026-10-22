// The system's GStreamer for H264 and AV1 (gst_video_encoder.cc,
// gst_video_decoder.cc).
//
// The few entry points the codecs need are resolved at runtime with dlopen:
// GStreamer stays an optional system dependency and its 1.x C ABI is stable.
// Types are opaque except the public structs read below (GstMapInfo,
// GstBuffer timestamps, GstVideoMeta planes), which are ABI-frozen.
#ifndef GELABBER_GST_COMMON_H
#define GELABBER_GST_COMMON_H

#include <glib-object.h>

#include <cstdint>
#include <initializer_list>
#include <string>

namespace gelabber::gst
{
	struct GstElement;
	struct GstBus;
	struct GstMessage;
	struct GstSample;
	struct GstEvent;
	struct GstMemory;
	struct GstCaps;
	struct GstStructure;
	using GstClockTime = uint64_t;
	constexpr GstClockTime kClockTimeNone = ~GstClockTime(0);
	constexpr GstClockTime kSecond        = 1'000'000'000;
	// Input timestamps start here. Elements built on GstVideoEncoder and
	// GstVideoDecoder (x264enc, va, nvcodec, libav) shift timestamps below
	// their "min PTS" (1000 hours) upwards, which would break matching an
	// output buffer to its input frame by PTS.
	constexpr GstClockTime kPtsBase = GstClockTime(10'000) * 3600 * kSecond;

	struct GstMiniObject
	{
		GType type;
		gint refcount;
		gint lockstate;
		guint flags;
		void* copy;
		void* dispose;
		void* free;
		guint priv_uint;
		gpointer priv_pointer;
	};
	struct GstBuffer
	{
		GstMiniObject mini_object;
		void* pool;
		GstClockTime pts;
		GstClockTime dts;
		GstClockTime duration;
		guint64 offset;
		guint64 offset_end;
	};
	struct GstMapInfo
	{
		GstMemory* memory;
		int flags;
		guint8* data;
		gsize size;
		gsize maxsize;
		gpointer user_data[4];
		gpointer reserved[4];
	};
	struct GstMeta
	{
		int flags;
		const void* info;
	};
	// The leading fields of GstVideoMeta (gst/video/gstvideometa.h, 1.0):
	// where a decoder put the planes of a frame in its buffer.
	struct GstVideoMeta
	{
		GstMeta meta;
		GstBuffer* buffer;
		int flags;
		int format;
		gint id;
		guint width;
		guint height;
		guint n_planes;
		gsize offset[4];
		gint stride[4];
	};
	// GST_BUFFER_FLAG_DELTA_UNIT: a buffer that is no key frame.
	constexpr guint kBufferFlagDeltaUnit = 1u << 13;
	constexpr int kMapRead            = 1;
	constexpr int kMapWrite           = 2;
	constexpr int kStateNull          = 1;
	constexpr int kStatePlaying       = 4;
	constexpr int kStateChangeFailure = 0;
	constexpr int kMessageError       = 1 << 1;

	struct Api
	{
		gboolean (*init_check)(int*, char***, GError**);
		GstElement* (*parse_launch)(const char*, GError**);
		GstElement* (*bin_get_by_name)(GstElement*, const char*);
		int (*element_set_state)(GstElement*, int);
		GstBus* (*element_get_bus)(GstElement*);
		gboolean (*element_send_event)(GstElement*, GstEvent*);
		void* (*element_factory_find)(const char*);
		void (*object_unref)(gpointer);
		GstMessage* (*bus_pop_filtered)(GstBus*, int);
		void (*message_parse_error)(GstMessage*, GError**, char**);
		void (*mini_object_unref)(void*);
		GstBuffer* (*buffer_new_allocate)(void*, gsize, void*);
		gboolean (*buffer_map)(GstBuffer*, GstMapInfo*, int);
		void (*buffer_unmap)(GstBuffer*, GstMapInfo*);
		GstBuffer* (*sample_get_buffer)(GstSample*);
		GstCaps* (*sample_get_caps)(GstSample*);
		GstStructure* (*caps_get_structure)(const GstCaps*, guint);
		gboolean (*structure_get_int)(const GstStructure*, const char*, gint*);
		void (*util_set_object_arg)(GObject*, const char*, const char*);
		int (*app_src_push_buffer)(GstElement*, GstBuffer*);
		GstSample* (*app_sink_try_pull_sample)(GstElement*, GstClockTime);
		GstEvent* (*video_event_new_upstream_force_key_unit)(GstClockTime, gboolean, guint);
		GstVideoMeta* (*buffer_get_video_meta)(GstBuffer*);

		bool ok{ false };
	};

	// GStreamer, loaded and initialised on first use; `ok` is false without it.
	const Api& Gst();

	bool HasElement(const std::string& name);

	// Sets a property by its string form (numbers, booleans, enum nicks,
	// flags) when the element has it; elements differ between plugin versions.
	bool SetArg(GstElement* element, const char* name, const char* value);
	void SetFirst(GstElement* element, const char* name, std::initializer_list<const char*> values);

	// Logs and drops the pipeline's error messages; true when there was one.
	bool PopErrors(GstBus* bus, const std::string& element);
} // namespace gelabber::gst

#endif

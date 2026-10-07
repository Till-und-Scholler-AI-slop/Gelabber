// Test-only "monitor" for fake-screencast-session.sh: a PipeWire Video/Source
// that behaves like a compositor's screencast stream (xdg-desktop-portal-wlr,
// -hyprland): it drives the graph itself and fills shared-memory (MemFd)
// buffers it allocates itself, with a moving BGRx pattern. libwebrtc's
// capturer reads only MemFd and DMA-BUF buffers; buffers PipeWire allocates
// (GStreamer's pipewiresink, or a stream without ALLOC_BUFFERS) arrive as
// MemPtr and are dropped.
//
// Build: cc -O2 -o fake_screen fake_screen.c $(pkg-config --cflags --libs libpipewire-0.3)
// Usage: fake_screen <width> <height> <fps>; prints the node id when ready.
#define _GNU_SOURCE
#include <pipewire/pipewire.h>
#include <spa/buffer/meta.h>
#include <spa/param/buffers.h>
#include <spa/param/video/format-utils.h>

#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

struct state
{
	struct pw_main_loop* loop;
	struct pw_stream* stream;
	struct spa_source* timer;
	int width, height, fps;
	uint32_t frame;
	int announced;
};

static void on_state(void* data, enum pw_stream_state old, enum pw_stream_state now, const char* error)
{
	struct state* s = data;
	fprintf(stderr, "fake screen: %s -> %s %s\n", pw_stream_state_as_string(old),
	        pw_stream_state_as_string(now), error ? error : "");
	if (now == PW_STREAM_STATE_PAUSED && !s->announced)
	{
		printf("%u\n", pw_stream_get_node_id(s->stream));
		fflush(stdout);
		s->announced = 1;
	}
	if (now == PW_STREAM_STATE_ERROR)
		pw_main_loop_quit(s->loop);
}

static void on_param_changed(void* data, uint32_t id, const struct spa_pod* param)
{
	struct state* s = data;
	if (!param || id != SPA_PARAM_Format)
		return;
	uint8_t buffer[1024];
	struct spa_pod_builder b = SPA_POD_BUILDER_INIT(buffer, sizeof(buffer));
	const int stride         = s->width * 4;
	const struct spa_pod* params[2];
	params[0] = spa_pod_builder_add_object(
	  &b, SPA_TYPE_OBJECT_ParamBuffers, SPA_PARAM_Buffers,
	  SPA_PARAM_BUFFERS_buffers, SPA_POD_CHOICE_RANGE_Int(4, 2, 16),
	  SPA_PARAM_BUFFERS_blocks, SPA_POD_Int(1),
	  SPA_PARAM_BUFFERS_size, SPA_POD_Int(stride * s->height),
	  SPA_PARAM_BUFFERS_stride, SPA_POD_Int(stride),
	  SPA_PARAM_BUFFERS_dataType, SPA_POD_CHOICE_FLAGS_Int(1 << SPA_DATA_MemFd));
	params[1] = spa_pod_builder_add_object(
	  &b, SPA_TYPE_OBJECT_ParamMeta, SPA_PARAM_Meta,
	  SPA_PARAM_META_type, SPA_POD_Id(SPA_META_Header),
	  SPA_PARAM_META_size, SPA_POD_Int(sizeof(struct spa_meta_header)));
	pw_stream_update_params(s->stream, params, 2);
}

static void on_add_buffer(void* data, struct pw_buffer* buffer)
{
	struct state* s    = data;
	struct spa_data* d = buffer->buffer->datas;
	// Before allocation, type holds the negotiated data types as flags.
	if (!(d[0].type & (1 << SPA_DATA_MemFd)))
	{
		fprintf(stderr, "fake screen: MemFd not negotiated (types 0x%x)\n", d[0].type);
		return;
	}
	d[0].type      = SPA_DATA_MemFd;
	d[0].flags     = SPA_DATA_FLAG_READWRITE | SPA_DATA_FLAG_MAPPABLE;
	d[0].fd        = memfd_create("gelabber-test-screen", MFD_CLOEXEC | MFD_ALLOW_SEALING);
	d[0].mapoffset = 0;
	d[0].maxsize   = s->width * 4 * s->height;
	if (d[0].fd < 0 || ftruncate(d[0].fd, d[0].maxsize) < 0)
	{
		perror("fake screen: memfd");
		return;
	}
	fcntl(d[0].fd, F_ADD_SEALS, F_SEAL_GROW | F_SEAL_SHRINK | F_SEAL_SEAL);
	d[0].data = mmap(NULL, d[0].maxsize, PROT_READ | PROT_WRITE, MAP_SHARED, d[0].fd, 0);
	if (d[0].data == MAP_FAILED)
		d[0].data = NULL;
}

static void on_remove_buffer(void* data, struct pw_buffer* buffer)
{
	(void)data;
	struct spa_data* d = buffer->buffer->datas;
	if (d[0].type != SPA_DATA_MemFd)
		return;
	if (d[0].data)
		munmap(d[0].data, d[0].maxsize);
	close(d[0].fd);
}

static void on_timeout(void* data, uint64_t expirations)
{
	struct state* s = data;
	struct pw_buffer* b = pw_stream_dequeue_buffer(s->stream);
	if (!b)
		return;
	struct spa_buffer* buf = b->buffer;
	uint8_t* pixels        = buf->datas[0].data;
	if (!pixels)
	{
		pw_stream_queue_buffer(s->stream, b);
		return;
	}
	const int stride = s->width * 4;
	// Diagonal gradient that moves every frame, plus a square crossing it.
	for (int y = 0; y < s->height; ++y)
	{
		uint8_t* row = pixels + y * stride;
		for (int x = 0; x < s->width; ++x)
		{
			row[x * 4 + 0] = (uint8_t)(x + s->frame * 3);
			row[x * 4 + 1] = (uint8_t)(y + s->frame * 2);
			row[x * 4 + 2] = (uint8_t)((x + y) / 4);
			row[x * 4 + 3] = 0xff;
		}
	}
	const int box = s->height / 4, bx = (s->frame * 8) % (s->width - box), by = s->height / 3;
	for (int y = by; y < by + box; ++y)
		memset(pixels + y * stride + bx * 4, 0xff, box * 4);

	struct spa_meta_header* header = spa_buffer_find_meta_data(buf, SPA_META_Header, sizeof(*header));
	if (header)
	{
		header->pts        = (int64_t)s->frame * SPA_NSEC_PER_SEC / s->fps;
		header->flags      = 0;
		header->seq        = s->frame;
		header->dts_offset = 0;
	}
	buf->datas[0].chunk->offset = 0;
	buf->datas[0].chunk->size   = stride * s->height;
	buf->datas[0].chunk->stride = stride;
	buf->datas[0].chunk->flags  = 0;
	s->frame++;
	pw_stream_queue_buffer(s->stream, b);
	pw_stream_trigger_process(s->stream);
}

static void on_process(void* data)
{
	(void)data;
}

static void on_signal(void* data, int signal_number)
{
	(void)signal_number;
	struct state* s = data;
	pw_main_loop_quit(s->loop);
}

static const struct pw_stream_events events = {
	PW_VERSION_STREAM_EVENTS,
	.state_changed = on_state,
	.param_changed = on_param_changed,
	.add_buffer    = on_add_buffer,
	.remove_buffer = on_remove_buffer,
	.process       = on_process,
};

int main(int argc, char** argv)
{
	if (argc != 4)
	{
		fprintf(stderr, "usage: %s <width> <height> <fps>\n", argv[0]);
		return 2;
	}
	struct state s = { 0 };
	s.width        = atoi(argv[1]);
	s.height       = atoi(argv[2]);
	s.fps          = atoi(argv[3]);
	pw_init(&argc, &argv);
	s.loop   = pw_main_loop_new(NULL);
	s.stream = pw_stream_new_simple(
	  pw_main_loop_get_loop(s.loop), "gelabber-test-screen",
	  pw_properties_new(PW_KEY_MEDIA_CLASS, "Video/Source", PW_KEY_NODE_NAME, "gelabber-test-screen", NULL),
	  &events, &s);

	uint8_t buffer[1024];
	struct spa_pod_builder b = SPA_POD_BUILDER_INIT(buffer, sizeof(buffer));
	struct spa_rectangle size  = SPA_RECTANGLE(s.width, s.height);
	struct spa_fraction rate   = SPA_FRACTION(0, 1);
	struct spa_fraction max    = SPA_FRACTION(s.fps, 1);
	struct spa_fraction minmax = SPA_FRACTION(1, 1);
	const struct spa_pod* params[1];
	params[0] = spa_pod_builder_add_object(
	  &b, SPA_TYPE_OBJECT_Format, SPA_PARAM_EnumFormat,
	  SPA_FORMAT_mediaType, SPA_POD_Id(SPA_MEDIA_TYPE_video),
	  SPA_FORMAT_mediaSubtype, SPA_POD_Id(SPA_MEDIA_SUBTYPE_raw),
	  SPA_FORMAT_VIDEO_format, SPA_POD_Id(SPA_VIDEO_FORMAT_BGRx),
	  SPA_FORMAT_VIDEO_size, SPA_POD_Rectangle(&size),
	  SPA_FORMAT_VIDEO_framerate, SPA_POD_Fraction(&rate),
	  SPA_FORMAT_VIDEO_maxFramerate, SPA_POD_CHOICE_RANGE_Fraction(&max, &minmax, &max));
	pw_stream_connect(s.stream, PW_DIRECTION_OUTPUT, PW_ID_ANY,
	                  PW_STREAM_FLAG_DRIVER | PW_STREAM_FLAG_ALLOC_BUFFERS, params, 1);

	pw_loop_add_signal(pw_main_loop_get_loop(s.loop), SIGINT, on_signal, &s);
	pw_loop_add_signal(pw_main_loop_get_loop(s.loop), SIGTERM, on_signal, &s);
	s.timer = pw_loop_add_timer(pw_main_loop_get_loop(s.loop), on_timeout, &s);
	struct timespec interval = { 0, 1000000000L / s.fps };
	pw_loop_update_timer(pw_main_loop_get_loop(s.loop), s.timer, &interval, &interval, false);

	pw_main_loop_run(s.loop);
	pw_stream_destroy(s.stream);
	pw_main_loop_destroy(s.loop);
	pw_deinit();
	return 0;
}

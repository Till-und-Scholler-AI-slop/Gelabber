/*
 * Gelabber native media core: C ABI over libmediasoupclient + libwebrtc.
 *
 * Only this C surface leaves the shared library. libwebrtc, libmediasoupclient
 * and Chromium's libc++ stay hidden inside it so they cannot clash with the
 * libstdc++ that WebKitGTK/Tauri load into the same process. On Windows the
 * DLL links its own static C/C++ runtime for the same reason.
 *
 * Conventions
 * - All JSON is UTF-8 and uses mediasoup's own parameter shapes, the same the
 *   server speaks in media/src/protocol.rs (RtpCapabilities, RtpParameters,
 *   DtlsParameters, IceParameters, IceCandidate).
 * - Strings returned by gm_* must be released with gm_string_free.
 * - A NULL/negative result means failure; gm_last_error() then describes it
 *   for the calling thread until the next gm_* call on that thread.
 * - gm_transport_produce and gm_transport_consume block the calling thread
 *   until the matching events (CONNECT, PRODUCE) were answered with
 *   gm_transport_respond. Never call them from the thread that answers.
 * - Events are delivered on internal threads. The callback must not block and
 *   must not call back into gm_* synchronously.
 */
#ifndef GELABBER_MEDIA_H
#define GELABBER_MEDIA_H

#include <stddef.h>
#include <stdint.h>

#if defined(_WIN32)
#  if defined(GM_BUILDING)
#    define GM_API __declspec(dllexport)
#  else
#    define GM_API __declspec(dllimport)
#  endif
#else
#  define GM_API __attribute__((visibility("default")))
#endif

#ifdef __cplusplus
extern "C" {
#endif

#define GM_ABI_VERSION 8

typedef struct gm_engine gm_engine;
typedef struct gm_device gm_device;
typedef struct gm_transport gm_transport;
typedef struct gm_source gm_source;
typedef struct gm_producer gm_producer;
typedef struct gm_consumer gm_consumer;

typedef enum gm_direction {
  GM_SEND = 0,
  GM_RECV = 1,
} gm_direction;

typedef enum gm_event_kind {
  /* Transport needs `connect` on the server. json: DtlsParameters.
   * Answer with gm_transport_respond(request, "{}", NULL) or an error. */
  GM_EVENT_CONNECT = 1,
  /* Transport needs `produce` on the server.
   * json: {"kind":"audio|video","rtpParameters":{...},"appData":{...}}.
   * Answer with gm_transport_respond(request, "{\"id\":\"<producerId>\"}", NULL). */
  GM_EVENT_PRODUCE = 2,
  /* json: {"state":"new|checking|connected|completed|failed|disconnected|closed"} */
  GM_EVENT_CONNECTION_STATE = 3,
} gm_event_kind;

typedef void (*gm_event_fn)(void* user, gm_event_kind kind, uint64_t request, const char* json);

/* Library */
GM_API uint32_t gm_abi_version(void);
GM_API const char* gm_last_error(void);
GM_API void gm_string_free(char* s);
/* Route libwebrtc logs to stderr: 0 none, 1 error, 2 warning, 3 info, 4 verbose. */
GM_API void gm_set_log_level(int level);

/* Engine: libwebrtc threads, PeerConnectionFactory, audio device module and
 * codec factories. One per process is enough.
 * options_json: {"audio":"default|dummy"} (dummy = no audio devices, for tests). */
GM_API gm_engine* gm_engine_new(const char* options_json);
GM_API void gm_engine_free(gm_engine* engine);

/* Audio devices of the engine's audio device module (Linux: PulseAudio API,
 * which PipeWire serves; Windows: Core Audio).
 * {"inputs":[{"id","name"}],"outputs":[{"id","name"}],
 * "input":"<id>","output":"<id>"}. The id "" is the system default; other ids
 * are the platform's device GUID where it reports one (Windows endpoint ids),
 * else the display name (PulseAudio). Monitor sources are not listed.
 * On Windows "" is the default device, not the default communications device:
 * streams on the latter make Windows turn other applications down. It is
 * looked up when capture or playout starts. */
GM_API char* gm_audio_devices(gm_engine* engine);
/* Live audio settings; every key is optional:
 * {"input"?: "<id>", "output"?: "<id>", "inputGain"?: 0..2}.
 * Switching a device restarts capture/playout if it runs. */
GM_API int gm_audio_configure(gm_engine* engine, const char* options_json);
/* Microphone test: with options ({"processingMode","inputGain"} as for
 * gm_source_new_microphone) capture runs and the meters below move without
 * a call; NULL ends the test. Capture a call needs keeps running either way.
 * The mode and gain apply engine-wide, also to a microphone sending. */
GM_API int gm_audio_monitor(gm_engine* engine, const char* options_json);
/* Microphone meters while capture runs, 0..100 like the web client's:
 * {"input": before RNNoise and gain, "processed": as sent, "clipping": bool,
 *  "denoised": bool (RNNoise ran), "blocks": 10 ms blocks processed so far
 *  (stops growing while capture is idle), "channels": of the last block}. */
GM_API char* gm_audio_levels(gm_engine* engine);

/* Device: loads router RTP capabilities (server `capabilities` frame). */
GM_API gm_device* gm_device_new(gm_engine* engine);
GM_API void gm_device_free(gm_device* device);
GM_API int gm_device_load(gm_device* device, const char* router_rtp_capabilities_json);
/* Receive capabilities to announce with the `capabilities` request. */
GM_API char* gm_device_rtp_capabilities(gm_device* device);
/* 1 if the device can produce kind ("audio"|"video"), 0 if not, -1 on error. */
GM_API int gm_device_can_produce(gm_device* device, const char* kind);

/* Transport from the server `transport` result:
 * {"id","iceParameters","iceCandidates","dtlsParameters","iceServers"?:[{urls,username?,credential?}],
 *  "iceTransportPolicy"?:"all|relay"} */
GM_API gm_transport* gm_device_create_transport(gm_device* device, gm_direction direction,
                                                const char* transport_json, gm_event_fn on_event,
                                                void* user);
GM_API void gm_transport_free(gm_transport* transport);
GM_API const char* gm_transport_id(gm_transport* transport);
/* Answer a CONNECT/PRODUCE event. Exactly one of result_json/error is non-NULL. */
GM_API int gm_transport_respond(gm_transport* transport, uint64_t request, const char* result_json,
                                const char* error);
GM_API int gm_transport_restart_ice(gm_transport* transport, const char* ice_parameters_json);
GM_API char* gm_transport_stats(gm_transport* transport);

/* Local sources. */
/* Microphone through the engine's audio device module and its processing,
 * matching the web client's modes. options_json (all optional):
 * {"processingMode": "enhanced|browser|original" (default enhanced),
 *  "echoCancellation": true, "noiseSuppression": true, "autoGainControl": true,
 *  "inputGain": 1.0}
 * enhanced: RNNoise, no WebRTC noise suppression/AGC; browser: WebRTC noise
 * suppression and AGC as configured; original: stereo, neither. Processing is
 * per engine (one capture path): the newest microphone source sets it. */
GM_API gm_source* gm_source_new_microphone(gm_engine* engine, const char* options_json);
/* Synthetic moving test pattern (I420) for build/pipeline checks. */
GM_API gm_source* gm_source_new_test_pattern(gm_engine* engine, int width, int height, int fps);
/* Screen or window picked in the desktop's own dialog (Linux: xdg-desktop-portal
 * ScreenCast + PipeWire). Returns at once; the dialog opens asynchronously and
 * frames flow after the user picked a source. Poll gm_source_state.
 * Not available on Windows yet: fails with an error there.
 * options_json: {"type"?: "any|screen|window", "fps"?: 30, "cursor"?: true,
 *                "contentHint"?: "detail|text|motion"} */
GM_API gm_source* gm_source_new_screen(gm_engine* engine, const char* options_json);
/* Applications playing sound, without this process:
 * [{"id","name","streams"}] (Linux: PipeWire playback streams; Windows: not
 * available yet, an empty list). What a virtual output device plays on to
 * the next (an echo canceller, an equaliser, a combined sink, a loopback) is
 * no application's sound and not listed. */
GM_API char* gm_audio_apps(gm_engine* engine);
/* Sound of other applications as an audio track for source audio, 48 kHz
 * stereo, separate from the microphone: {"app"?: id from gm_audio_apps;
 * default "" = every application but this one}. Applications that start
 * playing later are included. This process's own sound stays out also where
 * it plays through a virtual output device: each application is captured
 * where it plays, never where such a device plays it on.
 * gm_source_state adds "streams".
 * Not available on Windows yet: fails with an error there. */
GM_API gm_source* gm_source_new_app_audio(gm_engine* engine, const char* options_json);
/* Cameras: [{"id","name"}] (Linux: V4L2 devices; Windows: DirectShow; ids
 * are the module's unique ids). */
GM_API char* gm_video_devices(gm_engine* engine);
/* Camera at the closest format it supports:
 * {"device"?: id (default: first camera), "width"?: 1280, "height"?: 720,
 *  "fps"?: 30}. Fails when the camera is missing or busy. */
GM_API gm_source* gm_source_new_camera(gm_engine* engine, const char* options_json);
/* {"state":"pending|live|cancelled|ended|failed","width"?,"height"?,"frames"?}.
 * Microphone and test pattern are always live. */
GM_API char* gm_source_state(gm_source* source);
GM_API void gm_source_free(gm_source* source);
/* Disabled tracks keep their producer and send silence (audio) or black
 * frames (video), like MediaStreamTrack.enabled. */
GM_API int gm_source_set_enabled(gm_source* source, int enabled);

/* Produce a source. options_json:
 * {"codec"?: "video/H264"|"video/VP8"|..., "encodings"?: [{"scaleResolutionDownBy":4},{...}],
 *  "codecOptions"?: {...}, "appData"?: {...}}
 * Without "codec", video prefers H264 and falls back to VP8. The Windows
 * build has no H264.
 * Simulcast layers ("encodings") are a picture divided by their
 * scaleResolutionDownBy, to the pixel: a video source crops its picture to
 * what divides. That is up to 3 columns and rows for factors of 1, 2 and 4,
 * which every picture is ready for, and more for others once they are asked
 * for. */
GM_API gm_producer* gm_transport_produce(gm_transport* transport, gm_source* source,
                                         const char* options_json);
GM_API void gm_producer_free(gm_producer* producer);
GM_API const char* gm_producer_id(gm_producer* producer);
/* RtpParameters the server accepted (as sent in PRODUCE). */
GM_API char* gm_producer_rtp_parameters(gm_producer* producer);
GM_API int gm_producer_pause(gm_producer* producer, int paused);
GM_API char* gm_producer_stats(gm_producer* producer);
/* Swap the source keeping the producer (mediasoup replaceTrack). The source
 * must be of the producer's kind; the caller keeps the old one alive until
 * this returns. */
GM_API int gm_producer_replace_source(gm_producer* producer, gm_source* source);
/* The RTP sender's encodings, shaped like RTCRtpSendParameters:
 * {"encodings":[{"active","maxBitrate"?,"maxFramerate"?,"scaleResolutionDownBy"?,
 *  "priority"?,"networkPriority"?: "very-low|low|medium|high"}]} */
GM_API char* gm_producer_get_parameters(gm_producer* producer);
/* Update encodings by index with the same keys; null clears maxBitrate or
 * maxFramerate. Keys left out stay unchanged. priority and networkPriority
 * are per sender: set on any encoding, they apply to all. */
GM_API int gm_producer_set_parameters(gm_producer* producer, const char* parameters_json);

/* Consume a server `consumer` announcement:
 * {"id","producerId","kind":"audio|video","rtpParameters":{...},"appData"?:{...}} */
GM_API gm_consumer* gm_transport_consume(gm_transport* transport, const char* consumer_json);
GM_API void gm_consumer_free(gm_consumer* consumer);
GM_API const char* gm_consumer_id(gm_consumer* consumer);
GM_API int gm_consumer_pause(gm_consumer* consumer, int paused);
/* Playback volume of an audio consumer, 0..2 (1 = as received; 0 = silent). */
GM_API int gm_consumer_set_volume(gm_consumer* consumer, double volume);
/* A video frame in I420, valid only during the sink call. rotation is 0, 90,
 * 180 or 270 degrees clockwise to apply for display. source_width and
 * source_height are the size of the picture before the sink's limits scaled
 * it down (width and height when they did not). */
typedef struct gm_video_frame
{
  int width;
  int height;
  const uint8_t* y;
  const uint8_t* u;
  const uint8_t* v;
  int stride_y;
  int stride_u;
  int stride_v;
  int rotation;
  int64_t timestamp_us;
  int source_width;
  int source_height;
} gm_video_frame;
typedef void (*gm_video_frame_fn)(void* user, const gm_video_frame* frame);
/* What a video sink gets at most; 0 leaves a value unlimited. A picture
 * larger than max_width x max_height (as displayed, i.e. after rotation) is
 * scaled down to fit, aspect kept, to even dimensions; it is never scaled up.
 * Frames that arrive faster than max_fps are dropped. */
typedef struct gm_video_sink_limits
{
  int max_width;
  int max_height;
  int max_fps;
} gm_video_sink_limits;
/* Hands each decoded frame of a video consumer to fn on a decoder thread;
 * fn NULL removes the sink. Once this returns, the previous sink is not
 * running and is not called again. Free the consumer only after removing
 * a sink whose user data dies first. */
GM_API int gm_consumer_set_video_sink(gm_consumer* consumer, gm_video_frame_fn fn, void* user);
/* Limits for the consumer's sink, kept across sinks; NULL lifts them. */
GM_API int gm_consumer_set_video_sink_limits(gm_consumer* consumer,
                                             const gm_video_sink_limits* limits);
/* The same for a local video source (camera, screen, test pattern): each
 * frame as it goes to the encoders, black while the source is disabled, on
 * the source's capture thread, so fn has to return quickly. That is the
 * picture after the encoders' adaptation: while an encoder has the source
 * step down (a weak uplink, the first seconds of a producer) the sink gets
 * the smaller picture or lower rate too, and source_width/source_height are
 * that size, not the capture's. A sink keeps the source delivering without a
 * producer. gm_source_free removes a sink that is still set. Setting a sink
 * or limits on an audio source fails; removing a sink is fine for any
 * source. Calls for one source must not overlap. */
GM_API int gm_source_set_video_sink(gm_source* source, gm_video_frame_fn fn, void* user);
GM_API int gm_source_set_video_sink_limits(gm_source* source, const gm_video_sink_limits* limits);
/* {"framesReceived","width","height"} for video, {"audioLevel" 0..100,
 * "samplesPlayed"} for audio, plus libwebrtc stats under "rtc". Audio is only
 * decoded while playout runs. */
GM_API char* gm_consumer_stats(gm_consumer* consumer);

#ifdef __cplusplus
}
#endif

#endif

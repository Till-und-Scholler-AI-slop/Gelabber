/*
 * Gelabber native media core: C ABI over libmediasoupclient + libwebrtc.
 *
 * Only this C surface leaves the shared library. libwebrtc, libmediasoupclient
 * and Chromium's libc++ stay hidden inside it so they cannot clash with the
 * libstdc++ that WebKitGTK/Tauri load into the same process.
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

#define GM_ABI_VERSION 4

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
 * which PipeWire serves). {"inputs":[{"id","name"}],"outputs":[{"id","name"}],
 * "input":"<id>","output":"<id>"}. The id "" is the system default; other ids
 * are the platform's device GUID where it reports one, else the display name
 * (PulseAudio). Monitor sources are not listed. */
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
 * options_json: {"type"?: "any|screen|window", "fps"?: 30, "cursor"?: true,
 *                "contentHint"?: "detail|text|motion"} */
GM_API gm_source* gm_source_new_screen(gm_engine* engine, const char* options_json);
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
 * Without "codec", video prefers H264 and falls back to VP8. */
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
/* {"framesReceived","width","height"} for video, {"audioLevel" 0..100,
 * "samplesPlayed"} for audio, plus libwebrtc stats under "rtc". Audio is only
 * decoded while playout runs. */
GM_API char* gm_consumer_stats(gm_consumer* consumer);

#ifdef __cplusplus
}
#endif

#endif

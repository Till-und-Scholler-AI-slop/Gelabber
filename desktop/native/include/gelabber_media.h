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

#define GM_ABI_VERSION 1

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
/* Microphone through the engine's audio device module. */
GM_API gm_source* gm_source_new_microphone(gm_engine* engine);
/* Synthetic moving test pattern (I420) for build/pipeline checks. */
GM_API gm_source* gm_source_new_test_pattern(gm_engine* engine, int width, int height, int fps);
GM_API void gm_source_free(gm_source* source);

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

/* Consume a server `consumer` announcement:
 * {"id","producerId","kind":"audio|video","rtpParameters":{...},"appData"?:{...}} */
GM_API gm_consumer* gm_transport_consume(gm_transport* transport, const char* consumer_json);
GM_API void gm_consumer_free(gm_consumer* consumer);
GM_API const char* gm_consumer_id(gm_consumer* consumer);
GM_API int gm_consumer_pause(gm_consumer* consumer, int paused);
/* {"framesReceived","width","height"} for video, {} for audio, plus libwebrtc stats under "rtc". */
GM_API char* gm_consumer_stats(gm_consumer* consumer);

#ifdef __cplusplus
}
#endif

#endif

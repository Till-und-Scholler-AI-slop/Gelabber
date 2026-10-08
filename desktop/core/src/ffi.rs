//! Raw bindings for desktop/native/include/gelabber_media.h.
#![allow(non_camel_case_types)]

use std::os::raw::{c_char, c_int, c_void};

pub const GM_ABI_VERSION: u32 = 7;

#[repr(C)]
pub struct gm_video_frame {
    pub width: c_int,
    pub height: c_int,
    pub y: *const u8,
    pub u: *const u8,
    pub v: *const u8,
    pub stride_y: c_int,
    pub stride_u: c_int,
    pub stride_v: c_int,
    pub rotation: c_int,
    pub timestamp_us: i64,
}
pub type gm_video_frame_fn =
    Option<unsafe extern "C" fn(user: *mut c_void, frame: *const gm_video_frame)>;

#[repr(C)]
pub struct gm_engine {
    _private: [u8; 0],
}
#[repr(C)]
pub struct gm_device {
    _private: [u8; 0],
}
#[repr(C)]
pub struct gm_transport {
    _private: [u8; 0],
}
#[repr(C)]
pub struct gm_source {
    _private: [u8; 0],
}
#[repr(C)]
pub struct gm_producer {
    _private: [u8; 0],
}
#[repr(C)]
pub struct gm_consumer {
    _private: [u8; 0],
}

pub type gm_direction = c_int;
pub const GM_SEND: gm_direction = 0;
pub const GM_RECV: gm_direction = 1;

pub type gm_event_kind = c_int;
pub const GM_EVENT_CONNECT: gm_event_kind = 1;
pub const GM_EVENT_PRODUCE: gm_event_kind = 2;
pub const GM_EVENT_CONNECTION_STATE: gm_event_kind = 3;

pub type gm_event_fn =
    unsafe extern "C" fn(user: *mut c_void, kind: gm_event_kind, request: u64, json: *const c_char);

#[link(name = "gelabber_media")]
unsafe extern "C" {
    pub fn gm_abi_version() -> u32;
    pub fn gm_last_error() -> *const c_char;
    pub fn gm_string_free(s: *mut c_char);
    pub fn gm_set_log_level(level: c_int);

    pub fn gm_engine_new(options_json: *const c_char) -> *mut gm_engine;
    pub fn gm_engine_free(engine: *mut gm_engine);
    pub fn gm_audio_devices(engine: *mut gm_engine) -> *mut c_char;
    pub fn gm_audio_configure(engine: *mut gm_engine, options: *const c_char) -> c_int;
    pub fn gm_audio_monitor(engine: *mut gm_engine, options: *const c_char) -> c_int;
    pub fn gm_audio_levels(engine: *mut gm_engine) -> *mut c_char;

    pub fn gm_device_new(engine: *mut gm_engine) -> *mut gm_device;
    pub fn gm_device_free(device: *mut gm_device);
    pub fn gm_device_load(device: *mut gm_device, caps: *const c_char) -> c_int;
    pub fn gm_device_rtp_capabilities(device: *mut gm_device) -> *mut c_char;
    pub fn gm_device_can_produce(device: *mut gm_device, kind: *const c_char) -> c_int;

    pub fn gm_device_create_transport(
        device: *mut gm_device,
        direction: gm_direction,
        transport_json: *const c_char,
        on_event: gm_event_fn,
        user: *mut c_void,
    ) -> *mut gm_transport;
    pub fn gm_transport_free(transport: *mut gm_transport);
    pub fn gm_transport_id(transport: *mut gm_transport) -> *const c_char;
    pub fn gm_transport_respond(
        transport: *mut gm_transport,
        request: u64,
        result_json: *const c_char,
        error: *const c_char,
    ) -> c_int;
    pub fn gm_transport_restart_ice(transport: *mut gm_transport, ice: *const c_char) -> c_int;
    pub fn gm_transport_stats(transport: *mut gm_transport) -> *mut c_char;

    pub fn gm_source_new_microphone(
        engine: *mut gm_engine,
        options: *const c_char,
    ) -> *mut gm_source;
    pub fn gm_source_new_test_pattern(
        engine: *mut gm_engine,
        width: c_int,
        height: c_int,
        fps: c_int,
    ) -> *mut gm_source;
    pub fn gm_source_new_screen(engine: *mut gm_engine, options: *const c_char) -> *mut gm_source;
    pub fn gm_audio_apps(engine: *mut gm_engine) -> *mut c_char;
    pub fn gm_source_new_app_audio(
        engine: *mut gm_engine,
        options: *const c_char,
    ) -> *mut gm_source;
    pub fn gm_video_devices(engine: *mut gm_engine) -> *mut c_char;
    pub fn gm_source_new_camera(engine: *mut gm_engine, options: *const c_char) -> *mut gm_source;
    pub fn gm_source_state(source: *mut gm_source) -> *mut c_char;
    pub fn gm_source_free(source: *mut gm_source);
    pub fn gm_source_set_enabled(source: *mut gm_source, enabled: c_int) -> c_int;

    pub fn gm_transport_produce(
        transport: *mut gm_transport,
        source: *mut gm_source,
        options_json: *const c_char,
    ) -> *mut gm_producer;
    pub fn gm_producer_free(producer: *mut gm_producer);
    pub fn gm_producer_id(producer: *mut gm_producer) -> *const c_char;
    pub fn gm_producer_rtp_parameters(producer: *mut gm_producer) -> *mut c_char;
    pub fn gm_producer_pause(producer: *mut gm_producer, paused: c_int) -> c_int;
    pub fn gm_producer_stats(producer: *mut gm_producer) -> *mut c_char;
    pub fn gm_producer_replace_source(producer: *mut gm_producer, source: *mut gm_source) -> c_int;
    pub fn gm_producer_get_parameters(producer: *mut gm_producer) -> *mut c_char;
    pub fn gm_producer_set_parameters(
        producer: *mut gm_producer,
        parameters_json: *const c_char,
    ) -> c_int;

    pub fn gm_transport_consume(
        transport: *mut gm_transport,
        consumer_json: *const c_char,
    ) -> *mut gm_consumer;
    pub fn gm_consumer_free(consumer: *mut gm_consumer);
    pub fn gm_consumer_id(consumer: *mut gm_consumer) -> *const c_char;
    pub fn gm_consumer_pause(consumer: *mut gm_consumer, paused: c_int) -> c_int;
    pub fn gm_consumer_set_volume(consumer: *mut gm_consumer, volume: f64) -> c_int;
    pub fn gm_consumer_stats(consumer: *mut gm_consumer) -> *mut c_char;
    pub fn gm_consumer_set_video_sink(
        consumer: *mut gm_consumer,
        sink: gm_video_frame_fn,
        user: *mut c_void,
    ) -> c_int;
}

// Shared by build.rs (permissions) and media.rs.
/// Every command, for the build script's permissions and `generate_handler!`.
pub const MEDIA_COMMANDS: &[&str] = &[
    "media_info",
    "media_audio_devices",
    "media_audio_configure",
    "media_audio_levels",
    "media_audio_monitor",
    "media_device_load",
    "media_device_close",
    "media_transport_create",
    "media_transport_respond",
    "media_transport_restart_ice",
    "media_transport_stats",
    "media_transport_close",
    "media_source_microphone",
    "media_source_screen",
    "media_video_devices",
    "media_audio_apps",
    "media_source_app_audio",
    "media_source_camera",
    "media_source_state",
    "media_source_set_enabled",
    "media_source_close",
    "media_produce",
    "media_producer_pause",
    "media_producer_replace_source",
    "media_producer_parameters",
    "media_producer_set_parameters",
    "media_producer_stats",
    "media_producer_close",
    "media_consume",
    "media_consumer_pause",
    "media_consumer_set_volume",
    "media_consumer_stats",
    "media_consumer_close",
    "media_viewer_open",
    "media_viewer_close",
    "media_view_open",
    "media_view_configure",
    "media_view_frame",
    "media_view_close",
];

/// Window, taskbar and tray commands of the server origin (src/shell.rs).
pub const SHELL_COMMANDS: &[&str] = &[
    "shell_info",
    "shell_close_to_tray",
    "shell_focus",
    "shell_unread",
];
/// The update commands (update.rs). The server origin gets `update_state`,
/// `update_install` and `update_dismiss` (permissions/update.toml), the
/// bundled setup page all of them (capabilities/setup.json).
pub const UPDATE_COMMANDS: &[&str] = &[
    "update_state",
    "update_install",
    "update_dismiss",
    "update_check",
    "update_set_auto_check",
];

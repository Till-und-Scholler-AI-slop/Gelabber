//! Keeps the call in step with the rest of the app once per frame: the
//! saved media settings (voice/settings.ts), the call's messages as error
//! toasts (`onError: notifyError`), and no call after signing out.

use gelabber_call::{Call, CallSettings, ProcessingMode};

use crate::screens::settings::prefs::{self, Prefs};
use crate::toasts::Toasts;

/// The call's settings from the device settings.
pub fn settings(p: &Prefs) -> CallSettings {
    CallSettings {
        processing_mode: ProcessingMode::parse(&p.processing_mode),
        echo_cancellation: p.echo_cancellation,
        noise_suppression: p.noise_suppression,
        auto_gain_control: p.auto_gain_control,
        input_gain: p.input_gain,
        input: p.audio_input_id.clone(),
        output: p.audio_output_id.clone(),
        camera: p.video_input_id.clone(),
        output_volume: p.output_volume,
        stream_volume: p.source_audio_volume,
        stream_muted: p.source_audio_muted,
        camera_profile: p.camera_profile.clone(),
        screen_profile: p.screen_profile.clone(),
        keep_text_sharp: p.screen_mode == "sharp",
        economy_mode: p.economy_mode,
        video_upload_limit: p.video_upload_limit,
        source_audio_app: p.source_audio_app.clone(),
        share_source_audio: p.source_audio_share != "off",
        call_sounds: p.call_sounds,
        call_sound_volume: p.call_sound_volume,
        other_mute_sounds: p.other_mute_sounds,
        message_sound: p.message_sound,
        // Without a key push-to-talk keeps the microphone closed, as on the web.
        push_to_talk: p.voice_input == "ptt",
        ptt_release_delay_ms: p.ptt_release_delay.max(0.0).round() as u64,
        gate_auto: p.gate_auto,
        gate_threshold_db: p.gate_threshold_db,
        join_muted: p.join_muted,
        join_deafened: p.join_deafened,
    }
}

pub fn sync(ctx: &egui::Context, call: &Call, toasts: &mut Toasts, signed_in: bool) {
    keep_start(ctx, call);
    let wanted = settings(&prefs::get(ctx));
    if call.settings() != wanted {
        call.set_settings(wanted);
    }
    for text in call.take_notices() {
        toasts.error(text);
    }
    if !signed_in && call.with_state(|s| s.channel.is_some()) {
        call.leave();
    }
}

/// In a call, the device keeps its mute choice and deafen for the next join
/// (GEL-114), as `rememberJoinFlags` does on the web.
fn keep_start(ctx: &egui::Context, call: &Call) {
    let Some(kept) = call.with_state(|s| {
        s.channel
            .is_some()
            .then_some((s.pre_deafen_muted, s.deafened))
    }) else {
        return;
    };
    prefs::update(ctx, |p| (p.join_muted, p.join_deafened) = kept);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefs_reach_the_call() {
        let p = Prefs {
            voice_input: "ptt".into(),
            ..Prefs::default()
        };
        assert!(settings(&p).push_to_talk, "even without a key");
        assert!(!settings(&Prefs::default()).push_to_talk);
        assert_eq!(settings(&p).ptt_release_delay_ms, 200);
        assert_eq!(settings(&p).call_sound_volume, 0.35);
    }

    /// GEL-114: what a call leaves is how the next one starts; outside a
    /// call the kept start stays.
    #[test]
    fn a_call_keeps_its_mute_and_deafen_for_the_next_join() {
        use gelabber_call::{CallState, CallStatus};
        let _config = crate::platform::paths::testing::TempConfig::new("join-flags");
        let ctx = egui::Context::default();
        let call = Call::new(gelabber_client::Client::new("http://127.0.0.1:9").unwrap());
        let mut toasts = Toasts::default();
        call.set_state(CallState {
            status: CallStatus::Connected,
            server: Some(uuid::Uuid::from_u128(1)),
            channel: Some(uuid::Uuid::from_u128(2)),
            muted: true,
            deafened: true,
            pre_deafen_muted: false,
            ..CallState::default()
        });
        sync(&ctx, &call, &mut toasts, true);
        let p = prefs::get(&ctx);
        assert_eq!((p.join_muted, p.join_deafened), (false, true));
        assert!(call.settings().join_deafened && !call.settings().join_muted);
        call.set_state(CallState::default());
        sync(&ctx, &call, &mut toasts, true);
        let p = prefs::get(&ctx);
        assert_eq!((p.join_muted, p.join_deafened), (false, true));
    }
}

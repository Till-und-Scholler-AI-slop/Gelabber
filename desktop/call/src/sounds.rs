//! Plays the call cues (web/src/voice/callSounds.ts): one at a time, a new
//! one cuts the previous, on the system's default output. The media core has
//! no way to play a sound of its own, so Linux pipes the WAV into the sound
//! server's player and Windows hands it to PlaySound.

use std::collections::HashMap;

use parking_lot::Mutex;

use crate::synth::{self, Cue};

/// What decides whether and how loud a cue plays (`MediaSettings`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SoundSettings {
    pub enabled: bool,
    pub volume: f64,
    pub output_volume: f64,
    pub other_mute: bool,
}

/// `wanted` in callSounds.ts.
pub(crate) fn wanted(settings: SoundSettings, cue: Cue) -> bool {
    settings.enabled
        && settings.volume > 0.0
        && settings.output_volume > 0.0
        && (settings.other_mute || !matches!(cue, Cue::MuteOther | Cue::UnmuteOther))
}

/// The element volume of the web: the cue volume times the output volume,
/// which an element cannot raise above 100 %.
pub(crate) fn volume(settings: SoundSettings) -> f64 {
    settings.volume * settings.output_volume.min(1.0)
}

#[derive(Default)]
pub(crate) struct Player {
    rendered: Mutex<HashMap<Cue, std::sync::Arc<Vec<f32>>>>,
    deafened: Mutex<bool>,
    output: Mutex<Output>,
}

impl Player {
    fn samples(&self, cue: Cue) -> std::sync::Arc<Vec<f32>> {
        self.rendered
            .lock()
            .entry(cue)
            .or_insert_with(|| std::sync::Arc::new(synth::render(cue, cue.rate())))
            .clone()
    }

    /// A deafened call hears none of the call's events; switching stops
    /// what plays.
    pub(crate) fn set_deafened(&self, deafened: bool) {
        let mut current = self.deafened.lock();
        if *current != deafened {
            self.stop();
        }
        *current = deafened;
    }

    pub(crate) fn stop(&self) {
        self.output.lock().stop();
    }

    /// `remote`: an event of the call, as [`Cue::remote`] says by default.
    pub(crate) fn play(&self, cue: Cue, remote: Option<bool>, settings: SoundSettings) {
        let remote = remote.unwrap_or(cue.remote());
        if !wanted(settings, cue) || (*self.deafened.lock() && remote) {
            return;
        }
        let samples = self.samples(cue);
        let wav = synth::wave(&samples, cue.rate(), volume(settings));
        self.output.lock().play(wav);
    }
}

#[derive(Default)]
struct Output {
    #[cfg(unix)]
    child: Option<std::process::Child>,
    #[cfg(unix)]
    player: Option<usize>,
    #[cfg(windows)]
    playing: Option<Vec<u8>>,
}

/// Players that take a WAV on stdin, best first.
#[cfg(unix)]
const PLAYERS: &[&[&str]] = &[
    &["paplay", "--client-name=Gelabber"],
    &["pw-play", "-"],
    &["aplay", "-q"],
];

impl Output {
    #[cfg(unix)]
    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            // Reaped off this thread; it ends at once after the kill.
            std::thread::spawn(move || child.wait());
        }
    }

    #[cfg(unix)]
    fn play(&mut self, wav: Vec<u8>) {
        use std::io::Write;
        use std::process::{Command, Stdio};
        self.stop();
        let first = self.player.unwrap_or(0);
        for (index, args) in PLAYERS.iter().enumerate().skip(first) {
            let spawned = Command::new(args[0])
                .args(&args[1..])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            let Ok(mut child) = spawned else { continue };
            self.player = Some(index);
            if let Some(mut stdin) = child.stdin.take() {
                // A cue is a few hundred kilobytes at most; the pipe is
                // written off this thread so a slow player holds nobody up.
                std::thread::spawn(move || stdin.write_all(&wav));
            }
            self.child = Some(child);
            return;
        }
        tracing::debug!("no sound player for call cues");
    }

    #[cfg(windows)]
    fn stop(&mut self) {
        use windows_sys::Win32::Media::Audio::PlaySoundW;
        // SAFETY: a null sound stops what plays; it returns once stopped,
        // so the buffer of the previous cue can go after it.
        unsafe { PlaySoundW(std::ptr::null(), std::ptr::null_mut(), 0) };
        self.playing = None;
    }

    #[cfg(windows)]
    fn play(&mut self, wav: Vec<u8>) {
        use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
        self.stop();
        let wav = self.playing.insert(wav);
        // SAFETY: SND_MEMORY reads the WAV in place; it stays in `playing`
        // until the next stop, which waits for the sound to end.
        unsafe {
            PlaySoundW(
                wav.as_ptr().cast(),
                std::ptr::null_mut(),
                SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
            )
        };
    }

    #[cfg(not(any(unix, windows)))]
    fn stop(&mut self) {}

    #[cfg(not(any(unix, windows)))]
    fn play(&mut self, _wav: Vec<u8>) {}
}

impl Drop for Output {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> SoundSettings {
        SoundSettings {
            enabled: true,
            volume: 0.5,
            output_volume: 1.0,
            other_mute: false,
        }
    }

    #[test]
    fn sounds_follow_the_switches() {
        assert!(wanted(on(), Cue::JoinSelf));
        assert!(
            !wanted(on(), Cue::MuteOther),
            "others' mute needs its switch"
        );
        assert!(wanted(
            SoundSettings {
                other_mute: true,
                ..on()
            },
            Cue::UnmuteOther
        ));
        assert!(!wanted(
            SoundSettings {
                output_volume: 0.0,
                ..on()
            },
            Cue::JoinSelf
        ));
        assert!(!wanted(
            SoundSettings {
                enabled: false,
                ..on()
            },
            Cue::JoinSelf
        ));
    }

    #[test]
    fn volume_never_boosts() {
        let settings = SoundSettings {
            output_volume: 3.0,
            ..on()
        };
        assert_eq!(volume(settings), 0.5);
        let quiet = SoundSettings {
            output_volume: 0.5,
            ..on()
        };
        assert_eq!(volume(quiet), 0.25);
    }

    #[test]
    fn remote_cues_are_the_calls_events() {
        assert!(Cue::JoinOther.remote());
        assert!(Cue::LeaveSelf.remote());
        assert!(!Cue::MuteSelf.remote());
        assert!(!Cue::LiveStart.remote());
    }
}

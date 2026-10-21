//! Global push-to-talk: the key the page bound, held anywhere on the
//! desktop, not only while the app window has the focus.
//!
//! The page keeps the mode, the key and the microphone gate
//! (`web/src/voice/pushToTalk.ts`); this only reports the key going down and
//! up again. One binding at a time, and only while the page asks for it (in
//! a call); a page load drops it.
//!
//! - Windows: a low-level keyboard hook, or a mouse hook for the side
//!   buttons, on a thread of its own (`ptt/hook.rs`). It watches the bound
//!   key only and passes every key on: the key still reaches the window in
//!   front.
//! - Linux: the GlobalShortcuts portal of xdg-desktop-portal
//!   (`ptt/portal.rs`). The desktop owns the key: the page's key is only the
//!   suggestion KDE and GNOME offer when they ask the user; on Hyprland the
//!   user binds it in `hyprland.conf`.

use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{State, ipc::Channel};

#[cfg(windows)]
mod hook;
#[cfg(target_os = "linux")]
mod portal;

/// What the page hears about its binding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Event {
    /// The key works globally. `trigger` is the key as the desktop shows it
    /// (Linux), when it says.
    Bound {
        trigger: Option<String>,
    },
    /// No global key: the page keeps its own, focused-window listener.
    /// Windows refuses a hook at once, as the command's error.
    #[cfg_attr(windows, allow(dead_code))]
    Failed {
        reason: String,
    },
    Press,
    Release,
}

pub type Report = Arc<dyn Fn(Event) + Send + Sync>;

/// A running global binding; dropping it ends it.
pub trait Binding: Send {}

#[derive(Default)]
pub struct Ptt {
    binding: Mutex<Option<Box<dyn Binding>>>,
}

impl Ptt {
    /// Ends the binding of a page that went away.
    pub fn reset(&self) {
        let ended = self.binding.lock().map(|mut held| held.take());
        drop(ended);
    }
}

/// Binds `key` (a `KeyboardEvent.code`, or `Mouse4`/`Mouse5` for the side
/// buttons) and reports through `events`: `bound` or `failed`, then `press`
/// and `release`. Replaces an earlier binding of the page.
#[tauri::command]
pub fn ptt_bind(ptt: State<'_, Ptt>, key: String, events: Channel<Event>) -> Result<(), String> {
    if !KEYS.iter().any(|known| known.code == key) {
        return Err(format!("unknown key: {key}"));
    }
    let report: Report = Arc::new(move |event| {
        let _ = events.send(event);
    });
    let mut held = ptt.binding.lock().map_err(|e| e.to_string())?;
    // The old binding ends before the new one starts: on Linux both would
    // be portal sessions with the same shortcut.
    drop(held.take());
    *held = Some(start(&key, report)?);
    Ok(())
}

#[tauri::command]
pub fn ptt_unbind(ptt: State<'_, Ptt>) {
    ptt.reset();
}

#[cfg(windows)]
fn start(key: &str, report: Report) -> Result<Box<dyn Binding>, String> {
    let trigger = windows_trigger(key).ok_or("key without a Windows trigger")?;
    Ok(Box::new(hook::start(trigger, report)?))
}

#[cfg(target_os = "linux")]
fn start(key: &str, report: Report) -> Result<Box<dyn Binding>, String> {
    Ok(Box::new(portal::start(xkb_trigger(key), report)))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn start(_key: &str, _report: Report) -> Result<Box<dyn Binding>, String> {
    Err("no global keys on this system".into())
}

/// What a page's key means on Windows: a virtual key where Windows tells
/// left and right apart that way (modifiers, function keys), a scan code
/// where the key's place counts, as in `KeyboardEvent.code` (letters,
/// punctuation, the number pad whatever Num Lock says), or a side button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    Virtual(u32),
    Scan {
        code: u32,
        extended: bool,
    },
    /// `XBUTTON1` (1) or `XBUTTON2` (2).
    Mouse(u16),
}

struct Key {
    code: &'static str,
    windows: Trigger,
    /// The key's xkb keysym name, for the portal's suggested trigger; none
    /// for keys a desktop does not take alone (modifiers, mouse buttons).
    xkb: Option<&'static str>,
}

const fn vk(code: &'static str, key: u32, xkb: Option<&'static str>) -> Key {
    Key {
        code,
        windows: Trigger::Virtual(key),
        xkb,
    }
}

const fn sc(code: &'static str, scan: u32, xkb: &'static str) -> Key {
    Key {
        code,
        windows: Trigger::Scan {
            code: scan,
            extended: false,
        },
        xkb: Some(xkb),
    }
}

const fn ext(code: &'static str, scan: u32, xkb: &'static str) -> Key {
    Key {
        code,
        windows: Trigger::Scan {
            code: scan,
            extended: true,
        },
        xkb: Some(xkb),
    }
}

/// Every key a page may bind. Scan codes are set 1, as Windows reports them
/// (W3C UI Events KeyboardEvent code values, Windows column).
const KEYS: &[Key] = &[
    sc("Backquote", 0x29, "grave"),
    sc("Digit1", 0x02, "1"),
    sc("Digit2", 0x03, "2"),
    sc("Digit3", 0x04, "3"),
    sc("Digit4", 0x05, "4"),
    sc("Digit5", 0x06, "5"),
    sc("Digit6", 0x07, "6"),
    sc("Digit7", 0x08, "7"),
    sc("Digit8", 0x09, "8"),
    sc("Digit9", 0x0a, "9"),
    sc("Digit0", 0x0b, "0"),
    sc("Minus", 0x0c, "minus"),
    sc("Equal", 0x0d, "equal"),
    sc("Tab", 0x0f, "Tab"),
    sc("KeyQ", 0x10, "q"),
    sc("KeyW", 0x11, "w"),
    sc("KeyE", 0x12, "e"),
    sc("KeyR", 0x13, "r"),
    sc("KeyT", 0x14, "t"),
    sc("KeyY", 0x15, "y"),
    sc("KeyU", 0x16, "u"),
    sc("KeyI", 0x17, "i"),
    sc("KeyO", 0x18, "o"),
    sc("KeyP", 0x19, "p"),
    sc("BracketLeft", 0x1a, "bracketleft"),
    sc("BracketRight", 0x1b, "bracketright"),
    sc("KeyA", 0x1e, "a"),
    sc("KeyS", 0x1f, "s"),
    sc("KeyD", 0x20, "d"),
    sc("KeyF", 0x21, "f"),
    sc("KeyG", 0x22, "g"),
    sc("KeyH", 0x23, "h"),
    sc("KeyJ", 0x24, "j"),
    sc("KeyK", 0x25, "k"),
    sc("KeyL", 0x26, "l"),
    sc("Semicolon", 0x27, "semicolon"),
    sc("Quote", 0x28, "apostrophe"),
    sc("Backslash", 0x2b, "backslash"),
    sc("IntlBackslash", 0x56, "less"),
    sc("KeyZ", 0x2c, "z"),
    sc("KeyX", 0x2d, "x"),
    sc("KeyC", 0x2e, "c"),
    sc("KeyV", 0x2f, "v"),
    sc("KeyB", 0x30, "b"),
    sc("KeyN", 0x31, "n"),
    sc("KeyM", 0x32, "m"),
    sc("Comma", 0x33, "comma"),
    sc("Period", 0x34, "period"),
    sc("Slash", 0x35, "slash"),
    sc("Space", 0x39, "space"),
    sc("CapsLock", 0x3a, "Caps_Lock"),
    sc("ScrollLock", 0x46, "Scroll_Lock"),
    sc("Numpad0", 0x52, "KP_0"),
    sc("Numpad1", 0x4f, "KP_1"),
    sc("Numpad2", 0x50, "KP_2"),
    sc("Numpad3", 0x51, "KP_3"),
    sc("Numpad4", 0x4b, "KP_4"),
    sc("Numpad5", 0x4c, "KP_5"),
    sc("Numpad6", 0x4d, "KP_6"),
    sc("Numpad7", 0x47, "KP_7"),
    sc("Numpad8", 0x48, "KP_8"),
    sc("Numpad9", 0x49, "KP_9"),
    sc("NumpadDecimal", 0x53, "KP_Decimal"),
    sc("NumpadAdd", 0x4e, "KP_Add"),
    sc("NumpadSubtract", 0x4a, "KP_Subtract"),
    sc("NumpadMultiply", 0x37, "KP_Multiply"),
    ext("NumpadDivide", 0x35, "KP_Divide"),
    ext("NumpadEnter", 0x1c, "KP_Enter"),
    ext("Insert", 0x52, "Insert"),
    ext("Delete", 0x53, "Delete"),
    ext("Home", 0x47, "Home"),
    ext("End", 0x4f, "End"),
    ext("PageUp", 0x49, "Prior"),
    ext("PageDown", 0x51, "Next"),
    ext("ArrowUp", 0x48, "Up"),
    ext("ArrowDown", 0x50, "Down"),
    ext("ArrowLeft", 0x4b, "Left"),
    ext("ArrowRight", 0x4d, "Right"),
    ext("ContextMenu", 0x5d, "Menu"),
    vk("Pause", 0x13, Some("Pause")),
    vk("F1", 0x70, Some("F1")),
    vk("F2", 0x71, Some("F2")),
    vk("F3", 0x72, Some("F3")),
    vk("F4", 0x73, Some("F4")),
    vk("F5", 0x74, Some("F5")),
    vk("F6", 0x75, Some("F6")),
    vk("F7", 0x76, Some("F7")),
    vk("F8", 0x77, Some("F8")),
    vk("F9", 0x78, Some("F9")),
    vk("F10", 0x79, Some("F10")),
    vk("F11", 0x7a, Some("F11")),
    vk("F12", 0x7b, Some("F12")),
    vk("F13", 0x7c, Some("F13")),
    vk("F14", 0x7d, Some("F14")),
    vk("F15", 0x7e, Some("F15")),
    vk("F16", 0x7f, Some("F16")),
    vk("F17", 0x80, Some("F17")),
    vk("F18", 0x81, Some("F18")),
    vk("F19", 0x82, Some("F19")),
    vk("F20", 0x83, Some("F20")),
    vk("F21", 0x84, Some("F21")),
    vk("F22", 0x85, Some("F22")),
    vk("F23", 0x86, Some("F23")),
    vk("F24", 0x87, Some("F24")),
    // Left and right apart, as the low-level hook reports them.
    vk("ShiftLeft", 0xa0, None),
    vk("ShiftRight", 0xa1, None),
    vk("ControlLeft", 0xa2, None),
    vk("ControlRight", 0xa3, None),
    vk("AltLeft", 0xa4, None),
    vk("AltRight", 0xa5, None),
    vk("MetaLeft", 0x5b, None),
    vk("MetaRight", 0x5c, None),
    Key {
        code: "Mouse4",
        windows: Trigger::Mouse(1),
        xkb: None,
    },
    Key {
        code: "Mouse5",
        windows: Trigger::Mouse(2),
        xkb: None,
    },
];

fn key(code: &str) -> Option<&'static Key> {
    KEYS.iter().find(|key| key.code == code)
}

#[cfg_attr(not(windows), allow(dead_code))]
fn windows_trigger(code: &str) -> Option<Trigger> {
    key(code).map(|key| key.windows)
}

/// The portal's suggested trigger, in the shortcuts specification's form:
/// a keysym name.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn xkb_trigger(code: &str) -> Option<&'static str> {
    key(code).and_then(|key| key.xkb)
}

#[cfg(test)]
mod tests {
    use super::{Event, KEYS, Trigger, windows_trigger, xkb_trigger};
    use crate::media::PTT_COMMANDS;
    use std::collections::HashSet;

    #[test]
    fn each_key_once_and_each_windows_trigger_once() {
        let codes: HashSet<_> = KEYS.iter().map(|key| key.code).collect();
        assert_eq!(codes.len(), KEYS.len());
        let triggers: Vec<_> = KEYS.iter().map(|key| key.windows).collect();
        for (at, trigger) in triggers.iter().enumerate() {
            assert!(!triggers[at + 1..].contains(trigger), "{trigger:?} twice");
        }
    }

    #[test]
    fn keys_map_by_place_modifiers_and_function_keys_by_virtual_key() {
        assert_eq!(
            windows_trigger("KeyA"),
            Some(Trigger::Scan {
                code: 0x1e,
                extended: false
            })
        );
        // The same scan code as Numpad8, told apart by the extended flag.
        assert_eq!(
            windows_trigger("ArrowUp"),
            Some(Trigger::Scan {
                code: 0x48,
                extended: true
            })
        );
        assert_eq!(
            windows_trigger("ControlRight"),
            Some(Trigger::Virtual(0xa3))
        );
        assert_eq!(windows_trigger("F13"), Some(Trigger::Virtual(0x7c)));
        assert_eq!(windows_trigger("Mouse5"), Some(Trigger::Mouse(2)));
        assert_eq!(windows_trigger("Escape"), None);
        assert_eq!(windows_trigger(""), None);
    }

    #[test]
    fn the_portal_gets_keysyms_and_nothing_for_modifiers_or_buttons() {
        assert_eq!(xkb_trigger("Backquote"), Some("grave"));
        assert_eq!(xkb_trigger("F13"), Some("F13"));
        assert_eq!(xkb_trigger("KeyV"), Some("v"));
        assert_eq!(xkb_trigger("ControlLeft"), None);
        assert_eq!(xkb_trigger("Mouse4"), None);
    }

    #[test]
    fn events_reach_the_page_as_tagged_objects() {
        let json = |event: Event| serde_json::to_value(event).unwrap();
        assert_eq!(json(Event::Press), serde_json::json!({ "type": "press" }));
        assert_eq!(
            json(Event::Bound {
                trigger: Some("F13".into())
            }),
            serde_json::json!({ "type": "bound", "trigger": "F13" })
        );
        assert_eq!(
            json(Event::Failed {
                reason: "no portal".into()
            }),
            serde_json::json!({ "type": "failed", "reason": "no portal" })
        );
    }

    /// The server origin's `ptt` set grants exactly these commands.
    #[test]
    fn ptt_permission_set_matches_the_commands() {
        let set = include_str!("../permissions/ptt.toml");
        let granted: Vec<&str> = set
            .lines()
            .filter_map(|line| line.trim().strip_prefix("\"allow-"))
            .filter_map(|rest| rest.strip_suffix("\","))
            .collect();
        let expected: Vec<String> = PTT_COMMANDS.iter().map(|c| c.replace('_', "-")).collect();
        assert_eq!(granted, expected);
        let main = include_str!("main.rs");
        for command in PTT_COMMANDS {
            assert!(
                main.contains(&format!("ptt::{command},")),
                "{command} missing from generate_handler!"
            );
        }
    }
}

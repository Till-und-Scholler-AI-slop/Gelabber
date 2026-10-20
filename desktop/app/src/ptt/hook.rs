//! Windows: the bound key through a low-level keyboard or mouse hook.
//!
//! The hook runs on a thread of its own, whose message loop Windows calls it
//! from. It must answer fast (`LowLevelHooksTimeout`, after which Windows
//! drops it unasked), so it only notes the bound key's state and hands
//! changes to a second thread that talks to the page. It never swallows a
//! key. Windows keeps a hook of an ordinary process away from windows of
//! elevated ones: while such a window is in front the key is not seen.

use std::{cell::RefCell, ptr, sync::mpsc, thread};

use windows_sys::Win32::{
    Foundation::{LPARAM, LRESULT, WPARAM},
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::WindowsAndMessaging::{
        CallNextHookEx, GetMessageW, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, MSG,
        MSLLHOOKSTRUCT, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, SetWindowsHookExW,
        UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_QUIT,
        WM_SYSKEYDOWN, WM_SYSKEYUP, WM_USER, WM_XBUTTONDOWN, WM_XBUTTONUP,
    },
};

use super::{Binding, Event, Report, Trigger};

/// The hook thread; dropping it ends the hook.
pub struct Hook {
    thread: u32,
}

impl Binding for Hook {}

impl Drop for Hook {
    fn drop(&mut self) {
        // SAFETY: plain message post; a thread that already ended makes it
        // fail, which is the outcome wanted anyway.
        unsafe { PostThreadMessageW(self.thread, WM_QUIT, 0, 0) };
    }
}

/// What the hook procedure sees: the trigger, whether it is down, and where
/// changes go. Lives on the hook thread.
struct Watch {
    trigger: Trigger,
    down: bool,
    changes: mpsc::Sender<bool>,
}

thread_local! {
    static WATCH: RefCell<Option<Watch>> = const { RefCell::new(None) };
}

/// The key's new state, when this message changes it.
fn change(trigger: Trigger, message: u32, data: Data) -> Option<bool> {
    match (trigger, data) {
        (Trigger::Virtual(_) | Trigger::Scan { .. }, Data::Key { vk, scan, extended }) => {
            let ours = match trigger {
                Trigger::Virtual(key) => vk == key,
                Trigger::Scan { code, extended: e } => scan == code && extended == e,
                Trigger::Mouse(_) => false,
            };
            match message {
                _ if !ours => None,
                WM_KEYDOWN | WM_SYSKEYDOWN => Some(true),
                WM_KEYUP | WM_SYSKEYUP => Some(false),
                _ => None,
            }
        }
        (Trigger::Mouse(button), Data::Mouse { button: pressed }) if pressed == button => {
            match message {
                WM_XBUTTONDOWN => Some(true),
                WM_XBUTTONUP => Some(false),
                _ => None,
            }
        }
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum Data {
    Key { vk: u32, scan: u32, extended: bool },
    Mouse { button: u16 },
}

fn note(message: u32, data: Data) {
    WATCH.with_borrow_mut(|watch| {
        let Some(watch) = watch else { return };
        // Held keys repeat their key-down; only changes count.
        if let Some(down) = change(watch.trigger, message, data)
            && down != watch.down
        {
            watch.down = down;
            let _ = watch.changes.send(down);
        }
    });
}

unsafe extern "system" fn keyboard(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: for HC_ACTION, lparam points to the event's KBDLLHOOKSTRUCT.
        let event = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        note(
            wparam as u32,
            Data::Key {
                vk: event.vkCode,
                scan: event.scanCode,
                extended: event.flags & LLKHF_EXTENDED != 0,
            },
        );
    }
    // SAFETY: passing the event on, as every hook must.
    unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
}

unsafe extern "system" fn mouse(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let message = wparam as u32;
    if code == HC_ACTION as i32 && matches!(message, WM_XBUTTONDOWN | WM_XBUTTONUP) {
        // SAFETY: for HC_ACTION, lparam points to the event's MSLLHOOKSTRUCT.
        let event = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
        // The high word names the side button.
        let button = (event.mouseData >> 16) as u16;
        note(message, Data::Mouse { button });
    }
    // SAFETY: as in `keyboard`.
    unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
}

/// Starts the hook thread and waits until the hook is in place.
pub fn start(trigger: Trigger, report: Report) -> Result<Hook, String> {
    let (changes, changed) = mpsc::channel::<bool>();
    let (ready, started) = mpsc::channel::<Result<u32, String>>();
    thread::Builder::new()
        .name("gelabber-ptt-hook".into())
        .spawn(move || run(trigger, changes, ready))
        .map_err(|e| e.to_string())?;
    let thread = started
        .recv()
        .map_err(|_| "push-to-talk hook thread ended".to_owned())??;
    // Ends when the hook thread drops its sender.
    thread::Builder::new()
        .name("gelabber-ptt-events".into())
        .spawn(move || {
            report(Event::Bound { trigger: None });
            for down in changed {
                report(if down { Event::Press } else { Event::Release });
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(Hook { thread })
}

fn run(trigger: Trigger, changes: mpsc::Sender<bool>, ready: mpsc::Sender<Result<u32, String>>) {
    let (kind, procedure) = match trigger {
        Trigger::Mouse(_) => (
            WH_MOUSE_LL,
            mouse as unsafe extern "system" fn(_, _, _) -> _,
        ),
        _ => (
            WH_KEYBOARD_LL,
            keyboard as unsafe extern "system" fn(_, _, _) -> _,
        ),
    };
    let mut message: MSG = unsafe { std::mem::zeroed() };
    // SAFETY: plain Win32 calls on this thread. PeekMessage gives the
    // thread its message queue, so the WM_QUIT of `Hook::drop` cannot get
    // lost before the loop runs.
    let hook = unsafe {
        PeekMessageW(&mut message, ptr::null_mut(), WM_USER, WM_USER, PM_NOREMOVE);
        SetWindowsHookExW(kind, Some(procedure), GetModuleHandleW(ptr::null()), 0)
    };
    if hook.is_null() {
        let error = std::io::Error::last_os_error();
        let _ = ready.send(Err(format!("SetWindowsHookEx: {error}")));
        return;
    }
    WATCH.set(Some(Watch {
        trigger,
        down: false,
        changes,
    }));
    // SAFETY: as above.
    let _ = ready.send(Ok(unsafe { GetCurrentThreadId() }));
    // The hook procedures run inside GetMessage; it returns 0 for WM_QUIT
    // and -1 on failure.
    while unsafe { GetMessageW(&mut message, ptr::null_mut(), 0, 0) } > 0 {}
    // SAFETY: the hook this thread set.
    unsafe { UnhookWindowsHookEx(hook) };
    // Drops the sender: the event thread ends. A key still held ends with
    // a release, so the page does not keep sending.
    if let Some(watch) = WATCH.take()
        && watch.down
    {
        let _ = watch.changes.send(false);
    }
}

#[cfg(test)]
mod tests {
    use super::{Data, Trigger, change};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_XBUTTONDOWN, WM_XBUTTONUP,
    };

    fn key(vk: u32, scan: u32, extended: bool) -> Data {
        Data::Key { vk, scan, extended }
    }

    #[test]
    fn scan_triggers_need_the_extended_flag_to_match() {
        let up_arrow = Trigger::Scan {
            code: 0x48,
            extended: true,
        };
        assert_eq!(
            change(up_arrow, WM_KEYDOWN, key(0x26, 0x48, true)),
            Some(true)
        );
        assert_eq!(
            change(up_arrow, WM_KEYUP, key(0x26, 0x48, true)),
            Some(false)
        );
        // Numpad 8: same scan code, no extended flag.
        assert_eq!(change(up_arrow, WM_KEYDOWN, key(0x68, 0x48, false)), None);
    }

    #[test]
    fn virtual_triggers_count_alt_combinations() {
        let alt = Trigger::Virtual(0xa4);
        assert_eq!(
            change(alt, WM_SYSKEYDOWN, key(0xa4, 0x38, false)),
            Some(true)
        );
        assert_eq!(change(alt, WM_KEYDOWN, key(0xa5, 0x38, true)), None);
    }

    #[test]
    fn mouse_triggers_take_their_button_only() {
        let back = Trigger::Mouse(1);
        assert_eq!(
            change(back, WM_XBUTTONDOWN, Data::Mouse { button: 1 }),
            Some(true)
        );
        assert_eq!(
            change(back, WM_XBUTTONUP, Data::Mouse { button: 1 }),
            Some(false)
        );
        assert_eq!(
            change(back, WM_XBUTTONDOWN, Data::Mouse { button: 2 }),
            None
        );
        assert_eq!(change(back, WM_KEYDOWN, key(0x05, 0, false)), None);
    }
}

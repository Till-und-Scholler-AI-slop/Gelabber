//! The Windows AppUserModelID of the process: the one installer.nsi writes
//! into the shortcuts and toast.rs registers, so the window groups with a
//! pinned shortcut on the taskbar and the toasts belong to the app.

/// The bundle id of the Tauri app before 0.9, which its installs keep.
pub const AUMID: &str = super::paths::IDENTIFIER;

/// Before the first window: Windows reads the id when the window appears.
#[cfg(windows)]
pub fn set_for_process() -> Result<(), String> {
    use windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
    let id: Vec<u16> = AUMID.encode_utf16().chain([0]).collect();
    // SAFETY: a NUL-terminated UTF-16 string that outlives the call.
    let result = unsafe { SetCurrentProcessExplicitAppUserModelID(id.as_ptr()) };
    if result < 0 {
        return Err(format!(
            "SetCurrentProcessExplicitAppUserModelID: {result:#x}"
        ));
    }
    Ok(())
}

/// Only Windows has one.
#[cfg(not(windows))]
pub fn set_for_process() -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_process_has_the_id_of_the_shortcuts_and_the_toasts() {
        let nsi = include_str!("../../../packaging/native/windows/installer.nsi");
        assert!(nsi.contains(&format!("!define BUNDLEID \"{AUMID}\"")));
        // The Tauri app's identifier: its shortcuts and toast entry carry it.
        assert_eq!(AUMID, "io.github.till-und-scholler-ai-slop.gelabber");
        let toast = include_str!("notify/toast.rs");
        assert!(toast.contains("crate::platform::paths::IDENTIFIER"));
        assert_eq!(set_for_process(), Ok(()));
    }
}

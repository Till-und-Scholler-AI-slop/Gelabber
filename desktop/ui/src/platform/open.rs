//! Opening a link in the system's browser or mail program: the one way the
//! app hands a URL to the OS. Only http, https and mailto pass, and the URL
//! never goes through a shell (a chat link's `&` must not start a command).

/// The schemes the app opens outside itself.
const SCHEMES: [&str; 3] = ["http", "https", "mailto"];

/// The URL as it may be opened, or why not.
fn checked(url: &str) -> Result<String, String> {
    let parsed = url::Url::parse(url.trim()).map_err(|e| format!("{url}: {e}"))?;
    if !SCHEMES.contains(&parsed.scheme()) {
        return Err(format!("{url}: only http, https and mailto links open")); // i18n-ok
    }
    Ok(parsed.into())
}

/// Opens `url` in the default browser (mail program for `mailto:`).
pub fn open_url(url: &str) -> Result<(), String> {
    let url = checked(url)?;
    #[cfg(test)]
    if testing::record(&url) {
        return Ok(());
    }
    launch(&url)
}

/// Tests: the URLs this thread opened instead of handing them to the OS.
#[cfg(test)]
pub(crate) mod testing {
    use std::cell::RefCell;

    thread_local! {
        static OPENED: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
    }

    /// Until the guard drops, this thread's opened URLs are kept, not opened.
    pub struct Recording(());

    impl Recording {
        pub fn start() -> Self {
            OPENED.with(|o| *o.borrow_mut() = Some(Vec::new()));
            Self(())
        }

        pub fn opened(&self) -> Vec<String> {
            OPENED.with(|o| o.borrow().clone().unwrap_or_default())
        }
    }

    impl Drop for Recording {
        fn drop(&mut self) {
            OPENED.with(|o| *o.borrow_mut() = None);
        }
    }

    /// True when the URL was kept for a test.
    pub(super) fn record(url: &str) -> bool {
        OPENED.with(|o| match o.borrow_mut().as_mut() {
            Some(list) => {
                list.push(url.to_owned());
                true
            }
            None => false,
        })
    }
}

/// Logs what [`open_url`] refused or could not do; for clicks with no place to show it.
pub fn open_url_logged(url: &str) {
    if let Err(e) = open_url(url) {
        tracing::warn!("open url: {e}");
    }
}

/// The opener with the URL as its single, unquoted argument.
#[cfg(any(target_os = "linux", target_os = "macos", test))]
fn command(url: &str) -> std::process::Command {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let mut command = std::process::Command::new(opener);
    command
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null());
    command
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn launch(url: &str) -> Result<(), String> {
    let mut child = command(url).spawn().map_err(|e| format!("{url}: {e}"))?;
    // Reaped, so that each click leaves no zombie behind.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// ShellExecuteW as for the update's installer: the URL is the file, no cmd.
#[cfg(windows)]
fn launch(url: &str) -> Result<(), String> {
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let verb = wide("open");
    let file = wide(url);
    // SAFETY: NUL-terminated UTF-16 strings that outlive the call.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // Values up to 32 are errors.
    if result as isize <= 32 {
        return Err(format!("{url}: ShellExecuteW {}", result as isize));
    }
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn launch(url: &str) -> Result<(), String> {
    Err(format!("{url}: no opener on this system")) // i18n-ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_and_mail_links_pass() {
        assert!(checked("https://example.com/a?b=c").is_ok());
        assert!(checked("http://example.com").is_ok());
        assert!(checked("mailto:a@example.com").is_ok());
        for refused in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "smb://host/share",
            "ms-settings:",
            "C:\\Windows\\System32\\calc.exe",
            "/usr/bin/xterm",
            "calc",
            "",
        ] {
            assert!(checked(refused).is_err(), "{refused}");
        }
    }

    #[test]
    fn an_ampersand_stays_inside_the_one_argument() {
        let url = checked("https://example.com/?a&calc|x^y%PATH%").unwrap();
        let command = command(&url);
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args.len(), 1);
        assert_eq!(
            args[0].to_str().unwrap(),
            "https://example.com/?a&calc|x^y%PATH%"
        );
    }
}

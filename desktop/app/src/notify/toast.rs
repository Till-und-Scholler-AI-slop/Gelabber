//! Windows: toast notifications, as other Windows apps show them: in the
//! corner, in the notification centre, under "Gelabber" in the system's
//! notification settings, and quiet under Do not disturb.
//!
//! The app is not a packaged one, so Windows learns its name and icon from
//! its AppUserModelID's entry under `HKCU\Software\Classes\AppUserModelId`,
//! written at every start (the installer's start menu shortcut carries the
//! same id, the bundle identifier). A click reaches the app while it runs,
//! through the toast's `Activated` event; one after the app has ended could
//! not, so the app takes its toasts down at the start and the end.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
};

use tauri::AppHandle;
use windows::{
    Data::Xml::Dom::XmlDocument,
    Foundation::TypedEventHandler,
    UI::Notifications::{
        ToastDismissalReason, ToastDismissedEventArgs, ToastNotification, ToastNotificationManager,
        ToastNotifier,
    },
    core::{HSTRING, IInspectable, Ref},
};

use super::{Notice, Report, matches, toast_xml};

/// All of the app's toasts are in one group; the tags tell them apart.
const GROUP: &str = "gelabber";
const APP_NAME: &str = "Gelabber";

type Callback = Arc<dyn Fn(Report) + Send + Sync>;

pub struct Backend {
    aumid: HSTRING,
    notifier: ToastNotifier,
    shown: Arc<Mutex<HashMap<String, Shown>>>,
    /// Toast tags are at most 64 characters, the page's longer: each of the
    /// page's tags gets a short one of its own.
    next: AtomicU32,
    report: Callback,
}

struct Shown {
    tag: HSTRING,
    toast: ToastNotification,
}

fn text(error: windows::core::Error) -> String {
    error.message()
}

impl Backend {
    pub fn start(
        app: &AppHandle,
        report: Box<dyn Fn(Report) + Send + Sync>,
    ) -> Result<Self, String> {
        let aumid = app.config().identifier.clone();
        register(&aumid);
        let aumid = HSTRING::from(aumid);
        let notifier = ToastNotificationManager::CreateToastNotifierWithId(&aumid).map_err(text)?;
        // From an earlier run that did not end on its own.
        if let Ok(history) = ToastNotificationManager::History() {
            let _ = history.ClearWithId(&aumid);
        }
        Ok(Self {
            aumid,
            notifier,
            shown: Arc::default(),
            next: AtomicU32::new(1),
            report: Arc::from(report),
        })
    }

    /// Switched off for the app in the system's settings, the toasts are not
    /// shown, as the user wants; the page does not try the webview instead.
    pub fn available(&self) -> bool {
        true
    }

    pub fn show(&self, notice: Notice) -> Result<(), String> {
        let xml = XmlDocument::new().map_err(text)?;
        xml.LoadXml(&HSTRING::from(toast_xml(&notice.title, &notice.body)))
            .map_err(text)?;
        let toast = ToastNotification::CreateToastNotification(&xml).map_err(text)?;
        let mut shown = self.shown.lock().map_err(|e| e.to_string())?;
        let tag = match shown.get(&notice.tag) {
            Some(earlier) => earlier.tag.clone(),
            None => HSTRING::from(format!("m{}", self.next.fetch_add(1, Ordering::Relaxed))),
        };
        // Same tag and group: Windows replaces the older toast.
        toast.SetTag(&tag).map_err(text)?;
        toast.SetGroup(&HSTRING::from(GROUP)).map_err(text)?;
        let clicked = {
            let (key, report, all) = (notice.tag.clone(), self.report.clone(), self.shown.clone());
            TypedEventHandler::new(move |toast: Ref<ToastNotification>, _: Ref<IInspectable>| {
                forget(&all, &key, toast.as_ref());
                report(Report::Clicked {
                    tag: key.clone(),
                    activation: None,
                });
                Ok(())
            })
        };
        toast.Activated(&clicked).map_err(text)?;
        // Swiped away: gone. Timed out it waits in the notification centre,
        // where a click still counts.
        let dismissed = {
            let (key, all) = (notice.tag.clone(), self.shown.clone());
            TypedEventHandler::new(
                move |toast: Ref<ToastNotification>, args: Ref<ToastDismissedEventArgs>| {
                    let canceled = args
                        .as_ref()
                        .and_then(|args| args.Reason().ok())
                        .is_some_and(|reason| reason == ToastDismissalReason::UserCanceled);
                    if canceled {
                        forget(&all, &key, toast.as_ref());
                    }
                    Ok(())
                },
            )
        };
        toast.Dismissed(&dismissed).map_err(text)?;
        self.notifier.Show(&toast).map_err(text)?;
        // Kept for its events: they come while the toast object lives.
        shown.insert(notice.tag, Shown { tag, toast });
        Ok(())
    }

    pub fn close(&self, tag: &str, prefix: bool) {
        let Ok(mut shown) = self.shown.lock() else {
            return;
        };
        let history = ToastNotificationManager::History().ok();
        let group = HSTRING::from(GROUP);
        shown.retain(|key, toast| {
            if !matches(key, tag, prefix) {
                return true;
            }
            let _ = self.notifier.Hide(&toast.toast);
            if let Some(history) = &history {
                let _ = history.RemoveGroupedTagWithId(&toast.tag, &group, &self.aumid);
            }
            false
        });
    }

    pub fn clear(&self) {
        if let Ok(history) = ToastNotificationManager::History() {
            let _ = history.ClearWithId(&self.aumid);
        }
        if let Ok(mut shown) = self.shown.lock() {
            shown.clear();
        }
    }
}

/// Drops the entry of `key` if it is still this toast, not a newer one.
fn forget(all: &Mutex<HashMap<String, Shown>>, key: &str, toast: Option<&ToastNotification>) {
    if let Ok(mut shown) = all.lock()
        && shown
            .get(key)
            .is_some_and(|entry| toast.is_none_or(|toast| *toast == entry.toast))
    {
        shown.remove(key);
    }
}

/// Name and icon of the app's toasts. The icon is the `icon.png` the
/// installer puts next to the binary; without it Windows shows a generic one.
fn register(aumid: &str) {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegSetKeyValueW};
    let wide = |text: &str| -> Vec<u16> { text.encode_utf16().chain(Some(0)).collect() };
    let key = wide(&format!(r"Software\Classes\AppUserModelId\{aumid}"));
    let set = |name: &str, value: &str| {
        let (name, value) = (wide(name), wide(value));
        // SAFETY: NUL-terminated UTF-16 strings that outlive the call; the
        // size is the value's in bytes, with its NUL.
        let status = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                REG_SZ,
                value.as_ptr().cast(),
                (value.len() * 2) as u32,
            )
        };
        if status != 0 {
            eprintln!("gelabber: notifications: registering {name:?} failed ({status})");
        }
    };
    set("DisplayName", APP_NAME);
    if let Some(icon) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("icon.png")))
        .filter(|icon| icon.is_file())
    {
        set("IconUri", &icon.display().to_string());
    }
}

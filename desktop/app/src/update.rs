//! Updates of the Windows app (issue #253). The app looks for a newer release
//! shortly after it starts and every few hours, and offers it in a notice over
//! the page ([`BANNER`]). Installing it restarts the app, so the notice is not
//! shown while the page is in a call ([`Media::busy`]) and the install refuses
//! then.
//!
//! What is offered is `latest.json` of the latest release on GitHub
//! (desktop-native.yml writes it). The installer it names is run only when
//! its signature checks out against the public key built into the app
//! (build.rs: `updater.pub`) and was made for the version announced. No page
//! is granted the updater plugin's own commands; the pages get the commands
//! here (permissions/update.toml, capabilities/setup.json).
//!
//! tauri.conf.json sets the plugin's `requireSignedVersion` and the passive
//! install; the key is this build's ([`plugin`]), the endpoint set per check.
//!
//! Linux has no updater, the plugin is not even linked there: the pacman
//! package is updated by pacman, the tarball by a new tarball.
use crate::media::Media;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant, SystemTime},
};
use tauri::{AppHandle, Manager, Runtime, State};

/// See build.rs; empty when the build has no key.
const PUBKEY: &str = env!("GELABBER_UPDATER_PUBKEY");
#[cfg(windows)]
const ENDPOINT: &str =
    "https://github.com/Till-und-Scholler-AI-slop/Gelabber/releases/latest/download/latest.json";
/// The first check, after the start; then one every [`CHECK_EVERY`], looked
/// at every [`LOOK_EVERY`]: a computer that slept through a check time
/// checks when it wakes.
const FIRST_CHECK: Duration = Duration::from_secs(15);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
const LOOK_EVERY: Duration = Duration::from_secs(60);
const IN_CALL: &str =
    "Während eines Anrufs startet Gelabber nicht neu. Das Update kommt nach dem Auflegen.";
/// What the page hears after a "Nein" in the app's question.
const DECLINED: &str = "Nicht installiert.";
/// After a "Nein", the question is not asked again for this long, so a page
/// cannot pop it up again and again; short, for a "Nein" by mistake.
const ASK_AGAIN_AFTER: Duration = Duration::from_secs(2 * 60);

/// How an install that did not fail ended.
#[cfg_attr(not(windows), allow(dead_code))]
enum Outcome {
    /// The installer runs; the app is about to end.
    Installed,
    /// "Nein" in the app's question: not an error of the update.
    Declined,
}

/// Runs in every page of the window next to the shortcuts (main.rs): asks
/// for the update state every 30 seconds and shows the notice while the app
/// offers an update, with the progress of an install that was asked for. Styles are set through the CSSOM, which the server's
/// Content-Security-Policy does not restrict, in a closed shadow root, which
/// the page's styles do not reach. Stops asking on a page that may not ask,
/// and in an app without updates.
pub const BANNER: &str = r#"
(() => {
  if (window.top !== window) return;
  const invoke = (command) => window.__TAURI_INTERNALS__.invoke(command);
  // Polls every 30 s; after a failed call later and later (up to 10 min), and
  // not at all once the app says this page may not ask.
  const EVERY = 30000, AT_MOST = 600000;
  let host, text, restart, later, shown = null, wait = EVERY;
  const button = (label, primary) => {
    const element = document.createElement("button");
    element.type = "button";
    element.textContent = label;
    element.style.cssText = "flex:none;padding:6px 12px;border-radius:8px;font:inherit;cursor:pointer;"
      + (primary ? "border:0;background:#f3b752;color:#191b1b;font-weight:600"
                 : "border:1px solid #3a3f42;background:transparent;color:inherit");
    return element;
  };
  // What a click on "Neu starten" is at, from the app's state.
  const progress = (state) => {
    if (state.phase === "confirming") return "Bitte im Fenster von Gelabber bestätigen …";
    if (state.phase === "installing") return "Update wird installiert, Gelabber startet neu …";
    if (state.phase !== "downloading") return null;
    const percent = state.total ? ` ${Math.floor((100 * state.downloaded) / state.total)} %` : "";
    return `Update wird geladen …${percent}`;
  };
  const build = () => {
    host = document.createElement("div");
    const root = host.attachShadow({ mode: "closed" });
    const box = document.createElement("div");
    box.setAttribute("role", "status");
    // Top centre: clear of the composer and the call dock at the bottom.
    box.style.cssText = "position:fixed;top:12px;left:50%;transform:translateX(-50%);"
      + "z-index:2147483647;display:flex;gap:10px;align-items:center;"
      + "width:max-content;max-width:calc(100vw - 32px);box-sizing:border-box;"
      + "padding:10px 12px;border-radius:12px;border:1px solid #2c3032;background:#1d2021;"
      + "color:#eef0ee;box-shadow:0 6px 24px rgba(0,0,0,.35);"
      + "font:14px/1.4 system-ui,-apple-system,'Segoe UI',sans-serif";
    text = document.createElement("span");
    text.style.cssText = "flex:1";
    restart = button("Neu starten", true);
    later = button("Später", false);
    restart.addEventListener("click", async () => {
      restart.disabled = later.disabled = true;
      text.textContent = "Bitte im Fenster von Gelabber bestätigen …";
      const watching = setInterval(async () => {
        const line = progress(await invoke("update_state").catch(() => ({})));
        if (line) text.textContent = line;
      }, 1000);
      try {
        // Asks natively first; ends the app when it works, and the
        // installer starts it again.
        await invoke("update_install");
      } catch (error) {
        text.textContent = String(error);
        restart.disabled = later.disabled = false;
      } finally {
        clearInterval(watching);
      }
    });
    later.addEventListener("click", () => {
      invoke("update_dismiss").catch(() => {});
      hide();
    });
    box.append(text, restart, later);
    root.append(box);
  };
  const show = (version) => {
    if (!host) build();
    if (shown !== version) {
      text.textContent = `Gelabber ${version} ist da. Zum Installieren startet die App neu.`;
      restart.disabled = later.disabled = false;
      shown = version;
    }
    if (!host.isConnected) document.documentElement.append(host);
  };
  const hide = () => {
    host?.remove();
    shown = null;
  };
  const poll = async () => {
    let state;
    try {
      state = await invoke("update_state");
      wait = EVERY;
    } catch (error) {
      // A page without the permission: Tauri's refusal names the command.
      if (/not allowed|denied/i.test(String(error))) return;
      wait = Math.min(wait * 2, AT_MOST);
      setTimeout(poll, wait);
      return;
    }
    if (!state.supported) return;
    if (state.offer) show(state.available);
    // Not while the click on "Neu starten" is at work.
    else if (state.phase === "idle") hide();
    setTimeout(poll, wait);
  };
  if (!window.__TAURI_INTERNALS__) return;
  if (document.readyState === "loading") addEventListener("DOMContentLoaded", poll, { once: true });
  else poll();
})();
"#;

/// Why this app does not update itself, if it does not.
fn unsupported() -> Option<&'static str> {
    if cfg!(not(windows)) {
        Some(
            "Unter Linux kommen Updates über pacman (Paket gelabber-desktop) \
             oder als neues Archiv von der Release-Seite.",
        )
    } else if PUBKEY.is_empty() {
        Some("Dieser Build hat keinen Schlüssel für Updates und aktualisiert sich nicht selbst.")
    } else {
        None
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Settings {
    auto_check: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { auto_check: true }
    }
}

/// Next to desktop.json, which main.rs writes whole.
fn settings_path<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("update.json"))
}

fn load_settings<R: Runtime>(app: &AppHandle<R>) -> Settings {
    settings_path(app)
        .and_then(|path| fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[derive(Clone, Copy, Default, PartialEq, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
enum Phase {
    #[default]
    Idle,
    Checking,
    /// The native question whether to install now is open.
    Confirming,
    #[cfg_attr(not(windows), allow(dead_code))]
    Downloading,
    #[cfg_attr(not(windows), allow(dead_code))]
    Installing,
}

/// An update the last check found. Only the Windows app finds any.
#[cfg_attr(not(windows), allow(dead_code))]
struct Found {
    version: String,
    notes: Option<String>,
    #[cfg(windows)]
    update: tauri_plugin_updater::Update,
    /// The installer, once downloaded and checked: a call that began during
    /// the download only puts the install off.
    #[cfg(windows)]
    installer: Option<std::sync::Arc<Vec<u8>>>,
}

#[derive(Default)]
struct Inner {
    phase: Phase,
    found: Option<Found>,
    /// The version whose notice was put away ("Später"), until the next
    /// start of the app.
    dismissed: Option<String>,
    last_check: Option<SystemTime>,
    error: Option<String>,
    /// When the question was last answered "Nein".
    declined: Option<Instant>,
    /// Bytes of the installer so far, and its size when the server said it.
    downloaded: u64,
    total: Option<u64>,
}

#[derive(Default)]
pub struct Updates(Mutex<Inner>);

/// What the pages get to know.
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct View {
    current: String,
    supported: bool,
    reason: Option<&'static str>,
    auto_check: bool,
    phase: Phase,
    available: Option<String>,
    notes: Option<String>,
    /// Whether the notice is to be shown: there is an update, its notice was
    /// not put away, and the page is in no call.
    offer: bool,
    busy: bool,
    error: Option<String>,
    /// Seconds since 1970 of the last check that got an answer or failed.
    last_check: Option<u64>,
    downloaded: u64,
    total: Option<u64>,
}

fn view_of(
    inner: &Inner,
    current: String,
    reason: Option<&'static str>,
    auto_check: bool,
    busy: bool,
) -> View {
    let available = inner.found.as_ref().map(|found| found.version.clone());
    View {
        current,
        supported: reason.is_none(),
        reason,
        auto_check,
        phase: inner.phase,
        offer: reason.is_none() && available.is_some() && inner.dismissed != available && !busy,
        available,
        notes: inner.found.as_ref().and_then(|found| found.notes.clone()),
        busy,
        error: inner.error.clone(),
        last_check: inner
            .last_check
            .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|since| since.as_secs()),
        downloaded: inner.downloaded,
        total: inner.total,
    }
}

fn view<R: Runtime>(app: &AppHandle<R>) -> View {
    let busy = app.state::<Media>().busy();
    let updates = app.state::<Updates>();
    let inner = updates.0.lock().unwrap();
    view_of(
        &inner,
        app.package_info().version.to_string(),
        unsupported(),
        load_settings(app).auto_check,
        busy,
    )
}

/// The updater plugin with the key of this build. Its commands are not
/// granted to any page.
#[cfg(windows)]
pub fn plugin<R: Runtime>() -> tauri::plugin::TauriPlugin<R, tauri_plugin_updater::Config> {
    tauri_plugin_updater::Builder::new().pubkey(PUBKEY).build()
}

#[cfg(windows)]
async fn find<R: Runtime>(app: &AppHandle<R>) -> Result<Option<Found>, String> {
    use tauri_plugin_updater::UpdaterExt;
    // Another release channel to try an update with, such as a test
    // release's latest.json. Deliberately in release builds too, to try an
    // update with the app as it ships: HTTPS and the signature with this
    // build's key apply all the same.
    let endpoint = std::env::var("GELABBER_UPDATE_ENDPOINT")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| ENDPOINT.to_owned());
    let endpoint = url::Url::parse(&endpoint).map_err(err)?;
    let update = app
        .updater_builder()
        .endpoints(vec![endpoint])
        .map_err(err)?
        // For the download too, which has no time limit as a whole (a big
        // installer on a slow line) but must not stall.
        .configure_client(|client| {
            client
                .connect_timeout(Duration::from_secs(30))
                .read_timeout(Duration::from_secs(60))
        })
        // The whole check; the download drops it (`install`).
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(err)?
        .check()
        .await
        .map_err(err)?;
    Ok(update.map(|update| Found {
        version: update.version.clone(),
        notes: update.body.clone(),
        update,
        installer: None,
    }))
}

#[cfg(not(windows))]
async fn find<R: Runtime>(_app: &AppHandle<R>) -> Result<Option<Found>, String> {
    Ok(None)
}

/// One check, unless one is at work or an update is being installed.
async fn check<R: Runtime>(app: &AppHandle<R>) {
    if unsupported().is_some() {
        return;
    }
    let updates = app.state::<Updates>();
    {
        let mut inner = updates.0.lock().unwrap();
        if inner.phase != Phase::Idle {
            return;
        }
        inner.phase = Phase::Checking;
    }
    let result = find(app).await;
    let mut inner = updates.0.lock().unwrap();
    inner.phase = Phase::Idle;
    inner.last_check = Some(SystemTime::now());
    match result {
        Ok(found) => {
            inner.error = None;
            // The installer already downloaded for the same version stays.
            let same = |old: &Found| found.as_ref().is_some_and(|new| new.version == old.version);
            if !inner.found.as_ref().is_some_and(same) {
                inner.found = found;
            }
        }
        Err(error) => {
            eprintln!("gelabber: update check: {error}");
            inner.error = Some(error);
        }
    }
}

/// The checks of the running app: the first soon after the start, then one
/// every [`CHECK_EVERY`], all only while the automatic check is on.
pub fn start<R: Runtime>(app: &AppHandle<R>) {
    if unsupported().is_some() {
        return;
    }
    let app = app.clone();
    let checks = std::thread::Builder::new()
        .name("gelabber-update".into())
        .spawn(move || {
            std::thread::sleep(FIRST_CHECK);
            loop {
                let last = app.state::<Updates>().0.lock().unwrap().last_check;
                if load_settings(&app).auto_check && due(last, SystemTime::now()) {
                    tauri::async_runtime::block_on(check(&app));
                }
                std::thread::sleep(LOOK_EVERY);
            }
        });
    if let Err(error) = checks {
        eprintln!("gelabber: update checks: {error}");
    }
}

/// Whether the next automatic check is due; a clock that went back counts
/// as due.
fn due(last: Option<SystemTime>, now: SystemTime) -> bool {
    last.is_none_or(|last| {
        now.duration_since(last)
            .is_ok_and(|since| since >= CHECK_EVERY)
    }) || last.is_some_and(|last| last > now)
}

/// The state of updates, for the notice and the setup page.
#[tauri::command]
pub fn update_state(app: AppHandle) -> View {
    view(&app)
}

/// "Später": no notice for this version until the app starts again.
#[tauri::command]
pub fn update_dismiss(updates: State<'_, Updates>) {
    let mut inner = updates.0.lock().unwrap();
    inner.dismissed = inner.found.as_ref().map(|found| found.version.clone());
}

/// Setup page only: check now.
#[tauri::command]
pub async fn update_check(app: AppHandle) -> Result<View, String> {
    if let Some(reason) = unsupported() {
        return Err(reason.into());
    }
    check(&app).await;
    Ok(view(&app))
}

/// Setup page only: the automatic check on or off.
#[tauri::command]
pub fn update_set_auto_check(app: AppHandle, enabled: bool) -> Result<View, String> {
    let path = settings_path(&app).ok_or("no config directory")?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(err)?;
    }
    let json = serde_json::to_vec_pretty(&Settings {
        auto_check: enabled,
    })
    .map_err(err)?;
    fs::write(path, json).map_err(err)?;
    Ok(view(&app))
}

/// Asks natively whether to install now, then downloads the update found,
/// checks its signature and runs its installer, which ends the app and
/// starts the new version. Refused during a call. Returns only when that did
/// not happen. The question is the app's own window, not the page's: a page
/// of the server, or a script injected into one, cannot answer it, so it
/// cannot restart the app unasked.
#[tauri::command]
pub async fn update_install(app: AppHandle) -> Result<(), String> {
    if let Some(reason) = unsupported() {
        return Err(reason.into());
    }
    if app.state::<Media>().busy() {
        return Err(IN_CALL.into());
    }
    let updates = app.state::<Updates>();
    {
        let mut inner = updates.0.lock().unwrap();
        if inner.found.is_none() {
            return Err("Es gibt kein Update.".into());
        }
        if inner.phase != Phase::Idle {
            return Err("Das Update wird schon geladen.".into());
        }
        if inner
            .declined
            .is_some_and(|declined| declined.elapsed() < ASK_AGAIN_AFTER)
        {
            return Err("Gerade abgelehnt. In ein paar Minuten geht es wieder.".into());
        }
        inner.phase = Phase::Confirming;
        inner.error = None;
    }
    let result = install(&app).await;
    let mut inner = updates.0.lock().unwrap();
    inner.phase = Phase::Idle;
    match result {
        Ok(Outcome::Installed) => Ok(()),
        Ok(Outcome::Declined) => {
            inner.declined = Some(Instant::now());
            Err(DECLINED.into())
        }
        Err(error) => {
            eprintln!("gelabber: update: {error}");
            inner.error = Some(error.clone());
            Err(error)
        }
    }
}

#[cfg(windows)]
async fn install(app: &AppHandle) -> Result<Outcome, String> {
    let updates = app.state::<Updates>();
    let (mut update, installer) = {
        let inner = updates.0.lock().unwrap();
        let found = inner.found.as_ref().ok_or("Es gibt kein Update.")?;
        (found.update.clone(), found.installer.clone())
    };
    let asking = app.clone();
    let version = update.version.clone();
    let yes = tauri::async_runtime::spawn_blocking(move || confirm(&asking, &version))
        .await
        .map_err(err)?;
    if !yes {
        return Ok(Outcome::Declined);
    }
    // A call may have begun while the question was open.
    if app.state::<Media>().busy() {
        return Err(IN_CALL.into());
    }
    let installer = match installer {
        Some(installer) => installer,
        // Checks the signature too.
        None => {
            {
                let mut inner = updates.0.lock().unwrap();
                inner.phase = Phase::Downloading;
                inner.downloaded = 0;
                inner.total = None;
            }
            // Only the read and connect limits of the client (`find`).
            update.timeout = None;
            let progress = app.clone();
            let installer = update
                .download(
                    move |chunk, total| {
                        let updates = progress.state::<Updates>();
                        let mut inner = updates.0.lock().unwrap();
                        inner.downloaded += chunk as u64;
                        inner.total = total;
                    },
                    || {},
                )
                .await
                .map_err(err)?;
            let installer = std::sync::Arc::new(installer);
            let mut inner = updates.0.lock().unwrap();
            if let Some(found) = inner
                .found
                .as_mut()
                .filter(|found| found.version == update.version)
            {
                found.installer = Some(std::sync::Arc::clone(&installer));
            }
            installer
        }
    };
    // A call may have begun during the download.
    if app.state::<Media>().busy() {
        return Err(IN_CALL.into());
    }
    updates.0.lock().unwrap().phase = Phase::Installing;
    // Checks the signature again; ends the app once the installer runs,
    // after the plugin's `on_before_exit` (Tauri's `cleanup_before_exit`).
    update.install(installer.as_slice()).map_err(err)?;
    Ok(Outcome::Installed)
}

/// The app's own question, a message box over its window, whether to
/// install `version` now. Blocks until it is answered.
#[cfg(windows)]
fn confirm(app: &AppHandle, version: &str) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        IDYES, MB_DEFBUTTON2, MB_ICONQUESTION, MB_SETFOREGROUND, MB_YESNO, MessageBoxW,
    };
    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let owner = app
        .get_webview_window("main")
        .and_then(|window| window.hwnd().ok())
        .map_or(std::ptr::null_mut(), |hwnd| hwnd.0);
    let text = wide(&format!(
        "Gelabber {version} jetzt installieren?\n\nDie App wird dafür beendet und startet danach neu."
    ));
    let title = wide("Gelabber-Update");
    // "Nein" is the default: an Enter meant for the page (the question can
    // come up while someone types) must not restart the app.
    // SAFETY: both strings are NUL-terminated UTF-16 and outlive the call;
    // the owner is the app's window or none.
    let answer = unsafe {
        MessageBoxW(
            owner,
            text.as_ptr(),
            title.as_ptr(),
            MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2 | MB_SETFOREGROUND,
        )
    };
    answer == IDYES
}

#[cfg(not(windows))]
async fn install(_app: &AppHandle) -> Result<Outcome, String> {
    Err(unsupported().unwrap_or_default().into())
}

#[cfg(test)]
mod tests {
    use super::*;
    // media.rs takes commands.rs in for its tests.
    use crate::media::UPDATE_COMMANDS;

    #[test]
    fn every_update_command_is_handled_and_granted() {
        let main = include_str!("main.rs");
        for command in UPDATE_COMMANDS {
            assert!(
                main.contains(&format!("update::{command},")),
                "{command} missing from generate_handler!"
            );
        }
        let server = include_str!("../permissions/update.toml");
        let setup = include_str!("../capabilities/setup.json");
        for command in UPDATE_COMMANDS {
            let permission = format!("\"allow-{}\"", command.replace('_', "-"));
            let for_server = matches!(
                *command,
                "update_state" | "update_install" | "update_dismiss"
            );
            assert_eq!(
                server.contains(&permission),
                for_server,
                "{command} for the server origin"
            );
            assert!(
                for_server || setup.contains(&permission),
                "{command} for the setup page"
            );
        }
        assert!(
            setup.contains("\"update\""),
            "the setup page gets the update set"
        );
        assert!(
            main.contains(".permission(\"update\")"),
            "the server origin gets the update set"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn the_notice_waits_for_the_end_of_a_call_and_not_for_who_put_it_away() {
        // Found holds the plugin's Update on Windows, which only a check makes.
        let mut inner = Inner {
            found: Some(Found {
                version: "0.7.1".into(),
                notes: None,
            }),
            ..Inner::default()
        };
        let view = |inner: &Inner, busy| view_of(inner, "0.7.0".into(), None, true, busy);
        assert!(view(&inner, false).offer);
        assert_eq!(view(&inner, false).available.as_deref(), Some("0.7.1"));
        assert!(!view(&inner, true).offer, "offered during a call");
        inner.dismissed = Some("0.7.1".into());
        assert!(!view(&inner, false).offer, "offered again after \"Später\"");
        inner.dismissed = Some("0.7.0".into());
        assert!(
            view(&inner, false).offer,
            "a newer version is offered again"
        );
        assert!(!view(&Inner::default(), false).offer);
        // An app without updates offers none, and says why.
        let linux = view_of(&inner, "0.7.0".into(), unsupported(), true, false);
        assert!(!linux.supported && !linux.offer);
        assert!(linux.reason.is_some_and(|reason| reason.contains("pacman")));
    }

    /// The updater plugin's check of a download (`verify_signature` in
    /// tauri-plugin-updater 2.14) with `requireSignedVersion`, as
    /// tauri.conf.json sets it: `pubkey` and `signature` as `tauri signer`
    /// writes them, base64 of minisign's text.
    fn verify(data: &[u8], signature: &str, pubkey: &str, version: &str) -> Result<(), String> {
        use base64::Engine;
        let decode = |text: &str| {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(text.trim())
                .map_err(err)?;
            String::from_utf8(bytes).map_err(err)
        };
        let key = minisign_verify::PublicKey::decode(&decode(pubkey)?).map_err(err)?;
        let signature = minisign_verify::Signature::decode(&decode(signature)?).map_err(err)?;
        key.verify(data, &signature, true).map_err(err)?;
        let signed = signature
            .trusted_comment()
            .split('\t')
            .find_map(|field| field.strip_prefix("version:"))
            .ok_or("signed without a version")?;
        // As the plugin compares them, for the versions we write.
        (signed.trim_start_matches('v') == version.trim_start_matches('v'))
            .then_some(())
            .ok_or_else(|| format!("signed for {signed}, not {version}"))
    }

    #[test]
    fn signatures_hold_for_their_bytes_key_and_version_only() {
        // `tauri signer sign --app-version 0.7.0` of b"gelabber" with a key
        // made for this test.
        const KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEVBNjEyRDkzOEVDOTk3MjgKUldRb2w4bU9reTFoNmswZldrczNPK1kveENldW5IVGo4Qm9XQ3lqdzZFL2hhZlB2SFRZdmxrLzEK";
        const SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVRb2w4bU9reTFoNm1ibm9XSmovRi9QNFBMcjdVZVQ3a1ozYVd3NFRxc2N1a3ExTmZHMkxWNUhuTDczS04vZm4wNG5EemhtWXg0NHphaXRxWUZoMWpDZnZPRFlaMzFlWUE0PQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkyMTk0NjY2CWZpbGU6Zml4dHVyZQl2ZXJzaW9uOjAuNy4wCkN4KzVxdTlBaEg3Nzc0SmU0L0tnZGJPaVFNemJ4aUQ3VmpqMFRaaTQ5dEVrZ2V5dGtzT0N6YjZFcXlncFNlTXJtVVZVdlN4VGdMUGR0WGV4NFcwa0NnPT0K";
        assert_eq!(verify(b"gelabber", SIGNATURE, KEY, "0.7.0"), Ok(()));
        assert!(verify(b"gelabbex", SIGNATURE, KEY, "0.7.0").is_err());
        assert!(verify(b"gelabber", SIGNATURE, KEY, "0.7.1").is_err());
        assert!(verify(b"gelabber", "", KEY, "0.7.0").is_err());
        if !PUBKEY.is_empty() && PUBKEY != KEY {
            assert!(verify(b"gelabber", SIGNATURE, PUBKEY, "0.7.0").is_err());
        }
    }

    /// The installer CI built and signed (desktop-native.yml sets the
    /// variable): its signature holds for the key this build carries and for
    /// this version, as the installed app will check it, so a release key
    /// that does not belong to `updater.pub` stops the release. Nothing to
    /// check without the variable.
    #[test]
    fn the_installer_built_here_passes_the_updaters_check() {
        let installer = std::env::var_os("GELABBER_UPDATE_TEST_INSTALLER");
        let Some(installer) = installer.filter(|path| !path.is_empty()) else {
            return;
        };
        assert!(!PUBKEY.is_empty(), "this build has no updater key");
        let mut bytes = fs::read(&installer).unwrap();
        let mut path = installer;
        path.push(".sig");
        let signature = fs::read_to_string(path).unwrap();
        let version = env!("CARGO_PKG_VERSION");
        assert_eq!(verify(&bytes, &signature, PUBKEY, version), Ok(()));
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        assert!(verify(&bytes, &signature, PUBKEY, version).is_err());
    }

    #[test]
    fn checks_are_due_every_six_hours_and_after_the_clock_went_back() {
        let now = SystemTime::now();
        assert!(due(None, now));
        assert!(!due(Some(now - Duration::from_secs(60)), now));
        assert!(due(Some(now - CHECK_EVERY), now));
        assert!(due(Some(now + Duration::from_secs(3600)), now));
    }

    #[test]
    fn automatic_check_is_on_unless_switched_off() {
        assert!(Settings::default().auto_check);
        let read = |json: &str| serde_json::from_str::<Settings>(json).unwrap().auto_check;
        assert!(read("{}"));
        assert!(!read(r#"{"autoCheck":false}"#));
    }
}

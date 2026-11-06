//! Updates as the Tauri app (desktop/app/src/update.rs) does them: shortly
//! after the start and every six hours the manifest of the latest release is
//! read, and a newer version shows the notice of its `BANNER` ([`Notice`]).
//! Never during a call ([`CallLock`]).
//!
//! What the notice offers depends on how the app was installed ([`Install`]):
//! the Windows setup downloads the installer named in the manifest, checks its
//! minisign signature by `desktop/app/updater.pub` made for the announced
//! version ([`verify`], `requireSignedVersion`) and runs it passively with
//! `/UPDATE` once the app has ended ([`run_pending`]), as the updater plugin
//! does with `installMode: passive`. The tarball links the release page; the
//! pacman package says that pacman updates it.
//!
//! The manifest is this app's own, `latest-native.json` (desktop-release.yml
//! writes it in the format of the Tauri app's `latest.json`): the Tauri
//! releases' file names the Tauri installer, which this app must not run.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use egui::{Color32, Context, CornerRadius, Id, Rect, Sense, Stroke, StrokeKind, Vec2, pos2};
use gelabber_client::Client;
use parking_lot::Mutex;
use serde::Deserialize;

use crate::text::{Block, TextStyle};

const REPOSITORY: &str = "https://github.com/Till-und-Scholler-AI-slop/Gelabber";
/// The native app's manifest; `latest.json` stays the Tauri app's.
pub const MANIFEST: &str = "latest-native.json";
/// `releases/latest` leaves pre-releases out, as for the Tauri app.
const ENDPOINT: &str = "https://github.com/Till-und-Scholler-AI-slop/Gelabber/releases/latest/download/latest-native.json";
/// The Arch package's folder (desktop/packaging/arch/native/PKGBUILD).
const PACMAN_DIR: &str = "/opt/gelabber";
const PACMAN_PACKAGE: &str = "gelabber-desktop-native";

/// Where "Herunterladen" leads: the release page of `version` (as
/// [`newer`] reports it, without `v`).
pub fn release_page(version: &str) -> String {
    format!(
        "{REPOSITORY}/releases/tag/v{}",
        version.trim_start_matches('v')
    )
}

pub const FIRST_CHECK: Duration = Duration::from_secs(15);
pub const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// How long after a call (or the microphone test) the app still counts as
/// in one, as media.rs `BUSY_AFTER`: a quick rejoin is not cut off.
pub const BUSY_AFTER: Duration = Duration::from_secs(60);

/// The releases' key, or the build's own (`GELABBER_UPDATER_PUBKEY`, CI
/// test builds), as desktop/app/build.rs picks it.
const PUBKEY: &str = match option_env!("GELABBER_UPDATER_PUBKEY") {
    Some(key) => key,
    None => include_str!("../../../app/updater.pub"),
};

pub fn pubkey() -> &'static str {
    PUBKEY.trim()
}

/// The manifest as desktop-release.yml writes it (the Tauri app's format).
#[derive(Clone, Debug, Deserialize)]
pub struct Latest {
    pub version: String,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub platforms: std::collections::HashMap<String, Platform>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Platform {
    pub signature: String,
    pub url: String,
}

/// An update the check found.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub version: String,
    pub notes: Option<String>,
    /// The signed package for this system (url, signature), when the
    /// release has one.
    pub package: Option<(String, String)>,
}

/// Tauri's platform key: `linux-x86_64`, `windows-x86_64`, `darwin-aarch64`.
pub fn target() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    format!("{os}-{}", std::env::consts::ARCH)
}

/// The newer version in `latest`, if any; `current` is this build's.
pub fn newer(latest: &Latest, current: &str, target: &str) -> Option<Found> {
    let parse = |v: &str| semver::Version::parse(v.trim().trim_start_matches('v')).ok();
    let (next, now) = (parse(&latest.version)?, parse(current)?);
    if next <= now {
        return None;
    }
    Some(Found {
        version: next.to_string(),
        notes: latest.notes.clone(),
        package: latest
            .platforms
            .get(target)
            .map(|p| (p.url.clone(), p.signature.clone())),
    })
}

/// The endpoint, or `GELABBER_UPDATE_ENDPOINT` to try a test release.
pub fn endpoint() -> String {
    std::env::var("GELABBER_UPDATE_ENDPOINT")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| ENDPOINT.to_owned())
}

/// One check over HTTPS.
pub async fn check(http: reqwest::Client) -> Result<Option<Found>, String> {
    let response = http
        .get(endpoint())
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("{MANIFEST}: {}", response.status()));
    }
    let bytes = response.bytes().await.map_err(|e| e.to_string())?;
    let latest: Latest = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    Ok(newer(&latest, env!("CARGO_PKG_VERSION"), &target()))
}

/// The updater plugin's check of a package (`verify_signature` with
/// `requireSignedVersion`): `signature` and `pubkey` as `tauri signer`
/// writes them (base64 of minisign's text), signed for `version`.
pub fn verify(data: &[u8], signature: &str, pubkey: &str, version: &str) -> Result<(), String> {
    use base64::Engine;
    let decode = |text: &str| {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(text.trim())
            .map_err(|e| e.to_string())?;
        String::from_utf8(bytes).map_err(|e| e.to_string())
    };
    let key = minisign_verify::PublicKey::decode(&decode(pubkey)?).map_err(|e| e.to_string())?;
    let signature =
        minisign_verify::Signature::decode(&decode(signature)?).map_err(|e| e.to_string())?;
    key.verify(data, &signature, true)
        .map_err(|e| e.to_string())?;
    let signed = signature
        .trusted_comment()
        .split('\t')
        .find_map(|field| field.strip_prefix("version:"))
        .ok_or("signed without a version")?;
    (signed.trim_start_matches('v') == version.trim_start_matches('v'))
        .then_some(())
        .ok_or_else(|| format!("signed for {signed}, not {version}"))
}

/// `auto_check` of the Tauri app's `update.json`, on unless switched off.
pub fn auto_check() -> bool {
    super::paths::read_json("update.json")
        .get("autoCheck")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true)
}

/// Whether the next automatic check is due; a clock that went back counts.
pub fn due(last: Option<SystemTime>, now: SystemTime) -> bool {
    last.is_none_or(|last| {
        now.duration_since(last)
            .is_ok_and(|since| since >= CHECK_EVERY)
    }) || last.is_some_and(|last| last > now)
}

/// How this copy of the app was installed, which decides what the notice offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Install {
    /// The Windows setup (installer.nsi, or the Tauri app's that it
    /// replaces): its uninstaller is next to the binary in `dir`.
    Setup { dir: PathBuf },
    /// The Arch package in [`PACMAN_DIR`].
    Pacman,
    /// Anything else (the tarball, a copied binary): the release page.
    Manual,
}

impl Install {
    /// This process's.
    pub fn current() -> Self {
        match std::env::current_exe() {
            Ok(exe) => Self::of(std::env::consts::OS, &exe, |p| p.is_file()),
            Err(_) => Self::Manual,
        }
    }

    /// For the binary `exe` on `os`; `exists` looks for the uninstaller.
    pub fn of(os: &str, exe: &Path, exists: impl Fn(&Path) -> bool) -> Self {
        let dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
        match os {
            "windows" if exists(&dir.join("uninstall.exe")) => Self::Setup { dir },
            "linux" if exe.starts_with(PACMAN_DIR) => Self::Pacman,
            _ => Self::Manual,
        }
    }
}

/// The HTTP client of a download: no limit for the whole (a big installer on
/// a slow line), but it must not stall (the Tauri app's `configure_client`).
pub fn download_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(60))
        .user_agent(concat!("Gelabber-Desktop/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())
}

/// The file name of a download: the URL's last segment when it is a plain
/// name, so nothing lands outside `dir`.
fn file_name(url: &str) -> String {
    let name = url
        .split(['?', '#'])
        .next()
        .and_then(|path| path.rsplit('/').next())
        .unwrap_or_default();
    let plain = !name.is_empty()
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    if plain {
        name.to_owned()
    } else {
        "gelabber-setup.exe".to_owned()
    }
}

/// Downloads `url` into `dir` and keeps it only when its signature holds
/// for `pubkey` and `version`; `progress` hears the bytes so far and the
/// size when the server says it.
pub async fn download(
    http: &reqwest::Client,
    url: &str,
    signature: &str,
    pubkey: &str,
    version: &str,
    dir: &Path,
    progress: impl Fn(u64, Option<u64>),
) -> Result<PathBuf, String> {
    let mut response = http.get(url).send().await.map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("{}: {}", file_name(url), response.status()));
    }
    let total = response.content_length();
    let mut bytes = Vec::with_capacity(total.unwrap_or(0).min(512 << 20) as usize);
    progress(0, total);
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        bytes.extend_from_slice(&chunk);
        progress(bytes.len() as u64, total);
    }
    verify(&bytes, signature, pubkey, version)
        .map_err(|e| format!("Die Signatur des Updates stimmt nicht: {e}"))?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join(file_name(url));
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

/// A folder of its own per version in the temporary directory, as the
/// updater plugin's `<app>-<version>-updater-…`.
fn download_dir(version: &str) -> PathBuf {
    std::env::temp_dir().join(format!("gelabber-{version}-updater-{}", std::process::id()))
}

/// The Tauri updater's quoting of an argument for `/ARGS`: also a `/`, so
/// that NSIS does not take it for one of its own options.
fn nsis_arg(arg: &str) -> String {
    let quote = arg.is_empty() || arg.contains([' ', '\t', '/']);
    let mut out = String::new();
    if quote {
        out.push('"');
    }
    let mut backslashes = 0;
    for c in arg.chars() {
        if c == '\\' {
            backslashes += 1;
        } else {
            if c == '"' {
                out.extend(std::iter::repeat_n('\\', backslashes + 1));
            }
            backslashes = 0;
        }
        out.push(c);
    }
    if quote {
        out.extend(std::iter::repeat_n('\\', backslashes));
        out.push('"');
    }
    out
}

/// The setup's command line (installer.nsi's head): passive (`/P`), the
/// shortcuts kept (`/UPDATE`), the app started again (`/R`) with the
/// arguments it had, into the folder it runs from (`/D=`, last and unquoted).
pub fn installer_parameters(app_args: &[String], dir: &Path) -> String {
    let mut parts = vec![
        "/P".to_owned(),
        "/UPDATE".into(),
        "/R".into(),
        "/ARGS".into(),
    ];
    parts.extend(app_args.iter().map(|a| nsis_arg(a)));
    parts.push(format!("/D={}", dir.display()));
    parts.join(" ")
}

/// The installer to run once the app has ended, and the folder it installs to.
static PENDING: Mutex<Option<(PathBuf, PathBuf)>> = Mutex::new(None);

/// After the window closed (main.rs): runs the installer an accepted update
/// left, which replaces the binary and starts the new version. `Ok(false)`
/// when there is none.
pub fn run_pending() -> Result<bool, String> {
    let Some((installer, dir)) = PENDING.lock().take() else {
        return Ok(false);
    };
    // args() would panic on an argument that is no Unicode.
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let parameters = installer_parameters(&args, &dir);
    tracing::info!("update: {} {parameters}", installer.display());
    start_installer(&installer, &parameters).map(|()| true)
}

/// ShellExecuteW, as the updater plugin starts it, so that Windows may ask
/// for rights when a setup needs them.
#[cfg(windows)]
fn start_installer(installer: &Path, parameters: &str) -> Result<(), String> {
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOW};
    let wide = |text: &std::ffi::OsStr| {
        use std::os::windows::ffi::OsStrExt;
        text.encode_wide().chain([0]).collect::<Vec<u16>>()
    };
    let verb = wide("open".as_ref());
    let file = wide(installer.as_os_str());
    let parameters = wide(parameters.as_ref());
    // SAFETY: NUL-terminated UTF-16 strings that outlive the call.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            parameters.as_ptr(),
            std::ptr::null(),
            SW_SHOW,
        )
    };
    // Values up to 32 are errors.
    if result as isize <= 32 {
        return Err(format!("ShellExecuteW: {}", result as isize));
    }
    Ok(())
}

#[cfg(not(windows))]
fn start_installer(installer: &Path, _parameters: &str) -> Result<(), String> {
    Err(format!(
        "{}: only the Windows setup installs",
        installer.display()
    ))
}

/// Whether the app is in a call: the Tauri app's `Media::busy` (a transport
/// or a source, or one less than [`BUSY_AFTER`] ago). Restarting would end it.
#[derive(Debug, Default)]
pub struct CallLock {
    last_busy: Option<Instant>,
}

impl CallLock {
    /// `busy`: a call or the microphone test right now.
    pub fn locked(&mut self, busy: bool, now: Instant) -> bool {
        if busy {
            self.last_busy = Some(now);
        }
        self.last_busy
            .is_some_and(|last| now.saturating_duration_since(last) < BUSY_AFTER)
    }
}

/// A call or the microphone test holds the devices.
pub fn call_busy(call: &gelabber_call::Call) -> bool {
    use gelabber_call::{CallStatus, MicTestPhase};
    let in_call = call.with_state(|s| {
        s.channel.is_some()
            || matches!(
                s.status,
                CallStatus::Connecting | CallStatus::Connected | CallStatus::Reconnecting
            )
    });
    in_call
        || matches!(
            call.mic_test().phase,
            MicTestPhase::Pending | MicTestPhase::Active
        )
}

/// What the notice's main button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Offer {
    /// "Neu starten": download, check and install (the Windows setup).
    Restart,
    /// "Herunterladen": the release page.
    Download,
    /// Only "Später": pacman updates the package.
    PackageManager,
}

impl Offer {
    pub fn for_install(install: &Install, found: &Found) -> Self {
        match install {
            Install::Setup { .. } if found.package.is_some() => Self::Restart,
            Install::Pacman => Self::PackageManager,
            _ => Self::Download,
        }
    }
}

/// Where an accepted update is.
#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    /// Offered; the buttons work.
    Offer,
    Downloading {
        downloaded: u64,
        total: Option<u64>,
    },
    /// Downloaded and checked; installs once no call holds the app.
    Ready,
    Installing,
    /// The last try failed, why; the buttons work again.
    Failed(String),
}

/// A click in the notice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Click {
    Restart,
    Download,
    Later,
}

/// The update notice of the Tauri app (`BANNER` in desktop/app/src/update.rs),
/// with its texts, buttons and look.
#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub version: String,
    pub offer: Offer,
    pub phase: Phase,
}

// The BANNER's colours, the same in every theme.
const BG: Color32 = crate::theme::hex(0x1d2021);
const BORDER: Color32 = crate::theme::hex(0x2c3032);
const INK: Color32 = crate::theme::hex(0xeef0ee);
const ACCENT: Color32 = crate::theme::hex(0xf3b752);
const ACCENT_INK: Color32 = crate::theme::hex(0x191b1b);
const LINE: Color32 = crate::theme::hex(0x3a3f42);

impl Notice {
    pub fn new(version: &str, offer: Offer) -> Self {
        Self {
            version: version.to_owned(),
            offer,
            phase: Phase::Offer,
        }
    }

    /// The line, as the BANNER's `show` and `progress` write it.
    pub fn text(&self) -> String {
        match &self.phase {
            Phase::Offer => match self.offer {
                Offer::Restart => format!(
                    "Gelabber {} ist da. Zum Installieren startet die App neu.",
                    self.version
                ),
                Offer::Download => format!("Gelabber {} ist da.", self.version),
                Offer::PackageManager => format!(
                    "Gelabber {} ist da. Updates kommen über pacman (Paket {PACMAN_PACKAGE}).",
                    self.version
                ),
            },
            Phase::Downloading { downloaded, total } => {
                let percent = total
                    .filter(|t| *t > 0)
                    .map(|t| format!(" {} %", 100 * downloaded / t))
                    .unwrap_or_default();
                format!("Update wird geladen …{percent}")
            }
            Phase::Ready | Phase::Installing => {
                "Update wird installiert, Gelabber startet neu …".to_owned()
            }
            Phase::Failed(reason) => reason.clone(),
        }
    }

    /// The buttons, the primary first.
    pub fn buttons(&self) -> &'static [(&'static str, Click)] {
        match self.offer {
            Offer::Restart => &[("Neu starten", Click::Restart), ("Später", Click::Later)],
            Offer::Download => &[("Herunterladen", Click::Download), ("Später", Click::Later)],
            Offer::PackageManager => &[("Später", Click::Later)],
        }
    }

    /// Off while the update is at work, as the BANNER disables them.
    pub fn enabled(&self) -> bool {
        matches!(self.phase, Phase::Offer | Phase::Failed(_))
    }

    /// Over everything at the top centre, clear of the composer and the call
    /// dock: the BANNER's box (14px/1.4 system-ui, padding 10px 12px, gap 10px).
    pub fn paint(&self, ctx: &Context) -> Option<Click> {
        crate::fonts::use_system_ui(ctx);
        let style = TextStyle::new(14.0, 400.0, 1.4).system_ui();
        let enabled = self.enabled();
        let mut clicked = None;
        egui::Area::new(Id::unique("gelabber-update-notice"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 12.0))
            .constrain(false)
            .show(ctx, |ui| {
                let (pad_x, pad_y, gap, line) = (12.0, 10.0, 10.0, style.line_px());
                let buttons: Vec<(Block, Click, bool)> = self
                    .buttons()
                    .iter()
                    .enumerate()
                    .map(|(i, (label, click))| {
                        let primary = i == 0 && *click != Click::Later;
                        let (weight, ink) = if primary {
                            (600.0, ACCENT_INK)
                        } else {
                            (400.0, INK)
                        };
                        (
                            Block::single(ui, label, style.weight(weight), ink),
                            *click,
                            primary,
                        )
                    })
                    .collect();
                // padding 6px 12px; the secondary button's 1px border.
                let button_size = |block: &Block, primary: bool| {
                    let border = if primary { 0.0 } else { 2.0 };
                    Vec2::new(block.width() + 24.0 + border, line + 12.0 + border)
                };
                let buttons_w: f32 = buttons
                    .iter()
                    .map(|(b, _, p)| button_size(b, *p).x + gap)
                    .sum();
                // max-width: calc(100vw - 32px); the text wraps first.
                let screen = ctx.content_rect();
                let room = (screen.width() - 32.0 - 2.0 * (pad_x + 1.0) - buttons_w).max(40.0);
                let text = self.text();
                let mut block = Block::single(ui, &text, style, INK);
                if block.width() > room {
                    block = Block::wrapped(ui, &text, style, INK, room);
                }
                let row_h = buttons
                    .iter()
                    .map(|(b, _, p)| button_size(b, *p).y)
                    .fold(block.height(), f32::max);
                let size = Vec2::new(
                    block.width() + buttons_w + 2.0 * (pad_x + 1.0),
                    row_h + 2.0 * (pad_y + 1.0),
                );
                let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
                crate::widgets::announce(&response, egui::Role::Status, &text);
                let painter = ui.painter();
                // box-shadow: 0 6px 24px rgba(0,0,0,.35).
                painter.add(
                    egui::Shadow {
                        offset: [0, 6],
                        blur: 24,
                        spread: 0,
                        color: Color32::from_black_alpha(89),
                    }
                    .as_shape(rect, CornerRadius::same(12)),
                );
                painter.rect(
                    rect,
                    CornerRadius::same(12),
                    BG,
                    Stroke::new(1.0, BORDER),
                    StrokeKind::Inside,
                );
                let middle = rect.top() + 1.0 + pad_y + row_h / 2.0;
                let mut x = rect.left() + 1.0 + pad_x;
                block.paint(painter, pos2(x, middle - block.height() / 2.0));
                x += block.width();
                for (label, click, primary) in &buttons {
                    x += gap;
                    let size = button_size(label, *primary);
                    let button = Rect::from_min_size(pos2(x, middle - size.y / 2.0), size);
                    let (fill, stroke) = if *primary {
                        (ACCENT, Stroke::NONE)
                    } else {
                        (Color32::TRANSPARENT, Stroke::new(1.0, LINE))
                    };
                    painter.rect(
                        button,
                        CornerRadius::same(8),
                        fill,
                        stroke,
                        StrokeKind::Inside,
                    );
                    let inner = button.center();
                    label.paint(
                        painter,
                        pos2(inner.x - label.width() / 2.0, inner.y - line / 2.0),
                    );
                    let name = label.galley.text().to_owned();
                    let response = ui.interact(
                        button,
                        Id::unique(("gelabber-update", *click)),
                        Sense::click(),
                    );
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::Button, enabled, &name)
                    });
                    if response.hovered() && enabled {
                        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if response.clicked() && enabled {
                        clicked = Some(*click);
                    }
                    x += size.x;
                }
            });
        clicked
    }
}

type Slot<T> = Arc<Mutex<Option<T>>>;

/// The checks, the notice and an accepted install of the running app
/// (Platform owns one).
pub struct Updater {
    install: Install,
    /// `ctx` time of the next look.
    next: f64,
    last: Option<SystemTime>,
    checking: Option<Slot<Result<Option<Found>, String>>>,
    found: Option<Found>,
    /// "Später": no notice for this version until the next start.
    dismissed: Option<String>,
    phase: Phase,
    progress: Arc<Mutex<(u64, Option<u64>)>>,
    downloading: Option<Slot<Result<PathBuf, String>>>,
    ready: Option<PathBuf>,
    /// "Neu starten" was clicked; the next tick starts the download.
    requested: bool,
    lock: CallLock,
    locked: bool,
}

impl Default for Updater {
    fn default() -> Self {
        Self::new(Install::current())
    }
}

impl Updater {
    pub fn new(install: Install) -> Self {
        Self {
            install,
            next: 0.0,
            last: None,
            checking: None,
            found: None,
            dismissed: None,
            phase: Phase::Offer,
            progress: Arc::default(),
            downloading: None,
            ready: None,
            requested: false,
            lock: CallLock::default(),
            locked: false,
        }
    }

    /// The first look, [`FIRST_CHECK`] after `now`.
    pub fn start(&mut self, now: f64) {
        self.next = now + FIRST_CHECK.as_secs_f64();
    }

    /// Every pass with the app: checks when due, runs an accepted update and
    /// says when the app is to end for its installer. `busy`: [`call_busy`].
    pub fn tick(&mut self, client: &Client, now: f64, busy: bool) -> bool {
        self.locked = self.lock.locked(busy, Instant::now());
        self.take_check();
        self.take_download();
        if self.requested {
            self.requested = false;
            if !self.locked {
                self.start_download(client);
            }
        }
        // An install put off by a call goes on once it ended.
        if self.phase == Phase::Ready
            && !self.locked
            && let (Some(installer), Install::Setup { dir }) = (self.ready.clone(), &self.install)
        {
            *PENDING.lock() = Some((installer, dir.clone()));
            self.phase = Phase::Installing;
            return true;
        }
        if self.checking.is_none() && self.at_rest() && now >= self.next {
            self.next = now + 60.0;
            if auto_check() && due(self.last, SystemTime::now()) {
                let slot: Slot<_> = Arc::default();
                self.checking = Some(slot.clone());
                let http = client.http().reqwest().clone();
                client.spawn(async move {
                    *slot.lock() = Some(check(http).await);
                });
            }
        }
        false
    }

    /// No accepted update at work: a check may replace what was found.
    fn at_rest(&self) -> bool {
        matches!(self.phase, Phase::Offer | Phase::Failed(_))
    }

    fn take_check(&mut self) {
        let Some(result) = self.checking.as_ref().and_then(|s| s.lock().take()) else {
            return;
        };
        self.checking = None;
        self.last = Some(SystemTime::now());
        match result {
            Ok(found) => {
                if found.as_ref().map(|f| &f.version) != self.found.as_ref().map(|f| &f.version) {
                    self.phase = Phase::Offer;
                }
                self.found = found;
            }
            Err(e) => tracing::info!("update check: {e}"),
        }
    }

    fn start_download(&mut self, client: &Client) {
        let Some(Found {
            version,
            package: Some((url, signature)),
            ..
        }) = self.found.clone()
        else {
            return;
        };
        *self.progress.lock() = (0, None);
        self.phase = Phase::Downloading {
            downloaded: 0,
            total: None,
        };
        let slot: Slot<_> = Arc::default();
        self.downloading = Some(slot.clone());
        let progress = self.progress.clone();
        let wake = client.clone();
        client.spawn(async move {
            let result = match download_client() {
                Ok(http) => {
                    download(
                        &http,
                        &url,
                        &signature,
                        pubkey(),
                        &version,
                        &download_dir(&version),
                        |done, total| {
                            *progress.lock() = (done, total);
                            wake.repaint();
                        },
                    )
                    .await
                }
                Err(e) => Err(e),
            };
            *slot.lock() = Some(result);
        });
    }

    fn take_download(&mut self) {
        if let Phase::Downloading { .. } = self.phase {
            let (downloaded, total) = *self.progress.lock();
            self.phase = Phase::Downloading { downloaded, total };
        }
        let Some(result) = self.downloading.as_ref().and_then(|s| s.lock().take()) else {
            return;
        };
        self.downloading = None;
        match result {
            Ok(installer) => {
                self.ready = Some(installer);
                self.phase = Phase::Ready;
            }
            Err(e) => {
                tracing::warn!("update: {e}");
                self.phase = Phase::Failed(e);
            }
        }
    }

    /// The notice as it stands, `None` while there is none to show.
    pub fn notice(&self) -> Option<Notice> {
        let found = self.found.as_ref()?;
        // Not over a call, as the Tauri app's notice; nor once put away.
        if self.locked || self.dismissed.as_deref() == Some(found.version.as_str()) {
            return None;
        }
        Some(Notice {
            version: found.version.clone(),
            offer: Offer::for_install(&self.install, found),
            phase: self.phase.clone(),
        })
    }

    pub fn paint(&mut self, ctx: &Context) {
        let Some(notice) = self.notice() else { return };
        match notice.paint(ctx) {
            Some(Click::Later) => self.dismissed = Some(notice.version),
            Some(Click::Download) => {
                ctx.open_url(egui::OpenUrl::new_tab(release_page(&notice.version)));
            }
            Some(Click::Restart) => {
                self.requested = true;
                ctx.request_repaint();
            }
            None => {}
        }
    }

    /// Tests: a check's answer as if it had come.
    #[cfg(test)]
    fn set_found(&mut self, found: Option<Found>) {
        self.found = found;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Keeps the tests that touch [`PENDING`] apart.
    static PENDING_TESTS: Mutex<()> = Mutex::new(());

    // `tauri signer sign --app-version 0.7.0` of b"gelabber", the test key
    // of desktop/app/src/update.rs.
    const KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEVBNjEyRDkzOEVDOTk3MjgKUldRb2w4bU9reTFoNmswZldrczNPK1kveENldW5IVGo4Qm9XQ3lqdzZFL2hhZlB2SFRZdmxrLzEK";
    const SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVRb2w4bU9reTFoNm1ibm9XSmovRi9QNFBMcjdVZVQ3a1ozYVd3NFRxc2N1a3ExTmZHMkxWNUhuTDczS04vZm4wNG5EemhtWXg0NHphaXRxWUZoMWpDZnZPRFlaMzFlWUE0PQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkyMTk0NjY2CWZpbGU6Zml4dHVyZQl2ZXJzaW9uOjAuNy4wCkN4KzVxdTlBaEg3Nzc0SmU0L0tnZGJPaVFNemJ4aUQ3VmpqMFRaaTQ5dEVrZ2V5dGtzT0N6YjZFcXlncFNlTXJtVVZVdlN4VGdMUGR0WGV4NFcwa0NnPT0K";

    // Two throwaway keys made for these tests (Ed25519 with BLAKE2b prehash,
    // the "ED" signatures `tauri signer` writes); A signed INSTALLER.
    const INSTALLER: &[u8] = b"MZ gelabber test installer\n";
    const KEY_A: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDYyMkIzMjVCNzU0RkVFREIKUldUYjdrOTFXeklyWW95SUgvOG1WcnhhMVk5YUxEd0JDYzRIV3l0QWFVMW0xTDlZejFoTFZSak8K";
    const KEY_B: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDE5MUUyMTUxNzJFRUNFRUMKUldUc3p1NXlVU0VlR1l3ZmRab3ExTzU4dVlFK1lCWkhnK1RtallZNGI3QVVBUHVROEhYT2EveVgK";
    // version:0.9.1 in the trusted comment.
    const SIG_A: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVUYjdrOTFXeklyWWhJMVZ1YWhuM1RqWlRXQnNGNGN0UUozUXk3ODVHRVh5WTRaNDN3RXNNcklyR3l5RVFkR1BVQVlWcGFTanNYejJJYU9zM3JYREN3ZXUvQVBjZXZtMGdZPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkzMDAwMDAwCWZpbGU6Z2VsYWJiZXItbmF0aXZlLXdpbmRvd3MteDY0LXNldHVwLmV4ZQl2ZXJzaW9uOjAuOS4xClFoQXI4b3VoQm1SUzZyTW9aY2ZBT3VGZnphNTZ1MG55RnJydWtGdjBqMXdxZzExN0hDak8zMjZGMktoY2dmeGRFRHo3UmNFQnBiejlsQitxb1FJK0JRPT0K";
    // The same signature without a version in the trusted comment.
    const SIG_A_NO_VERSION: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVUYjdrOTFXeklyWWhJMVZ1YWhuM1RqWlRXQnNGNGN0UUozUXk3ODVHRVh5WTRaNDN3RXNNcklyR3l5RVFkR1BVQVlWcGFTanNYejJJYU9zM3JYREN3ZXUvQVBjZXZtMGdZPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkzMDAwMDAwCWZpbGU6Z2VsYWJiZXItbmF0aXZlLXdpbmRvd3MteDY0LXNldHVwLmV4ZQpST0xxT1Nsa2NTTTYxQTBHMmdwNkJrUzh6bzVBM0dpOWY1M0tkNkRXektuMnA2bEhqallJYm45amNBcFdaS1BHWmtPUm4rQ3EzaHlTUFZGY2tNYXpDQT09Cg==";
    // B's signature of INSTALLER for 0.9.1.
    const SIG_B: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVUc3p1NXlVU0VlR1FNb0RKTGQwQ2FPbjcvM2hqeUhkdGwvOU1aVFlUQy9aV1M0SlFrd2t6VDhIb3JLR3RudkhodEUxYmd2QWQrem1wWDVRSVEzbmRaT1VPZEE2Qm9hWVE4PQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkzMDAwMDAwCWZpbGU6Z2VsYWJiZXItbmF0aXZlLXdpbmRvd3MteDY0LXNldHVwLmV4ZQl2ZXJzaW9uOjAuOS4xCjJGbnA3bWtTK1ZDNWNoKytOazMxb3NWYjZSUXdUV0hsdjJzVW9CWi9yNXlyVjc0RlUxZTE3M2VCQzdsT2tyUEhxMDdXSnN2L1B4RVM5cDhvWnNkakFRPT0K";

    #[test]
    fn signatures_hold_for_their_bytes_key_and_version_only() {
        assert_eq!(verify(b"gelabber", SIGNATURE, KEY, "0.7.0"), Ok(()));
        assert!(verify(b"gelabbex", SIGNATURE, KEY, "0.7.0").is_err());
        assert!(verify(b"gelabber", SIGNATURE, KEY, "0.7.1").is_err());
        assert!(verify(b"gelabber", "", KEY, "0.7.0").is_err());
        // The releases' key is a different one.
        assert!(!pubkey().is_empty());
        assert!(verify(b"gelabber", SIGNATURE, pubkey(), "0.7.0").is_err());
    }

    #[test]
    fn an_installer_passes_only_with_its_key_version_and_bytes() {
        assert_eq!(verify(INSTALLER, SIG_A, KEY_A, "0.9.1"), Ok(()));
        assert_eq!(verify(INSTALLER, SIG_A, KEY_A, "v0.9.1"), Ok(()));
        // Wrong key: B's key for A's signature, A's key for B's.
        assert!(verify(INSTALLER, SIG_A, KEY_B, "0.9.1").is_err());
        assert!(verify(INSTALLER, SIG_B, KEY_A, "0.9.1").is_err());
        assert_eq!(verify(INSTALLER, SIG_B, KEY_B, "0.9.1"), Ok(()));
        // Wrong version: an older installer announced as a newer one.
        let older = verify(INSTALLER, SIG_A, KEY_A, "0.9.2").unwrap_err();
        assert!(older.contains("signed for 0.9.1"), "{older}");
        // requireSignedVersion: a signature that names no version is refused.
        assert_eq!(
            verify(INSTALLER, SIG_A_NO_VERSION, KEY_A, "0.9.1"),
            Err("signed without a version".into())
        );
        // Tampered: one bit, one byte more, one less.
        let mut tampered = INSTALLER.to_vec();
        tampered[3] ^= 1;
        assert!(verify(&tampered, SIG_A, KEY_A, "0.9.1").is_err());
        let longer = [INSTALLER, b"x"].concat();
        assert!(verify(&longer, SIG_A, KEY_A, "0.9.1").is_err());
        assert!(verify(&INSTALLER[1..], SIG_A, KEY_A, "0.9.1").is_err());
        // Nor with the releases' key.
        assert!(verify(INSTALLER, SIG_A, pubkey(), "0.9.1").is_err());
    }

    #[test]
    fn only_a_newer_version_is_offered_with_this_systems_package() {
        let latest: Latest = serde_json::from_str(
            r#"{"version":"v0.9.1","notes":"Neu","pub_date":"2026-11-01T00:00:00Z",
                "platforms":{"windows-x86_64":{"signature":"sig","url":"https://x/setup.exe"}}}"#,
        )
        .unwrap();
        let found = newer(&latest, "0.9.0", "windows-x86_64").unwrap();
        assert_eq!(found.version, "0.9.1");
        assert_eq!(found.notes.as_deref(), Some("Neu"));
        assert_eq!(
            found.package,
            Some(("https://x/setup.exe".into(), "sig".into()))
        );
        assert_eq!(
            newer(&latest, "0.9.0", "linux-x86_64").unwrap().package,
            None
        );
        assert!(newer(&latest, "0.9.1", "linux-x86_64").is_none());
        assert!(newer(&latest, "1.0.0", "linux-x86_64").is_none());
        assert!(newer(&latest, "kaputt", "linux-x86_64").is_none());
        assert!(target().contains('-'));
        // A pre-release is newer than the release before it only.
        let beta: Latest = serde_json::from_str(r#"{"version":"0.9.1-beta.2"}"#).unwrap();
        assert!(newer(&beta, "0.9.0", "linux-x86_64").is_some());
        assert!(newer(&beta, "0.9.1", "linux-x86_64").is_none());
        // What is not a manifest is refused, not offered.
        assert!(serde_json::from_str::<Latest>(r#"{"platforms":{}}"#).is_err());
        assert!(
            serde_json::from_str::<Latest>(
                r#"{"version":"0.9.1","platforms":{"windows-x86_64":{"url":"u"}}}"#
            )
            .is_err()
        );
    }

    #[test]
    fn the_native_app_reads_its_own_manifest_not_the_tauri_apps() {
        let _config = crate::platform::paths::testing::TempConfig::new("endpoint");
        assert_eq!(
            endpoint(),
            format!("{REPOSITORY}/releases/latest/download/{MANIFEST}")
        );
        assert_eq!(MANIFEST, "latest-native.json");
        assert!(!endpoint().ends_with("/latest.json"));
        assert_eq!(
            release_page("0.9.0-beta.2"),
            format!("{REPOSITORY}/releases/tag/v0.9.0-beta.2")
        );
        assert_eq!(
            release_page("v0.9.0"),
            format!("{REPOSITORY}/releases/tag/v0.9.0")
        );
    }

    #[test]
    fn the_manifest_reads_as_desktop_release_writes_it() {
        // The jq program of release-asset-windows in desktop-release.yml.
        let url =
            format!("{REPOSITORY}/releases/download/v0.9.1/gelabber-native-windows-x64-setup.exe");
        let manifest = serde_json::json!({
            "version": "0.9.1",
            "pub_date": "2026-11-05T12:00:00Z",
            "platforms": {"windows-x86_64": {"url": url, "signature": "c2ln"}},
        });
        let latest: Latest = serde_json::from_value(manifest).unwrap();
        let found = newer(&latest, "0.9.0", "windows-x86_64").unwrap();
        assert_eq!(found.package, Some((url.clone(), "c2ln".into())));
        assert_eq!(found.notes, None);
        assert_eq!(file_name(&url), "gelabber-native-windows-x64-setup.exe");
    }

    #[test]
    fn checks_are_due_every_six_hours_and_after_the_clock_went_back() {
        let now = std::time::SystemTime::now();
        assert!(due(None, now));
        assert!(!due(Some(now - Duration::from_secs(60)), now));
        assert!(due(Some(now - CHECK_EVERY), now));
        assert!(due(Some(now + Duration::from_secs(3600)), now));
    }

    #[test]
    fn a_check_reads_latest_json_from_the_endpoint() {
        let (addr, _) = serve(vec![(
            "application/json",
            br#"{"version":"99.0.0","platforms":{}}"#.to_vec(),
        )]);
        let _config = crate::platform::paths::testing::TempConfig::new("update");
        // SAFETY: the TempConfig lock keeps environment tests apart.
        unsafe {
            std::env::set_var(
                "GELABBER_UPDATE_ENDPOINT",
                format!("http://{addr}/latest.json"),
            )
        };
        let client = gelabber_client::Client::new("http://127.0.0.1:9").unwrap();
        let http = client.http().reqwest().clone();
        let found = client.block_on(check(http));
        // SAFETY: as above.
        unsafe { std::env::remove_var("GELABBER_UPDATE_ENDPOINT") };
        assert_eq!(found.unwrap().unwrap().version, "99.0.0");
        assert!(auto_check());
    }

    /// A local HTTP server answering one request per body, in order; the
    /// paths it was asked for.
    fn serve(bodies: Vec<(&'static str, Vec<u8>)>) -> (std::net::SocketAddr, Slot<Vec<String>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let asked: Slot<Vec<String>> = Arc::new(Mutex::new(Some(Vec::new())));
        let log = asked.clone();
        std::thread::spawn(move || {
            for (kind, body) in bodies {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);
                let path = request.split(' ').nth(1).unwrap_or_default().to_owned();
                if let Some(log) = log.lock().as_mut() {
                    log.push(path);
                }
                let status = if kind == "404" {
                    "404 Not Found"
                } else {
                    "200 OK"
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                // In two parts, so progress is heard more than once.
                let half = body.len() / 2;
                let _ = stream.write_all(&body[..half]);
                let _ = stream.flush();
                std::thread::sleep(Duration::from_millis(20));
                let _ = stream.write_all(&body[half..]);
            }
        });
        (addr, asked)
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "gelabber-update-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_signed_installer_is_downloaded_into_the_folder_and_a_forged_one_is_not() {
        let tampered = [INSTALLER, b"!"].concat();
        let (addr, asked) = serve(vec![
            ("application/octet-stream", INSTALLER.to_vec()),
            ("application/octet-stream", tampered),
            ("404", b"gone".to_vec()),
        ]);
        let url = format!("http://{addr}/download/v0.9.1/gelabber-native-windows-x64-setup.exe");
        let dir = temp_dir("download");
        let client = gelabber_client::Client::new("http://127.0.0.1:9").unwrap();
        let http = download_client().unwrap();
        let heard = Mutex::new(Vec::new());
        let path = client
            .block_on(download(
                &http,
                &url,
                SIG_A,
                KEY_A,
                "0.9.1",
                &dir,
                |done, total| heard.lock().push((done, total)),
            ))
            .unwrap();
        assert_eq!(path, dir.join("gelabber-native-windows-x64-setup.exe"));
        assert_eq!(std::fs::read(&path).unwrap(), INSTALLER);
        let heard = heard.into_inner();
        let size = INSTALLER.len() as u64;
        assert_eq!(heard.first(), Some(&(0, Some(size))));
        assert_eq!(heard.last(), Some(&(size, Some(size))));
        std::fs::remove_file(&path).unwrap();

        // A changed file is not written at all.
        let forged = client.block_on(download(
            &http,
            &url,
            SIG_A,
            KEY_A,
            "0.9.1",
            &dir,
            |_, _| {},
        ));
        assert!(
            forged
                .unwrap_err()
                .starts_with("Die Signatur des Updates stimmt nicht")
        );
        assert!(!path.exists());
        let missing = client.block_on(download(
            &http,
            &url,
            SIG_A,
            KEY_A,
            "0.9.1",
            &dir,
            |_, _| {},
        ));
        assert!(missing.unwrap_err().contains("404"));
        assert_eq!(
            asked.lock().as_ref().unwrap().as_slice(),
            ["/download/v0.9.1/gelabber-native-windows-x64-setup.exe"; 3]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_download_lands_under_a_plain_name_only() {
        assert_eq!(file_name("https://x/a/setup.exe?raw=1"), "setup.exe");
        assert_eq!(
            file_name("https://x/a/..%2F..%2Fevil.exe"),
            "gelabber-setup.exe"
        );
        assert_eq!(file_name("https://x/a/"), "gelabber-setup.exe");
        assert_eq!(file_name("https://x/.."), "gelabber-setup.exe");
        assert!(download_dir("0.9.1").starts_with(std::env::temp_dir()));
    }

    #[test]
    fn the_setup_runs_passively_into_the_same_folder_with_the_apps_arguments() {
        let dir = Path::new(r"C:\Users\Anna Berg\AppData\Local\Gelabber");
        let args = ["--server".to_owned(), "https://gelabber.example.org".into()];
        assert_eq!(
            installer_parameters(&args, dir),
            r#"/P /UPDATE /R /ARGS --server "https://gelabber.example.org" /D=C:\Users\Anna Berg\AppData\Local\Gelabber"#
        );
        assert_eq!(
            installer_parameters(&[], dir),
            r"/P /UPDATE /R /ARGS /D=C:\Users\Anna Berg\AppData\Local\Gelabber"
        );
        // The updater plugin's escaping.
        assert_eq!(nsis_arg(""), r#""""#);
        assert_eq!(nsis_arg("a b"), r#""a b""#);
        assert_eq!(nsis_arg(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(nsis_arg(r"C:\dir\ x\"), r#""C:\dir\ x\\""#);
        // The options installer.nsi reads.
        let nsi = include_str!("../../../packaging/native/windows/installer.nsi");
        for option in ["\"/P\"", "\"/UPDATE\"", "\"/R\"", "\"/ARGS\""] {
            assert!(
                nsi.contains(&format!("GetOptions}} $R0 {option}")),
                "{option}"
            );
        }
        assert!(nsi.contains(r#"InstallDirRegKey HKCU "${MANUPRODUCTKEY}" """#));
    }

    #[test]
    fn without_an_accepted_update_nothing_runs_after_the_window_closed() {
        let _pending = PENDING_TESTS.lock();
        assert_eq!(run_pending(), Ok(false));
        #[cfg(not(windows))]
        {
            *PENDING.lock() = Some(("/tmp/setup.exe".into(), "/tmp".into()));
            assert!(run_pending().is_err());
            assert_eq!(run_pending(), Ok(false));
        }
    }

    #[test]
    fn the_way_it_was_installed_decides_the_offer() {
        let none = |_: &Path| false;
        let all = |_: &Path| true;
        let setup = Install::of("windows", Path::new(r"C:\Gelabber\gelabber.exe"), |p| {
            p.ends_with("uninstall.exe")
        });
        assert!(matches!(setup, Install::Setup { .. }));
        assert_eq!(
            Install::of("windows", Path::new(r"C:\Downloads\gelabber.exe"), none),
            Install::Manual
        );
        assert_eq!(
            Install::of("linux", Path::new("/opt/gelabber/gelabber"), none),
            Install::Pacman
        );
        assert_eq!(
            Install::of("linux", Path::new("/opt/gelabber-desktop/gelabber"), all),
            Install::Manual
        );
        assert_eq!(
            Install::of(
                "linux",
                Path::new("/home/a/gelabber-native-linux-x64/gelabber"),
                all
            ),
            Install::Manual
        );
        // The PKGBUILD's folder.
        let pkgbuild = include_str!("../../../packaging/arch/native/PKGBUILD");
        assert!(pkgbuild.contains(&format!("\"${{pkgdir}}{PACMAN_DIR}/gelabber\"")));
        assert!(pkgbuild.contains(&format!("pkgname={PACMAN_PACKAGE}\n")));

        let found = Found {
            version: "0.9.1".into(),
            notes: None,
            package: Some(("u".into(), "s".into())),
        };
        let unsigned = Found {
            package: None,
            ..found.clone()
        };
        assert_eq!(Offer::for_install(&setup, &found), Offer::Restart);
        assert_eq!(Offer::for_install(&setup, &unsigned), Offer::Download);
        assert_eq!(
            Offer::for_install(&Install::Pacman, &found),
            Offer::PackageManager
        );
        assert_eq!(
            Offer::for_install(&Install::Manual, &found),
            Offer::Download
        );
    }

    #[test]
    fn the_notice_says_what_the_tauri_banner_says() {
        let banner = include_str!("../../../app/src/update.rs");
        let mut notice = Notice::new("0.9.1", Offer::Restart);
        assert_eq!(
            notice.text(),
            "Gelabber 0.9.1 ist da. Zum Installieren startet die App neu."
        );
        assert!(
            banner.contains("`Gelabber ${version} ist da. Zum Installieren startet die App neu.`")
        );
        assert_eq!(notice.buttons()[0].0, "Neu starten");
        assert_eq!(notice.buttons()[1].0, "Später");
        assert!(banner.contains(r#"button("Neu starten", true)"#));
        assert!(banner.contains(r#"button("Später", false)"#));
        assert!(notice.enabled());
        notice.phase = Phase::Downloading {
            downloaded: 0,
            total: None,
        };
        assert_eq!(notice.text(), "Update wird geladen …");
        notice.phase = Phase::Downloading {
            downloaded: 19,
            total: Some(47),
        };
        assert_eq!(notice.text(), "Update wird geladen … 40 %");
        assert!(banner.contains("`Update wird geladen …${percent}`"));
        assert!(!notice.enabled());
        notice.phase = Phase::Installing;
        assert_eq!(
            notice.text(),
            "Update wird installiert, Gelabber startet neu …"
        );
        assert!(banner.contains("\"Update wird installiert, Gelabber startet neu …\""));
        notice.phase = Phase::Failed("kaputt".into());
        assert_eq!(notice.text(), "kaputt");
        assert!(notice.enabled());
        for color in [
            "#1d2021", "#2c3032", "#eef0ee", "#f3b752", "#191b1b", "#3a3f42",
        ] {
            assert!(banner.contains(color), "{color}");
        }

        let link = Notice::new("0.9.1", Offer::Download);
        assert_eq!(link.text(), "Gelabber 0.9.1 ist da.");
        assert_eq!(link.buttons()[0].0, "Herunterladen");
        let pacman = Notice::new("0.9.1", Offer::PackageManager);
        assert!(
            pacman
                .text()
                .contains("pacman (Paket gelabber-desktop-native)")
        );
        assert_eq!(pacman.buttons(), &[("Später", Click::Later)]);
    }

    #[test]
    fn the_call_lock_holds_during_a_call_and_a_minute_after() {
        let mut lock = CallLock::default();
        let t = Instant::now();
        assert!(!lock.locked(false, t));
        assert!(lock.locked(true, t));
        assert!(lock.locked(false, t + Duration::from_secs(59)));
        assert!(!lock.locked(false, t + BUSY_AFTER));
        // A clock read out of order does not lift it early.
        assert!(lock.locked(true, t + BUSY_AFTER));
        assert!(lock.locked(false, t));
    }

    #[test]
    fn a_call_hides_the_notice_and_puts_the_install_off_until_it_ended() {
        let _pending = PENDING_TESTS.lock();
        let client = gelabber_client::Client::new("http://127.0.0.1:9").unwrap();
        let dir = temp_dir("lock");
        let mut updater = Updater::new(Install::Setup { dir: dir.clone() });
        updater.set_found(Some(Found {
            version: "0.9.1".into(),
            notes: None,
            package: Some(("http://127.0.0.1:9/setup.exe".into(), SIG_A.into())),
        }));
        updater.next = f64::MAX;
        assert!(!updater.tick(&client, 0.0, false));
        assert_eq!(updater.notice().unwrap().offer, Offer::Restart);
        // In a call: no notice, and "Neu starten" cannot have been clicked.
        assert!(!updater.tick(&client, 0.0, true));
        assert_eq!(updater.notice(), None);
        // The download was done when a call began: the install waits.
        updater.ready = Some(dir.join("setup.exe"));
        updater.phase = Phase::Ready;
        assert!(!updater.tick(&client, 0.0, true));
        assert_eq!(*PENDING.lock(), None);
        assert_eq!(updater.notice(), None);
        // Still within a minute after the call.
        assert!(!updater.tick(&client, 0.0, false));
        // A minute later the install goes on, into the same folder.
        updater.lock.last_busy = Some(Instant::now() - BUSY_AFTER);
        assert!(updater.tick(&client, 0.0, false));
        assert_eq!(updater.phase, Phase::Installing);
        assert_eq!(
            PENDING.lock().take(),
            Some((dir.join("setup.exe"), dir.clone()))
        );
        assert_eq!(
            updater.notice().unwrap().text(),
            "Update wird installiert, Gelabber startet neu …"
        );
    }

    #[test]
    fn restart_downloads_checks_and_then_asks_the_app_to_end() {
        let _pending = PENDING_TESTS.lock();
        let (addr, _) = serve(vec![("application/octet-stream", INSTALLER.to_vec())]);
        let client = gelabber_client::Client::new("http://127.0.0.1:9").unwrap();
        let mut updater = Updater::new(Install::Setup {
            dir: temp_dir("restart"),
        });
        updater.next = f64::MAX;
        updater.set_found(Some(Found {
            version: "0.9.1".into(),
            notes: None,
            package: Some((format!("http://{addr}/setup.exe"), SIG_A.into())),
        }));
        updater.requested = true;
        assert!(!updater.tick(&client, 0.0, false));
        assert!(matches!(updater.phase, Phase::Downloading { .. }));
        assert!(!updater.notice().unwrap().enabled());
        // The releases' key refuses the test installer: the notice says so
        // and can be tried again.
        let start = Instant::now();
        while matches!(updater.phase, Phase::Downloading { .. }) {
            assert!(start.elapsed() < Duration::from_secs(10), "download hangs");
            std::thread::sleep(Duration::from_millis(10));
            assert!(!updater.tick(&client, 0.0, false));
        }
        let Phase::Failed(reason) = &updater.phase else {
            panic!("{:?}", updater.phase)
        };
        assert!(
            reason.starts_with("Die Signatur des Updates stimmt nicht"),
            "{reason}"
        );
        assert!(updater.notice().unwrap().enabled());
        assert_eq!(*PENDING.lock(), None);
    }

    #[test]
    fn the_notice_paints_and_its_buttons_answer() {
        use egui_kittest::kittest::Queryable;
        let notice = Arc::new(Mutex::new(Notice::new("0.9.1", Offer::Restart)));
        let clicks = Arc::new(Mutex::new(Vec::new()));
        let (n, c) = (notice.clone(), clicks.clone());
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .build_ui(move |ui| {
                if let Some(click) = n.lock().paint(ui.ctx()) {
                    c.lock().push(click);
                }
            });
        harness.run_steps(3);
        harness.get_by_label("Neu starten").click();
        harness.run_steps(2);
        harness.get_by_label("Später").click();
        harness.run_steps(2);
        assert_eq!(*clicks.lock(), [Click::Restart, Click::Later]);
        // At work: the buttons show but do nothing.
        notice.lock().phase = Phase::Downloading {
            downloaded: 1,
            total: Some(2),
        };
        harness.run_steps(2);
        harness.get_by_label("Update wird geladen … 50 %");
        harness.get_by_label("Neu starten").click();
        harness.run_steps(2);
        assert_eq!(clicks.lock().len(), 2);
    }
}

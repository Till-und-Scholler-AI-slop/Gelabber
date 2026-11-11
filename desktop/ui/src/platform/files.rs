//! Choosing files to open and where to save one, where the web has
//! `<input type=file>` and downloads. On Linux the FileChooser portal of
//! xdg-desktop-portal over zbus, then zenity or kdialog; on Windows the
//! system's open and save dialogs; on macOS `choose file`. Without any of
//! them a path field in the window picks, and a save goes to the Downloads
//! folder. A save says in a toast where the file went, or why it did not.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gelabber_i18n::app as texts;
use parking_lot::Mutex;

/// What is asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    Open,
    OpenMany,
    /// Where to save a file suggested as `name`.
    Save {
        name: String,
    },
}

#[derive(Clone, Debug)]
pub struct Request {
    pub title: String,
    /// Glob patterns ("*.png"); none for any file.
    pub filter: Vec<String>,
    pub mode: Mode,
}

impl Request {
    fn new(title: &str, filter: &[&str], mode: Mode) -> Self {
        Self {
            title: title.to_owned(),
            filter: filter.iter().map(|f| (*f).to_owned()).collect(),
            mode,
        }
    }
}

/// What a chooser said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    /// One path, or for [`Mode::OpenMany`] one or more.
    Chosen(Vec<PathBuf>),
    Cancelled,
    /// No chooser answered: ask in the window (save: the Downloads folder).
    InApp,
}

/// Where to look for choosers; tests point it at a private bus and at
/// programs that do not exist.
#[derive(Clone, Debug)]
pub struct Choosers {
    /// A bus address; the user's session bus when `None`.
    pub bus: Option<String>,
    pub portal: bool,
    /// The system's own dialogs (Windows).
    pub native: bool,
    /// Programs by name; empty for none. Not on Windows, where a bare name
    /// would run whatever `zenity.exe` the PATH or the folder holds.
    pub zenity: String,
    pub kdialog: String,
    /// Where a save goes without a dialog; the user's Downloads when `None`.
    pub downloads: Option<PathBuf>,
}

impl Default for Choosers {
    fn default() -> Self {
        let program = |name: &str| if cfg!(windows) { "" } else { name }.to_owned();
        Self {
            bus: None,
            portal: cfg!(target_os = "linux"),
            native: cfg!(windows),
            zenity: program("zenity"),
            kdialog: program("kdialog"),
            downloads: None,
        }
    }
}

fn choosers_id() -> egui::Id {
    egui::Id::unique("gelabber-file-choosers")
}

/// The choosers `ctx` uses (tests set their own).
pub fn choosers(ctx: &egui::Context) -> Choosers {
    ctx.data(|d| d.get_temp::<Choosers>(choosers_id()))
        .unwrap_or_default()
}

pub fn set_choosers(ctx: &egui::Context, choosers: Choosers) {
    ctx.data_mut(|d| d.insert_temp(choosers_id(), choosers));
}

/// Asks the choosers in order, on the calling thread.
pub fn choose(choosers: &Choosers, request: &Request) -> Choice {
    #[cfg(target_os = "linux")]
    if choosers.portal
        && let Some(choice) = portal(choosers.bus.as_deref(), request)
    {
        return choice;
    }
    #[cfg(windows)]
    if choosers.native
        && let Some(choice) = native::dialog(request)
    {
        return choice;
    }
    for (program, args) in programs(choosers, request) {
        if program.is_empty() {
            continue;
        }
        match std::process::Command::new(&program).args(&args).output() {
            Ok(out) if out.status.success() => {
                let mut paths: Vec<PathBuf> = String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .map(str::trim_end)
                    .filter(|l| !l.is_empty())
                    .map(PathBuf::from)
                    .collect();
                if request.mode != Mode::OpenMany {
                    paths.truncate(1);
                }
                return if paths.is_empty() {
                    Choice::Cancelled
                } else {
                    Choice::Chosen(paths)
                };
            }
            // Exit 1 is a cancelled dialog in all of them; anything else (no
            // display, a broken install) tries the next chooser.
            Ok(out) if out.status.code() == Some(1) => return Choice::Cancelled,
            Ok(_) | Err(_) => continue,
        }
    }
    Choice::InApp
}

/// The dialog programs to try, with their arguments for `request`.
fn programs(choosers: &Choosers, request: &Request) -> Vec<(String, Vec<String>)> {
    let title = request.title.clone();
    let patterns = request.filter.join(" ");
    let many = request.mode == Mode::OpenMany;
    if cfg!(target_os = "macos") {
        if matches!(request.mode, Mode::Save { .. }) {
            return Vec::new();
        }
        let choose = if many {
            "set f to choose file with multiple selections allowed" // i18n-ok
        } else {
            "set f to {choose file}" // i18n-ok
        };
        let script = [
            choose,
            "set out to \"\"",                             // i18n-ok
            "repeat with x in f",                          // i18n-ok
            "set out to out & POSIX path of x & linefeed", // i18n-ok
            "end repeat",                                  // i18n-ok
            "out",
        ];
        let args = script
            .iter()
            .flat_map(|l| ["-e".to_owned(), (*l).to_owned()]);
        return vec![("osascript".into(), args.collect())];
    }
    let (zenity, kdialog) = match &request.mode {
        Mode::Save { name } => {
            let suggested = downloads_dir(choosers)
                .map(|d| d.join(name))
                .unwrap_or_else(|| PathBuf::from(name))
                .to_string_lossy()
                .into_owned();
            (
                vec![
                    "--file-selection".into(),
                    "--save".into(),
                    "--confirm-overwrite".into(),
                    format!("--title={title}"),
                    format!("--filename={suggested}"),
                ],
                vec![
                    "--title".into(),
                    title,
                    "--getsavefilename".into(),
                    suggested,
                ],
            )
        }
        Mode::Open | Mode::OpenMany => {
            let mut zenity = vec!["--file-selection".into(), format!("--title={title}")];
            let mut kdialog = vec!["--title".into(), title];
            if !patterns.is_empty() {
                zenity.push(format!("--file-filter={patterns}"));
            }
            if many {
                zenity.extend(["--multiple".into(), "--separator=\n".into()]);
                kdialog.extend(["--multiple".into(), "--separate-output".into()]);
            }
            kdialog.extend(["--getopenfilename".into(), ".".into()]);
            if !patterns.is_empty() {
                kdialog.push(patterns);
            }
            (zenity, kdialog)
        }
    };
    vec![
        (choosers.zenity.clone(), zenity),
        (choosers.kdialog.clone(), kdialog),
    ]
}

/// `org.freedesktop.portal.FileChooser` OpenFile or SaveFile; `None` when
/// there is no portal (or no session bus) to ask.
#[cfg(target_os = "linux")]
fn portal(bus: Option<&str>, request: &Request) -> Option<Choice> {
    use std::collections::HashMap;
    use zbus::blocking::{MessageIterator, connection};
    use zbus::message::Type;
    use zbus::zvariant::{OwnedValue, Value};

    const DESKTOP: &str = "org.freedesktop.portal.Desktop";
    const PATH: &str = "/org/freedesktop/portal/desktop";
    const REQUEST: &str = "org.freedesktop.portal.Request";

    let builder = match bus {
        Some(address) => connection::Builder::address(address).ok()?,
        None => connection::Builder::session().ok()?,
    };
    let conn = builder.build().ok()?;
    let sender = conn
        .unique_name()?
        .trim_start_matches(':')
        .replace('.', "_");
    let token = "gelabber_file";
    let path = format!("{PATH}/request/{sender}/{token}");
    // Subscribed before the call, so the answer cannot come first.
    let rule = zbus::MatchRule::builder()
        .msg_type(Type::Signal)
        .interface(REQUEST)
        .ok()?
        .member("Response")
        .ok()?
        .path(path.as_str())
        .ok()?
        .build();
    let mut answers = MessageIterator::for_match_rule(rule, &conn, None).ok()?;
    let title = request.title.as_str();
    let patterns: Vec<(u32, String)> = request.filter.iter().map(|p| (0u32, p.clone())).collect();
    let mut options: HashMap<&str, Value<'_>> = HashMap::from([
        ("handle_token", Value::from(token)),
        ("modal", Value::from(true)),
    ]);
    if !patterns.is_empty() {
        let filters = vec![(title.to_owned(), patterns.clone())];
        options.insert("filters", Value::from(filters.clone()));
        options.insert("current_filter", Value::from(filters[0].clone()));
    }
    let method = match &request.mode {
        Mode::Open => "OpenFile",
        Mode::OpenMany => {
            options.insert("multiple", Value::from(true));
            "OpenFile"
        }
        Mode::Save { name } => {
            options.insert("current_name", Value::from(name.as_str()));
            "SaveFile"
        }
    };
    // No portal, or one without a file chooser: the next chooser.
    conn.call_method(
        Some(DESKTOP),
        PATH,
        Some("org.freedesktop.portal.FileChooser"),
        method,
        &("", title, options),
    )
    .ok()?;
    // The user may take their time; the portal always answers.
    let message = answers.next()?.ok()?;
    let (code, results): (u32, HashMap<String, OwnedValue>) = message.body().deserialize().ok()?;
    if code != 0 {
        return Some(Choice::Cancelled);
    }
    let uris = results
        .get("uris")
        .and_then(|v| Vec::<String>::try_from(v.try_clone().ok()?).ok())
        .unwrap_or_default();
    let mut paths: Vec<PathBuf> = uris
        .iter()
        .filter_map(|uri| url::Url::parse(uri).ok()?.to_file_path().ok())
        .collect();
    if request.mode != Mode::OpenMany {
        paths.truncate(1);
    }
    Some(if paths.is_empty() {
        Choice::Cancelled
    } else {
        Choice::Chosen(paths)
    })
}

/// The Windows open and save dialogs (IFileOpenDialog, IFileSaveDialog).
#[cfg(windows)]
mod native {
    use std::path::PathBuf;

    use windows::Win32::Foundation::ERROR_CANCELLED;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance,
        CoInitializeEx, CoTaskMemFree, CoUninitialize,
    };
    use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
    use windows::Win32::UI::Shell::{
        FOS_ALLOWMULTISELECT, FOS_FORCEFILESYSTEM, FOS_OVERWRITEPROMPT, FileOpenDialog,
        FileSaveDialog, IFileOpenDialog, IFileSaveDialog, IShellItem, SIGDN_FILESYSPATH,
    };
    use windows::core::{HRESULT, HSTRING, PCWSTR};

    use super::{Choice, Mode, Request};

    /// `None` when the dialog cannot be had: the window asks then.
    pub fn dialog(request: &Request) -> Option<Choice> {
        // SAFETY: an apartment of this (worker) thread, ended below after
        // every COM object of `run` is gone.
        let init =
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        if init.is_err() {
            tracing::warn!("file dialog: {init:?}");
            return None;
        }
        // SAFETY: COM is initialized on this thread.
        let result = unsafe { run(request) };
        // SAFETY: pairs the successful CoInitializeEx above.
        unsafe { CoUninitialize() };
        match result {
            Ok(choice) => Some(choice),
            Err(e) if e.code() == HRESULT::from_win32(ERROR_CANCELLED.0) => Some(Choice::Cancelled),
            Err(e) => {
                tracing::warn!("file dialog: {e}");
                None
            }
        }
    }

    unsafe fn path_of(item: &IShellItem) -> windows::core::Result<PathBuf> {
        // SAFETY: the name is the shell's allocation, freed once copied.
        unsafe {
            let name = item.GetDisplayName(SIGDN_FILESYSPATH)?;
            let path = String::from_utf16_lossy(name.as_wide());
            CoTaskMemFree(Some(name.0 as *const _));
            Ok(PathBuf::from(path))
        }
    }

    unsafe fn run(request: &Request) -> windows::core::Result<Choice> {
        let title = HSTRING::from(request.title.as_str());
        // SAFETY: COM calls on this thread's apartment; the strings outlive them.
        unsafe {
            if let Mode::Save { name } = &request.mode {
                let dialog: IFileSaveDialog =
                    CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)?;
                dialog
                    .SetOptions(dialog.GetOptions()? | FOS_OVERWRITEPROMPT | FOS_FORCEFILESYSTEM)?;
                dialog.SetTitle(&title)?;
                dialog.SetFileName(&HSTRING::from(name.as_str()))?;
                dialog.Show(None)?;
                return Ok(Choice::Chosen(vec![path_of(&dialog.GetResult()?)?]));
            }
            let dialog: IFileOpenDialog =
                CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
            let mut options = dialog.GetOptions()? | FOS_FORCEFILESYSTEM;
            if request.mode == Mode::OpenMany {
                options |= FOS_ALLOWMULTISELECT;
            }
            dialog.SetOptions(options)?;
            dialog.SetTitle(&title)?;
            let spec = HSTRING::from(request.filter.join(";"));
            if !request.filter.is_empty() {
                let filters = [COMDLG_FILTERSPEC {
                    pszName: PCWSTR(title.as_ptr()),
                    pszSpec: PCWSTR(spec.as_ptr()),
                }];
                dialog.SetFileTypes(&filters)?;
            }
            dialog.Show(None)?;
            let items = dialog.GetResults()?;
            let mut paths = Vec::new();
            for i in 0..items.GetCount()? {
                paths.push(path_of(&items.GetItemAt(i)?)?);
            }
            Ok(Choice::Chosen(paths))
        }
    }
}

/// Where a save goes without a dialog: the given folder, else the user's
/// Downloads (else home) folder.
fn downloads_dir(choosers: &Choosers) -> Option<PathBuf> {
    if let Some(dir) = &choosers.downloads {
        return Some(dir.clone());
    }
    let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
    let home = if cfg!(windows) {
        var("USERPROFILE")
    } else {
        var("HOME")
    }
    .map(PathBuf::from)?;
    let downloads = var("XDG_DOWNLOAD_DIR")
        .filter(|_| !cfg!(windows))
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join("Downloads"));
    Some(if downloads.is_dir() { downloads } else { home })
}

/// `name` in `dir`, or `name (2)` and on when it is taken: no dialog asked
/// whether to replace it.
fn free_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.split_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (name, String::new()),
    };
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

/// Writes `bytes` to a hidden file beside `path` and renames it: a write
/// that fails (a full disk) leaves no partial file under the chosen name.
/// The folder must exist, as for a plain write.
fn write_whole(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // Short: the chosen name may already be near the file system's limit.
    let partial = path.with_file_name(format!(".gelabber-{}-{n}.partial", std::process::id()));
    let written = std::fs::File::create(&partial).and_then(|mut out| {
        out.write_all(bytes)?;
        out.sync_all()
    });
    let done = written.and_then(|()| std::fs::rename(&partial, path));
    if done.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    done
}

/// What a save said, for the app's toasts: the text and whether it failed.
type Notices = Arc<Mutex<Vec<(String, bool)>>>;

fn notices_id() -> egui::Id {
    egui::Id::unique("gelabber-file-notices")
}

fn notices(ctx: &egui::Context) -> Notices {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Notices>(notices_id()).clone())
}

/// The saves' reports since the last call; the app shows them as toasts.
pub fn take_notices(ctx: &egui::Context) -> Vec<(String, bool)> {
    ctx.data(|d| d.get_temp::<Notices>(notices_id()))
        .map(|n| std::mem::take(&mut *n.lock()))
        .unwrap_or_default()
}

/// Saves `bytes` where the user chooses (the web's download), off the UI
/// thread; without a dialog into the Downloads folder. A toast says where.
pub fn save(ctx: &egui::Context, title: &str, name: &str, bytes: Vec<u8>) {
    let ctx = ctx.clone();
    // Only the last part of `name`: a path in it must not leave the folder.
    let Some(name) = Path::new(name)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
    else {
        tracing::warn!("save: {name:?} is no file name");
        notices(&ctx)
            .lock()
            .push((texts::file_save_no_name().into(), true));
        ctx.request_repaint();
        return;
    };
    let name = name.as_str();
    let request = Request::new(
        title,
        &[],
        Mode::Save {
            name: name.to_owned(),
        },
    );
    let choosers = choosers(&ctx);
    let notices = notices(&ctx);
    let name = name.to_owned();
    std::thread::spawn(move || {
        let target = match choose(&choosers, &request) {
            Choice::Chosen(paths) => paths.into_iter().next(),
            Choice::Cancelled => None,
            Choice::InApp => match downloads_dir(&choosers) {
                Some(dir) => Some(free_path(&dir, &name)),
                None => {
                    let e = texts::file_no_downloads();
                    notices.lock().push((texts::file_not_saved(e), true));
                    None
                }
            },
        };
        if let Some(path) = target {
            let notice = match write_whole(&path, &bytes) {
                Ok(()) => (texts::file_saved_to(&path.display().to_string()), false),
                Err(e) => {
                    tracing::warn!("save {}: {e}", path.display());
                    (texts::file_not_saved(&e.to_string()), true)
                }
            };
            notices.lock().push(notice);
        }
        ctx.request_repaint();
    });
}

/// A file the user picked: its bytes and name, or `None` when nothing was
/// chosen (the slot of `pick_file` in the settings).
pub type Picked = Arc<Mutex<Option<Option<(Vec<u8>, String)>>>>;

/// Files chosen to attach, once the chooser closed; empty when none.
pub type PickedMany = Arc<Mutex<Option<Vec<PathBuf>>>>;

/// Reads `path` for a slot: the bytes and the file name.
pub fn read(path: &Path) -> Option<(Vec<u8>, String)> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    std::fs::read(path).ok().map(|bytes| (bytes, name))
}

/// Takes the path the window's field was given, or `None`.
type Done = Box<dyn FnOnce(Option<PathBuf>) + Send>;

/// A choice that ended in the window: the path field's state.
struct InApp {
    title: String,
    path: String,
    error: Option<String>,
    done: Option<Done>,
}

fn in_app_id() -> egui::Id {
    egui::Id::unique("gelabber-file-path-field")
}

/// Asks off the UI thread; `done` gets the paths (empty when none), from the
/// chooser or from the path field in the window.
fn open(ctx: &egui::Context, request: Request, done: impl FnOnce(Vec<PathBuf>) + Send + 'static) {
    let ctx = ctx.clone();
    let choosers = choosers(&ctx);
    std::thread::spawn(move || {
        match choose(&choosers, &request) {
            Choice::Chosen(paths) => done(paths),
            Choice::Cancelled => done(Vec::new()),
            Choice::InApp => {
                let field = InApp {
                    title: request.title,
                    path: String::new(),
                    error: None,
                    done: Some(Box::new(move |path| done(path.into_iter().collect()))),
                };
                ctx.data_mut(|d| d.insert_temp(in_app_id(), Arc::new(Mutex::new(Some(field)))));
            }
        }
        ctx.request_repaint();
    });
}

/// Opens the file chooser for one file; `slot` fills with the file, or `None`.
pub fn pick(ctx: &egui::Context, title: &str, filter: &[&str], slot: Picked) {
    open(ctx, Request::new(title, filter, Mode::Open), move |paths| {
        *slot.lock() = Some(paths.first().and_then(|p| read(p)));
    });
}

/// Opens the file chooser for any number of files.
pub fn pick_many(ctx: &egui::Context, title: &str) -> PickedMany {
    let slot: PickedMany = Arc::default();
    let out = slot.clone();
    open(
        ctx,
        Request::new(title, &[], Mode::OpenMany),
        move |paths| {
            *out.lock() = Some(paths);
        },
    );
    slot
}

type Shared = Arc<Mutex<Option<InApp>>>;

/// The path field, when a choice fell through to the window. The app
/// calls it every frame.
pub fn show_in_app(ctx: &egui::Context) {
    let Some(shared) = ctx.data(|d| d.get_temp::<Shared>(in_app_id())) else {
        return;
    };
    let mut guard = shared.lock();
    let Some(field) = guard.as_mut() else {
        drop(guard);
        ctx.data_mut(|d| d.remove::<Shared>(in_app_id()));
        return;
    };
    let (cancel, submit) = (std::cell::Cell::new(false), std::cell::Cell::new(false));
    let title = field.title.clone();
    let error = field.error.clone();
    let path = &mut field.path;
    let placeholder = if cfg!(windows) {
        texts::file_path_placeholder_windows()
    } else {
        texts::file_path_placeholder()
    };
    let output = crate::widgets::Modal::new("file-path", &title).show(
        ctx,
        |ui| {
            let input = crate::widgets::TextField::new("file-path", path)
                .label(texts::file_path_label())
                .placeholder(placeholder)
                .autofocus(true)
                .error(error.as_deref())
                .show(ui);
            if input.submitted {
                submit.set(true);
            }
            input.response.changed()
        },
        Some(&mut |ui: &mut egui::Ui| {
            crate::screens::shell::overlays::row(ui, 36.0, 8.0, |ui| {
                if ui
                    .add(crate::widgets::Button::new(gelabber_i18n::common::cancel()).ghost())
                    .clicked()
                {
                    cancel.set(true);
                }
                if ui
                    .add(crate::widgets::Button::new(texts::file_open()).primary())
                    .clicked()
                {
                    submit.set(true);
                }
            });
        }),
    );
    if output.inner {
        field.error = None;
    }
    let mut done = None;
    if output.close_requested || cancel.get() {
        done = Some(None);
    } else if submit.get() {
        done = typed(field).map(Some);
    }
    if let Some(result) = done {
        if let Some(finish) = field.done.take() {
            finish(result);
        }
        *guard = None;
    }
}

/// The typed path when it names a file that opens, or the field's error.
fn typed(field: &mut InApp) -> Option<PathBuf> {
    let typed = field.path.trim();
    let path = typed
        .strip_prefix("~/")
        .and_then(|rest| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(rest)))
        .unwrap_or_else(|| PathBuf::from(typed));
    if path.is_file() && std::fs::File::open(&path).is_ok() {
        Some(path)
    } else {
        field.error = Some(texts::file_cannot_open().into());
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nothing() -> Choosers {
        Choosers {
            bus: None,
            portal: false,
            native: false,
            zenity: "gelabber-test-no-zenity".into(),
            kdialog: "gelabber-test-no-kdialog".into(),
            downloads: None,
        }
    }

    fn image(mode: Mode) -> Request {
        Request::new("Bild", &["*.png"], mode)
    }

    /// Without a portal on the bus and without the programs: the window.
    #[test]
    fn without_choosers_the_window_asks() {
        #[cfg(target_os = "linux")]
        {
            let Some((_daemon, address)) = crate::platform::testing::private_bus("files-none")
            else {
                eprintln!("no dbus-daemon: skipped");
                return;
            };
            let choosers = Choosers {
                bus: Some(address),
                portal: true,
                ..nothing()
            };
            assert_eq!(choose(&choosers, &image(Mode::Open)), Choice::InApp);
        }
        assert_eq!(choose(&nothing(), &image(Mode::Open)), Choice::InApp);
        // A program that ran and found nothing chosen: cancelled, no field.
        let cancelled = Choosers {
            zenity: "false".into(),
            ..nothing()
        };
        assert_eq!(choose(&cancelled, &image(Mode::Open)), Choice::Cancelled);
        // One that failed otherwise (here ls on zenity's options, exit 2):
        // the next chooser, and at the end the window.
        let broken = Choosers {
            zenity: "ls".into(),
            ..nothing()
        };
        assert_eq!(choose(&broken, &image(Mode::Open)), Choice::InApp);
        let chosen = Choosers {
            zenity: "echo".into(),
            ..nothing()
        };
        assert!(matches!(
            choose(&chosen, &image(Mode::Open)),
            Choice::Chosen(_)
        ));
    }

    /// A program that prints `lines` and exits 0, as a chooser does.
    #[cfg(unix)]
    fn chooser_printing(dir: &Path, lines: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        let program = dir.join("chooser");
        std::fs::write(&program, format!("#!/bin/sh\nprintf '{lines}'\n")).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        program.to_string_lossy().into_owned()
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "gelabber-files-{name}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// "Datei anhängen" takes every file the chooser names.
    #[cfg(unix)]
    #[test]
    fn many_files_come_back_from_the_chooser() {
        let dir = temp("many");
        let chosen = Choosers {
            zenity: chooser_printing(&dir, "/tmp/a.txt\\n/tmp/b c.png\\n"),
            ..nothing()
        };
        assert_eq!(
            choose(
                &chosen,
                &Request::new("Datei anhängen", &[], Mode::OpenMany)
            ),
            Choice::Chosen(vec!["/tmp/a.txt".into(), "/tmp/b c.png".into()])
        );
        // One file asked for: the first only.
        assert_eq!(
            choose(&chosen, &image(Mode::Open)),
            Choice::Chosen(vec!["/tmp/a.txt".into()])
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn saved(ctx: &egui::Context) -> Vec<(String, bool)> {
        let start = std::time::Instant::now();
        loop {
            let notices = take_notices(ctx);
            if !notices.is_empty() {
                return notices;
            }
            assert!(start.elapsed().as_secs() < 10, "no notice");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// Without a dialog the file goes to Downloads under a name not taken,
    /// and a toast says where; one that cannot be written says why.
    #[test]
    fn a_save_without_a_dialog_lands_in_downloads_and_says_so() {
        let dir = temp("save");
        std::fs::write(dir.join("theme.json"), b"old").unwrap();
        let ctx = egui::Context::default();
        set_choosers(
            &ctx,
            Choosers {
                downloads: Some(dir.clone()),
                ..nothing()
            },
        );
        save(&ctx, "Theme exportieren", "theme.json", b"new".to_vec());
        let target = dir.join("theme (2).json");
        assert_eq!(
            saved(&ctx),
            vec![(format!("Gespeichert unter {}.", target.display()), false)]
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert_eq!(std::fs::read(dir.join("theme.json")).unwrap(), b"old");
        // Written beside it and renamed: nothing else is left.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        set_choosers(
            &ctx,
            Choosers {
                downloads: Some(dir.join("fehlt")),
                ..nothing()
            },
        );
        save(&ctx, "Theme exportieren", "theme.json", b"new".to_vec());
        let notices = saved(&ctx);
        assert!(notices[0].1, "an error");
        assert!(
            notices[0].0.starts_with("Datei nicht gespeichert."),
            "{notices:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A suggested name is only a name: a path in it stays out of the folder.
    #[test]
    fn a_save_never_leaves_the_folder_by_its_name() {
        let dir = temp("save-name");
        let inner = dir.join("downloads");
        std::fs::create_dir_all(&inner).unwrap();
        let ctx = egui::Context::default();
        set_choosers(
            &ctx,
            Choosers {
                downloads: Some(inner.clone()),
                ..nothing()
            },
        );
        save(&ctx, "Export", "../evil.json", b"x".to_vec());
        let target = inner.join("evil.json");
        assert_eq!(
            saved(&ctx),
            vec![(format!("Gespeichert unter {}.", target.display()), false)]
        );
        assert!(!dir.join("evil.json").exists());
        for bad in ["..", "", "/"] {
            save(&ctx, "Export", bad, b"x".to_vec());
            let notices = saved(&ctx);
            assert!(notices[0].1, "{bad:?}: {notices:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The chooser's place is where the file goes.
    #[cfg(unix)]
    #[test]
    fn a_save_goes_where_the_dialog_says() {
        let dir = temp("save-dialog");
        let target = dir.join("diagnose.json");
        let ctx = egui::Context::default();
        set_choosers(
            &ctx,
            Choosers {
                zenity: chooser_printing(&dir, &format!("{}\\n", target.display())),
                downloads: Some(dir.join("nicht hier")),
                ..nothing()
            },
        );
        save(&ctx, "Diagnose speichern", "diagnose.json", b"{}".to_vec());
        assert_eq!(
            saved(&ctx),
            vec![(format!("Gespeichert unter {}.", target.display()), false)]
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"{}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The portal comes first and its answer is the file.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_portal_answers_first() {
        use std::collections::HashMap;
        use zbus::zvariant::{OwnedValue, Value};
        struct Fake;
        #[zbus::interface(name = "org.freedesktop.portal.FileChooser")]
        impl Fake {
            async fn open_file(
                &self,
                #[zbus(header)] header: zbus::message::Header<'_>,
                #[zbus(connection)] conn: &zbus::Connection,
                _parent: String,
                _title: String,
                options: HashMap<String, OwnedValue>,
            ) -> zbus::zvariant::OwnedObjectPath {
                let sender = header.sender().unwrap().to_string();
                let token = String::try_from(options["handle_token"].try_clone().unwrap()).unwrap();
                let path = format!(
                    "/org/freedesktop/portal/desktop/request/{}/{token}",
                    sender.trim_start_matches(':').replace('.', "_")
                );
                let results =
                    HashMap::from([("uris", Value::from(vec!["file:///tmp/gelabber%20bild.png"]))]);
                conn.emit_signal(
                    Some(sender.as_str()),
                    path.as_str(),
                    "org.freedesktop.portal.Request",
                    "Response",
                    &(0u32, results),
                )
                .await
                .unwrap();
                zbus::zvariant::ObjectPath::try_from(path).unwrap().into()
            }
        }
        let Some((_daemon, address)) = crate::platform::testing::private_bus("files-portal") else {
            eprintln!("no dbus-daemon: skipped");
            return;
        };
        let _service = zbus::blocking::connection::Builder::address(address.as_str())
            .unwrap()
            .name("org.freedesktop.portal.Desktop")
            .unwrap()
            .serve_at("/org/freedesktop/portal/desktop", Fake)
            .unwrap()
            .build()
            .unwrap();
        let choosers = Choosers {
            bus: Some(address),
            portal: true,
            ..nothing()
        };
        assert_eq!(
            choose(&choosers, &image(Mode::Open)),
            Choice::Chosen(vec!["/tmp/gelabber bild.png".into()])
        );
    }

    /// The in-app field: it appears, takes a path and fills the slot.
    #[test]
    fn the_window_asks_for_a_path() {
        use egui_kittest::kittest::{NodeT, Queryable};
        let file = std::env::temp_dir().join(format!("gelabber-pick-{}.png", std::process::id()));
        std::fs::write(&file, b"png").unwrap();
        let slot: Picked = Arc::default();
        let shared = slot.clone();
        let mut asked = false;
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .with_context_setup(|ctx| {
                crate::fonts::install(ctx);
                set_choosers(ctx, nothing());
            })
            .build_ui(move |ui| {
                if !asked {
                    asked = true;
                    pick(ui.ctx(), "Bild hochladen", &["*.png"], shared.clone());
                }
                show_in_app(ui.ctx());
            });
        let start = std::time::Instant::now();
        while harness.query_by_label("Dateipfad").is_none() {
            assert!(start.elapsed().as_secs() < 10, "no path field");
            harness.step();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let field = harness
            .query_all_by_label("Dateipfad")
            .find(|n| n.accesskit_node().role() == egui::accesskit::Role::TextInput)
            .unwrap();
        field.focus();
        harness.step();
        harness
            .query_all_by_label("Dateipfad")
            .find(|n| n.accesskit_node().role() == egui::accesskit::Role::TextInput)
            .unwrap()
            .type_text(file.to_str().unwrap());
        harness.step();
        harness.get_by_label("Öffnen").click();
        harness.run_steps(3);
        let picked = slot.lock().clone();
        let _ = std::fs::remove_file(&file);
        let (bytes, name) = picked.flatten().expect("the file");
        assert_eq!(bytes, b"png");
        assert!(name.starts_with("gelabber-pick-"));
        assert!(harness.query_by_label("Dateipfad").is_none());
    }
}

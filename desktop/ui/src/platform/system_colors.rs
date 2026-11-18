//! Linux: the desktop theme's colours for "System", beyond the portal's
//! light or dark (scheme.rs). Omarchy's current theme first (its
//! `colors.toml`), else the KDE colour scheme (`kdeglobals`) in a Plasma
//! session. Read at once, then again whenever one of the files changes.
//!
//! Omarchy (`bin/omarchy-theme-set`, `docs/theming.md`) moves the staged
//! theme to `~/.local/state/omarchy/current/theme/` and only then writes
//! the slug to `.../current/theme.name`, so a changed `theme.name` means a
//! complete `colors.toml`. Its keys and fallbacks follow
//! `bin/omarchy-theme-color`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use egui::Color32;

use crate::theme::{self, BaseColors, Definition, DesktopPalette, Mode, Style};

/// Theme files are a few hundred bytes; anything far larger is not one.
const MAX_BYTES: u64 = 64 * 1024;
/// How often the files are looked at: a stat each, cheap enough to keep
/// theme switches feeling instant without an inotify dependency.
const POLL: Duration = Duration::from_secs(1);

/// Where the desktop keeps its theme files.
#[derive(Clone, Debug)]
pub struct Places {
    home: PathBuf,
    config: PathBuf,
    /// A Plasma session (`XDG_CURRENT_DESKTOP` names KDE): only then does
    /// `kdeglobals` speak for the desktop, not for a few Qt apps.
    kde: bool,
}

impl Places {
    /// The session's: `$HOME`, `$XDG_CONFIG_HOME`, `$XDG_CURRENT_DESKTOP`.
    pub fn from_env() -> Option<Self> {
        let home = PathBuf::from(std::env::var_os("HOME")?);
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".config"));
        let kde = std::env::var("XDG_CURRENT_DESKTOP")
            .is_ok_and(|d| d.split(':').any(|part| part.eq_ignore_ascii_case("KDE")));
        Some(Self { home, config, kde })
    }

    fn omarchy_state(&self) -> PathBuf {
        self.home.join(".local/state/omarchy/current")
    }

    /// Before the state directory: a symlink to the theme's directory.
    fn omarchy_legacy(&self) -> PathBuf {
        self.config.join("omarchy/current/theme")
    }

    fn kdeglobals(&self) -> PathBuf {
        self.config.join("kdeglobals")
    }

    /// The files whose change means another palette.
    fn watched(&self) -> [PathBuf; 4] {
        let state = self.omarchy_state();
        [
            state.join("theme.name"),
            state.join("theme/colors.toml"),
            self.omarchy_legacy().join("colors.toml"),
            self.kdeglobals(),
        ]
    }
}

/// Calls `palette` on a thread of its own: at once, then on every change.
pub fn watch(places: Places, palette: impl Fn(Option<DesktopPalette>) + Send + 'static) {
    let spawned = std::thread::Builder::new()
        .name("gelabber-system-colors".into())
        .spawn(move || {
            let mut seen = None;
            loop {
                let stamp = stamps(&places.watched());
                if seen.as_ref() != Some(&stamp) {
                    seen = Some(stamp);
                    palette(read(&places));
                }
                std::thread::sleep(POLL);
            }
        });
    if let Err(e) = spawned {
        tracing::info!("system colours: {e}");
    }
}

/// Each file's modification time and length, `None` where it is missing.
fn stamps(files: &[PathBuf]) -> Vec<Option<(SystemTime, u64)>> {
    files
        .iter()
        .map(|f| {
            std::fs::metadata(f)
                .ok()
                .and_then(|m| Some((m.modified().ok()?, m.len())))
        })
        .collect()
}

/// The desktop's palette now: Omarchy's, else KDE's in Plasma.
pub fn read(places: &Places) -> Option<DesktopPalette> {
    omarchy(places).or_else(|| {
        places
            .kde
            .then(|| read_small(&places.kdeglobals()))
            .flatten()
            .and_then(|text| kde(&text))
            .map(DesktopPalette::Kde)
    })
}

fn read_small(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

// ---------------------------------------------------------------- Omarchy

fn omarchy(places: &Places) -> Option<DesktopPalette> {
    let state = places.omarchy_state();
    let (slug, dir) = match read_small(&state.join("theme.name")) {
        Some(name) => (name.trim().to_owned(), state.join("theme")),
        None => {
            // Older Omarchy: the symlink's target is named by the slug.
            let link = places.omarchy_legacy();
            let target = std::fs::canonicalize(&link).ok()?;
            let slug = target.file_name()?.to_str()?.to_owned();
            (slug, link)
        }
    };
    if slug.is_empty() {
        return None;
    }
    let text = read_small(&dir.join("colors.toml"))?;
    let light_marker = dir.join("light.mode").exists();
    let colors = omarchy_definition(&slug, &text, light_marker)?;
    Some(DesktopPalette::Omarchy { slug, colors })
}

/// `colors.toml`'s top-level `key = "value"` pairs; the file is flat.
fn toml_pairs(text: &str) -> HashMap<String, String> {
    let mut pairs = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        let quoted = ['"', '\''].into_iter().find_map(|q| {
            let rest = value.strip_prefix(q)?;
            rest.split(q).next()
        });
        let value = match quoted {
            Some(inner) => inner,
            None => value.split('#').next().unwrap_or_default().trim(),
        };
        pairs.insert(key.trim().to_owned(), value.to_owned());
    }
    pairs
}

/// `#rrggbb` (or `rrggbb`, or `0xrrggbb` as some themes write it).
fn hex_color(value: &str) -> Option<Color32> {
    let digits = value
        .strip_prefix('#')
        .or_else(|| value.strip_prefix("0x"))
        .unwrap_or(value);
    if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let rgb = u32::from_str_radix(digits, 16).ok()?;
    Some(theme::hex(rgb))
}

/// "tokyo-night" as `omarchy-theme-current` shows it: "Tokyo Night".
fn display_name(slug: &str) -> String {
    slug.split('-')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A theme from `colors.toml`: the mapping Gelabber's adapted Omarchy
/// presets use, with `omarchy-theme-color`'s fallbacks. `None` without a
/// readable background and foreground.
fn omarchy_definition(slug: &str, text: &str, light_marker: bool) -> Option<Definition> {
    let pairs = toml_pairs(text);
    // Canonical names first, then the legacy short ones.
    let get = |keys: &[&str]| {
        keys.iter()
            .find_map(|k| pairs.get(*k).and_then(|v| hex_color(v)))
    };
    let background = get(&["background", "bg"])?;
    let foreground = get(&["foreground", "fg"])?;
    let colors = BaseColors {
        background,
        panel: get(&["dark_background", "dark_bg"])
            .unwrap_or_else(|| theme::mix(background, Color32::BLACK, 0.25)),
        surface: get(&["lighter_background", "lighter_bg", "color0"]).unwrap_or(background),
        rail: get(&["darker_background", "darker_bg"])
            .unwrap_or_else(|| theme::mix(background, Color32::BLACK, 0.5)),
        text: foreground,
        muted: get(&["light_foreground", "light_fg", "color7"]).unwrap_or(foreground),
        accent: get(&["accent", "color4", "blue"]).unwrap_or(foreground),
        border: get(&["selection", "selection_background", "color8", "color0"])
            .unwrap_or(background),
    };
    let mode = match pairs
        .get("mode")
        .or_else(|| pairs.get("theme_type"))
        .map(String::as_str)
    {
        Some("light") => Mode::Light,
        Some("dark") => Mode::Dark,
        _ if light_marker => Mode::Light,
        _ if theme::is_dark_tone(background) => Mode::Dark,
        _ => Mode::Light,
    };
    Some(Definition {
        id: "system".into(),
        name: display_name(slug),
        mode,
        style: Style::Clear,
        colors,
    })
}

// -------------------------------------------------------------------- KDE

/// `kdeglobals`' `[group] key=value` entries.
fn ini(text: &str) -> HashMap<(String, String), String> {
    let mut entries = HashMap::new();
    let mut group = String::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            group = name.to_owned();
        } else if let Some((key, value)) = line.split_once('=')
            && !line.starts_with('#')
        {
            entries.insert(
                (group.clone(), key.trim().to_owned()),
                value.trim().to_owned(),
            );
        }
    }
    entries
}

/// KDE's `r,g,b` (an alpha after it is ignored).
fn rgb(value: &str) -> Option<Color32> {
    let mut parts = value.split(',').map(|p| p.trim().parse::<u8>());
    let (r, g, b) = (
        parts.next()?.ok()?,
        parts.next()?.ok()?,
        parts.next()?.ok()?,
    );
    Some(Color32::from_rgb(r, g, b))
}

/// A theme from the KDE colour scheme: content on `View`, the chrome
/// around it on `Window`, the accent from `[General] AccentColor` or the
/// selection. `None` without view colours.
fn kde(text: &str) -> Option<Definition> {
    let entries = ini(text);
    let get = |group: &str, key: &str| {
        entries
            .get(&(format!("Colors:{group}"), key.to_owned()))
            .and_then(|v| rgb(v))
    };
    let background = get("View", "BackgroundNormal")?;
    let text_color = get("View", "ForegroundNormal")?;
    let panel = get("Window", "BackgroundNormal").unwrap_or(background);
    let colors = BaseColors {
        background,
        panel,
        surface: get("Button", "BackgroundNormal").unwrap_or(panel),
        rail: get("Header", "BackgroundNormal")
            .unwrap_or_else(|| theme::mix(panel, Color32::BLACK, 0.25)),
        text: text_color,
        muted: get("View", "ForegroundInactive")
            .unwrap_or_else(|| theme::mix(text_color, background, 0.35)),
        accent: entries
            .get(&("General".to_owned(), "AccentColor".to_owned()))
            .and_then(|v| rgb(v))
            .or_else(|| get("Selection", "BackgroundNormal"))
            .unwrap_or(text_color),
        border: theme::mix(background, text_color, 0.15),
    };
    Some(Definition {
        id: "system".into(),
        name: "KDE".into(), // i18n-ok
        mode: if theme::is_dark_tone(background) {
            Mode::Dark
        } else {
            Mode::Light
        },
        style: Style::Clear,
        colors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::hex;

    const TOKYO_NIGHT: &str = r##"
mode = "dark"

accent = "#7aa2f7"
selection = "#292e42"
muted = "#414868"

background = "#1a1b26"
dark_background = "#13141c"
darker_background = "#0e0e14"
lighter_background = "#24283b"

foreground = "#a9b1d6"
dark_foreground = "#565f89"
light_foreground = "#b4bee6"
bright_foreground = "#c0caf5"
"##;

    struct Home(PathBuf);

    impl Home {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "gelabber-system-colors-{name}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn places(&self, kde: bool) -> Places {
            Places {
                home: self.0.clone(),
                config: self.0.join(".config"),
                kde,
            }
        }

        fn write(&self, path: &str, text: &str) {
            let path = self.0.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
    }

    impl Drop for Home {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The mapping of the adapted presets: Tokyo Night's colours.toml
    /// gives the eight colours of Gelabber's "Tokyo Night".
    #[test]
    fn colors_toml_maps_like_the_presets() {
        let d = omarchy_definition("tokyo-night", TOKYO_NIGHT, false).unwrap();
        assert_eq!(d.name, "Tokyo Night");
        assert_eq!(d.mode, Mode::Dark);
        let c = d.colors;
        assert_eq!(
            [
                c.background,
                c.panel,
                c.surface,
                c.rail,
                c.text,
                c.muted,
                c.accent,
                c.border
            ],
            [
                hex(0x1a1b26),
                hex(0x13141c),
                hex(0x24283b),
                hex(0x0e0e14),
                hex(0xa9b1d6),
                hex(0xb4bee6),
                hex(0x7aa2f7),
                hex(0x292e42),
            ]
        );
    }

    /// Legacy names, missing keys and the light marker, as
    /// omarchy-theme-color resolves them.
    #[test]
    fn colors_toml_fallbacks() {
        let legacy = "bg = '#ffffff'\nfg = \"#111111\" # comment\ncolor4 = 0x2050c0\n";
        let d = omarchy_definition("paper", legacy, false).unwrap();
        assert_eq!(d.colors.background, hex(0xffffff));
        assert_eq!(d.colors.text, hex(0x111111));
        assert_eq!(d.colors.accent, hex(0x2050c0));
        assert_eq!(
            d.colors.panel,
            theme::mix(hex(0xffffff), Color32::BLACK, 0.25)
        );
        assert_eq!(
            d.colors.rail,
            theme::mix(hex(0xffffff), Color32::BLACK, 0.5)
        );
        assert_eq!(d.colors.surface, hex(0xffffff));
        assert_eq!(d.colors.muted, hex(0x111111));
        assert_eq!(d.mode, Mode::Light, "a light background without a mode");
        let dark_bg = "background = \"#202020\"\nforeground = \"#eeeeee\"\n";
        assert_eq!(
            omarchy_definition("x", dark_bg, false).unwrap().mode,
            Mode::Dark
        );
        assert_eq!(
            omarchy_definition("x", dark_bg, true).unwrap().mode,
            Mode::Light
        );
        let typed = "theme_type = \"light\"\nbackground = \"#202020\"\nforeground = \"#eeeeee\"\n";
        assert_eq!(
            omarchy_definition("x", typed, false).unwrap().mode,
            Mode::Light
        );
        assert!(
            omarchy_definition(
                "x",
                "background = \"#zzzzzz\"\nforeground = \"#fff\"",
                false
            )
            .is_none()
        );
        assert!(omarchy_definition("x", "", false).is_none());
    }

    #[test]
    fn breeze_dark_from_kdeglobals() {
        let text = "[General]\nColorScheme=BreezeDark\nAccentColor=61,174,233\n\n[Colors:View]\nBackgroundNormal=20,22,24\nForegroundNormal=252,252,252\nForegroundInactive=161,169,177\n\n[Colors:Window]\nBackgroundNormal=32,35,38\n\n[Colors:Button]\nBackgroundNormal=41,44,48\n\n[Colors:Header]\nBackgroundNormal=41,44,48\n\n[Colors:Selection]\nBackgroundNormal=61,174,233\n";
        let d = kde(text).unwrap();
        assert_eq!(d.mode, Mode::Dark);
        assert_eq!(d.colors.background, Color32::from_rgb(20, 22, 24));
        assert_eq!(d.colors.panel, Color32::from_rgb(32, 35, 38));
        assert_eq!(d.colors.surface, Color32::from_rgb(41, 44, 48));
        assert_eq!(d.colors.rail, Color32::from_rgb(41, 44, 48));
        assert_eq!(d.colors.text, Color32::from_rgb(252, 252, 252));
        assert_eq!(d.colors.muted, Color32::from_rgb(161, 169, 177));
        assert_eq!(d.colors.accent, Color32::from_rgb(61, 174, 233));
    }

    #[test]
    fn breeze_light_and_broken_kdeglobals() {
        let text = "[Colors:View]\nBackgroundNormal=255,255,255\nForegroundNormal=35,38,41\n[Colors:Selection]\nBackgroundNormal=61,174,233,255\n";
        let d = kde(text).unwrap();
        assert_eq!(d.mode, Mode::Light);
        assert_eq!(d.colors.panel, Color32::WHITE);
        assert_eq!(d.colors.accent, Color32::from_rgb(61, 174, 233));
        assert!(kde("[Colors:View]\nBackgroundNormal=1,2\nForegroundNormal=3,4,5\n").is_none());
        assert!(kde("[General]\nName=Nothing\n").is_none());
    }

    /// Omarchy first, KDE only in Plasma, nothing elsewhere; the legacy
    /// symlink where the state directory is missing.
    #[test]
    fn sources_in_order() {
        let home = Home::new("order");
        assert_eq!(read(&home.places(true)), None);
        home.write(
            ".config/kdeglobals",
            "[Colors:View]\nBackgroundNormal=255,255,255\nForegroundNormal=35,38,41\n",
        );
        assert!(matches!(
            read(&home.places(true)),
            Some(DesktopPalette::Kde(_))
        ));
        assert_eq!(read(&home.places(false)), None);

        // Older Omarchy: ~/.config/omarchy/current/theme -> themes/nord.
        home.write(".config/omarchy/themes/nord/colors.toml", TOKYO_NIGHT);
        std::fs::create_dir_all(home.0.join(".config/omarchy/current")).unwrap();
        std::os::unix::fs::symlink(
            home.0.join(".config/omarchy/themes/nord"),
            home.0.join(".config/omarchy/current/theme"),
        )
        .unwrap();
        assert!(matches!(
            read(&home.places(true)),
            Some(DesktopPalette::Omarchy { slug, .. }) if slug == "nord"
        ));

        home.write(
            ".local/state/omarchy/current/theme/colors.toml",
            TOKYO_NIGHT,
        );
        home.write(".local/state/omarchy/current/theme.name", "tokyo-night\n");
        let Some(DesktopPalette::Omarchy { slug, colors }) = read(&home.places(true)) else {
            panic!("Omarchy's palette");
        };
        assert_eq!(slug, "tokyo-night");
        assert_eq!(colors.colors.accent, hex(0x7aa2f7));

        // A broken colors.toml: the next source.
        home.write(".local/state/omarchy/current/theme/colors.toml", "nonsense");
        assert!(matches!(
            read(&home.places(true)),
            Some(DesktopPalette::Kde(_))
        ));
    }

    /// A theme switch reaches the callback without a restart.
    #[test]
    fn a_theme_switch_is_followed() {
        let home = Home::new("switch");
        home.write(
            ".local/state/omarchy/current/theme/colors.toml",
            TOKYO_NIGHT,
        );
        home.write(".local/state/omarchy/current/theme.name", "tokyo-night\n");
        let (tx, rx) = std::sync::mpsc::channel();
        watch(home.places(false), move |p| {
            let _ = tx.send(p);
        });
        let wait = Duration::from_secs(10);
        let slug = |p: Option<DesktopPalette>| match p {
            Some(DesktopPalette::Omarchy { slug, .. }) => slug,
            other => panic!("{other:?}"),
        };
        assert_eq!(slug(rx.recv_timeout(wait).unwrap()), "tokyo-night");
        home.write(
            ".local/state/omarchy/current/theme/colors.toml",
            "mode = \"light\"\nbackground = \"#eff1f5\"\nforeground = \"#4c4f69\"\n",
        );
        home.write(
            ".local/state/omarchy/current/theme.name",
            "catppuccin-latte\n",
        );
        // Two writes may land in two polls: the last answer counts.
        let mut last = slug(rx.recv_timeout(wait).unwrap());
        while let Ok(next) = rx.recv_timeout(Duration::from_millis(1500)) {
            last = slug(next);
        }
        assert_eq!(last, "catppuccin-latte");
    }
}

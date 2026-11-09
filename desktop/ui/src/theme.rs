//! The design tokens of web/src (the `--lr-*` variables of
//! theme/model.ts `themeVariables` plus the fixed ones in index.css), derived
//! the same way from a theme's eight colours. The active [`Theme`] lives in
//! the egui context; [`Theme::of`] reads it.

use std::sync::Arc;

use egui::{Color32, CornerRadius, Shadow};

/// `mode` of a theme definition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Dark,
    Light,
}

/// `style` of a theme definition: radii and the font family follow it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Clear,
    Soft,
    Terminal,
}

/// The eight editable colours (theme/model.ts `ThemeColors`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BaseColors {
    pub background: Color32,
    pub panel: Color32,
    pub surface: Color32,
    pub rail: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub border: Color32,
}

/// A theme as stored and shared (`ThemeDefinition`).
#[derive(Clone, Debug, PartialEq)]
pub struct Definition {
    pub id: String,
    pub name: String,
    pub mode: Mode,
    pub style: Style,
    pub colors: BaseColors,
}

impl Definition {
    /// "Gelabber Dunkel", the default (theme/defaults.json).
    pub fn dark() -> Self {
        Self {
            id: "dark".into(),
            name: "Gelabber Dunkel".into(),
            mode: Mode::Dark,
            style: Style::Clear,
            colors: BaseColors {
                background: hex(0x17191a),
                panel: hex(0x1d2021),
                surface: hex(0x272b2d),
                rail: hex(0x131617),
                text: hex(0xeef0ee),
                muted: hex(0xa4abb1),
                accent: hex(0xf3b752),
                border: hex(0x2c3032),
            },
        }
    }

    /// "Gelabber Hell".
    pub fn light() -> Self {
        Self {
            id: "light".into(),
            name: "Gelabber Hell".into(),
            mode: Mode::Light,
            style: Style::Clear,
            colors: BaseColors {
                background: hex(0xf8f5f0),
                panel: hex(0xf4f0e9),
                surface: hex(0xece7df),
                rail: hex(0xeee9e1),
                text: hex(0x252829),
                muted: hex(0x5c646b),
                accent: hex(0xe8a638),
                border: hex(0xdcd6cc),
            },
        }
    }
}

/// Every `--lr-*` colour variable, resolved.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Colors {
    pub bg: Color32,
    pub panel: Color32,
    pub surface: Color32,
    pub rail: Color32,
    pub elevated: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub subtle: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub accent: Color32,
    pub accent_ink: Color32,
    pub accent_text: Color32,
    pub focus: Color32,
    pub hover: Color32,
    pub selected: Color32,
    pub online: Color32,
    pub online_text: Color32,
    pub warning: Color32,
    pub danger: Color32,
    pub danger_bg: Color32,
    pub on_danger: Color32,
    /// `--lr-live`: the on-air coral, a fill.
    pub live: Color32,
    /// `--lr-on-live`: text on [`Colors::live`].
    pub on_live: Color32,
    /// `--lr-live-text`: the coral for text and icons on the surfaces.
    pub live_text: Color32,
    /// Unmultiplied `--lr-backdrop`.
    pub backdrop: Color32,
    pub speaking: Color32,
    /// `--lr-tile-1` … `--lr-tile-8`.
    pub tiles: [Color32; 8],
}

/// `--lr-radius-*` in px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Radii {
    pub xs: f32,
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
    pub xl: f32,
    pub full: f32,
}

/// A CSS radius in px as an egui corner radius.
pub fn corner(r: f32) -> CornerRadius {
    CornerRadius::same(r.round().clamp(0.0, 255.0) as u8)
}

/// The active theme: resolved colours, radii and shadows.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub definition: Definition,
    pub c: Colors,
    pub radius: Radii,
    /// `--lr-shadow-1`: menus, popovers, tooltips.
    pub shadow_1: Shadow,
    /// `--lr-shadow-2`: dialogs, toasts, sticky bars.
    pub shadow_2: Shadow,
    /// `--lr-shadow-card`; none for the clear style.
    pub shadow_card: Shadow,
    /// Primary button hover: accent through `filter: brightness(1.07)`.
    pub primary_hover: Color32,
    /// Danger button hover: `brightness(1.1)`.
    pub danger_hover: Color32,
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}

impl Theme {
    pub fn dark() -> Self {
        Self::from_definition(Definition::dark())
    }

    pub fn light() -> Self {
        Self::from_definition(Definition::light())
    }

    pub fn is_dark(&self) -> bool {
        self.definition.mode == Mode::Dark
    }

    /// `themeVariables(fixContrast(definition))` from theme/model.ts.
    pub fn from_definition(definition: Definition) -> Self {
        let definition = fix_contrast(definition);
        let c = definition.colors;
        let dark = definition.mode == Mode::Dark;
        let elevated = if dark {
            mix(c.panel, c.text, 0.04)
        } else {
            mix(c.background, Color32::WHITE, 0.6)
        };
        let selected = mix(c.panel, c.accent, 0.18);
        let hover = mix(c.panel, c.text, 0.08);
        let online = ensure_contrast(
            if dark { hex(0x85d69b) } else { hex(0x287a45) },
            &[c.background, c.panel, c.surface],
            3.0,
        );
        let accent_text =
            ensure_contrast(c.accent, &[c.background, c.panel, c.surface, selected], 4.5);
        let muted = ensure_contrast(c.muted, &[c.panel, c.surface, hover], 4.5);
        // On air: a warm coral apart from the danger red and the accent; it
        // stands out from the surfaces (3:1) unless that costs its ink 4.5:1.
        let coral = live_apart_from(if dark { hex(0xff7b5c) } else { hex(0xdb5b24) }, c.accent);
        let raised = ensure_contrast(coral, &[c.background, c.panel, c.surface], 3.0);
        let live = if contrast(readable_ink(raised), raised) >= 4.5 {
            raised
        } else {
            coral
        };
        // Hints, timestamps and disabled labels sit on every surface: fields on
        // the page colour, lists and disabled buttons on `surface`, rows on hover.
        let subtle = ensure_contrast(
            mix(muted, c.background, 0.25),
            &[c.background, c.panel, c.surface, elevated, hover],
            4.5,
        );
        let border_strong = ensure_contrast(
            mix(c.border, c.text, 0.35),
            &[c.background, c.panel, c.surface, elevated],
            3.0,
        );
        let tiles = TILE_HUES.map(|hue| {
            let tile = if dark {
                hsl(hue, 0.3, 0.3)
            } else {
                hsl(hue, 0.45, 0.85)
            };
            (0..=100)
                .step_by(5)
                .map(|step| mix(tile, c.background, f64::from(step) / 100.0))
                .find(|next| contrast(c.text, *next) >= 4.5)
                .unwrap_or(c.background)
        });
        let colors = Colors {
            bg: c.background,
            panel: c.panel,
            surface: c.surface,
            rail: c.rail,
            elevated,
            text: c.text,
            muted,
            subtle,
            border: c.border,
            border_strong,
            accent: c.accent,
            accent_ink: readable_ink(c.accent),
            accent_text,
            focus: accent_text,
            hover,
            selected,
            online,
            online_text: ensure_contrast(online, &[c.panel, c.surface], 4.5),
            warning: ensure_contrast(hex(0xd8a657), &[c.background, c.panel, c.surface], 3.0),
            danger: ensure_contrast(
                if dark { hex(0xf39b98) } else { hex(0xa12b29) },
                &[c.background, c.panel, c.surface],
                4.5,
            ),
            danger_bg: if dark { hex(0x8f302d) } else { hex(0xb33b38) },
            on_danger: Color32::WHITE,
            live,
            on_live: readable_ink(live),
            live_text: ensure_contrast(live, &[c.background, c.panel, c.surface], 4.5),
            backdrop: if dark {
                Color32::from_rgba_unmultiplied(9, 12, 13, 184)
            } else {
                Color32::from_rgba_unmultiplied(37, 40, 41, 115)
            },
            speaking: online,
            tiles,
        };
        let [xs, sm, md, lg, xl] = match definition.style {
            Style::Clear => [3.0, 5.0, 7.0, 10.0, 14.0],
            Style::Soft => [6.0, 8.0, 12.0, 16.0, 20.0],
            Style::Terminal => [1.0, 2.0, 2.0, 2.0, 3.0],
        };
        let shadow = |y: i8, blur: u8, color: Color32| Shadow {
            offset: [0, y],
            blur,
            spread: 0,
            color,
        };
        let (shadow_1, shadow_2) = if dark {
            (
                shadow(8, 24, Color32::from_black_alpha(82)),
                shadow(18, 48, Color32::from_black_alpha(128)),
            )
        } else {
            (
                shadow(8, 24, Color32::from_rgba_unmultiplied(60, 45, 20, 31)),
                shadow(18, 48, Color32::from_rgba_unmultiplied(60, 45, 20, 46)),
            )
        };
        let shadow_card = if definition.style == Style::Soft {
            shadow(6, 22, Color32::from_black_alpha(0x12))
        } else {
            Shadow::NONE
        };
        Self {
            c: colors,
            radius: Radii {
                xs,
                sm,
                md,
                lg,
                xl,
                full: 999.0,
            },
            shadow_1,
            shadow_2,
            shadow_card,
            primary_hover: brightness(c.accent, 1.07),
            danger_hover: brightness(colors.danger_bg, 1.1),
            definition,
        }
    }

    /// The active theme of `ctx` (dark until [`set`] installs another).
    pub fn of(ctx: &egui::Context) -> Arc<Theme> {
        ctx.data(|d| d.get_temp::<Arc<Theme>>(egui::Id::NULL))
            .unwrap_or_else(|| {
                let theme = Arc::new(Theme::dark());
                ctx.data_mut(|d| d.insert_temp(egui::Id::NULL, theme.clone()));
                theme
            })
    }

    /// `--lr-tile-N` for a 1-based tone from `initials::tile_tone`/`person_tint`.
    pub fn tile(&self, tone: usize) -> Color32 {
        self.c.tiles[(tone.clamp(1, 8)) - 1]
    }
}

fn os_scheme_id() -> egui::Id {
    egui::Id::unique("gelabber-os-color-scheme")
}

/// The desktop's dark preference where egui gets none from the window
/// system (Linux: the Settings portal, platform/scheme.rs).
pub fn set_os_dark(ctx: &egui::Context, dark: bool) {
    ctx.data_mut(|d| d.insert_temp(os_scheme_id(), dark));
    ctx.request_repaint();
}

/// `prefers-color-scheme: dark`: the desktop's preference, else what egui
/// says, dark when nobody does.
pub fn system_dark(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(os_scheme_id()))
        .unwrap_or_else(|| ctx.system_theme() != Some(egui::Theme::Light))
}

/// Installs the dark default unless a theme is active already (a second
/// app on the same window keeps the one shown).
pub fn ensure(ctx: &egui::Context) {
    if ctx
        .data(|d| d.get_temp::<Arc<Theme>>(egui::Id::NULL))
        .is_none()
    {
        set(ctx, Theme::dark());
    }
}

/// Installs `theme` as the active one and restyles egui with it.
pub fn set(ctx: &egui::Context, theme: Theme) {
    // `--lr-font` follows the style: monospace for terminal themes.
    crate::fonts::set_family(
        ctx,
        if theme.definition.style == Style::Terminal {
            crate::fonts::Family::Terminal
        } else {
            crate::fonts::Family::Inter
        },
    );
    let theme = Arc::new(theme);
    crate::style::apply(ctx, &theme);
    ctx.data_mut(|d| d.insert_temp(egui::Id::NULL, theme));
}

const TILE_HUES: [f64; 8] = [18.0, 42.0, 88.0, 145.0, 188.0, 214.0, 258.0, 322.0];

/// `#rrggbb` as a colour.
pub const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// `color-mix(in srgb, a (1-t), b t)` and model.ts `mix`: a per-channel
/// lerp, rounded.
pub fn mix(a: Color32, b: Color32, t: f64) -> Color32 {
    let ch = |x: u8, y: u8| (f64::from(x) * (1.0 - t) + f64::from(y) * t).round() as u8;
    Color32::from_rgb(ch(a.r(), b.r()), ch(a.g(), b.g()), ch(a.b(), b.b()))
}

/// CSS `filter: brightness(f)` on an opaque colour.
pub fn brightness(c: Color32, f: f64) -> Color32 {
    let ch = |x: u8| (f64::from(x) * f).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(ch(c.r()), ch(c.g()), ch(c.b()))
}

/// The colour a translucent `top` gives over the opaque `under`.
pub fn over(under: Color32, top: Color32, alpha: f64) -> Color32 {
    mix(under, top, alpha)
}

fn luminance(c: Color32) -> f64 {
    let lin = |v: u8| {
        let n = f64::from(v) / 255.0;
        if n <= 0.04045 {
            n / 12.92
        } else {
            ((n + 0.055) / 1.055).powf(2.4)
        }
    };
    lin(c.r()) * 0.2126 + lin(c.g()) * 0.7152 + lin(c.b()) * 0.0722
}

/// WCAG contrast ratio.
pub fn contrast(a: Color32, b: Color32) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

const INK_DARK: Color32 = hex(0x111111);

fn readable_ink(background: Color32) -> Color32 {
    if contrast(Color32::WHITE, background) > contrast(INK_DARK, background) {
        Color32::WHITE
    } else {
        INK_DARK
    }
}

fn ensure_contrast(foreground: Color32, backgrounds: &[Color32], ratio: f64) -> Color32 {
    let passes = |c: Color32| backgrounds.iter().all(|bg| contrast(c, *bg) >= ratio);
    if passes(foreground) {
        return foreground;
    }
    let worst = |target: Color32| {
        backgrounds
            .iter()
            .map(|bg| contrast(target, *bg))
            .fold(f64::INFINITY, f64::min)
    };
    // The target that keeps the larger minimum contrast; white on a tie, as
    // the stable sort in model.ts does.
    let target = if worst(INK_DARK) > worst(Color32::WHITE) {
        INK_DARK
    } else {
        Color32::WHITE
    };
    (1..=100)
        .map(|step| mix(foreground, target, f64::from(step) / 100.0))
        .find(|c| passes(*c))
        .unwrap_or(target)
}

fn hsl(hue: f64, saturation: f64, lightness: f64) -> Color32 {
    let k = |n: f64| (n + hue / 30.0) % 12.0;
    let a = saturation * lightness.min(1.0 - lightness);
    let ch = |n: f64| {
        let v = lightness - a * (k(n) - 3.0).min(9.0 - k(n)).clamp(-1.0, 1.0);
        (v * 255.0).round() as u8
    };
    Color32::from_rgb(ch(0.0), ch(8.0), ch(4.0))
}

/// Hue (degrees), saturation and lightness, as model.ts `toHsl`.
fn to_hsl(c: Color32) -> (f64, f64, f64) {
    let [r, g, b] = [c.r(), c.g(), c.b()].map(|v| f64::from(v) / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let (l, d) = ((max + min) / 2.0, max - min);
    if d == 0.0 {
        return (0.0, 0.0, l);
    }
    let sector = if max == r {
        ((g - b) / d + 6.0) % 6.0
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (sector * 60.0, d / (1.0 - (2.0 * l - 1.0).abs()), l)
}

/// The on-air coral, unless the accent shares its hue (Ristretto's salmon):
/// then Live turns 22° past the accent, away from it (model.ts
/// `liveApartFrom`), so an unread count or a primary button never reads as
/// Live.
fn live_apart_from(coral: Color32, accent: Color32) -> Color32 {
    let (ah, as_, _) = to_hsl(accent);
    let (kh, ks, kl) = to_hsl(coral);
    let apart = (ah - kh + 540.0) % 360.0 - 180.0;
    if as_ < 0.3 || apart.abs() >= 15.0 {
        return coral;
    }
    let turn = if apart > 0.0 { -22.0 } else { 22.0 };
    hsl((ah + turn + 360.0).round() % 360.0, ks, kl)
}

fn fix_contrast(mut definition: Definition) -> Definition {
    let c = &mut definition.colors;
    let backgrounds = [c.background, c.panel, c.surface, c.rail];
    c.text = ensure_contrast(c.text, &backgrounds, 4.5);
    c.muted = ensure_contrast(c.muted, &backgrounds, 4.5);
    let ink = readable_ink(c.accent);
    if contrast(ink, c.accent) < 4.5 {
        let away = if ink == Color32::WHITE {
            Color32::BLACK
        } else {
            Color32::WHITE
        };
        let accent = c.accent;
        for step in 1..=100 {
            c.accent = mix(accent, away, f64::from(step) / 100.0);
            if contrast(readable_ink(c.accent), c.accent) >= 4.5 {
                break;
            }
        }
    }
    definition
}

/// `fixContrast` of theme/model.ts, for the theme editor's "Kontraste
/// verbessern" and the gallery swatches.
pub fn fixed(definition: Definition) -> Definition {
    fix_contrast(definition)
}

/// `contrastIssues` of theme/model.ts: what the theme editor warns about,
/// computed from the colours as entered.
pub fn contrast_issues(definition: &Definition) -> Vec<&'static str> {
    let c = definition.colors;
    let dark = definition.mode == Mode::Dark;
    let elevated = if dark {
        mix(c.panel, c.text, 0.04)
    } else {
        mix(c.background, Color32::WHITE, 0.6)
    };
    let selected = mix(c.panel, c.accent, 0.18);
    let accent_text = ensure_contrast(c.accent, &[c.background, c.panel, c.surface, selected], 4.5);
    let border_strong = ensure_contrast(
        mix(c.border, c.text, 0.35),
        &[c.background, c.panel, c.surface, elevated],
        3.0,
    );
    let backgrounds = [c.background, c.panel, c.surface, c.rail];
    let mut issues = Vec::new();
    if backgrounds.iter().any(|bg| contrast(c.text, *bg) < 4.5) {
        issues.push("Text ist auf mindestens einer Fläche schwer lesbar.");
    }
    if backgrounds.iter().any(|bg| contrast(c.muted, *bg) < 4.5) {
        issues.push("Sekundärtext braucht mehr Kontrast.");
    }
    if contrast(readable_ink(c.accent), c.accent) < 4.5 {
        issues.push("Schrift auf der Akzentfarbe ist schwer lesbar.");
    }
    if [c.background, c.panel, c.surface, selected]
        .iter()
        .any(|bg| contrast(accent_text, *bg) < 4.5)
    {
        issues.push("Links in der Akzentfarbe heben sich zu wenig ab.");
    }
    if [c.background, c.panel, c.surface, elevated]
        .iter()
        .any(|bg| contrast(border_strong, *bg) < 3.0)
    {
        issues.push("Rahmen von Eingabefeldern heben sich zu wenig ab.");
    }
    issues
}

/// `--lr-space-*` (4px grid).
pub mod space {
    pub const S0_5: f32 = 2.0;
    pub const S1: f32 = 4.0;
    pub const S1_5: f32 = 6.0;
    pub const S2: f32 = 8.0;
    pub const S3: f32 = 12.0;
    pub const S4: f32 = 16.0;
    pub const S5: f32 = 20.0;
    pub const S6: f32 = 24.0;
    pub const S8: f32 = 32.0;
    pub const S10: f32 = 40.0;
}

/// `--lr-control-*` heights (desktop: fine pointer).
pub mod control {
    pub const XS: f32 = 28.0;
    pub const SM: f32 = 32.0;
    pub const MD: f32 = 36.0;
    /// Large buttons (`.ui-button--lg`).
    pub const LG: f32 = 44.0;
}

/// Shell geometry from index.css.
pub mod layout {
    pub const RAIL_WIDTH: f32 = 64.0;
    pub const SIDEBAR_WIDTH: f32 = 224.0;
    pub const NAVIGATION_WIDTH: f32 = RAIL_WIDTH + SIDEBAR_WIDTH;
    pub const MEMBERS_WIDTH: f32 = 248.0;
    pub const HEADER_HEIGHT: f32 = 52.0;
    pub const DOCK_HEIGHT: f32 = 56.0;
}

/// `--lr-dur-*` in seconds and the press scale.
pub mod motion {
    pub const MICRO: f32 = 0.100;
    pub const FAST: f32 = 0.120;
    pub const STANDARD: f32 = 0.180;
    pub const LARGE: f32 = 0.240;
    pub const EXIT: f32 = 0.130;
    pub const LARGE_EXIT: f32 = 0.170;
    pub const PRESS_SCALE: f32 = 0.97;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(c: Color32) -> String {
        format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
    }

    // Hints, timestamps and disabled labels read on every surface they sit
    // on, in every preset and in a custom theme with a faint secondary
    // colour (web model.test.ts "meets the contrast targets").
    #[test]
    fn subtle_reads_on_every_surface() {
        let mut all = crate::screens::settings::themes::presets();
        all.push(Definition {
            id: "custom-faint".into(),
            name: "Blass".into(),
            mode: Mode::Dark,
            style: Style::Clear,
            colors: BaseColors {
                background: hex(0x202428),
                panel: hex(0x2a2f34),
                surface: hex(0x353b41),
                rail: hex(0x181b1e),
                text: hex(0xe8e8e8),
                muted: hex(0x50565c),
                accent: hex(0x6aa0e0),
                border: hex(0x3a4046),
            },
        });
        for definition in all {
            let c = Theme::from_definition(definition.clone()).c;
            for bg in [c.bg, c.panel, c.surface, c.elevated, c.hover] {
                let ratio = contrast(c.subtle, bg);
                assert!(
                    ratio >= 4.5,
                    "{}: subtle {} on {} is {ratio:.2}",
                    definition.id,
                    h(c.subtle),
                    h(bg)
                );
            }
        }
    }

    // The values themeVariables() produces for both defaults
    // (spec ui-tokens-shell §1.2).
    #[test]
    fn dark_matches_web() {
        let t = Theme::dark();
        let c = t.c;
        let got = [
            c.bg,
            c.panel,
            c.surface,
            c.rail,
            c.elevated,
            c.text,
            c.muted,
            c.subtle,
            c.border,
            c.border_strong,
            c.accent,
            c.accent_ink,
            c.accent_text,
            c.hover,
            c.selected,
            c.online,
            c.online_text,
            c.warning,
            c.danger,
            c.danger_bg,
            c.live,
            c.on_live,
            c.live_text,
        ]
        .map(h);
        let want = [
            "#17191a", "#1d2021", "#272b2d", "#131617", "#252829", "#eef0ee", "#a4abb1", "#93989b",
            "#2c3032", "#717475", "#f3b752", "#111111", "#f3b752", "#2e3131", "#443b2a", "#85d69b",
            "#85d69b", "#d8a657", "#f39b98", "#8f302d", "#ff7b5c", "#111111", "#ff7b5c",
        ];
        assert_eq!(got, want);
        assert_eq!(
            c.tiles.map(h),
            [
                "#634336", "#635636", "#4e6336", "#366349", "#365d63", "#364963", "#433663",
                "#633653"
            ]
        );
        assert_eq!(h(t.primary_hover), "#ffc458");
        assert_eq!(h(t.danger_hover), "#9d3532");
    }

    // Live on coral-ish custom surfaces, as model.ts derives it: the fill
    // keeps 3:1 to them and its ink 4.5:1, the text colour reads on them.
    #[test]
    fn live_derives_like_web_for_custom_themes() {
        let mut light = Definition::light();
        light.colors.background = hex(0xf08a70);
        light.colors.panel = hex(0xf29a84);
        light.colors.surface = hex(0xe9806a);
        let c = Theme::from_definition(light).c;
        assert_eq!(
            [c.live, c.on_live, c.live_text].map(h),
            ["#823a1c", "#ffffff", "#542917"]
        );
        let mut dark = Definition::dark();
        dark.colors.background = hex(0xffd0c0);
        dark.colors.panel = hex(0xffc4b0);
        dark.colors.surface = hex(0xffb8a0);
        dark.colors.text = hex(0x111111);
        dark.colors.muted = hex(0x444444);
        let c = Theme::from_definition(dark).c;
        assert_eq!(
            [c.live, c.on_live, c.live_text].map(h),
            ["#ac5642", "#ffffff", "#824335"]
        );
    }

    // Ristretto's accent is a salmon: Live turns pinker, past it, as in
    // model.ts. No preset's Live shares the accent's hue.
    #[test]
    fn live_keeps_apart_from_the_accent() {
        let c = Theme::from_definition(crate::screens::settings::themes::preset("ristretto")).c;
        assert_eq!(
            [c.live, c.on_live, c.live_text].map(h),
            ["#ff5c74", "#111111", "#ff667c"]
        );
        let hue_apart = |a: Color32, b: Color32| {
            let d = (to_hsl(a).0 - to_hsl(b).0).abs() % 360.0;
            d.min(360.0 - d)
        };
        for preset in crate::screens::settings::themes::presets() {
            let accent = preset.colors.accent;
            let c = Theme::from_definition(preset).c;
            assert_ne!(c.live, c.danger_bg);
            assert_eq!(c.on_live, INK_DARK);
            if to_hsl(accent).1 >= 0.3 {
                assert!(hue_apart(c.live, accent) > 15.0, "{}", h(c.live));
            }
        }
    }

    #[test]
    fn light_matches_web() {
        let t = Theme::light();
        let c = t.c;
        let got = [
            c.bg,
            c.panel,
            c.surface,
            c.rail,
            c.elevated,
            c.text,
            c.muted,
            c.subtle,
            c.border,
            c.border_strong,
            c.accent,
            c.accent_ink,
            c.accent_text,
            c.hover,
            c.selected,
            c.online,
            c.online_text,
            c.warning,
            c.danger,
            c.danger_bg,
            c.live,
            c.on_live,
            c.live_text,
        ]
        .map(h);
        let want = [
            "#f8f5f0", "#f4f0e9", "#ece7df", "#eee9e1", "#fcfbf9", "#252829", "#5c646b", "#616467",
            "#dcd6cc", "#86837e", "#e8a638", "#111111", "#836026", "#e3e0da", "#f2e3c9", "#287a45",
            "#277643", "#a27e44", "#a12b29", "#b33b38", "#db5b24", "#111111", "#ad4a20",
        ];
        assert_eq!(got, want);
        assert_eq!(
            c.tiles.map(h),
            [
                "#ead2c8", "#eae0c8", "#daeac8", "#c8ead6", "#c8e5ea", "#c8d6ea", "#d2c8ea",
                "#eac8dd"
            ]
        );
    }
}

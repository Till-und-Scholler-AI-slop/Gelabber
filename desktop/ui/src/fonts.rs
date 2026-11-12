//! Inter 4.1 as the web serves it (web/src/fonts/INTER-PROVENANCE.json):
//! the same InterVariable files, with weight and optical size set per text
//! style through variation coordinates, as Chromium does with
//! `font-optical-sizing: auto`. Emoji and other scripts come from the
//! system fonts, like in the browser.
//!
//! Two faces come from the system as the browser resolves them: the
//! monospace font of terminal themes (`--lr-font: ui-monospace, …,
//! monospace` in theme/model.ts) and `system-ui` of the desktop setup page.
//! Those files have no weight axis, so bold is its own file and family.

use std::cell::Cell;
use std::sync::{Arc, OnceLock};

use egui::{FontData, FontDefinitions, FontFamily};

pub const INTER: &str = "Inter";
pub const INTER_ITALIC: &str = "Inter Italic"; // i18n-ok
const BOLD: &str = "Gelabber Bold"; // i18n-ok
const SYSTEM_UI: &str = "System UI"; // i18n-ok
const SYSTEM_UI_BOLD: &str = "System UI Bold"; // i18n-ok

/// Inter's vertical metrics (unitsPerEm 2048: ascender 1984, descender 494).
pub const INTER_METRICS: Metrics = Metrics {
    ascent: 1984.0 / 2048.0,
    descent: 494.0 / 2048.0,
};

/// Ascent and descent in em, as Blink reads them from the font.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub ascent: f32,
    pub descent: f32,
}

/// Which faces the app's text uses: Inter, or the system monospace font of a
/// terminal theme. The text styles read it without a context.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Family {
    #[default]
    Inter,
    Terminal,
}

thread_local! {
    // The UI thread's active family; one per thread, so test harnesses
    // running side by side do not see each other's theme.
    static ACTIVE: Cell<Family> = const { Cell::new(Family::Inter) };
}

/// The family the app's text is in right now.
pub fn active() -> Family {
    ACTIVE.with(Cell::get)
}

/// Metrics of the regular face of the app's text.
pub fn ui_metrics() -> Metrics {
    match active() {
        Family::Terminal => system()
            .and_then(|s| s.mono.as_ref())
            .map_or(INTER_METRICS, |f| f.metrics),
        Family::Inter => INTER_METRICS,
    }
}

/// Metrics of `system-ui` (Inter until the system faces are loaded).
pub fn system_ui_metrics() -> Metrics {
    system()
        .and_then(|s| s.sans.as_ref())
        .map_or(INTER_METRICS, |f| f.metrics)
}

/// The italic face as its own family: egui has no italic switch that picks
/// another file.
pub fn italic_family() -> FontFamily {
    FontFamily::Name(INTER_ITALIC.into())
}

/// The app's text at weight 600 and more: Inter through its weight axis,
/// the bold file of a terminal theme's font.
pub fn bold_family() -> FontFamily {
    FontFamily::Name(BOLD.into())
}

/// `system-ui` as Chromium resolves it, regular or bold.
pub fn system_ui_family(bold: bool) -> FontFamily {
    FontFamily::Name(if bold { SYSTEM_UI_BOLD } else { SYSTEM_UI }.into())
}

/// `ui-monospace` in the browser: the system's monospace font.
pub fn mono_family() -> FontFamily {
    FontFamily::Monospace
}

/// One system font file and its metrics.
struct Face {
    data: Arc<FontData>,
    metrics: Metrics,
}

/// The system faces, looked up once per process (enumerating the system
/// fonts takes a moment, so only when a terminal theme or the setup needs it).
struct SystemFaces {
    sans: Option<Face>,
    sans_bold: Option<Face>,
    mono: Option<Face>,
    mono_bold: Option<Face>,
    mono_italic: Option<Face>,
}

static SYSTEM: OnceLock<Option<SystemFaces>> = OnceLock::new();

fn system() -> Option<&'static SystemFaces> {
    SYSTEM.get().and_then(Option::as_ref)
}

fn load_system() -> &'static Option<SystemFaces> {
    SYSTEM.get_or_init(|| {
        use fontique::{
            Attributes, Collection, CollectionOptions, FontStyle, FontWeight, FontWidth,
            GenericFamily, QueryFamily, QueryStatus, SourceCache,
        };
        let mut collection = Collection::new(CollectionOptions {
            shared: false,
            system_fonts: true,
        });
        let mut cache = SourceCache::default();
        let mut find_styled =
            |generic: GenericFamily, weight: FontWeight, style: FontStyle| -> Option<Face> {
                let mut query = collection.query(&mut cache);
                query.set_families([QueryFamily::Generic(generic)]);
                query.set_attributes(Attributes::new(FontWidth::NORMAL, style, weight));
                let mut found = None;
                query.matches_with(|font| {
                    found = Some((font.blob.clone(), font.index));
                    QueryStatus::Stop
                });
                let (blob, index) = found?;
                let metrics = metrics_of(blob.as_ref(), index)?;
                let (data, _) = blob.into_raw_parts();
                Some(Face {
                    data: Arc::new(FontData::from_blob(data, index)),
                    metrics,
                })
            };
        let mono_italic = find_styled(
            GenericFamily::Monospace,
            FontWeight::NORMAL,
            FontStyle::Italic,
        );
        let mut find = |generic, weight| find_styled(generic, weight, FontStyle::Normal);
        let faces = SystemFaces {
            mono_italic,
            sans: find(GenericFamily::SystemUi, FontWeight::NORMAL)
                .or_else(|| find(GenericFamily::SansSerif, FontWeight::NORMAL)),
            sans_bold: find(GenericFamily::SystemUi, FontWeight::BOLD)
                .or_else(|| find(GenericFamily::SansSerif, FontWeight::BOLD)),
            mono: find(GenericFamily::Monospace, FontWeight::NORMAL),
            mono_bold: find(GenericFamily::Monospace, FontWeight::BOLD),
        };
        Some(faces)
    })
}

/// `hhea` ascender and descender over `head` unitsPerEm, which is what
/// FreeType (and so Chromium on Linux) uses for the line box.
fn metrics_of(data: &[u8], index: u32) -> Option<Metrics> {
    let be16 = |at: usize| -> Option<u16> {
        Some(u16::from_be_bytes(data.get(at..at + 2)?.try_into().ok()?))
    };
    let be32 = |at: usize| -> Option<u32> {
        Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
    };
    // A collection (`ttcf`) lists the offsets of its fonts.
    let start = if data.get(0..4)? == b"ttcf" {
        be32(12 + 4 * index as usize)? as usize
    } else {
        0
    };
    let tables = be16(start + 4)? as usize;
    let table = |tag: &[u8; 4]| -> Option<usize> {
        (0..tables).find_map(|i| {
            let record = start + 12 + 16 * i;
            (data.get(record..record + 4)? == tag).then(|| be32(record + 8).map(|o| o as usize))?
        })
    };
    let head = table(b"head")?;
    let hhea = table(b"hhea")?;
    let upem = f32::from(be16(head + 18)?);
    let ascender = f32::from(be16(hhea + 4)? as i16);
    let descender = f32::from(be16(hhea + 6)? as i16);
    (upem > 0.0).then(|| Metrics {
        ascent: ascender / upem,
        descent: -descender / upem,
    })
}

/// Marks a context's fonts: which family they were built for.
fn built_id() -> egui::Id {
    egui::Id::unique("gelabber-fonts-built")
}

pub fn install(ctx: &egui::Context) {
    // Once per context: the setup screen installs them before the app does.
    if ctx
        .data(|d| d.get_temp::<(Family, bool)>(built_id()))
        .is_some()
    {
        return;
    }
    build(ctx, Family::Inter, false);
    ctx.add_font_provider(Arc::new(
        egui_system_font_provider::SystemFontProvider::default(),
    ));
}

/// Puts the app's text in `family` (theme::set calls it for every theme).
pub fn set_family(ctx: &egui::Context, family: Family) {
    ACTIVE.with(|a| a.set(family));
    let (built, system) = ctx
        .data(|d| d.get_temp::<(Family, bool)>(built_id()))
        .unwrap_or((Family::Inter, false));
    if built != family {
        build(ctx, family, system || family == Family::Terminal);
    }
}

/// Loads `system-ui` for the setup screen.
pub fn use_system_ui(ctx: &egui::Context) {
    let (built, system) = ctx
        .data(|d| d.get_temp::<(Family, bool)>(built_id()))
        .unwrap_or((Family::Inter, false));
    if !system {
        build(ctx, built, true);
    }
}

fn build(ctx: &egui::Context, family: Family, with_system: bool) {
    let faces = if with_system {
        load_system().as_ref()
    } else {
        None
    };
    let mut fonts = FontDefinitions::empty();
    fonts.font_data.insert(
        INTER.into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/InterVariable.ttf"
        ))),
    );
    fonts.font_data.insert(
        INTER_ITALIC.into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/InterVariable-Italic.ttf"
        ))),
    );
    let mut add = |name: &str, face: Option<&Face>| -> Option<String> {
        let face = face?;
        fonts.font_data.insert(name.into(), face.data.clone());
        Some(name.to_owned())
    };
    let sans = add("system-sans", faces.and_then(|f| f.sans.as_ref()));
    let sans_bold = add("system-sans-bold", faces.and_then(|f| f.sans_bold.as_ref()));
    let mono = add("system-mono", faces.and_then(|f| f.mono.as_ref()));
    let mono_bold = add("system-mono-bold", faces.and_then(|f| f.mono_bold.as_ref()));
    let mono_italic = add(
        "system-mono-italic",
        faces.and_then(|f| f.mono_italic.as_ref()),
    );
    let chain = |first: &[&Option<String>], rest: &[&str]| -> Vec<String> {
        first
            .iter()
            .filter_map(|n| (*n).clone())
            .chain(rest.iter().map(|n| (*n).to_owned()))
            .collect()
    };
    let terminal = family == Family::Terminal && mono.is_some();
    let (regular, bold, italic) = if terminal {
        (
            chain(&[&mono], &[INTER]),
            chain(&[&mono_bold, &mono], &[INTER]),
            chain(&[&mono_italic, &mono], &[INTER_ITALIC, INTER]),
        )
    } else {
        (
            vec![INTER.into()],
            vec![INTER.into()],
            vec![INTER_ITALIC.into(), INTER.into()],
        )
    };
    fonts.families.insert(FontFamily::Proportional, regular);
    fonts.families.insert(bold_family(), bold);
    fonts.families.insert(italic_family(), italic);
    fonts
        .families
        .insert(system_ui_family(false), chain(&[&sans], &[INTER]));
    fonts.families.insert(
        system_ui_family(true),
        chain(&[&sans_bold, &sans], &[INTER]),
    );
    // Monospace and everything Inter lacks (emoji, CJK, …) come from the
    // system, as the browser's `ui-monospace` and fallback chain do.
    fonts.families.insert(FontFamily::Monospace, Vec::new());
    ctx.set_fonts(fonts);
    ctx.data_mut(|d| d.insert_temp(built_id(), (family, with_system)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inter_metrics_from_the_file() {
        let data = include_bytes!("../assets/fonts/InterVariable.ttf");
        assert_eq!(metrics_of(data, 0), Some(INTER_METRICS));
    }
}

//! Inter 4.1 as the web serves it (web/src/fonts/INTER-PROVENANCE.json):
//! the same InterVariable files, with weight and optical size set per text
//! style through variation coordinates, as Chromium does with
//! `font-optical-sizing: auto`. Emoji and other scripts come from the
//! system fonts, like in the browser.

use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily};

pub const INTER: &str = "Inter";
pub const INTER_ITALIC: &str = "Inter Italic";

/// The italic face as its own family: egui has no italic switch that picks
/// another file.
pub fn italic_family() -> FontFamily {
    FontFamily::Name(INTER_ITALIC.into())
}

/// `ui-monospace` in the browser: the system's monospace font.
pub fn mono_family() -> FontFamily {
    FontFamily::Monospace
}

pub fn install(ctx: &egui::Context) {
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
    fonts
        .families
        .insert(FontFamily::Proportional, vec![INTER.into()]);
    fonts
        .families
        .insert(italic_family(), vec![INTER_ITALIC.into(), INTER.into()]);
    // Monospace and everything Inter lacks (emoji, CJK, …) come from the
    // system, as the browser's `ui-monospace` and fallback chain do.
    fonts.families.insert(FontFamily::Monospace, Vec::new());
    ctx.set_fonts(fonts);
    ctx.add_font_provider(Arc::new(
        egui_system_font_provider::SystemFontProvider::default(),
    ));
}

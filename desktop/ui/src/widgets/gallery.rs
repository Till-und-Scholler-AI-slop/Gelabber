//! Every field state (idle and in keyboard focus) in four themes, compared
//! with the pictures in tests/snapshots/fields (#561). The web's counterpart
//! is web/scripts/field-gallery.mjs; both draw one frame per field on the
//! `--lr-field-*` tokens.
//!
//! After a deliberate change:
//! `UPDATE_SNAPSHOTS=1 cargo test -p gelabber-ui --lib widgets::gallery`
//! and look at the new pictures before committing them.

use egui_kittest::{Harness, SnapshotOptions, SnapshotResults};

use super::field::{TextField, field_id};
use crate::screens::settings::controls::{self, TextareaExtras};
use crate::theme::{Definition, Style, Theme};

/// Dark and light (clear), and one each of the soft and terminal styles,
/// as the web gallery picks them.
fn themes() -> Vec<Definition> {
    let presets = crate::screens::settings::themes::presets();
    let pick =
        |f: &dyn Fn(&Definition) -> bool| presets.iter().find(|d| f(d)).cloned().expect("a preset");
    vec![
        pick(&|d| d.id == "dark"),
        pick(&|d| d.id == "light"),
        pick(&|d| d.style == Style::Soft),
        pick(&|d| d.style == Style::Terminal),
    ]
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    Input,
    Filled,
    Invalid,
    Disabled,
    Password,
    Select,
    Textarea,
}

impl Kind {
    const ALL: [Kind; 7] = [
        Kind::Input,
        Kind::Filled,
        Kind::Invalid,
        Kind::Disabled,
        Kind::Password,
        Kind::Select,
        Kind::Textarea,
    ];

    fn name(self) -> &'static str {
        match self {
            Kind::Input => "input",
            Kind::Filled => "filled",
            Kind::Invalid => "invalid",
            Kind::Disabled => "disabled",
            Kind::Password => "password",
            Kind::Select => "select",
            Kind::Textarea => "textarea",
        }
    }
}

fn draw(ui: &mut egui::Ui, kind: Kind, text: &mut String, choice: &mut String) {
    let theme = Theme::of(ui.ctx());
    ui.painter()
        .rect_filled(ui.max_rect().expand(16.0), 0.0, theme.c.bg);
    match kind {
        Kind::Input => {
            TextField::new("gallery", text)
                .placeholder("Status")
                .show(ui);
        }
        Kind::Filled => {
            TextField::new("gallery", text).show(ui);
        }
        Kind::Invalid => {
            TextField::new("gallery", text).error(Some("")).show(ui);
        }
        Kind::Disabled => {
            TextField::new("gallery", text).disabled(true).show(ui);
        }
        Kind::Password => {
            TextField::new("gallery", text).password().show(ui);
        }
        Kind::Select => {
            let options = [
                ("a".to_owned(), "Erste".to_owned()),  // i18n-ok: a test
                ("b".to_owned(), "Zweite".to_owned()), // i18n-ok: a test
            ];
            controls::select(ui, "gallery", &options, choice);
        }
        Kind::Textarea => {
            controls::textarea_with(
                ui,
                "gallery",
                "Text",
                text,
                64.0,
                false,
                TextareaExtras::default(),
            );
        }
    }
}

fn shot(results: &mut SnapshotResults, definition: &Definition, kind: Kind, focus: bool) {
    let theme = Theme::from_definition(definition.clone());
    let text = match kind {
        Kind::Input => String::new(),
        Kind::Invalid => "zu lang".to_owned(), // i18n-ok: a test
        Kind::Disabled => "gesperrt".to_owned(), // i18n-ok: a test
        Kind::Password => "geheim".to_owned(), // i18n-ok: a test
        Kind::Textarea => "Eine Nachricht".to_owned(), // i18n-ok: a test
        _ => "Bin im Call".to_owned(),         // i18n-ok: a test
    };
    // A picture test: the fields have no labels to name them.
    let mut h = Harness::builder()
        .with_accessibility_check(false)
        .with_size(egui::vec2(
            276.0,
            if matches!(kind, Kind::Textarea) {
                84.0
            } else {
                53.0
            },
        ))
        .with_context_setup(move |ctx| {
            crate::fonts::install(ctx);
            crate::theme::set(ctx, theme.clone());
        })
        .build_ui_state(
            move |ui, (text, choice): &mut (String, String)| {
                egui::Frame::NONE
                    .inner_margin(8.0)
                    .show(ui, |ui| draw(ui, kind, text, choice));
            },
            (text, "b".to_owned()),
        );
    h.run();
    if focus {
        // Keyboard focus, as Tab gives it: the ring of :focus-visible.
        let id = match kind {
            Kind::Select => None,
            _ => Some(field_id("gallery")),
        };
        super::set_keyboard_focus(&h.ctx, true);
        match id {
            Some(id) => h.ctx.memory_mut(|m| m.request_focus(id)),
            None => {
                h.key_press(egui::Key::Tab);
            }
        }
        h.run();
        h.run();
    }
    let name = format!(
        "fields/{}-{}{}",
        definition.id,
        kind.name(),
        if focus { "-focus" } else { "" }
    );
    h.snapshot_options(&name, &SnapshotOptions::new().max_failed_pixels(4));
    results.extend_harness(&mut h);
}

#[test]
fn every_field_state_in_four_themes() {
    let mut results = SnapshotResults::new();
    for definition in themes() {
        for kind in Kind::ALL {
            shot(&mut results, &definition, kind, false);
            if !matches!(kind, Kind::Disabled) {
                shot(&mut results, &definition, kind, true);
            }
        }
    }
    // Every picture is compared before the test fails on the first.
    results.unwrap();
}

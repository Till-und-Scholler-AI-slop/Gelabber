//! "Bild beschreiben" (#575; components/AltTextDialog.tsx): the alt text of
//! one picture, written before sending (a composer file) or later by its
//! author (a sent picture, saved as an edit of the message).

use egui::{Ui, vec2};
use gelabber_i18n::{chat as texts, common};
use uuid::Uuid;

use crate::text::{Block, TextStyle};
use crate::theme::Theme;

/// The server's limit, in characters.
pub const ALT_TEXT_MAX: usize = 1000;

#[derive(Clone, Debug, PartialEq)]
pub enum AltTarget {
    /// The composer's file at this index.
    Composer(usize),
    /// A picture of my sent message.
    Sent { message: Uuid, attachment: Uuid },
}

#[derive(Clone, Debug, PartialEq)]
pub struct AltDialog {
    pub target: AltTarget,
    pub name: String,
    pub text: String,
    initial: String,
}

impl AltDialog {
    pub fn new(target: AltTarget, name: &str, text: Option<&str>) -> Self {
        let text = text.unwrap_or_default().to_owned();
        Self {
            target,
            name: name.to_owned(),
            initial: text.clone(),
            text,
        }
    }
}

pub enum AltOutcome {
    Open,
    Close,
    /// Save this text; blank clears it.
    Save(String),
}

/// The text as the server keeps it: trimmed, `None` when blank.
pub fn normalised(text: &str) -> Option<String> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

pub fn show(ctx: &egui::Context, dialog: &mut AltDialog) -> AltOutcome {
    let (cancel, save) = (std::cell::Cell::new(false), std::cell::Cell::new(false));
    let dirty = dialog.text.trim() != dialog.initial.trim();
    let label = texts::alt_label(&dialog.name);
    let text = &mut dialog.text;
    let output = crate::widgets::Modal::new("alt-text", texts::alt_title())
        .dirty(dirty)
        .show(
            ctx,
            |ui: &mut Ui| {
                let theme = Theme::of(ui.ctx());
                let width = ui.available_width();
                let style = TextStyle::new(14.0, 600.0, 1.5);
                let title = Block::wrapped(ui, &label, style, theme.c.text, width);
                let (rect, _) =
                    ui.allocate_exact_size(vec2(width, title.height()), egui::Sense::hover());
                title.paint(ui.painter(), rect.min);
                let hint = Block::wrapped(
                    ui,
                    texts::alt_hint(),
                    TextStyle::new(13.0, 400.0, 1.5),
                    theme.c.muted,
                    width,
                );
                let (rect, _) =
                    ui.allocate_exact_size(vec2(width, hint.height()), egui::Sense::hover());
                hint.paint(ui.painter(), rect.min);
                ui.add_space(6.0);
                let response = crate::screens::settings::controls::textarea_with(
                    ui,
                    "alt-text-field",
                    &label,
                    text,
                    110.0,
                    false,
                    crate::screens::settings::controls::TextareaExtras {
                        char_limit: Some(ALT_TEXT_MAX),
                        ..Default::default()
                    },
                );
                if crate::widgets::field::mounted(ui.ctx(), egui::Id::unique("alt-text-open")) {
                    response.request_focus();
                }
                // Ctrl+Enter saves; Enter alone is a line break.
                if response.has_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command)
                {
                    save.set(true);
                }
                let count = Block::single(
                    ui,
                    &texts::alt_count(text.trim().chars().count(), ALT_TEXT_MAX),
                    TextStyle::new(12.0, 400.0, 1.5),
                    theme.c.muted,
                );
                let (rect, _) =
                    ui.allocate_exact_size(vec2(width, count.height()), egui::Sense::hover());
                count.paint(
                    ui.painter(),
                    egui::pos2(rect.right() - count.width(), rect.top()),
                );
            },
            Some(&mut |ui: &mut Ui| {
                crate::screens::shell::overlays::row(ui, 36.0, 8.0, |ui| {
                    if ui
                        .add(crate::widgets::Button::new(common::cancel()).ghost())
                        .clicked()
                    {
                        cancel.set(true);
                    }
                    if ui
                        .add(crate::widgets::Button::new(common::save()).primary())
                        .clicked()
                    {
                        save.set(true);
                    }
                });
            }),
        );
    if save.get() {
        AltOutcome::Save(dialog.text.clone())
    } else if output.close_requested || cancel.get() {
        AltOutcome::Close
    } else {
        AltOutcome::Open
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_is_none_and_line_breaks_are_kept() {
        assert_eq!(normalised("  \n "), None);
        assert_eq!(
            normalised(" Ein Hund\r\nam Strand "),
            Some("Ein Hund\nam Strand".into())
        );
    }

    #[test]
    fn a_send_names_alt_texts_only_when_there_are_some() {
        let id = Uuid::from_u128(5);
        let plain = gelabber_client::NewMessage {
            content: "x".into(),
            attachment_ids: Some(vec![id]),
            ..Default::default()
        };
        let json = serde_json::to_value(&plain).expect("json");
        assert!(json.get("attachment_descriptions").is_none());
        let described = gelabber_client::NewMessage {
            attachment_descriptions: [(id, "Ein Hund".to_owned())].into(),
            ..plain
        };
        let json = serde_json::to_value(&described).expect("json");
        assert_eq!(json["attachment_descriptions"][id.to_string()], "Ein Hund");
    }
}

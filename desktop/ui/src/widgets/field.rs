//! `FormField` + `Input` and `PasswordField` (components/ui/FormField.tsx,
//! Input.tsx, PasswordField.tsx; `.ui-form-field`, `.ui-field`,
//! `.ui-password` in ui.css): label, a 37px field, then the error or the
//! hint underneath.

use egui::text::LayoutJob;
use egui::{
    Color32, Frame, Id, Margin, Rect, Response, Sense, Stroke, StrokeKind, TextBuffer, TextEdit,
    Ui, UiBuilder, Vec2, pos2,
};

use super::{IconButton, focus_ring, snap};
use crate::icons::Icon;
use crate::text::{self, Block};
use crate::theme::{Theme, corner, space};

/// What a field reports after a frame.
pub struct FieldOutput {
    /// The text edit's response (focus, `changed()`, `lost_focus()`).
    pub response: Response,
    /// Enter was pressed in the field: submit the form.
    pub submitted: bool,
    /// The whole field, label to hint.
    pub rect: Rect,
}

/// A labelled single-line input. `id` is the HTML id (`email`,
/// `password`, …): it keys focus, so forms can focus a field by name.
#[must_use]
pub struct TextField<'a> {
    text: &'a mut String,
    id: Id,
    label: Option<&'a str>,
    hint: Option<&'a str>,
    error: Option<&'a str>,
    placeholder: Option<&'a str>,
    password: bool,
    autofocus: bool,
    disabled: bool,
    char_limit: Option<usize>,
}

impl<'a> TextField<'a> {
    pub fn new(id: impl std::hash::Hash + std::fmt::Debug, text: &'a mut String) -> Self {
        Self {
            text,
            id: field_id(id),
            label: None,
            hint: None,
            error: None,
            placeholder: None,
            password: false,
            autofocus: false,
            disabled: false,
            char_limit: None,
        }
    }

    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    /// The muted line under the field, shown while there is no error.
    pub fn hint(mut self, hint: Option<&'a str>) -> Self {
        self.hint = hint;
        self
    }

    /// The error under the field; also turns the border red.
    pub fn error(mut self, error: Option<&'a str>) -> Self {
        self.error = error;
        self
    }

    pub fn placeholder(mut self, placeholder: &'a str) -> Self {
        self.placeholder = Some(placeholder);
        self
    }

    /// A `PasswordField`: masked, with the "Passwort anzeigen" toggle.
    pub fn password(mut self) -> Self {
        self.password = true;
        self
    }

    /// `autoFocus`: takes focus the first time it is shown.
    pub fn autofocus(mut self, autofocus: bool) -> Self {
        self.autofocus = autofocus;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// `maxLength`.
    pub fn char_limit(mut self, limit: usize) -> Self {
        self.char_limit = Some(limit);
        self
    }

    pub fn show(self, ui: &mut Ui) -> FieldOutput {
        let theme = Theme::of(ui.ctx());
        let c = &theme.c;
        let width = ui.available_width();
        let top = ui.cursor().min;
        let mut y = top.y;

        if let Some(label) = self.label {
            let block = Block::single(ui, label, text::UI, c.text);
            let rect =
                Rect::from_min_size(pos2(top.x, y), Vec2::new(block.width(), block.height()));
            block.paint(ui.painter(), rect.min);
            // Clicking the <label> focuses its input.
            let label_response = ui.interact(rect, self.id.with("label"), Sense::click());
            if label_response.clicked() {
                ui.memory_mut(|m| m.request_focus(self.id));
            }
            y += layout_unit(block.height()) + space::S1_5;
        }

        let field_rect = Rect::from_min_size(pos2(top.x, y), Vec2::new(width, 37.0));
        let shown_key = self.id.with("shown");
        let shown = self.password && ui.data(|d| d.get_temp::<bool>(shown_key)).unwrap_or(false);
        let masked = self.password && !shown;

        let autofocus_now = self.autofocus && mounted(ui.ctx(), self.id.with("autofocus"));
        if autofocus_now {
            ui.memory_mut(|m| m.request_focus(self.id));
        }

        // Paint the box first; the text edit draws its text on top.
        let painter = ui.painter().clone();
        let box_rect = snap(ui.ctx(), field_rect);
        let focused = ui.memory(|m| m.has_focus(self.id));
        let border = if self.error.is_some() {
            c.danger
        } else if focused {
            c.focus
        } else {
            c.border_strong
        };
        if focused {
            focus_ring(&painter, box_rect, theme.radius.sm, &theme);
        }
        painter.rect(
            box_rect,
            corner(theme.radius.sm),
            if self.disabled { c.surface } else { c.bg },
            Stroke::new(1.0, border),
            StrokeKind::Inside,
        );

        let right_pad = if self.password { 48.0 } else { 11.0 };
        let color = if self.disabled { c.subtle } else { c.text };
        let mut layouter = move |ui: &Ui, buf: &dyn TextBuffer, _wrap: f32| {
            let shown_text = if masked {
                std::iter::repeat_n('•', buf.as_str().chars().count()).collect()
            } else {
                buf.as_str().to_owned()
            };
            let mut job = LayoutJob::single_section(shown_text, text::BODY.format(color));
            job.keep_trailing_whitespace = true;
            ui.fonts_mut(|f| f.layout_job(job))
        };
        let inner = field_rect.shrink(1.0);
        let mut edit = TextEdit::singleline(self.text)
            .id(self.id)
            .frame(Frame::NONE.inner_margin(Margin {
                left: 11,
                right: right_pad as i8,
                top: 7,
                bottom: 7,
            }))
            .desired_width(inner.width())
            .min_size(inner.size())
            .align(egui::Align2::LEFT_TOP)
            .text_color(color)
            // Besides the bullets: no copying, no plain text for AccessKit or the IME.
            .password(masked)
            .layouter(&mut layouter)
            .interactive(!self.disabled);
        if let Some(limit) = self.char_limit {
            edit = edit.char_limit(limit);
        }
        if let Some(placeholder) = self.placeholder {
            // The typed text's 21px line box, so both sit on one baseline.
            edit = edit.hint_text(LayoutJob::single_section(
                placeholder.to_owned(),
                text::BODY.format(c.subtle),
            ));
        }
        let output = ui
            .scope_builder(UiBuilder::new().max_rect(inner), |ui| edit.show(ui))
            .inner;
        let response = output
            .response
            .response
            .accessible_name(self.label.or(self.placeholder).unwrap_or_default());
        // The click that opened this field (a menu item, a dialog button)
        // ends in this frame and egui surrenders focus on it; take it back.
        if autofocus_now && !response.has_focus() {
            response.request_focus();
        }
        let submitted = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if response.hovered() && !self.disabled {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        }

        if self.password {
            let toggle_rect = Rect::from_min_size(
                pos2(field_rect.right() - 44.0, field_rect.top()),
                Vec2::new(44.0, field_rect.height()),
            );
            let title = if shown {
                "Passwort verbergen"
            } else {
                "Passwort anzeigen"
            };
            let toggle = ui.add(
                IconButton::new(if shown { Icon::EyeOff } else { Icon::Eye }, title)
                    .icon_size(18.0)
                    .pressed(shown)
                    .plain()
                    .at(toggle_rect),
            );
            if toggle.clicked() {
                ui.data_mut(|d| d.insert_temp(shown_key, !shown));
            }
        }

        y = field_rect.bottom();
        let note = self
            .error
            .map(|e| (e, c.danger))
            .or(self.hint.map(|h| (h, c.muted)));
        if let Some((note, color)) = note {
            y += space::S1_5;
            let block = Block::wrapped(ui, note, text::META, color, width);
            block.paint(ui.painter(), pos2(top.x, y));
            y += layout_unit(block.height());
        }
        let rect = Rect::from_min_max(top, pos2(top.x + width, y));
        ui.advance_cursor_after_rect(rect);
        FieldOutput {
            response,
            submitted,
            rect,
        }
    }
}

/// Starts a new mount for every field: the app calls it when the location
/// changes, so `autoFocus` fires again like a remounted React page.
pub fn remount(ctx: &egui::Context) {
    ctx.data_mut(|d| *d.get_temp_mut_or_default::<u64>(Id::unique("gelabber-mount-epoch")) += 1);
}

/// True on the first frame `key` is shown in this mount: after a
/// [`remount`], or when it was not shown on the previous frame (a modal).
pub fn mounted(ctx: &egui::Context, key: Id) -> bool {
    let epoch = ctx.data(|d| d.get_temp::<u64>(Id::unique("gelabber-mount-epoch")));
    let frame = ctx.cumulative_frame_nr();
    let seen = ctx.data(|d| d.get_temp::<(Option<u64>, u64)>(key));
    ctx.data_mut(|d| d.insert_temp(key, (epoch, frame)));
    !seen.is_some_and(|(e, f)| e == epoch && f + 1 >= frame)
}

/// The egui id of the field with HTML id `id`.
pub fn field_id(id: impl std::hash::Hash + std::fmt::Debug) -> Id {
    Id::unique(("gelabber-field", id))
}

/// Blink stores lengths in 1/64 px, rounding line boxes down.
pub fn layout_unit(px: f32) -> f32 {
    (px * 64.0).floor() / 64.0
}

/// The colour of a field's bottom note, for callers drawing their own.
pub fn note_color(theme: &Theme, error: bool) -> Color32 {
    if error { theme.c.danger } else { theme.c.muted }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autofocus_fires_once_per_mount() {
        let ctx = egui::Context::default();
        let key = Id::unique("probe");
        let frame = |show: bool| {
            let mut fired = None;
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                fired = show.then(|| mounted(ui.ctx(), key));
            });
            // No renderer here takes the font atlas upload.
            output.textures_delta.clear();
            fired
        };
        assert_eq!(frame(true), Some(true));
        assert_eq!(frame(true), Some(false));
        // Hidden for a frame (a closed modal): the next showing is a mount.
        assert_eq!(frame(false), None);
        assert_eq!(frame(true), Some(true));
        // Navigating away and back without a gap still remounts.
        remount(&ctx);
        assert_eq!(frame(true), Some(true));
        assert_eq!(frame(true), Some(false));
    }
}

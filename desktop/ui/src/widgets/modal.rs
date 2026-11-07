//! `Modal` (components/Modal.tsx, `.gel-modal` in ui.css): a centred
//! elevated card over the backdrop, title, close button, scrolling body
//! and an optional footer.

use egui::{Id, Order, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2, pos2};

use super::IconButton;
use crate::icons::Icon;
use crate::text::{self, Block};
use crate::theme::{Theme, corner};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ModalWidth {
    /// 448px.
    #[default]
    Default,
    /// 512px.
    Wide,
    /// 640px.
    Large,
}

impl ModalWidth {
    fn px(self) -> f32 {
        match self {
            Self::Default => 448.0,
            Self::Wide => 512.0,
            Self::Large => 640.0,
        }
    }
}

pub struct ModalOutput<R> {
    pub inner: R,
    /// Esc, "Schließen" or a backdrop click (unless `dirty`).
    pub close_requested: bool,
}

#[must_use]
pub struct Modal<'a> {
    id: Id,
    title: &'a str,
    width: ModalWidth,
    dirty: bool,
}

impl<'a> Modal<'a> {
    pub fn new(id: impl std::hash::Hash + std::fmt::Debug, title: &'a str) -> Self {
        Self {
            id: Id::unique(("gelabber-modal", id)),
            title,
            width: ModalWidth::Default,
            dirty: false,
        }
    }

    pub fn width(mut self, width: ModalWidth) -> Self {
        self.width = width;
        self
    }

    /// With unsaved input a backdrop click does not close it.
    pub fn dirty(mut self, dirty: bool) -> Self {
        self.dirty = dirty;
        self
    }

    /// `body` fills the 8/24/24 padded body (column, gap 20 is up to the
    /// caller); `footer`, when given, the right-aligned footer row.
    pub fn show<R>(
        self,
        ctx: &egui::Context,
        body: impl FnOnce(&mut Ui) -> R,
        footer: Option<&mut dyn FnMut(&mut Ui)>,
    ) -> ModalOutput<R> {
        let theme = Theme::of(ctx);
        let c = theme.c;
        let screen = ctx.content_rect();
        let mut close = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut inner = None;
        let height_id = self.id.with("height");
        let last_height: f32 = ctx.data(|d| d.get_temp(height_id)).unwrap_or(200.0);
        // Focus starts on the first focusable element, the close button,
        // unless the body asks for its own (`autoFocus`, `initialFocus`).
        let first = super::field::mounted(ctx, self.id.with("mounted"));
        let area = egui::Area::new(self.id)
            .order(Order::Foreground)
            .fixed_pos(screen.min);
        // Like a modal <dialog>: what is under it takes no clicks or Tab.
        ctx.memory_mut(|m| m.set_modal_layer(area.layer()));
        super::opener::dialog_shown(ctx, area.layer());
        area.show(ctx, |ui| {
            // Backdrop.
            let backdrop = ui.allocate_rect(screen, Sense::click());
            ui.painter().rect_filled(screen, 0.0, c.backdrop);
            let width = self.width.px().min(screen.width() - 32.0);
            let max_h = screen.height() - 32.0;
            let height = last_height.min(max_h);
            let card = Rect::from_center_size(screen.center(), Vec2::new(width, height));
            let card = super::snap(ctx, card);
            if backdrop.clicked()
                && !self.dirty
                && !ui.input(|i| i.pointer.interact_pos().is_some_and(|p| card.contains(p)))
            {
                close = true;
            }
            let painter = ui.painter().clone();
            painter.add(theme.shadow_2.as_shape(card, corner(theme.radius.xl)));
            painter.rect(
                card,
                corner(theme.radius.xl),
                c.elevated,
                Stroke::new(1.0, c.border),
                StrokeKind::Inside,
            );
            // Header: padding 20 64 12 24.
            let title = Block::wrapped(ui, self.title, text::TITLE_SM, c.text, width - 2.0 - 88.0);
            title.paint(&painter, pos2(card.left() + 25.0, card.top() + 21.0));
            let heading = ui.interact(
                Rect::from_min_size(pos2(card.left() + 25.0, card.top() + 21.0), title.size()),
                ui.scope_id().with("modal-title"),
                Sense::hover(),
            );
            super::announce(&heading, egui::Role::Heading, self.title);
            let close_rect = Rect::from_min_size(
                pos2(card.right() - 12.0 - 36.0, card.top() + 12.0),
                Vec2::splat(36.0),
            );
            let focus_before = ctx.memory(|m| m.focused());
            let body_top = card.top() + 1.0 + 20.0 + title.height() + 12.0 + 8.0;
            // The scroll box takes the padding too: focus rings and the
            // picker's bleeding rows show in it, as in the browser.
            let body_rect = Rect::from_min_max(
                pos2(card.left() + 1.0, body_top - 8.0),
                pos2(card.right() - 1.0, card.top() + max_h),
            );
            let mut used_bottom = body_top;
            // `.gel-modal__body` scrolls once the card reaches its max-height.
            let footer_h = if footer.is_some() {
                1.0 + 12.0 + 36.0 + 16.0
            } else {
                0.0
            };
            // The bottom padding scrolls with the body, so a scrolled body shows
            // its content down to the footer, as `.gel-modal__body` does.
            let body_max = (card.top() + max_h - body_top - footer_h - 1.0).max(0.0) + 8.0;
            ui.scope_builder(UiBuilder::new().max_rect(body_rect), |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(self.id.with("body"))
                    .max_height(body_max)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        egui::Frame::NONE
                            .inner_margin(egui::Margin {
                                left: 24,
                                right: 24,
                                top: 8,
                                bottom: 24,
                            })
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                inner = Some(body(ui));
                            });
                    });
                used_bottom = ui.min_rect().bottom().max(body_top + 24.0);
            });
            let mut bottom = used_bottom;
            if let Some(footer) = footer {
                crate::widgets::rule_h(
                    &painter,
                    c.border,
                    card.left() + 1.0..=card.right() - 1.0,
                    bottom,
                );
                let footer_rect = Rect::from_min_max(
                    pos2(card.left() + 25.0, bottom + 1.0 + 12.0),
                    pos2(card.right() - 25.0, bottom + 1.0 + 12.0 + 36.0),
                );
                ui.scope_builder(
                    UiBuilder::new()
                        .max_rect(footer_rect)
                        .layout(egui::Layout::right_to_left(egui::Align::Center)),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        footer(ui);
                    },
                );
                bottom = footer_rect.bottom() + 16.0;
            }
            // Last in the Tab order, as it is last in the DOM; focused
            // first only when the dialog asked for nothing else.
            let close_button = ui.add(
                IconButton::new(Icon::Close, "Schließen")
                    .icon_size(18.0)
                    .at(close_rect),
            );
            if first && ctx.memory(|m| m.focused()) == focus_before {
                close_button.request_focus();
            }
            if close_button.clicked() {
                close = true;
            }
            let measured = bottom + 1.0 - card.top();
            if (measured - last_height).abs() > 0.5 {
                ctx.data_mut(|d| d.insert_temp(height_id, measured));
                ctx.request_discard("modal height");
            }
        });
        ModalOutput {
            inner: inner.expect("modal body ran"),
            close_requested: close,
        }
    }
}

/// The top-left of a centred box of `size` inside `screen`.
pub fn centered(screen: Rect, size: Vec2) -> Pos2 {
    screen.center() - size / 2.0
}

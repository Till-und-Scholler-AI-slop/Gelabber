//! Toasts (components/Toasts.tsx, ui.css `.gel-toast`): a stack at the
//! bottom right, info for 4s and errors for 10s, newest at the bottom.

use egui::{Id, Order, Pos2, Rect, Stroke, StrokeKind, Ui, Vec2, pos2};

use crate::icons::{self, Icon};
use crate::text::{self, Block};
use crate::theme::{Theme, corner};
use crate::widgets::IconButton;

#[derive(Clone, Debug, PartialEq)]
struct Toast {
    id: u64,
    text: String,
    error: bool,
    until: Option<f64>,
}

/// The composer drawn this frame (`anchor-name: --lr-composer`).
pub fn set_anchor(ctx: &egui::Context, rect: Rect) {
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| d.insert_temp(Id::unique("gelabber-toast-anchor"), (rect, frame)));
}

fn anchor(ctx: &egui::Context) -> Option<Rect> {
    let frame = ctx.cumulative_frame_nr();
    ctx.data(|d| d.get_temp::<(Rect, u64)>(Id::unique("gelabber-toast-anchor")))
        .filter(|(_, f)| *f == frame)
        .map(|(r, _)| r)
}

#[derive(Default)]
pub struct Toasts {
    items: Vec<Toast>,
    next: u64,
}

impl Toasts {
    /// `notify(text)`: an info toast.
    pub fn notify(&mut self, text: impl Into<String>) {
        self.push(text.into(), false);
    }

    /// `notifyError`: an error toast with the warning icon.
    pub fn error(&mut self, text: impl Into<String>) {
        self.push(text.into(), true);
    }

    fn push(&mut self, text: String, error: bool) {
        // Duplicates are dropped; at most three errors stay.
        if self
            .items
            .iter()
            .any(|t| t.text == text && t.error == error)
        {
            return;
        }
        if error
            && self.items.iter().filter(|t| t.error).count() >= 3
            && let Some(i) = self.items.iter().position(|t| t.error)
        {
            self.items.remove(i);
        }
        self.next += 1;
        self.items.push(Toast {
            id: self.next,
            text,
            error,
            until: None,
        });
    }

    /// Paints the stack; `outside_workspace` puts it 16px over the bottom
    /// (auth pages), else 12px.
    pub fn show(&mut self, ui: &Ui, outside_workspace: bool) {
        if self.items.is_empty() {
            return;
        }
        let ctx = ui.ctx().clone();
        let theme = Theme::of(&ctx);
        let c = theme.c;
        let now = ctx.input(|i| i.time);
        for toast in &mut self.items {
            let life = if toast.error { 10.0 } else { 4.0 };
            toast.until.get_or_insert(now + life);
        }
        self.items.retain(|t| t.until.is_none_or(|u| u > now));
        let screen = ctx.content_rect();
        let width = 380.0_f32.min(screen.width() - 32.0);
        // In the workspace 12px over the composer, 16px in from its right
        // edge; without one, 12px over the bottom.
        let composer = (!outside_workspace).then(|| anchor(&ctx)).flatten();
        let bottom = match composer {
            Some(r) => r.top() - 12.0,
            None => screen.bottom() - if outside_workspace { 16.0 } else { 12.0 },
        };
        let right = composer.map_or(screen.right(), |r| r.right()) - 16.0;
        let mut closed = None;
        // Info toasts sit above the error toasts.
        let mut order: Vec<&Toast> = self.items.iter().filter(|t| !t.error).collect();
        order.extend(self.items.iter().filter(|t| t.error));
        egui::Area::new(Id::unique("gelabber-toasts"))
            .order(Order::Foreground)
            .fixed_pos(screen.min)
            .interactable(true)
            .show(&ctx, |ui| {
                let mut y = bottom;
                for toast in order.iter().rev() {
                    let left_pad = if toast.error {
                        12.0 + 3.0 + 16.0 + 12.0
                    } else {
                        13.0
                    };
                    let text_w = width - left_pad - 12.0 - 32.0 - 9.0;
                    let block = Block::wrapped(ui, &toast.text, text::BODY, c.text, text_w);
                    let height = block.height().max(32.0) + 24.0 + 2.0;
                    let rect = Rect::from_min_max(pos2(right - width, y - height), pos2(right, y));
                    let painter = ui.painter();
                    painter.add(theme.shadow_2.as_shape(rect, corner(theme.radius.lg)));
                    painter.rect(
                        rect,
                        corner(theme.radius.lg),
                        c.elevated,
                        Stroke::new(1.0, c.border),
                        StrokeKind::Inside,
                    );
                    if toast.error {
                        painter
                            .with_clip_rect(Rect::from_min_max(
                                rect.min,
                                pos2(rect.left() + 3.0, rect.bottom()),
                            ))
                            .rect_filled(rect, corner(theme.radius.lg), c.danger);
                        icons::paint(
                            painter,
                            Icon::Warning,
                            Pos2::new(rect.left() + 15.0, rect.center().y - 8.0),
                            16.0,
                            c.danger,
                        );
                    }
                    block.paint(
                        painter,
                        Pos2::new(
                            rect.left() + left_pad,
                            rect.center().y - block.height() / 2.0,
                        ),
                    );
                    let close = Rect::from_min_size(
                        pos2(rect.right() - 9.0 - 32.0, rect.center().y - 16.0),
                        Vec2::splat(32.0),
                    );
                    if ui
                        .add(
                            IconButton::new(Icon::Close, "Schließen")
                                .size(32.0)
                                .at(close),
                        )
                        .clicked()
                    {
                        closed = Some(toast.id);
                    }
                    y = rect.top() - 8.0;
                }
            });
        if let Some(id) = closed {
            self.items.retain(|t| t.id != id);
        }
        if let Some(next) = self.items.iter().filter_map(|t| t.until).reduce(f64::min) {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64((next - now).max(0.0)));
        }
    }
}

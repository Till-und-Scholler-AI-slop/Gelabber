//! `NotFoundPage` (pages/NotFoundPage.tsx) for addresses Gelabber has not:
//! the plain app header (AppShell's `.app-header`) over the notice card.

use egui::{Id, Rect, Ui, Vec2, pos2};

use super::layout;
use crate::app::AppCtx;
use crate::text::{Block, TextStyle};
use crate::theme::{Theme, space};
use crate::widgets::{Link, hairline_h};

/// `text-lg font-semibold tracking-tight`.
const HEADER_BRAND: TextStyle = TextStyle::new(18.0, 600.0, 28.0 / 18.0).tracking(-0.025);

pub fn show(ui: &mut Ui, app: &mut AppCtx) {
    let ctx = ui.ctx().clone();
    let theme = Theme::of(&ctx);
    let screen = ctx.content_rect();

    // Header: 56px on the panel colour, a hairline under it.
    let header = Rect::from_min_size(screen.min, Vec2::new(screen.width(), 56.0));
    ui.painter().rect_filled(header, 0.0, theme.c.panel);
    hairline_h(
        ui.painter(),
        &theme,
        screen.left()..=screen.right(),
        header.bottom(),
    );
    let brand = Block::single(ui, "Gelabber", HEADER_BRAND, theme.c.text);
    let brand_rect = Rect::from_min_size(
        pos2(
            screen.left() + space::S4,
            header.center().y - brand.height() / 2.0,
        ),
        brand.size(),
    );
    brand.paint(ui.painter(), brand_rect.min);
    if Link::interact(
        ui,
        brand_rect,
        Id::unique("gelabber-not-found-brand"),
        "Gelabber",
    )
    .clicked()
    {
        app.nav.navigate("/");
    }

    // `main.mx-auto.max-w-3xl.px-6.py-10` around a `max-w-sm` card.
    let main_w = screen.width().min(768.0) - 2.0 * space::S6;
    let width = main_w.min(384.0);
    let top = header.bottom() + 1.0 + space::S10;
    let left = screen.center().x - width / 2.0;
    layout::card_at(ui, app, pos2(left, top), width, |ui, app| {
        layout::heading(ui, "Seite nicht gefunden");
        layout::lead(ui, "Diese Adresse gibt es in Gelabber nicht.");
        ui.add_space(space::S6);
        if layout::link_line(ui, "Zur Startseite") {
            app.nav.navigate("/");
        }
    });
}

//! "App" (pwa/InstallApp.tsx, install.css): the install panel. The desktop
//! app is installed already; the panel keeps the web's wording.

use egui::{Color32, Rect, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::images;
use crate::text::{Block, TextStyle};
use crate::theme::{Theme, corner};

const ICON: &[u8] = include_bytes!("../../../assets/images/icon-192.png");

pub fn area(ui: &mut Ui) {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let width = ui.available_width().min(640.0);
    let body = TextStyle::new(14.0, 400.0, 1.6);
    let title = TextStyle::new(16.0, 650.0, 1.6);
    let inner_w = width - 2.0 - 36.0;
    let text = Block::wrapped(
        ui,
        "Die Installation findest du in der Adressleiste oder im Browsermenü, z. B. in Chrome oder Edge.",
        body,
        c.text,
        inner_w,
    );
    let note = Block::wrapped(
        ui,
        "Die App braucht weiterhin eine Internetverbindung.",
        body,
        c.muted,
        inner_w,
    );
    let h = 1.0 + 18.0 + 48.0 + 16.0 + text.height() + 12.0 + note.height() + 18.0 + 1.0;
    let top = ui.cursor().min;
    let rect = Rect::from_min_size(top, vec2(width, h));
    let painter = ui.painter();
    painter.rect(
        rect,
        corner(theme.radius.md),
        c.panel,
        Stroke::new(1.0, c.border),
        StrokeKind::Inside,
    );
    let x = rect.left() + 19.0;
    let y = rect.top() + 19.0;
    let icon = Rect::from_min_size(pos2(x, y), vec2(48.0, 48.0));
    if let Some(t) = images::bundled(ui.ctx(), "icon-192", ICON) {
        painter.add(
            egui::epaint::RectShape::filled(icon, corner(10.0), Color32::WHITE)
                .with_texture(t.id(), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))),
        );
    }
    Block::single(ui, "Gelabber als App", title, c.text).paint(painter, pos2(x + 60.0, y));
    Block::single(ui, "In einem eigenen Fenster öffnen.", body, c.muted)
        .paint(painter, pos2(x + 60.0, y + 25.6));
    text.paint(painter, pos2(x, y + 64.0));
    note.paint(painter, pos2(x, y + 64.0 + text.height() + 12.0));
    ui.allocate_exact_size(vec2(width, h), egui::Sense::hover());
}

//! The front-door frame (pages/AuthLayout.tsx, auth.css): the dimmed
//! Living Room artwork, the app icon with the wordmark over one centred
//! card, and the pieces every card uses (heading, lead, links).

use egui::epaint::{Mesh, RectShape, Vertex, WHITE_UV};
use egui::{
    Color32, CornerRadius, Id, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, UiBuilder, Vec2,
    pos2,
};

use gelabber_i18n::auth;

use crate::app::AppCtx;
use crate::images::{self, ImageState};
use crate::text::{self, Block, Rich, TextStyle};
use crate::theme::{Theme, corner, space};
use crate::widgets::field::layout_unit;
use crate::widgets::{Link, link};

/// The approved artwork (public/images/living-room).
pub const ARTWORK: &str = "/images/living-room/community-banner.png";
const ICON: &[u8] = include_bytes!("../../../assets/images/icon-192.png");

/// `.lr-auth-card h1`: `--lr-type-page`, letter-spacing −0.03em.
pub const HEADING: TextStyle = text::PAGE.tracking(-0.03);
/// `.auth-brand`: 700 20px/1.2, −0.02em.
const BRAND: TextStyle = TextStyle::new(20.0, 700.0, 1.2).tracking(-0.02);

/// The page: artwork, brand and a card whose content `card` lays out in a
/// 318px column (the card's padding is 32px, its border 1px).
pub fn frame(ui: &mut Ui, app: &mut AppCtx, id: &str, card: impl FnOnce(&mut Ui, &mut AppCtx)) {
    // Loaded only where it shows.
    let artwork = match wide(ui.ctx()).then(|| app.images.get(ui.ctx(), ARTWORK)) {
        Some(ImageState::Ready(texture)) => Some(texture),
        _ => None,
    };
    if page(ui, id, artwork.as_ref(), true, |ui| card(ui, app)) {
        app.nav.navigate("/");
    }
}

/// Whether the window is wide enough for the artwork (auth.css `@media
/// (min-width: 800px)`).
pub fn wide(ctx: &egui::Context) -> bool {
    ctx.content_rect().width() >= 800.0
}

/// The app's own copy of the artwork (a JPEG of public/images/living-room)
/// for the setup screen, which has no server yet to load it from.
const BUNDLED_ARTWORK: &[u8] = include_bytes!("../../../assets/images/community-banner.jpg");

/// The bundled artwork, decoded once per window.
pub fn bundled_artwork(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    images::bundled(ctx, "community-banner", BUNDLED_ARTWORK)
}

/// [`frame`] without an app: `artwork` behind it on wide screens, the
/// brand a link only with `brand_link`. True when the brand was clicked.
pub fn page(
    ui: &mut Ui,
    id: &str,
    artwork: Option<&egui::TextureHandle>,
    brand_link: bool,
    card: impl FnOnce(&mut Ui),
) -> bool {
    let ctx = ui.ctx().clone();
    let theme = Theme::of(&ctx);
    let c = theme.c;
    let screen = ctx.content_rect();
    if wide(&ctx) {
        backdrop(ui, artwork, screen);
    }

    let height_id = Id::unique(("gelabber-auth-column", id));
    let column_h: f32 = ctx.data(|d| d.get_temp(height_id)).unwrap_or(0.0);
    let padding_x = space::S4;
    let width = (screen.width() - 2.0 * padding_x).min(384.0);
    let page_h = (column_h + 2.0 * space::S8).max(screen.height());
    let mut brand_clicked = false;

    egui::ScrollArea::vertical()
        .id_salt(("auth-page", id))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let (page, _) =
                ui.allocate_exact_size(Vec2::new(screen.width(), page_h), Sense::hover());
            let top =
                page.top() + space::S8 + ((page_h - 2.0 * space::S8 - column_h) / 2.0).max(0.0);
            let left = page.left() + (page.width() - width) / 2.0;

            // Brand: the icon and "Gelabber", centred, linking to "/".
            let brand_text = Block::single(ui, "Gelabber", BRAND, c.text);
            let brand_w = 40.0 + 10.0 + brand_text.width();
            let brand = Rect::from_min_size(
                pos2(left + (width - brand_w) / 2.0, top),
                Vec2::new(brand_w, 40.0),
            );
            if let Some(icon) = images::bundled(&ctx, "icon-192", ICON) {
                let icon_rect = Rect::from_min_size(brand.min, Vec2::splat(40.0));
                ui.painter().add(
                    RectShape::filled(icon_rect, corner(theme.radius.md), Color32::WHITE)
                        .with_texture(icon.id(), Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0))),
                );
            }
            brand_text.paint(
                ui.painter(),
                pos2(
                    brand.left() + 50.0,
                    brand.center().y - brand_text.height() / 2.0,
                ),
            );
            if brand_link {
                let brand_response =
                    Link::interact(ui, brand, Id::unique("gelabber-auth-brand"), "Gelabber");
                brand_clicked = brand_response.clicked();
            }

            let card_rect = card_box(ui, pos2(left, brand.bottom() + space::S6), width, card);
            let measured = 40.0 + space::S6 + card_rect.height();
            // Half a pixel, as the modal's: the 1/64 px layout units round
            // by where the column starts, so a column centred by its own
            // height can measure 1/32 px apart every other frame.
            if (measured - column_h).abs() > 0.5 {
                ctx.data_mut(|d| d.insert_temp(height_id, measured));
                ctx.request_discard("auth column height"); // i18n-ok
            }
        });
    brand_clicked
}

/// `.lr-auth-card` with its top-left at `min`: the panel box (painted
/// under the content once its height is known) around `content`.
pub fn card_at(
    ui: &mut Ui,
    app: &mut AppCtx,
    min: Pos2,
    width: f32,
    content: impl FnOnce(&mut Ui, &mut AppCtx),
) -> Rect {
    card_box(ui, min, width, |ui| content(ui, app))
}

/// [`card_at`] without an app.
fn card_box(ui: &mut Ui, min: Pos2, width: f32, content: impl FnOnce(&mut Ui)) -> Rect {
    let ctx = ui.ctx().clone();
    let theme = Theme::of(&ctx);
    let background = ui.painter().add(Shape::Noop);
    let inner = Rect::from_min_size(
        min + Vec2::splat(1.0 + space::S8),
        Vec2::new(width - 2.0 - 2.0 * space::S8, f32::INFINITY),
    );
    let content_h = ui
        .scope_builder(UiBuilder::new().max_rect(inner), |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            content(ui);
            ui.min_rect().height()
        })
        .inner;
    let rect = Rect::from_min_size(min, Vec2::new(width, content_h + 2.0 + 2.0 * space::S8));
    ui.painter().set(
        background,
        RectShape::new(
            crate::widgets::snap(&ctx, rect),
            corner(theme.radius.lg),
            theme.c.panel,
            Stroke::new(1.0, theme.c.border),
            StrokeKind::Inside,
        ),
    );
    rect
}

/// `.auth-layout::before/::after`: the artwork with `cover` at opacity
/// .18, under `radial-gradient(ellipse at center, transparent 30%, bg 95%)`.
fn backdrop(ui: &Ui, artwork: Option<&egui::TextureHandle>, screen: Rect) {
    let theme = Theme::of(ui.ctx());
    let painter = ui.painter();
    if let Some(texture) = artwork {
        let uv = images::cover_uv(texture.size_vec2(), screen.size());
        painter.image(texture.id(), screen, uv, Color32::from_white_alpha(46));
    }
    painter.add(vignette(screen, theme.c.bg));
}

/// The ellipse-at-centre gradient as a mesh: rings at the colour stops,
/// alpha linear in the normalised radius like the CSS gradient line.
fn vignette(rect: Rect, color: Color32) -> Shape {
    // `farthest-corner` with the aspect ratio of `closest-side`.
    let center = rect.center();
    let radii = rect.size() / 2.0 * std::f32::consts::SQRT_2;
    let stops: [(f32, f32); 3] = [(0.3, 0.0), (0.95, 1.0), (1.2, 1.0)];
    const SEGMENTS: usize = 256;
    let mut mesh = Mesh::default();
    for (t, alpha) in stops {
        let fill = color.gamma_multiply(alpha);
        for i in 0..SEGMENTS {
            let angle = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
            let pos = center + Vec2::new(angle.cos() * radii.x, angle.sin() * radii.y) * t;
            mesh.vertices.push(Vertex {
                pos,
                uv: WHITE_UV,
                color: fill,
            });
        }
    }
    for ring in 0..stops.len() as u32 - 1 {
        let (a, b) = (ring * SEGMENTS as u32, (ring + 1) * SEGMENTS as u32);
        for i in 0..SEGMENTS as u32 {
            let j = (i + 1) % SEGMENTS as u32;
            mesh.add_triangle(a + i, a + j, b + i);
            mesh.add_triangle(a + j, b + j, b + i);
        }
    }
    Shape::mesh(mesh)
}

/// `.lr-auth-card h1`.
pub fn heading(ui: &mut Ui, title: &str) {
    let theme = Theme::of(ui.ctx());
    let block = Block::wrapped(ui, title, HEADING, theme.c.text, ui.available_width());
    show_block(ui, &block);
}

/// A heading that takes focus on mount (`useFocusOnMount`, tabindex -1):
/// its outline is off, but the global focus rule's page-coloured 2px
/// shadow shows around the block until the user moves on.
pub fn focused_heading(ui: &mut Ui, title: &str) {
    let ctx = ui.ctx().clone();
    let id = egui::Id::unique(("gelabber-focused-heading", title));
    let mount = crate::widgets::field::mounted(&ctx, id.with("mount"));
    if mount {
        ctx.data_mut(|d| d.insert_temp(id, true));
    }
    // It holds the focus, so the next Tab is the card's first control.
    crate::app::focus_anchor(ui, id.with("focus"), ui.cursor().min, mount);
    // A click or a key takes the focus elsewhere, as in the browser.
    if ctx.input(|i| {
        i.pointer.any_pressed()
            || i.events
                .iter()
                .any(|e| matches!(e, egui::Event::Key { pressed: true, .. }))
    }) {
        ctx.data_mut(|d| d.insert_temp(id, false));
    }
    let theme = Theme::of(&ctx);
    let block = Block::wrapped(ui, title, HEADING, theme.c.text, ui.available_width());
    let top = ui.cursor().min;
    show_block(ui, &block);
    if ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false) {
        let rect = Rect::from_min_size(top, Vec2::new(ui.available_width(), block.height()));
        ui.painter().rect_stroke(
            rect.expand(1.0),
            egui::CornerRadius::ZERO,
            egui::Stroke::new(2.0, theme.c.bg),
            egui::StrokeKind::Middle,
        );
    }
}

/// An `.auth-link` in `.auth-actions`: the card's 16px, not the footer's 14.
pub fn action_link(ui: &mut Ui, label: &str) -> bool {
    let style = text::TextStyle::new(16.0, 500.0, 1.5);
    let (row, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), style.line_px()),
        Sense::hover(),
    );
    let block = Block::single(ui, label, style, Color32::WHITE);
    let rect = Rect::from_min_size(row.min, Vec2::new(block.width(), row.height()));
    ui.put(rect, Link::new(label).style(style)).clicked()
}

/// `.auth-lead` (margin-top 4): muted body text.
pub fn lead(ui: &mut Ui, lead: &str) {
    let theme = Theme::of(ui.ctx());
    ui.add_space(space::S1);
    let block = Block::wrapped(ui, lead, text::BODY, theme.c.muted, ui.available_width());
    show_block(ui, &block);
}

/// A lead sentence ending in a link ("Noch kein Konto? Registrieren");
/// true when the link was clicked.
pub fn lead_with_link(ui: &mut Ui, sentence: &str, link_label: &str) -> bool {
    let theme = Theme::of(ui.ctx());
    ui.add_space(space::S1);
    let link_style = text::BODY.weight(500.0);
    let rich = Rich::new(text::BODY)
        .push(&format!("{sentence} "), text::BODY, theme.c.muted)
        .push(link_label, link_style, theme.c.accent_text)
        .wrap(ui.available_width());
    let block = rich.layout(ui);
    let top_left = ui.cursor().min;
    show_block(ui, &block);
    paragraph_link(ui, &block, top_left, 1, link_style, link_label)
}

/// Underline, hover and click for section `index` of a painted paragraph.
pub fn paragraph_link(
    ui: &mut Ui,
    block: &Block,
    top_left: Pos2,
    index: usize,
    style: TextStyle,
    label: &str,
) -> bool {
    let theme = Theme::of(ui.ctx());
    let Some(rect) = block.section_rect(top_left, index) else {
        return false;
    };
    link::underline(
        ui.painter(),
        style,
        rect.top(),
        rect.left(),
        rect.right(),
        theme.c.accent_text,
    );
    Link::interact(ui, rect, Id::unique(("gelabber-auth-link", label)), label).clicked()
}

/// A paragraph in the card at the cursor, its height in layout units.
pub fn show_block(ui: &mut Ui, block: &Block) {
    let size = Vec2::new(ui.available_width(), layout_unit(block.height()));
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    block.paint(ui.painter(), rect.min);
}

/// A standalone link on its own line (`.auth-link self-start`).
pub fn link_line(ui: &mut Ui, label: &str) -> bool {
    // One body line box; the link hugs its text at the left.
    let (row, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), text::BODY.line_px()),
        Sense::hover(),
    );
    let block = Block::single(ui, label, text::BODY.weight(500.0), Color32::WHITE);
    let rect = Rect::from_min_size(row.min, Vec2::new(block.width(), row.height()));
    ui.put(rect, Link::new(label)).clicked()
}

/// `AppPlaceholder`: icon, wordmark and "Verbinde…" while the session is
/// being checked.
pub fn placeholder(ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let theme = Theme::of(&ctx);
    let screen = ctx.content_rect();
    let word = Block::single(ui, "Gelabber", BRAND, theme.c.text);
    let status = Block::single(ui, auth::layout_connecting(), text::UI, theme.c.muted);
    let total = 48.0 + 12.0 + word.height() + 12.0 + status.height();
    let top = screen.center().y - total / 2.0;
    if let Some(icon) = images::bundled(&ctx, "icon-192", ICON) {
        let rect = Rect::from_min_size(pos2(screen.center().x - 24.0, top), Vec2::splat(48.0));
        ui.painter().add(
            RectShape::filled(rect, CornerRadius::same(12), Color32::WHITE)
                .with_texture(icon.id(), Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0))),
        );
    }
    let y = top + 48.0 + 12.0;
    word.paint(
        ui.painter(),
        pos2(screen.center().x - word.width() / 2.0, y),
    );
    let y = y + word.height() + 12.0;
    status.paint(
        ui.painter(),
        pos2(screen.center().x - status.width() / 2.0, y),
    );
}

/// Google's standard colour "G" (the web's inline SVG in
/// components/ProviderButton.tsx), drawn untinted.
const GOOGLE_MARK: &[u8] = include_bytes!("../../../assets/images/google-g.svg");

/// The Google mark at `size` px, rasterised once per physical size.
fn google_mark(ctx: &egui::Context, size: f32) -> Option<egui::TextureHandle> {
    use resvg::{tiny_skia, usvg};
    let px = (size * ctx.pixels_per_point()).round().max(1.0) as u32;
    let id = Id::unique(("gelabber-google-mark", px));
    if let Some(texture) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return Some(texture);
    }
    let tree = usvg::Tree::from_data(GOOGLE_MARK, &usvg::Options::default()).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(px, px)?;
    let scale = px as f32 / 48.0;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let image =
        egui::ColorImage::from_rgba_premultiplied([px as usize, px as usize], pixmap.data());
    let texture = ctx.load_texture(
        format!("google-mark-{px}"),
        image,
        egui::TextureOptions::LINEAR,
    );
    ctx.data_mut(|d| d.insert_temp(id, texture.clone()));
    Some(texture)
}

/// `.auth-provider--google`: 500 14px/20px, letter-spacing 0.25px, in
/// Google's own colours (its font is not bundled; the app's stands in, as
/// on the web).
const GOOGLE_LABEL: TextStyle = TextStyle::new(14.0, 500.0, 20.0 / 14.0).tracking(0.25 / 14.0);

/// `<ProviderButton>`: a full-width `size="lg"` button that leaves for the
/// provider. Google's keeps Google's light or dark button after the page's
/// tone on every theme; any other provider is a transparent secondary
/// button with its initial in a ring. `pending` swaps the label and keeps
/// the look (`aria-busy`); `disabled` fades it.
pub fn provider_button(
    ui: &mut Ui,
    provider: &gelabber_client::AuthProvider,
    pending: Option<&str>,
    disabled: bool,
) -> egui::Response {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let google = provider.id == "google";
    let own_label;
    let label = match pending {
        Some(label) => label,
        None if google => auth::oidc_sign_in_with_google(),
        None => {
            own_label = auth::oidc_sign_in_with(&provider.name);
            &own_label
        }
    };
    let enabled = pending.is_none() && !disabled;
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), crate::theme::control::LG),
        sense,
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let hovered = enabled && response.hovered();
    // (fill, hover fill, border, text)
    let (fill, hover, border, ink) = if !google {
        (Color32::TRANSPARENT, c.hover, c.border_strong, c.text)
    } else if crate::theme::is_dark_tone(c.bg) {
        let (bg, ink) = (
            Color32::from_rgb(0x13, 0x13, 0x14),
            Color32::from_rgb(0xe3, 0xe3, 0xe3),
        );
        (
            bg,
            crate::theme::mix(bg, ink, 0.08),
            Color32::from_rgb(0x8e, 0x91, 0x8f),
            ink,
        )
    } else {
        let (bg, ink) = (Color32::WHITE, Color32::from_rgb(0x1f, 0x1f, 0x1f));
        (
            bg,
            crate::theme::mix(bg, ink, 0.08),
            Color32::from_rgb(0x74, 0x77, 0x75),
            ink,
        )
    };
    let fill = crate::widgets::eased(ui.ctx(), response.id.with("hover"), hovered, fill, hover);
    // `opacity: 0.6` over the card while another provider's flow runs.
    let faded = |color: Color32| {
        if disabled && pending.is_none() {
            color.gamma_multiply(0.6)
        } else {
            color
        }
    };
    let painter = ui.painter();
    let box_rect = crate::widgets::snap(ui.ctx(), rect);
    if response.has_focus() && crate::widgets::keyboard_focus(ui.ctx()) {
        crate::widgets::focus_ring(painter, box_rect, theme.radius.sm, &theme);
    }
    painter.rect(
        box_rect,
        corner(theme.radius.sm),
        faded(fill),
        Stroke::new(1.0, faded(border)),
        StrokeKind::Inside,
    );
    let style = if google {
        GOOGLE_LABEL
    } else {
        text::UI.size(14.0).weight(600.0)
    };
    let block = Block::single(ui, label, style, faded(ink));
    // `gap: 10px` for Google's mark, the button's 8px otherwise.
    let gap = if google { 10.0 } else { 8.0 };
    let content = 20.0 + gap + block.width();
    let left = (rect.center().x - content / 2.0).round();
    let mark = Rect::from_min_size(
        pos2(left, (rect.center().y - 10.0).round()),
        Vec2::splat(20.0),
    );
    if google {
        if let Some(texture) = google_mark(ui.ctx(), 20.0) {
            painter.image(
                texture.id(),
                mark,
                Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
                faded(Color32::WHITE),
            );
        }
    } else {
        // `.auth-provider__mark`: the initial in a 1.5px ring.
        painter.circle_stroke(mark.center(), 9.25, Stroke::new(1.5, faded(ink)));
        let initial: String = provider
            .name
            .chars()
            .next()
            .map(|ch| ch.to_uppercase().collect())
            .unwrap_or_default();
        let letter = Block::single(ui, &initial, TextStyle::new(11.0, 700.0, 1.0), faded(ink));
        letter.paint(painter, mark.center() - letter.size() / 2.0);
    }
    block.paint(
        painter,
        pos2(left + 20.0 + gap, rect.center().y - style.line_px() / 2.0),
    );
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    } else if pending.is_some() && response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Progress);
    }
    response
}

/// `.auth-divider`: "oder" between two hairlines, `--lr-type-meta`, muted,
/// 12px apart.
pub fn divider(ui: &mut Ui) {
    let theme = Theme::of(ui.ctx());
    let block = Block::single(ui, auth::oidc_or(), text::META, theme.c.muted);
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), layout_unit(block.height())),
        Sense::hover(),
    );
    let left = rect.center().x - block.width() / 2.0;
    block.paint(ui.painter(), pos2(left, rect.top()));
    // The rules' 1px top borders, centred on the line box.
    let y = (rect.center().y - 0.5).round();
    let line = |from: f32, to: f32| {
        ui.painter().rect_filled(
            Rect::from_min_max(pos2(from, y), pos2(to, y + 1.0)),
            0.0,
            theme.c.border,
        );
    };
    line(rect.left(), left - space::S3);
    line(left + block.width() + space::S3, rect.right());
}

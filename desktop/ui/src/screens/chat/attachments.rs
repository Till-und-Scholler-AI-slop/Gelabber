//! What a message carries besides text (components/Attachments.tsx,
//! imageBox.ts, GifPicker.tsx `GifMessageButton`, LinkPreviews.tsx;
//! chat.css `.lr-attachments`, `.lr-gif`, `.lr-preview`): pictures, files,
//! GIFs and link previews, each sized before its image arrives.

use egui::{Color32, CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use gelabber_client::{Attachment, Gif, LinkPreview};

use crate::icons::{self, Icon};
use crate::images::{ImageState, Images, cover_uv};
use crate::text::{Block, TextStyle};
use crate::theme::{Theme, corner, mix};

const IMAGE_BOX_W: f32 = 360.0;
const IMAGE_BOX_H: f32 = 240.0;

pub fn is_image(content_type: &str) -> bool {
    matches!(
        content_type,
        "image/jpeg" | "image/png" | "image/gif" | "image/webp"
    )
}

/// `formatSize`: "1,5 MB" or "12 KB".
pub fn format_size(bytes: i64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0)).replace('.', ",")
    } else {
        format!("{} KB", ((bytes as f64 / 1024.0).round() as i64).max(1))
    }
}

pub fn attachment_url(id: uuid::Uuid) -> String {
    format!("/api/attachments/{id}")
}

/// What a click on a piece asks for.
pub enum MediaAction {
    /// Open a picture large (a URL the image cache knows).
    Open(String, String),
    /// Download or open externally.
    External(String),
}

enum Piece {
    /// A single picture: its box (border included).
    Image {
        url: String,
        name: String,
        size: Vec2,
    },
    Grid {
        cells: Vec<(String, String)>,
    },
    File(Box<FilePiece>),
}

struct FilePiece {
    att: Attachment,
    name: Block,
    size: Block,
    width: f32,
}

/// Laid-out attachments: pieces at y offsets inside a column.
pub struct Media {
    pieces: Vec<(f32, Piece)>,
    pub height: f32,
}

/// The box of a single picture: `imageFit` plus the 1px border.
pub fn image_box(natural: Option<Vec2>, width: f32) -> Vec2 {
    match natural {
        Some(n) if n.x > 0.0 && n.y > 0.0 => {
            let scale = 1f32
                .min((IMAGE_BOX_W - 2.0) / n.x)
                .min((IMAGE_BOX_H - 2.0) / n.y)
                .min((width - 2.0).max(1.0) / n.x);
            vec2((n.x * scale + 2.0).max(24.0), (n.y * scale + 2.0).max(24.0))
        }
        _ => vec2(IMAGE_BOX_W.min(width), IMAGE_BOX_H),
    }
}

fn natural(ui: &Ui, images: &Images, url: &str) -> Option<Vec2> {
    match images.get(ui.ctx(), url) {
        ImageState::Ready(t) => Some(t.size_vec2()),
        _ => None,
    }
}

pub fn layout(ui: &Ui, images: &Images, attachments: &[Attachment], width: f32) -> Option<Media> {
    if attachments.is_empty() {
        return None;
    }
    let theme = Theme::of(ui.ctx());
    let pictures: Vec<&Attachment> = attachments
        .iter()
        .filter(|a| is_image(&a.content_type))
        .collect();
    let mut pieces = Vec::new();
    let mut y = 0.0;
    let mut push = |pieces: &mut Vec<(f32, Piece)>, h: f32, piece: Piece| {
        if !pieces.is_empty() {
            y += 6.0;
        }
        pieces.push((y, piece));
        y += h;
    };
    if pictures.len() == 1 {
        let a = pictures[0];
        let url = attachment_url(a.id);
        let size = image_box(natural(ui, images, &url), width);
        push(
            &mut pieces,
            size.y,
            Piece::Image {
                url,
                name: a.filename.clone(),
                size,
            },
        );
    } else if pictures.len() > 1 {
        let rows = pictures.len().div_ceil(2) as f32;
        let cells = pictures
            .iter()
            .map(|a| (attachment_url(a.id), a.filename.clone()))
            .collect();
        push(
            &mut pieces,
            rows * 160.0 + (rows - 1.0) * 4.0,
            Piece::Grid { cells },
        );
    }
    for a in attachments.iter().filter(|a| !is_image(&a.content_type)) {
        let name = Block::single(
            ui,
            &a.filename,
            TextStyle::new(13.0, 600.0, 1.5),
            theme.c.text,
        );
        let size = Block::single(
            ui,
            &format_size(a.size),
            TextStyle::new(11.0, 400.0, 1.5),
            theme.c.muted,
        );
        let w = (10.0 + 15.0 + 8.0 + name.width() + 8.0 + size.width() + 12.0 + 2.0).min(width);
        push(
            &mut pieces,
            35.5,
            Piece::File(Box::new(FilePiece {
                att: a.clone(),
                name,
                size,
                width: w,
            })),
        );
    }
    Some(Media { pieces, height: y })
}

fn paint_image(ui: &Ui, images: &Images, url: &str, rect: Rect, cover: bool) {
    let theme = Theme::of(ui.ctx());
    let painter = ui.painter();
    if let ImageState::Ready(texture) = images.get(ui.ctx(), url) {
        let inner = rect.shrink(1.0);
        let uv = if cover {
            cover_uv(texture.size_vec2(), inner.size())
        } else {
            Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0))
        };
        painter.add(
            egui::epaint::RectShape::filled(inner, CornerRadius::same(6), Color32::WHITE)
                .with_texture(texture.id(), uv),
        );
    }
    painter.rect_stroke(
        rect,
        corner(7.0),
        Stroke::new(1.0, theme.c.border),
        StrokeKind::Inside,
    );
}

impl Media {
    pub fn paint(
        &self,
        ui: &mut Ui,
        images: &Images,
        origin: Pos2,
        id: egui::Id,
    ) -> Option<MediaAction> {
        let theme = Theme::of(ui.ctx());
        let c = theme.c;
        let mut action = None;
        for (n, (y, piece)) in self.pieces.iter().enumerate() {
            let top = origin.y + y;
            match piece {
                Piece::Image { url, name, size } => {
                    let rect = Rect::from_min_size(pos2(origin.x, top), *size);
                    paint_image(ui, images, url, rect, false);
                    let response = ui.interact(rect, id.with(("image", n)), Sense::click());
                    if response.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ZoomIn);
                    }
                    if response.clicked() {
                        action = Some(MediaAction::Open(url.clone(), name.clone()));
                    }
                }
                Piece::Grid { cells } => {
                    for (i, (url, name)) in cells.iter().enumerate() {
                        let col = (i % 2) as f32;
                        let row = (i / 2) as f32;
                        let cell_w = 198.0;
                        let rect = Rect::from_min_size(
                            pos2(origin.x + col * (cell_w + 4.0), top + row * 164.0),
                            vec2(cell_w, 160.0),
                        );
                        paint_image(ui, images, url, rect, true);
                        let response = ui.interact(rect, id.with(("cell", n, i)), Sense::click());
                        if response.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::ZoomIn);
                        }
                        if response.clicked() {
                            action = Some(MediaAction::Open(url.clone(), name.clone()));
                        }
                    }
                }
                Piece::File(file) => {
                    let FilePiece {
                        att,
                        name,
                        size,
                        width,
                    } = &**file;
                    let rect = Rect::from_min_size(pos2(origin.x, top), vec2(*width, 35.5));
                    let response = ui.interact(rect, id.with(("file", n)), Sense::click());
                    let fill = if response.hovered() { c.hover } else { c.panel };
                    ui.painter().rect(
                        rect,
                        corner(7.0),
                        fill,
                        Stroke::new(1.0, c.border),
                        StrokeKind::Inside,
                    );
                    icons::paint(
                        ui.painter(),
                        Icon::Paperclip,
                        pos2(rect.left() + 11.0, rect.center().y - 7.5),
                        15.0,
                        c.accent_text,
                    );
                    let x = rect.left() + 11.0 + 15.0 + 8.0;
                    let clip = ui
                        .painter()
                        .with_clip_rect(rect.shrink(1.0).intersect(ui.clip_rect()));
                    name.paint(&clip, pos2(x, rect.center().y - 9.75));
                    size.paint(
                        &clip,
                        pos2(x + name.width() + 8.0, rect.center().y - 8.25 + 0.5),
                    );
                    if response.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if response.clicked() {
                        action = Some(MediaAction::External(format!(
                            "{}?download=1",
                            attachment_url(att.id)
                        )));
                    }
                }
            }
        }
        action
    }
}

/// The picture large over the window (components/ImageLightbox.tsx):
/// filename, "Herunterladen", close; Esc or a click beside it closes.
/// Returns (close, download).
pub fn lightbox(ctx: &egui::Context, images: &Images, url: &str, name: &str) -> (bool, bool) {
    let screen = ctx.content_rect();
    let mut close = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let mut download = false;
    egui::Area::new(egui::Id::unique("gelabber-lightbox"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let backdrop = ui.allocate_rect(screen, Sense::click());
            ui.painter()
                .rect_filled(screen, 0.0, Color32::from_black_alpha(217));
            let area = Rect::from_min_max(
                pos2(screen.left() + 16.0, screen.top() + 56.0),
                pos2(screen.right() - 16.0, screen.bottom() - 16.0),
            );
            let mut image_rect = None;
            if let ImageState::Ready(t) = images.get(ctx, url) {
                let size = t.size_vec2();
                let scale = (area.width() / size.x).min(area.height() / size.y).min(1.0);
                let r = Rect::from_center_size(area.center(), size * scale);
                ui.painter().add(
                    egui::epaint::RectShape::filled(r, CornerRadius::same(7), Color32::WHITE)
                        .with_texture(t.id(), Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0))),
                );
                image_rect = Some(r);
            }
            let white = Color32::WHITE;
            let style = TextStyle::new(14.0, 400.0, 1.5);
            let close_rect = Rect::from_min_size(
                pos2(screen.right() - 16.0 - 40.0, screen.top() + 10.0),
                vec2(40.0, 40.0),
            );
            let dl = Block::single(ui, "Herunterladen", style, white);
            let dl_rect = Rect::from_min_size(
                pos2(
                    close_rect.left() - 16.0 - dl.width(),
                    close_rect.center().y - dl.height() / 2.0,
                ),
                dl.size(),
            );
            let title_w = (dl_rect.left() - 16.0 - screen.left() - 16.0).max(10.0);
            let mut job = crate::text::job(name, style, white, Some(title_w));
            job.wrap.max_rows = 1;
            job.wrap.break_anywhere = true;
            Block::new(ui, job, style)
                .paint(ui.painter(), pos2(screen.left() + 16.0, dl_rect.top()));
            dl.paint(ui.painter(), dl_rect.min);
            crate::widgets::link::underline(
                ui.painter(),
                style,
                dl_rect.top(),
                dl_rect.left(),
                dl_rect.right(),
                white,
            );
            let dl_resp = ui.interact(
                dl_rect,
                egui::Id::unique("gelabber-lightbox-dl"),
                Sense::click(),
            );
            let close_resp = ui.interact(
                close_rect,
                egui::Id::unique("gelabber-lightbox-close"),
                Sense::click(),
            );
            ui.painter()
                .circle_filled(close_rect.center(), 20.0, Color32::from_white_alpha(31));
            icons::paint_centered(ui.painter(), Icon::Close, close_rect, 18.0, white);
            if dl_resp.hovered() || close_resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            download = dl_resp.clicked();
            let on_image = ui
                .input(|i| i.pointer.interact_pos())
                .is_some_and(|p| image_rect.is_some_and(|r| r.contains(p)));
            if close_resp.clicked() || (backdrop.clicked() && !on_image) {
                close = true;
            }
        });
    (close, download)
}

/// `gifBox`: at most 320×240, rounded.
pub fn gif_box(gif: &Gif) -> Vec2 {
    let (w, h) = (gif.width.max(1) as f32, gif.height.max(1) as f32);
    let scale = 1f32.min(320.0 / w).min(240.0 / h);
    vec2((w * scale).round().max(1.0), (h * scale).round().max(1.0))
}

/// A GIF message's picture (first frame; the native client does not play it).
pub fn paint_gif(
    ui: &mut Ui,
    images: &Images,
    gif: &Gif,
    url: &str,
    rect: Rect,
    id: egui::Id,
) -> Option<MediaAction> {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    ui.painter().rect_filled(rect, corner(7.0), c.panel);
    match images.get(ui.ctx(), url) {
        ImageState::Ready(texture) => {
            ui.painter().add(
                egui::epaint::RectShape::filled(
                    rect.shrink(1.0),
                    CornerRadius::same(6),
                    Color32::WHITE,
                )
                .with_texture(texture.id(), cover_uv(texture.size_vec2(), rect.size())),
            );
        }
        _ => {
            let block = Block::single(
                ui,
                "GIF",
                TextStyle::new(13.0, 700.0, 1.5).tracking(0.04),
                c.muted,
            );
            block.paint(ui.painter(), rect.center() - block.size() / 2.0);
        }
    }
    ui.painter().rect_stroke(
        rect,
        corner(7.0),
        Stroke::new(1.0, c.border),
        StrokeKind::Inside,
    );
    // Paused: the "GIF" badge bottom-left.
    let badge_text = Block::single(
        ui,
        "GIF",
        TextStyle::new(11.0, 700.0, 1.5).tracking(0.04),
        Color32::WHITE,
    );
    let badge = Rect::from_min_size(
        pos2(
            rect.left() + 6.0,
            rect.bottom() - 6.0 - badge_text.height() - 2.0,
        ),
        vec2(badge_text.width() + 12.0, badge_text.height() + 2.0),
    );
    ui.painter()
        .rect_filled(badge, CornerRadius::same(5), Color32::from_black_alpha(166));
    badge_text.paint(ui.painter(), pos2(badge.left() + 6.0, badge.top() + 1.0));
    let response = ui.interact(rect, id.with("gif"), Sense::click());
    if !gif.title.is_empty() {
        crate::widgets::Tooltip::new(&gif.title)
            .on_hover(&response, crate::widgets::tooltip::Side::Above);
    }
    response
        .clicked()
        .then(|| MediaAction::External(gif.url.clone()))
}

/// Laid-out link previews.
pub struct Previews {
    cards: Vec<(f32, Card)>,
    pub height: f32,
}

struct Card {
    url: String,
    site: Block,
    title: Option<Block>,
    description: Option<Block>,
    image: Option<(String, bool)>,
    youtube: Option<String>,
    size: Vec2,
    text_h: f32,
}

fn host(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| {
            u.host_str()
                .map(|h| h.trim_start_matches("www.").to_owned())
        })
        .unwrap_or_default()
}

pub fn layout_previews(ui: &Ui, previews: &[LinkPreview], width: f32) -> Option<Previews> {
    if previews.is_empty() {
        return None;
    }
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let card_w = width.min(432.0);
    let mut cards = Vec::new();
    let mut y = 0.0;
    for p in previews.iter().take(3) {
        let youtube = p.youtube_id.clone();
        let large = youtube.is_some() || p.image.as_ref().is_some_and(|i| i.width >= 400);
        let thumb = p.image.is_some() && !large;
        let inner = card_w - 3.0 - 1.0 - 24.0;
        let text_w = if thumb { inner - 80.0 - 10.0 } else { inner };
        let site_name = p.site_name.clone().unwrap_or_else(|| host(&p.url));
        let site = Block::wrapped(
            ui,
            &site_name,
            TextStyle::new(12.0, 400.0, 1.5),
            c.muted,
            text_w,
        );
        let title = p.title.as_deref().map(|t| {
            Block::wrapped(
                ui,
                t,
                TextStyle::new(14.0, 600.0, 1.5),
                c.accent_text,
                text_w,
            )
        });
        let description = p.description.as_deref().map(|d| {
            let mut job = crate::text::job(
                d,
                TextStyle::new(13.0, 400.0, 1.4),
                mix(c.muted, c.text, 0.85),
                Some(text_w),
            );
            job.wrap.max_rows = 4;
            Block::new(ui, job, TextStyle::new(13.0, 400.0, 1.4))
        });
        let mut text_h = site.height();
        if let Some(t) = &title {
            text_h += 2.0 + t.height();
        }
        if let Some(d) = &description {
            text_h += 2.0 + d.height();
        }
        let media_h = if youtube.is_some() {
            inner * 9.0 / 16.0
        } else if large {
            p.image
                .as_ref()
                .map(|i| (inner * i.height as f32 / i.width.max(1) as f32).min(160.0))
                .unwrap_or(0.0)
        } else {
            0.0
        };
        let body = if thumb {
            text_h.max(80.0)
        } else if large {
            text_h + 10.0 + media_h
        } else {
            text_h
        };
        let size = vec2(card_w, body + 22.0);
        if !cards.is_empty() {
            y += 6.0;
        }
        cards.push((
            y,
            Card {
                url: p.url.clone(),
                site,
                title,
                description,
                image: p.image.as_ref().map(|i| (i.url.clone(), large)),
                youtube,
                size,
                text_h,
            },
        ));
        y += size.y;
    }
    Some(Previews { cards, height: y })
}

impl Previews {
    pub fn paint(
        &self,
        ui: &mut Ui,
        images: &Images,
        origin: Pos2,
        id: egui::Id,
    ) -> Option<MediaAction> {
        let theme = Theme::of(ui.ctx());
        let c = theme.c;
        let mut action = None;
        for (n, (y, card)) in self.cards.iter().enumerate() {
            let rect = Rect::from_min_size(pos2(origin.x, origin.y + y), card.size);
            let painter = ui.painter();
            painter.rect(
                rect,
                corner(5.0),
                c.panel,
                Stroke::new(1.0, c.border),
                StrokeKind::Inside,
            );
            painter.rect_filled(
                Rect::from_min_max(rect.min, pos2(rect.left() + 3.0, rect.bottom())),
                CornerRadius {
                    nw: 5,
                    sw: 5,
                    ne: 0,
                    se: 0,
                },
                mix(c.border, c.accent, 0.7),
            );
            let x = rect.left() + 3.0 + 12.0;
            let mut ty = rect.top() + 11.0;
            card.site.paint(painter, pos2(x, ty));
            ty += card.site.height() + 2.0;
            if let Some(title) = &card.title {
                let title_rect = Rect::from_min_size(pos2(x, ty), title.size());
                title.paint(ui.painter(), title_rect.min);
                let response =
                    ui.interact(title_rect, id.with(("preview-title", n)), Sense::click());
                if response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    crate::widgets::link::underline(
                        ui.painter(),
                        TextStyle::new(14.0, 600.0, 1.5),
                        ty,
                        x,
                        x + title.width(),
                        c.accent_text,
                    );
                }
                if response.clicked() {
                    action = Some(MediaAction::External(card.url.clone()));
                }
                ty += title.height() + 2.0;
            }
            if let Some(d) = &card.description {
                d.paint(ui.painter(), pos2(x, ty));
            }
            if let Some((url, large)) = &card.image {
                let inner_w = card.size.x - 3.0 - 1.0 - 24.0;
                let img_rect = if *large {
                    Rect::from_min_max(
                        pos2(x, rect.top() + 11.0 + card.text_h + 10.0),
                        pos2(x + inner_w, rect.bottom() - 11.0),
                    )
                } else {
                    Rect::from_min_size(
                        pos2(rect.right() - 13.0 - 80.0, rect.top() + 11.0),
                        vec2(80.0, 80.0),
                    )
                };
                if card.youtube.is_some() {
                    ui.painter()
                        .rect_filled(img_rect, corner(5.0), Color32::BLACK);
                }
                if let ImageState::Ready(t) = images.get(ui.ctx(), url) {
                    ui.painter().add(
                        egui::epaint::RectShape::filled(
                            img_rect,
                            CornerRadius::same(5),
                            Color32::WHITE,
                        )
                        .with_texture(t.id(), cover_uv(t.size_vec2(), img_rect.size())),
                    );
                }
                let response = ui.interact(img_rect, id.with(("preview-image", n)), Sense::click());
                if card.youtube.is_some() {
                    let badge = Rect::from_center_size(img_rect.center(), vec2(56.0, 40.0));
                    let fill = if response.hovered() {
                        Color32::from_rgb(0xcc, 0, 0)
                    } else {
                        Color32::from_black_alpha(178)
                    };
                    ui.painter()
                        .rect_filled(badge, CornerRadius::same(10), fill);
                    let play =
                        Block::single(ui, "▶", TextStyle::new(18.0, 400.0, 1.5), Color32::WHITE);
                    play.paint(ui.painter(), badge.center() - play.size() / 2.0);
                }
                if response.hovered() {
                    ui.ctx().set_cursor_icon(if card.youtube.is_some() {
                        egui::CursorIcon::PointingHand
                    } else {
                        egui::CursorIcon::ZoomIn
                    });
                }
                if response.clicked() {
                    action = Some(if card.youtube.is_some() {
                        MediaAction::External(card.url.clone())
                    } else {
                        MediaAction::Open(
                            url.clone(),
                            card.title
                                .as_ref()
                                .map(|_| String::new())
                                .unwrap_or_default(),
                        )
                    });
                }
            }
        }
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(format_size(1536 * 1024), "1,5 MB");
        assert_eq!(format_size(100), "1 KB");
        assert_eq!(format_size(12 * 1024 + 300), "12 KB");
        let b = image_box(Some(vec2(1920.0, 1080.0)), 656.0);
        assert!(
            (b.x - 360.0).abs() < 0.01 && (b.y - 203.37).abs() < 0.05,
            "{b:?}"
        );
        assert_eq!(image_box(None, 656.0), vec2(360.0, 240.0));
    }
}

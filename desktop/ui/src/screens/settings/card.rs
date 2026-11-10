//! "Profilkarte" (pages/ProfileCardForm.tsx) and the card itself
//! (components/ProfileCard.tsx, profileCard.css) with the avatar
//! decorations of index.css (`.gel-deco-*`), for member dialogs too.

use egui::{
    Color32, Id, Painter, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2,
};
use gelabber_client::{ProfilePatch, User};

use super::controls::{self, Picked};
use super::profile::uploaded;
use super::{adopt_user, op_pending, settled};
use crate::app::AppCtx;
use crate::icons::Icon;
use crate::images::{ImageState, Images};
use crate::rules::{Failure, FormErrors, field_message};
use crate::screens::auth::form_error;
use crate::text::{self, Block, TextStyle};
use crate::theme::{Theme, corner, hex, mix};
use crate::widgets::field::layout_unit;
use crate::widgets::{
    Avatar, AvatarSize, Button, IconButton, InlineStatus, Presence, StatusTone, TextField, eased,
};

pub const BIO_MAX: usize = 190;
pub const PRONOUNS_MAX: usize = 40;
pub const STATUS_TEXT_MAX: usize = 80;
pub const DECORATIONS: [&str; 4] = ["ring", "glow", "rainbow", "stars"];

pub fn decoration_label(d: &str) -> &'static str {
    match d {
        "ring" => "Ring",
        "glow" => "Leuchten",
        "rainbow" => "Regenbogen",
        "stars" => "Sterne",
        _ => "Keiner",
    }
}

/// `safeColor`: `#rrggbb` only.
pub fn safe_color(value: Option<&str>) -> Option<Color32> {
    let v = value?;
    let digits = v.strip_prefix('#')?;
    if digits.len() != 6 || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(digits, 16).ok().map(hex)
}

pub fn color_hex(c: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

/// What a card shows (`CardPerson`).
#[derive(Clone, Debug, Default)]
pub struct CardPerson {
    pub name: String,
    pub avatar_url: Option<String>,
    pub banner_url: Option<String>,
    pub banner_color: Option<String>,
    pub accent_color: Option<String>,
    pub pronouns: Option<String>,
    pub status_text: Option<String>,
    pub bio: Option<String>,
    pub avatar_decoration: Option<String>,
}

const NAME: TextStyle = TextStyle::new(22.0, 650.0, 1.5);
const SMALL: TextStyle = TextStyle::new(13.0, 400.0, 1.5);
const STATUS: TextStyle = text::BODY;
const ABOUT_HEAD: TextStyle = TextStyle::new(11.0, 650.0, 1.5).tracking(0.6 / 11.0);
const ABOUT: TextStyle = TextStyle::new(14.0, 400.0, 1.45);

/// Paints the card `width` wide with its top-left at `min`; returns its
/// height. `presence` adds the dot and the presence line.
pub fn paint_card(
    ui: &Ui,
    images: &Images,
    min: Pos2,
    width: f32,
    person: &CardPerson,
    presence: Option<Presence>,
) -> f32 {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let inner_w = width - 2.0 - 36.0;
    let name = Block::wrapped(ui, &person.name, NAME, c.text, inner_w);
    let mut lines: Vec<(Block, f32)> = Vec::new();
    let mut h = name.height();
    let nonempty = |v: &Option<String>| {
        v.as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(str::to_owned)
    };
    if let Some(p) = nonempty(&person.pronouns) {
        let b = Block::wrapped(ui, &p, SMALL, c.muted, inner_w);
        lines.push((b, h + 2.0));
        h += 2.0 + lines.last().map_or(0.0, |l| l.0.height());
    }
    if let Some(p) = presence {
        let b = Block::wrapped(ui, p.label(), SMALL, c.muted, inner_w);
        lines.push((b, h + 2.0));
        h += 2.0 + lines.last().map_or(0.0, |l| l.0.height());
    }
    let status = nonempty(&person.status_text)
        .map(|t| Block::wrapped(ui, &t, STATUS, c.text, inner_w - 3.0 - 20.0));
    let about = nonempty(&person.bio).map(|t| Block::wrapped(ui, &t, ABOUT, c.text, inner_w));
    let status_h = status.as_ref().map_or(0.0, |b| 12.0 + 16.0 + b.height());
    let about_h = about
        .as_ref()
        .map_or(0.0, |b| 12.0 + ABOUT_HEAD.line_px() + 4.0 + b.height());
    let height = 1.0 + 96.0 + 46.0 + h + status_h + about_h + 18.0 + 1.0;
    let rect = Rect::from_min_size(min, vec2(width, height));
    let painter = ui.painter();
    painter.rect(
        rect,
        corner(14.0),
        c.panel,
        Stroke::NONE,
        StrokeKind::Inside,
    );
    // Banner: the image, else banner colour, else accent, else the mix.
    let banner = Rect::from_min_size(rect.min, vec2(width, 1.0 + 96.0));
    let accent = safe_color(person.accent_color.as_deref());
    let fill = safe_color(person.banner_color.as_deref())
        .or(accent)
        .unwrap_or_else(|| mix(c.panel, c.accent, 0.65));
    let banner_radius = egui::CornerRadius {
        nw: 14,
        ne: 14,
        sw: 0,
        se: 0,
    };
    let texture = person
        .banner_url
        .as_deref()
        .filter(|u| uploaded(u.trim(), "/api/banners/"))
        .and_then(|u| match images.get(ui.ctx(), u.trim()) {
            ImageState::Ready(t) => Some(t),
            _ => None,
        });
    match texture {
        Some(t) => {
            let uv = crate::images::cover_uv(t.size_vec2(), banner.size());
            painter.add(
                egui::epaint::RectShape::filled(banner, banner_radius, Color32::WHITE)
                    .with_texture(t.id(), uv),
            );
        }
        None => {
            painter.rect_filled(banner, banner_radius, fill);
        }
    }
    painter.rect_stroke(
        rect,
        corner(14.0),
        Stroke::new(1.0, c.border),
        StrokeKind::Inside,
    );
    // Avatar: a 72px panel disc at (18, 60) holding the 64px avatar.
    let disc = Rect::from_min_size(
        pos2(rect.left() + 1.0 + 18.0, rect.top() + 1.0 + 60.0),
        vec2(72.0, 72.0),
    );
    painter.circle_filled(disc.center(), 36.0, c.panel);
    let av = disc.shrink(4.0);
    paint_decorated(
        ui,
        images,
        av,
        &person.name,
        person.avatar_url.as_deref(),
        person.avatar_decoration.as_deref(),
        accent,
        c.panel,
        presence,
    );
    let x = rect.left() + 1.0 + 18.0;
    let mut y = rect.top() + 1.0 + 96.0 + 46.0;
    name.paint(painter, pos2(x, y));
    for (b, dy) in &lines {
        b.paint(painter, pos2(x, y + dy));
    }
    y += h;
    if let Some(b) = status {
        y += 12.0;
        let box_rect = Rect::from_min_size(pos2(x, y), vec2(inner_w, 16.0 + b.height()));
        painter.rect_filled(box_rect, corner(8.0), c.surface);
        let bar = Rect::from_min_max(box_rect.min, pos2(box_rect.left() + 3.0, box_rect.bottom()));
        painter
            .with_clip_rect(bar)
            .rect_filled(box_rect, corner(8.0), accent.unwrap_or(c.accent));
        b.paint(painter, pos2(x + 13.0, y + 8.0));
        y += box_rect.height();
    }
    if let Some(b) = about {
        y += 12.0;
        Block::single(ui, "ÜBER MICH", ABOUT_HEAD, c.muted).paint(painter, pos2(x, y));
        y += ABOUT_HEAD.line_px() + 4.0;
        b.paint(painter, pos2(x, y));
    }
    height
}

/// An avatar in `rect` with its decoration around it; `gap` is the surface
/// the gap rings show (`--lr-bg` where the avatar sits).
#[allow(clippy::too_many_arguments)]
pub fn paint_decorated(
    ui: &Ui,
    images: &Images,
    rect: Rect,
    name: &str,
    url: Option<&str>,
    decoration: Option<&str>,
    accent: Option<Color32>,
    gap: Color32,
    presence: Option<Presence>,
) {
    let theme = Theme::of(ui.ctx());
    let color = accent.unwrap_or(theme.c.accent);
    let r = rect.width() / 2.0;
    let center = rect.center();
    let painter = ui.painter();
    match decoration {
        Some("ring") => {
            painter.circle_filled(center, r + 4.0, color);
            painter.circle_filled(center, r + 2.0, gap);
        }
        Some("glow") => {
            for i in 0..8 {
                let t = i as f32 / 8.0;
                let alpha = 0.55 * (1.0 - t) * 0.16;
                painter.circle_filled(center, r + 3.0 + 10.0 * t, color.gamma_multiply(alpha));
            }
            painter.circle_filled(center, r + 2.0, color);
        }
        Some("rainbow") => {
            const STOPS: [u32; 7] = [
                0xff5f6d, 0xffc371, 0xf9f871, 0x5ee7a1, 0x4fb3ff, 0xa07cff, 0xff5f6d,
            ];
            let outer = r + 3.0;
            let n = 72;
            let mut mesh = egui::Mesh::default();
            for i in 0..n {
                let a0 = i as f32 / n as f32;
                let a1 = (i + 1) as f32 / n as f32;
                let col = |t: f32| {
                    let seg = (t * 6.0).min(5.999);
                    let k = seg.floor() as usize;
                    crate::widgets::lerp(hex(STOPS[k]), hex(STOPS[k + 1]), seg - k as f32)
                };
                // Conic gradients start at 12 o'clock and run clockwise.
                let p = |t: f32| {
                    let ang = t * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
                    center + vec2(ang.cos(), ang.sin()) * outer
                };
                let base = mesh.vertices.len() as u32;
                mesh.colored_vertex(center, col(a0));
                mesh.colored_vertex(p(a0), col(a0));
                mesh.colored_vertex(p(a1), col(a1));
                mesh.add_triangle(base, base + 1, base + 2);
            }
            painter.add(Shape::mesh(mesh));
            painter.circle_filled(center, r + 1.5, gap);
        }
        Some("stars") => {
            painter.circle_filled(center, r + 3.5, color);
            painter.circle_filled(center, r + 2.0, gap);
        }
        _ => {}
    }
    let avatar = Avatar::new(name)
        .image(url, images)
        .size(AvatarSize::Px(rect.width().round() as u16))
        .presence(presence)
        .ring(gap);
    avatar.paint(ui, rect);
    if decoration == Some("stars") {
        let big = (rect.width() * 0.36).max(10.0);
        let small = (rect.width() * 0.24).max(8.0);
        // `top -14%; right -10%` and `bottom -6%; left -12%`, glyph boxes.
        let a = pos2(
            rect.right() + rect.width() * 0.10 - big * 0.42,
            rect.top() - rect.width() * 0.14 + big * 0.5,
        );
        let b = pos2(
            rect.left() - rect.width() * 0.12 + small * 0.42,
            rect.bottom() + rect.width() * 0.06 - small * 0.5,
        );
        star(painter, a, big * 0.36, color, gap);
        star(painter, b, small * 0.36, color, gap);
    }
}

/// "✦": a four-pointed star with a thin halo in the surface colour.
fn star(painter: &Painter, center: Pos2, r: f32, color: Color32, halo: Color32) {
    let points = |r: f32| -> Vec<Pos2> {
        (0..8)
            .map(|i| {
                let ang = i as f32 * std::f32::consts::FRAC_PI_4 - std::f32::consts::FRAC_PI_2;
                let rr = if i % 2 == 0 { r } else { r * 0.3 };
                center + vec2(ang.cos(), ang.sin()) * rr
            })
            .collect()
    };
    for (pts, col) in [
        (points(r + 1.0), halo.gamma_multiply(0.6)),
        (points(r), color),
    ] {
        // Concave: a fan of triangles from the centre.
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(center, col);
        for p in &pts {
            mesh.colored_vertex(*p, col);
        }
        for i in 0..pts.len() as u32 {
            mesh.add_triangle(0, 1 + i, 1 + (i + 1) % pts.len() as u32);
        }
        painter.add(Shape::mesh(mesh));
    }
}

// ----------------------------------------------------------------- form

/// `CardForm`: the editable fields as strings ("" = unset).
#[derive(Clone, Debug, Default, PartialEq)]
struct Form {
    bio: String,
    pronouns: String,
    status_text: String,
    accent_color: String,
    banner_color: String,
    avatar_decoration: String,
}

impl Form {
    fn of(u: &User) -> Self {
        Self {
            bio: u.bio.clone().unwrap_or_default(),
            pronouns: u.pronouns.clone().unwrap_or_default(),
            status_text: u.status_text.clone().unwrap_or_default(),
            accent_color: u.accent_color.clone().unwrap_or_default(),
            banner_color: u.banner_color.clone().unwrap_or_default(),
            avatar_decoration: u.avatar_decoration.clone().unwrap_or_default(),
        }
    }

    /// `profileCardPatch`: only the fields that differ, normalised.
    fn patch(&self, saved: &User) -> ProfilePatch {
        let norm = |s: &str| s.replace("\r\n", "\n").trim().to_owned();
        let lower = |s: &str| norm(s).to_lowercase();
        let differ =
            |v: String, old: &Option<String>| (v != old.clone().unwrap_or_default()).then_some(v);
        ProfilePatch {
            bio: differ(norm(&self.bio), &saved.bio),
            pronouns: differ(norm(&self.pronouns), &saved.pronouns),
            status_text: differ(norm(&self.status_text), &saved.status_text),
            accent_color: differ(lower(&self.accent_color), &saved.accent_color),
            banner_color: differ(lower(&self.banner_color), &saved.banner_color),
            avatar_decoration: differ(norm(&self.avatar_decoration), &saved.avatar_decoration),
            ..ProfilePatch::default()
        }
    }

    fn validate(&self) -> Vec<(&'static str, String)> {
        let mut out = Vec::new();
        let text = |raw: &str, max: usize, multiline: bool| -> Option<&'static str> {
            let v = raw.replace("\r\n", "\n");
            let v = v.trim();
            if v.is_empty() {
                return None;
            }
            if v.chars().count() > max {
                return Some("too_long");
            }
            v.chars()
                .any(|c| !(multiline && c == '\n') && is_control(c))
                .then_some("invalid")
        };
        for (field, code) in [
            ("bio", text(&self.bio, BIO_MAX, true)),
            ("pronouns", text(&self.pronouns, PRONOUNS_MAX, false)),
            (
                "status_text",
                text(&self.status_text, STATUS_TEXT_MAX, false),
            ),
            ("accent_color", color_code(&self.accent_color)),
            ("banner_color", color_code(&self.banner_color)),
        ] {
            if let Some(code) = code {
                out.push((field, field_message(field, code)));
            }
        }
        out
    }
}

fn color_code(raw: &str) -> Option<&'static str> {
    let v = raw.trim();
    (!v.is_empty() && safe_color(Some(v)).is_none()).then_some("invalid")
}

fn is_control(c: char) -> bool {
    let n = c as u32;
    n < 0x20
        || (0x7f..=0x9f).contains(&n)
        || (0x202a..=0x202e).contains(&n)
        || (0x2066..=0x2069).contains(&n)
}

fn is_empty_patch(p: &ProfilePatch) -> bool {
    p == &ProfilePatch::default()
}

#[derive(Default)]
pub struct CardState {
    form: Option<Form>,
    /// Read by the navigation guard.
    pub dirty: bool,
    saved: bool,
    banner_saved: bool,
    errors: FormErrors,
    running: Option<&'static str>,
    picking: Option<Picked>,
    /// The picked or dropped banner, read and cropped off the UI thread.
    preparing: Option<controls::Prepared>,
    /// Which colour picker is open.
    picker: Option<&'static str>,
    /// The preview's height last frame, for its sticky stop.
    card_h: f32,
}

const ORDER: [&str; 6] = [
    "banner_url",
    "banner_color",
    "accent_color",
    "pronouns",
    "status_text",
    "bio",
];

/// Settings area "Profilkarte": the form left, the live card right.
pub fn area(ui: &mut Ui, app: &mut AppCtx, user: &User, s: &mut CardState, viewport: Rect) {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let mut form = s.form.take().unwrap_or_else(|| Form::of(user));
    // Settled writes.
    if let Some(which) = s.running {
        // Saves and banner removals are both profile writes.
        if which != "upload"
            && let Some(result) = settled(app, "profile")
        {
            s.running = None;
            match result {
                Ok(()) => {
                    if let Some(updated) = app.client.with_state(|st| st.me().cloned()) {
                        adopt_user(app, &updated);
                        if which == "save" {
                            form = Form::of(&updated);
                            s.saved = true;
                        } else {
                            s.banner_saved = true;
                            s.errors.clear_field("banner_url");
                        }
                    }
                }
                Err(e) => s.errors.from_failure(&Failure::from_api(&e), &ORDER),
            }
        }
    }
    // The upload's answer is in the client's session, for every view of it.
    if s.running == Some("upload")
        && s.preparing.is_none()
        && let Some(result) = settled(app, "banner")
    {
        s.running = None;
        match result {
            Ok(()) => {
                if let Some(updated) = app.client.with_state(|st| st.me().cloned()) {
                    adopt_user(app, &updated);
                }
                s.banner_saved = true;
                s.errors.clear_field("banner_url");
            }
            Err(e) => s.errors.from_failure(&Failure::from_api(&e), &ORDER),
        }
    }
    if let Some(slot) = &s.picking {
        let done = slot.lock().take();
        if let Some(file) = done {
            s.picking = None;
            if let Some((bytes, _)) = file {
                prepare_banner(ui.ctx(), s, controls::ImageFile::Bytes(bytes));
            }
        }
    }
    if s.running.is_none()
        && let Some(file) = controls::dropped_image(ui.ctx())
    {
        prepare_banner(ui.ctx(), s, file);
    }
    if let Some(prepared) = crate::task::poll(&mut s.preparing) {
        match prepared {
            Ok((data, kind)) => app.client.upload_banner(kind, data),
            Err(message) => {
                s.running = None;
                s.errors.set_fields(vec![("banner_url", message)]);
            }
        }
    }
    // The client may hold a newer user than the one passed.
    let user = app
        .client
        .with_state(|st| st.me().cloned())
        .filter(|u| u.id == user.id)
        .unwrap_or_else(|| user.clone());
    let busy = s.running.is_some() || op_pending(app, "profile") || op_pending(app, "banner");
    let patch = form.patch(&user);
    let dirty = !is_empty_patch(&patch);
    s.dirty = dirty;

    let top = ui.cursor().min;
    let total_w = ui.available_width();
    // ≥ 760px: form `1fr`, gap 40, preview 320.
    let form_w = total_w - 40.0 - 320.0;
    let mut save_clicked = false;
    let mut discard = false;
    controls::column(ui, form_w, |ui| {
        // Banner.
        controls::paint(ui, ui.cursor().min, "Banner", text::UI, c.text);
        ui.add_space(layout_unit(text::UI.line_px()) + 6.0);
        controls::row(ui, 8.0, 32.0, |ui| {
            let uploading = s.running == Some("upload");
            if ui
                .add(
                    Button::new("Bannerbild hochladen")
                        .small()
                        .disabled(busy && !uploading)
                        .pending(uploading.then_some("Lädt hoch…")),
                )
                .clicked()
                && !busy
                && s.picking.is_none()
            {
                s.picking = Some(controls::pick_file(
                    ui.ctx(),
                    "Bannerbild auswählen",
                    &controls::IMAGE_FILTER,
                ));
            }
            if user.banner_url.is_some() {
                let removing = s.running == Some("remove");
                if ui
                    .add(
                        Button::new("Bannerbild entfernen")
                            .small()
                            .ghost()
                            .disabled(busy && !removing)
                            .pending(removing.then_some("Entfernt…")),
                    )
                    .clicked()
                    && !busy
                {
                    s.banner_saved = false;
                    s.running = Some("remove");
                    app.client.update_profile(ProfilePatch {
                        banner_url: Some(String::new()),
                        ..ProfilePatch::default()
                    });
                }
            }
            if s.banner_saved {
                ui.add(InlineStatus::new(
                    Some("Banner gespeichert."),
                    StatusTone::Success,
                ));
            }
        });
        ui.add_space(6.0);
        match s.errors.field("banner_url") {
            Some(e) => controls::paragraph(ui, e, text::META, c.danger, form_w),
            None => controls::paragraph(
                ui,
                "Wird auf 3 : 1 zugeschnitten und sofort gespeichert.",
                text::META,
                c.muted,
                form_w,
            ),
        };
        ui.add_space(20.0);

        // Two colours side by side.
        let pair_top = ui.cursor().min;
        let half = (form_w - 20.0) / 2.0;
        let unset_banner =
            safe_color(Some(&form.accent_color)).unwrap_or_else(|| mix(c.panel, c.accent, 0.65));
        let fallback_banner = safe_color(Some(&form.accent_color));
        let mut h = 0.0_f32;
        for (i, (field, label)) in [
            ("banner_color", "Bannerfarbe"),
            ("accent_color", "Akzentfarbe"),
        ]
        .into_iter()
        .enumerate()
        {
            let rect = Rect::from_min_size(
                pos2(pair_top.x + i as f32 * (half + 20.0), pair_top.y),
                vec2(half, 200.0),
            );
            let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
            child.spacing_mut().item_spacing = egui::Vec2::ZERO;
            let (value, unset, start) = if field == "banner_color" {
                (&mut form.banner_color, unset_banner, fallback_banner)
            } else {
                (&mut form.accent_color, c.accent, None)
            };
            if color_field(&mut child, s, field, label, value, unset, start) {
                s.saved = false;
                s.errors.clear_field(field);
            }
            h = h.max(child.min_rect().height());
        }
        ui.advance_cursor_after_rect(Rect::from_min_size(pair_top, vec2(form_w, h)));
        ui.add_space(20.0);

        // Avatar decorations.
        controls::paint(ui, ui.cursor().min, "Avatar-Rahmen", text::UI, c.text);
        ui.add_space(layout_unit(text::UI.line_px()) + 8.0);
        let cols = ((form_w + 8.0) / (76.0 + 8.0)).floor().max(1.0);
        let cell = (form_w - 8.0 * (cols - 1.0)) / cols;
        let grid_top = ui.cursor().min;
        let options: Vec<&str> = std::iter::once("").chain(DECORATIONS).collect();
        let rows = (options.len() as f32 / cols).ceil();
        for (i, deco) in options.iter().enumerate() {
            let col = (i as f32) % cols;
            let row = (i as f32 / cols).floor();
            let rect = Rect::from_min_size(
                pos2(grid_top.x + col * (cell + 8.0), grid_top.y + row * 92.0),
                vec2(cell, 84.0),
            );
            // Blink snaps the box edges, the content keeps its place.
            let frame = crate::widgets::snap(ui.ctx(), rect);
            let id = Id::unique(("gelabber-deco", *deco));
            let response = ui.interact(rect, id, Sense::click());
            let checked = form.avatar_decoration == *deco;
            let fill = eased(
                ui.ctx(),
                id,
                response.hovered(),
                Color32::TRANSPARENT,
                c.hover,
            );
            let painter = ui.painter();
            painter.rect(
                frame,
                corner(theme.radius.md),
                fill,
                Stroke::new(1.0, if checked { c.accent } else { c.border }),
                StrokeKind::Inside,
            );
            if checked {
                painter.rect_stroke(
                    frame.expand(0.5),
                    corner(theme.radius.md + 0.5),
                    Stroke::new(1.0, c.accent),
                    StrokeKind::Middle,
                );
            }
            let av = Rect::from_min_size(
                pos2(rect.center().x - 18.0, rect.top() + 13.0),
                vec2(36.0, 36.0),
            );
            paint_decorated(
                ui,
                app.images,
                av,
                &user.name,
                user.avatar_url.as_deref(),
                Some(deco).filter(|d| !d.is_empty()).copied(),
                safe_color(Some(&form.accent_color)),
                c.bg,
                None,
            );
            let label = Block::single(
                ui,
                decoration_label(deco),
                text::META.line_height(1.5),
                c.text,
            );
            label.paint(
                ui.painter(),
                pos2(rect.center().x - label.width() / 2.0, rect.top() + 57.0),
            );
            if response.clicked() && !checked {
                form.avatar_decoration = (*deco).to_owned();
                s.saved = false;
            }
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
        }
        ui.advance_cursor_after_rect(Rect::from_min_size(
            grid_top,
            vec2(form_w, rows * 84.0 + (rows - 1.0) * 8.0),
        ));
        ui.add_space(20.0);

        let pronouns = TextField::new("pronouns", &mut form.pronouns)
            .label("Pronomen")
            .char_limit(PRONOUNS_MAX)
            .placeholder("z. B. sie/ihr")
            .error(s.errors.field("pronouns"))
            .show(ui);
        if pronouns.response.changed() {
            s.saved = false;
            s.errors.clear_field("pronouns");
        }
        ui.add_space(20.0);
        let status = TextField::new("status_text", &mut form.status_text)
            .label("Status")
            .char_limit(STATUS_TEXT_MAX)
            .error(s.errors.field("status_text"))
            .hint(Some("Steht in der Mitgliederliste unter deinem Namen."))
            .show(ui);
        if status.response.changed() {
            s.saved = false;
            s.errors.clear_field("status_text");
        }
        ui.add_space(20.0);
        controls::paint(ui, ui.cursor().min, "Über mich", text::UI, c.text);
        ui.add_space(layout_unit(text::UI.line_px()) + 6.0);
        let len = form.bio.replace("\r\n", "\n").trim().chars().count();
        let invalid = s.errors.field("bio").is_some() || len > BIO_MAX;
        if controls::textarea(ui, "bio", "Über mich", &mut form.bio, 100.0, invalid).changed() {
            s.saved = false;
            s.errors.clear_field("bio");
        }
        if let Some(e) = s.errors.field("bio") {
            ui.add_space(6.0);
            controls::paragraph(ui, e, text::META, c.danger, form_w);
        }
        ui.add_space(6.0);
        controls::paragraph(
            ui,
            &format!("{len} / {BIO_MAX} Zeichen"),
            text::META,
            if len > BIO_MAX { c.danger } else { c.muted },
            form_w,
        );
        ui.add_space(20.0);
        form_error(ui, &s.errors);

        // The save bar: sticky at the bottom of the view while dirty.
        let saving = s.running == Some("save");
        if dirty || saving {
            let natural = ui.cursor().min;
            let bar_h = 10.0 + 36.0 + 10.0 + 2.0;
            let y = natural
                .y
                .min(viewport.bottom() - 12.0 - bar_h)
                .max(natural.y.min(viewport.top()));
            let rect = Rect::from_min_size(pos2(natural.x, y), vec2(form_w, bar_h));
            let painter = ui.painter();
            painter.add(theme.shadow_2.as_shape(rect, corner(theme.radius.lg)));
            painter.rect(
                rect,
                corner(theme.radius.lg),
                c.elevated,
                Stroke::new(1.0, c.border),
                StrokeKind::Inside,
            );
            controls::paint(
                ui,
                pos2(
                    rect.left() + 17.0,
                    rect.center().y - text::UI.line_px() / 2.0,
                ),
                "Ungespeicherte Änderungen",
                text::UI,
                c.text,
            );
            let buttons = Rect::from_min_max(
                pos2(rect.left() + 16.0, rect.top() + 11.0),
                pos2(rect.right() - 13.0, rect.bottom() - 11.0),
            );
            controls::place(
                ui,
                UiBuilder::new()
                    .max_rect(buttons)
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    save_clicked = ui
                        .add(
                            Button::new("Speichern")
                                .primary()
                                .disabled(busy && !saving)
                                .pending(saving.then_some("Speichern…")),
                        )
                        .clicked();
                    discard = ui
                        .add(Button::new("Verwerfen").ghost().disabled(saving))
                        .clicked();
                },
            );
            ui.advance_cursor_after_rect(Rect::from_min_size(natural, vec2(form_w, bar_h)));
        } else if s.saved {
            ui.add(InlineStatus::new(Some("Gespeichert."), StatusTone::Success));
        }
    });
    let form_bottom = ui.cursor().min.y;
    if discard {
        form = Form::of(&user);
        s.errors = FormErrors::default();
    }
    if save_clicked && !busy {
        s.saved = false;
        let fields = form.validate();
        if fields.is_empty() {
            s.running = Some("save");
            app.client.update_profile(form.patch(&user));
        } else {
            s.errors.set_fields(fields);
        }
    }
    // ProfileCardForm has no form ref, so focus stays where it was.
    s.errors.focus = None;

    // The preview sticks 24px under the top while the form scrolls.
    let person = CardPerson {
        name: user.name.clone(),
        avatar_url: user.avatar_url.clone(),
        banner_url: user.banner_url.clone(),
        banner_color: Some(form.banner_color.clone()).filter(|v| !v.is_empty()),
        accent_color: Some(form.accent_color.clone()).filter(|v| !v.is_empty()),
        pronouns: Some(form.pronouns.clone()),
        status_text: Some(form.status_text.clone()),
        bio: Some(form.bio.clone()),
        avatar_decoration: Some(form.avatar_decoration.clone()).filter(|v| !v.is_empty()),
    };
    let x = top.x + form_w + 40.0;
    // Sticky inside the editor: it stops where the form ends.
    let y = top
        .y
        .max(viewport.top() + 24.0)
        .min(form_bottom - s.card_h)
        .max(top.y);
    s.card_h = paint_card(ui, app.images, pos2(x, y), 320.0, &person, None);
    s.form = Some(form);
}

/// Reads and crops the banner off the UI thread; the upload follows.
fn prepare_banner(ctx: &egui::Context, s: &mut CardState, file: controls::ImageFile) {
    s.banner_saved = false;
    s.running = Some("upload");
    s.preparing = Some(controls::prepare(ctx, file, 960, 320));
}

/// `ColorField`: label, a 48×36 swatch that opens a picker, the value or
/// "Standard", and a reset button once set.
fn color_field(
    ui: &mut Ui,
    s: &mut CardState,
    field: &'static str,
    label: &str,
    value: &mut String,
    unset: Color32,
    start: Option<Color32>,
) -> bool {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let mut changed = false;
    controls::paint(ui, ui.cursor().min, label, text::UI, c.text);
    ui.add_space(layout_unit(text::UI.line_px()) + 6.0);
    let row_top = ui.cursor().min;
    let swatch = Rect::from_min_size(row_top, vec2(48.0, 36.0));
    let id = Id::unique(("gelabber-color-swatch", field));
    let response = ui.interact(swatch, id, Sense::click());
    let current = safe_color(Some(value.as_str()));
    let painter = ui.painter();
    painter.rect(
        swatch,
        corner(theme.radius.sm),
        Color32::TRANSPARENT,
        Stroke::new(1.0, c.border_strong),
        StrokeKind::Inside,
    );
    // The colour well inside Chromium's swatch: 4px in, a 1px grey edge.
    let well = swatch.shrink2(vec2(7.0, 7.0));
    painter.rect(
        well,
        0.0,
        current.unwrap_or(unset),
        Stroke::new(1.0, hex(0x777777)),
        StrokeKind::Inside,
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if response.clicked() {
        s.picker = if s.picker == Some(field) {
            None
        } else {
            Some(field)
        };
    }
    let shown = value.trim().to_lowercase();
    let text_block = Block::single(
        ui,
        if shown.is_empty() { "Standard" } else { &shown },
        controls::HINT,
        c.muted,
    );
    text_block.paint(ui.painter(), pos2(swatch.right() + 10.0, row_top.y + 9.0));
    if !shown.is_empty() {
        let reset = Rect::from_min_size(
            pos2(
                swatch.right() + 10.0 + text_block.width() + 10.0,
                row_top.y + 2.0,
            ),
            vec2(32.0, 32.0),
        );
        if ui
            .add(
                IconButton::new(Icon::Close, &format!("{label} zurücksetzen"))
                    .size(32.0)
                    .at(reset),
            )
            .clicked()
        {
            value.clear();
            changed = true;
        }
    }
    ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
    if s.picker == Some(field) {
        let mut color = current.or(start).unwrap_or(hex(0x7c6cf2));
        let open = crate::widgets::Popover::new(("color", field), swatch).show(ui.ctx(), |ui| {
            egui::widgets::color_picker::color_picker_color32(
                ui,
                &mut color,
                egui::widgets::color_picker::Alpha::Opaque,
            )
        });
        match open {
            Some(true) => {
                *value = color_hex(color);
                changed = true;
            }
            Some(false) => {}
            None => s.picker = None,
        }
    }
    if let Some(e) = s.errors.field(field) {
        ui.add_space(6.0);
        let w = ui.available_width();
        controls::paragraph(ui, e, text::META, c.danger, w);
    }
    changed
}

#[cfg(test)]
mod tests {
    use crate::app::testkit::{FakeApi, Reply, lena, wait_for};
    use crate::platform::paths::testing::TempConfig;

    #[derive(Debug)]
    struct Dropped(std::path::PathBuf);
    impl egui::DroppedFile for Dropped {
        fn path(&self) -> &std::path::Path {
            &self.0
        }
        fn bytes(&self) -> Result<Vec<u8>, String> {
            panic!("read on the UI thread")
        }
    }

    /// A dropped banner lands in the client's session, not only in the
    /// card: the rest of the app (the user panel, profile popouts) reads it there.
    #[test]
    fn an_uploaded_banner_reaches_the_session() {
        let config = TempConfig::new("banner-upload");
        let api = FakeApi::start(true, |method, path, _| {
            (method == "PUT" && path == "/me/banner").then(|| {
                let mut user = lena();
                user["banner_url"] = "/api/banners/neu.png".into();
                Reply::json(user)
            })
        });
        let client = gelabber_client::Client::new(&api.origin).expect("client");
        let watched = client.clone();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .with_max_steps(10_000)
            .with_step_dt(1.0 / 60.0)
            .build_eframe(move |cc| {
                crate::Launcher::with_app(cc, client.clone(), Some("/settings/profilkarte".into()))
            });
        wait_for(&mut harness, "the card", |h| {
            use egui_kittest::kittest::Queryable;
            h.query_by_label("Bannerbild hochladen").is_some()
        });
        let file = config.dir.join("banner.png");
        image::RgbaImage::from_pixel(96, 32, image::Rgba([200, 40, 40, 255]))
            .save(&file)
            .expect("png");
        harness
            .input_mut()
            .dropped_files
            .push(std::sync::Arc::new(Dropped(file)));
        let banner = || watched.with_state(|s| s.me().and_then(|u| u.banner_url.clone()));
        wait_for(&mut harness, "the banner in the session", |_| {
            banner().as_deref() == Some("/api/banners/neu.png")
        });
        assert!(api.saw("PUT /me/banner"));
    }
}

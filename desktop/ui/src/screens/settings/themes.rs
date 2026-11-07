//! Themes: the built-in palettes (theme/presets.ts), the account's theme
//! document (`/me/themes`, theme/account.ts) and which theme the window
//! shows, and the "Themes" area (theme/ThemesPanel.tsx, ThemePreview.tsx,
//! themes.css): gallery, large preview, editor, import and export.

use std::sync::Arc;

use egui::{Color32, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};
use gelabber_client::{ApiError, Client, ThemeColors, ThemeDocument};
use parking_lot::Mutex;

use super::controls::{self, Confirm, Picked, Status};
use super::op_pending;
use crate::app::AppCtx;
use crate::icons::{self, Icon};
use crate::task::{Pending, poll};
use crate::text::{self, Block, TextStyle};
use crate::theme::{self, BaseColors, Definition, Mode, Style, Theme, corner, hex};
use crate::widgets::{Button, ButtonVariant, Link, eased, focus_ring, keyboard_focus, link};

const MAX_THEMES: usize = 50;
const MAX_IMPORT_BYTES: usize = 64 * 1024;

// ---------------------------------------------------------------- presets

fn colors(v: [u32; 8]) -> BaseColors {
    BaseColors {
        background: hex(v[0]),
        panel: hex(v[1]),
        surface: hex(v[2]),
        rail: hex(v[3]),
        text: hex(v[4]),
        muted: hex(v[5]),
        accent: hex(v[6]),
        border: hex(v[7]),
    }
}

fn def(id: &str, name: &str, mode: Mode, style: Style, c: [u32; 8]) -> Definition {
    Definition {
        id: id.into(),
        name: name.into(),
        mode,
        style,
        colors: colors(c),
    }
}

/// `BUILTIN_THEMES`, contrast already fixed (computed once).
pub fn presets() -> Vec<Definition> {
    static PRESETS: std::sync::OnceLock<Vec<Definition>> = std::sync::OnceLock::new();
    PRESETS.get_or_init(build_presets).clone()
}

fn build_presets() -> Vec<Definition> {
    use Mode::{Dark, Light};
    use Style::{Clear, Soft, Terminal};
    [
        Definition::dark(),
        Definition::light(),
        def(
            "catppuccin",
            "Catppuccin",
            Dark,
            Soft,
            [
                0x1e1e2e, 0x161622, 0x313244, 0x101019, 0xcdd6f4, 0xbac2de, 0x89b4fa, 0x45475a,
            ],
        ),
        def(
            "catppuccin-latte",
            "Catppuccin Latte",
            Light,
            Soft,
            [
                0xeff1f5, 0xe3e4e8, 0xdce0e8, 0xd7d8dc, 0x4c4f69, 0x5c5f77, 0x1e66f5, 0xccd0da,
            ],
        ),
        def(
            "nord",
            "Nord",
            Dark,
            Clear,
            [
                0x2e3440, 0x222730, 0x3b4252, 0x191c23, 0xd8dee9, 0xadb5c4, 0x81a1c1, 0x434c5e,
            ],
        ),
        def(
            "gruvbox",
            "Gruvbox",
            Dark,
            Terminal,
            [
                0x282828, 0x1e1e1e, 0x3c3836, 0x161616, 0xd4be98, 0xbdae93, 0x7daea3, 0x504945,
            ],
        ),
        def(
            "tokyo-night",
            "Tokyo Night",
            Dark,
            Clear,
            [
                0x1a1b26, 0x13141c, 0x24283b, 0x0e0e14, 0xa9b1d6, 0xb4bee6, 0x7aa2f7, 0x292e42,
            ],
        ),
        def(
            "everforest",
            "Everforest",
            Dark,
            Soft,
            [
                0x2d353b, 0x21272c, 0x343f44, 0x181d20, 0xd3c6aa, 0x9da9a0, 0x7fbbb3, 0x3d484d,
            ],
        ),
        def(
            "kanagawa",
            "Kanagawa",
            Dark,
            Terminal,
            [
                0x1f1f28, 0x17171e, 0x223249, 0x111116, 0xdcd7ba, 0xc8c093, 0xdcd7ba, 0x363646,
            ],
        ),
        def(
            "rose-pine",
            "Rosé Pine Dawn",
            Light,
            Soft,
            [
                0xfaf4ed, 0xede7e1, 0xf2e9e1, 0xe1dbd5, 0x575279, 0x6e6a86, 0x56949f, 0xdfdad9,
            ],
        ),
        def(
            "flexoki-light",
            "Flexoki Light",
            Light,
            Clear,
            [
                0xfffcf0, 0xf2efe4, 0xe6e4d9, 0xe5e2d8, 0x100f0f, 0x403e3c, 0x205ea6, 0xcecdc3,
            ],
        ),
        def(
            "osaka-jade",
            "Osaka Jade",
            Dark,
            Terminal,
            [
                0x111c18, 0x0c1512, 0x23372b, 0x090f0d, 0xc1c497, 0xd6d5bc, 0x509475, 0x32473b,
            ],
        ),
        def(
            "matte-black",
            "Matte Black",
            Dark,
            Clear,
            [
                0x121212, 0x0d0d0d, 0x1e1e1e, 0x090909, 0xbebebe, 0x8a8a8d, 0xe68e0d, 0x2a2a2a,
            ],
        ),
        def(
            "ristretto",
            "Ristretto",
            Dark,
            Soft,
            [
                0x2c2525, 0x211b1b, 0x3d2f2a, 0x181414, 0xe6d9db, 0xc3b7b8, 0xf38d70, 0x403e41,
            ],
        ),
    ]
    .into_iter()
    .map(theme::fixed)
    .collect()
}

/// `builtinTheme`: the preset, or "Gelabber Dunkel".
pub fn preset(id: &str) -> Definition {
    presets()
        .into_iter()
        .find(|t| t.id == id)
        .unwrap_or_else(Definition::dark)
}

fn is_builtin(id: &str) -> bool {
    id == "system" || presets().iter().any(|t| t.id == id)
}

// ------------------------------------------------------------ conversion

fn parse_hex(s: &str) -> Option<Color32> {
    super::card::safe_color(Some(s))
}

fn hex_of(c: Color32) -> String {
    super::card::color_hex(c)
}

/// A stored theme as a definition; `None` for a malformed one.
pub fn definition_of(t: &gelabber_client::Theme) -> Option<Definition> {
    let c = &t.colors;
    Some(Definition {
        id: t.id.clone(),
        name: t.name.clone(),
        mode: if t.mode == "light" {
            Mode::Light
        } else {
            Mode::Dark
        },
        style: match t.style.as_str() {
            "soft" => Style::Soft,
            "terminal" => Style::Terminal,
            _ => Style::Clear,
        },
        colors: BaseColors {
            background: parse_hex(&c.background)?,
            panel: parse_hex(&c.panel)?,
            surface: parse_hex(&c.surface)?,
            rail: parse_hex(&c.rail)?,
            text: parse_hex(&c.text)?,
            muted: parse_hex(&c.muted)?,
            accent: parse_hex(&c.accent)?,
            border: parse_hex(&c.border)?,
        },
    })
}

pub fn stored_of(d: &Definition) -> gelabber_client::Theme {
    let c = &d.colors;
    gelabber_client::Theme {
        version: 1,
        id: d.id.clone(),
        name: d.name.clone(),
        mode: if d.mode == Mode::Light {
            "light"
        } else {
            "dark"
        }
        .into(),
        style: match d.style {
            Style::Clear => "clear",
            Style::Soft => "soft",
            Style::Terminal => "terminal",
        }
        .into(),
        colors: ThemeColors {
            background: hex_of(c.background),
            panel: hex_of(c.panel),
            surface: hex_of(c.surface),
            rail: hex_of(c.rail),
            text: hex_of(c.text),
            muted: hex_of(c.muted),
            accent: hex_of(c.accent),
            border: hex_of(c.border),
        },
    }
}

fn system_dark(ctx: &egui::Context) -> bool {
    crate::theme::system_dark(ctx)
}

/// The definition behind a theme id of `doc` ("system" follows the OS).
fn resolve(ctx: &egui::Context, doc: &ThemeDocument, id: &str) -> Definition {
    if id == "system" {
        return preset(if system_dark(ctx) { "dark" } else { "light" });
    }
    if let Some(d) = presets().into_iter().find(|t| t.id == id) {
        return d;
    }
    doc.custom_themes
        .iter()
        .find(|t| t.id == id)
        .and_then(definition_of)
        .unwrap_or_else(Definition::dark)
}

/// The name of `id` as the gallery says it.
pub fn theme_name(doc: &ThemeDocument, id: &str) -> String {
    if id == "system" {
        return "System".into();
    }
    presets()
        .into_iter()
        .find(|t| t.id == id)
        .map(|t| t.name)
        .or_else(|| {
            doc.custom_themes
                .iter()
                .find(|t| t.id == id)
                .map(|t| t.name.clone())
        })
        .unwrap_or_else(|| "Gelabber Dunkel".into())
}

// ------------------------------------------------------------------ sync

/// A change to the account's themes (`Operation` of account.ts).
#[derive(Clone, Debug)]
pub enum Op {
    Apply(String),
    Save(Box<gelabber_client::Theme>),
    Delete(String),
}

fn apply_op(doc: &ThemeDocument, op: &Op) -> Result<ThemeDocument, String> {
    let mut next = doc.clone();
    match op {
        Op::Save(t) => {
            next.custom_themes.retain(|x| x.id != t.id);
            if next.custom_themes.len() >= MAX_THEMES {
                return Err("Du kannst höchstens 50 eigene Themes speichern.".into());
            }
            next.custom_themes.push((**t).clone());
            next.active = t.id.clone();
        }
        Op::Delete(id) => {
            next.custom_themes.retain(|x| &x.id != id);
            if &next.active == id {
                next.active = "dark".into();
            }
        }
        Op::Apply(id) => next.active = id.clone(),
    }
    if !is_builtin(&next.active) && !next.custom_themes.iter().any(|t| t.id == next.active) {
        return Err("Das ausgewählte Theme ist nicht mehr verfügbar.".into());
    }
    Ok(next)
}

#[derive(Default)]
struct Sync {
    /// The document as changed here, ahead of the server.
    local: Option<ThemeDocument>,
    pending: Option<Op>,
    saving: bool,
    error: Option<String>,
    reload: Option<Pending<Result<ThemeDocument, ApiError>>>,
    /// Reapply `pending` on the reloaded document ("Erneut anwenden").
    reapply: bool,
    /// The definition last put on the window.
    shown: Option<Definition>,
    /// A gallery preview and the frame it was last asked for.
    preview: Option<(Definition, u64)>,
}

fn sync(ctx: &egui::Context) -> Arc<Mutex<Sync>> {
    let id = Id::unique("gelabber-theme-sync");
    ctx.data_mut(|d| {
        d.get_temp_mut_or_insert_with(id, || Arc::new(Mutex::new(Sync::default())))
            .clone()
    })
}

/// A fresh sync for another account; the theme on the window stays until
/// that account's document arrives.
pub fn forget(ctx: &egui::Context) {
    let shared = sync(ctx);
    let mut s = shared.lock();
    let shown = s.shown.take();
    *s = Sync {
        shown,
        ..Sync::default()
    };
}

/// What the pages read about the account's themes.
pub struct SyncView {
    pub doc: ThemeDocument,
    pub saving: bool,
    pub error: Option<String>,
    /// Loaded and not stuck on an unsynced change.
    pub ready: bool,
    pub pending: bool,
}

fn poll_sync(client: &Client, s: &mut Sync) {
    let state = client.with_state(|st| st.op("themes").cloned());
    match state {
        Some(gelabber_client::OpState::Done) => {
            client.clear_op("themes");
            s.saving = false;
            s.pending = None;
            s.local = None;
            s.error = None;
        }
        Some(gelabber_client::OpState::Failed(e)) => {
            client.clear_op("themes");
            s.saving = false;
            s.error = Some(if e.status() == Some(409) {
                "Auf einem anderen Gerät geändert. Deine Änderung bleibt erhalten. Erneut anwenden übernimmt sie auf den aktuellen Stand.".into()
            } else {
                "Noch nicht synchronisiert. Deine Änderung ist hier gespeichert.".into()
            });
        }
        _ => {}
    }
    if let Some(result) = poll(&mut s.reload) {
        match result {
            Ok(remote) => {
                if s.reapply
                    && let Some(op) = s.pending.clone()
                {
                    s.reapply = false;
                    match apply_op(&remote, &op) {
                        Ok(next) => {
                            s.local = Some(next.clone());
                            s.saving = true;
                            s.error = None;
                            client.save_themes(next);
                        }
                        Err(e) => s.error = Some(e),
                    }
                } else {
                    s.local = Some(remote);
                    s.error = None;
                }
            }
            // theme/account.ts: a document it cannot read is the server's.
            Err(ApiError::Decode(_)) => {
                s.error = Some("Unbekanntes Theme-Format vom Server.".into())
            }
            Err(_) => {
                s.error =
                    Some("Themes konnten nicht geladen werden. Bitte erneut versuchen.".into())
            }
        }
    }
}

fn effective(client: &Client, s: &Sync) -> Option<ThemeDocument> {
    s.local
        .clone()
        .or_else(|| client.with_state(|st| st.themes.clone()))
}

pub fn sync_state(ctx: &egui::Context, app: &AppCtx) -> SyncView {
    let shared = sync(ctx);
    let mut s = shared.lock();
    poll_sync(app.client, &mut s);
    let doc = effective(app.client, &s);
    let loaded = doc.is_some();
    SyncView {
        doc: doc.unwrap_or_default(),
        saving: s.saving || op_pending(app, "themes"),
        error: s.error.clone(),
        ready: loaded && s.pending.is_none() && s.reload.is_none(),
        pending: s.pending.is_some(),
    }
}

/// `changeTheme`: applied here at once, then saved to the account.
pub fn change(ctx: &egui::Context, app: &AppCtx, op: Op) -> bool {
    let shared = sync(ctx);
    let mut s = shared.lock();
    let Some(doc) = effective(app.client, &s) else {
        return false;
    };
    if s.saving || s.pending.is_some() {
        return false;
    }
    match apply_op(&doc, &op) {
        Ok(next) => {
            s.local = Some(next.clone());
            s.pending = Some(op);
            s.saving = true;
            s.error = None;
            app.client.save_themes(next);
            true
        }
        Err(e) => {
            s.error = Some(if e.is_empty() {
                "Theme konnte nicht gespeichert werden.".into()
            } else {
                e
            });
            false
        }
    }
}

/// "Erneut versuchen" / "Erneut anwenden": reload, then resend a pending change.
fn retry(ctx: &egui::Context, app: &AppCtx) {
    let shared = sync(ctx);
    let mut s = shared.lock();
    s.reapply = s.pending.is_some();
    let http = app.client.http().clone();
    s.reload = Some(Pending::spawn(
        app.client,
        async move { http.themes().await },
    ));
}

/// "Account-Stand laden": drop the local change and reload.
fn discard_pending(ctx: &egui::Context, app: &AppCtx) {
    {
        let shared = sync(ctx);
        let mut s = shared.lock();
        s.pending = None;
        s.local = None;
        s.error = None;
    }
    retry(ctx, app);
}

/// Shows the gallery's try-on across the app for this frame.
fn set_preview(ctx: &egui::Context, preview: Option<Definition>) {
    let shared = sync(ctx);
    let frame = ctx.cumulative_frame_nr();
    shared.lock().preview = preview.map(|d| (d, frame));
}

/// Puts the account's theme (or a gallery preview) on the window; cheap
/// to call every frame. The shell calls it so every page follows.
pub fn apply_theme(ctx: &egui::Context, client: &Client) {
    let shared = sync(ctx);
    let mut s = shared.lock();
    poll_sync(client, &mut s);
    let frame = ctx.cumulative_frame_nr();
    // A preview lasts while the gallery keeps asking for it.
    if s.preview.as_ref().is_some_and(|(_, f)| *f + 1 < frame) {
        s.preview = None;
    }
    let wanted = match (&s.preview, effective(client, &s)) {
        (Some((d, _)), _) => Some(d.clone()),
        (None, Some(doc)) => Some(resolve(ctx, &doc, &doc.active)),
        (None, None) => super::prefs::get(ctx)
            .theme
            .as_ref()
            .and_then(definition_of),
    };
    let Some(wanted) = wanted else { return };
    if s.shown.as_ref() == Some(&wanted) {
        return;
    }
    s.shown = Some(wanted.clone());
    let preview = s.preview.is_some();
    drop(s);
    theme::set(ctx, Theme::from_definition(wanted.clone()));
    // The first paint of the next start uses it (never a preview).
    if !preview {
        let stored = stored_of(&wanted);
        if super::prefs::get(ctx).theme.as_ref() != Some(&stored) {
            super::prefs::update(ctx, |p| p.theme = Some(stored));
        }
    }
}

// ------------------------------------------------------------- previews

/// `ThemePreview`: the sidebar, chat and call bar of `def`'s own tokens.
fn paint_preview(ui: &Ui, rect: Rect, def: &Definition, small: bool, clip: Rect) {
    let t = Theme::from_definition(def.clone());
    let c = &t.c;
    let painter = ui.painter().with_clip_rect(rect.intersect(clip));
    let radius = if small { 0.0 } else { t.radius.md };
    painter.rect_filled(rect, corner(radius), c.bg);
    let inner = if small { rect } else { rect.shrink(1.0) };
    let footer_h = if small { 0.0 } else { 46.0 };
    let side = Rect::from_min_size(
        inner.min,
        vec2(inner.width() * 0.3, inner.height() - footer_h),
    );
    let rad = |nw: f32, sw: f32| egui::CornerRadius {
        nw: nw as u8,
        sw: sw as u8,
        ne: 0,
        se: 0,
    };
    painter.rect_filled(
        side,
        if small {
            rad(0.0, 0.0)
        } else {
            rad(radius - 1.0, 0.0)
        },
        c.panel,
    );
    crate::widgets::rule_v(&painter, c.border, side.right() - 1.0, side.y_range());
    let f = |px: f32, small_px: f32| if small { small_px } else { px };
    let style = |size: f32, weight: f32| TextStyle::new(size, weight, 1.5);
    let clip = painter.with_clip_rect(side.shrink2(vec2(0.0, 0.0)).intersect(painter.clip_rect()));
    let pad_x = f(10.0, 6.0);
    let mut y = side.top() + f(16.0, 10.0);
    let x = side.left() + pad_x;
    let gap = f(15.0, 7.0);
    let line = |s: &str, st: TextStyle, color: Color32, pos: Pos2| {
        Block::single(ui, s, st, color).paint(&clip, pos);
    };
    line("Gelabber", style(f(16.0, 8.0), 700.0), c.text, pos2(x, y));
    y += style(f(16.0, 8.0), 700.0).line_px() + gap;
    let small_st = style(f(10.0, 5.0), 400.0);
    line("KANÄLE", small_st, c.muted, pos2(x, y));
    y += small_st.line_px() + gap;
    // The selected "Übersicht" row.
    let sel_st = style(f(11.0, 6.0), 700.0);
    let (py, px) = (f(8.0, 4.0), f(5.0, 2.0));
    let sel = Rect::from_min_size(
        pos2(x, y),
        vec2(
            side.width() - 2.0 * pad_x - 1.0,
            sel_st.line_px() + 2.0 * py,
        ),
    );
    clip.rect_filled(sel, corner(t.radius.sm), c.selected);
    let icon = f(12.0, 7.0);
    icons::paint(
        &clip,
        Icon::Chat,
        pos2(x + px, sel.center().y - icon / 2.0),
        icon,
        c.accent_text,
    );
    line(
        "Übersicht",
        sel_st,
        c.accent_text,
        pos2(x + px + icon + 4.0, y + py),
    );
    y += sel.height() + gap;
    for (ic, label) in [(Icon::Hash, "allgemein"), (Icon::Speaker, "Wohnzimmer")] {
        let center = y + small_st.line_px() / 2.0;
        icons::paint(&clip, ic, pos2(x, center - icon / 2.0), icon, c.muted);
        line(label, small_st, c.muted, pos2(x + icon + 4.0, y));
        y += small_st.line_px() + gap;
    }
    // Chat.
    let chat = Rect::from_min_max(
        pos2(side.right(), inner.top()),
        pos2(inner.right(), side.bottom()),
    );
    let clip = painter.with_clip_rect(chat.intersect(painter.clip_rect()));
    let pad = f(14.0, 8.0);
    let x = chat.left() + pad;
    let mut y = chat.top() + pad;
    let head = style(f(12.0, 7.0), 700.0);
    let hicon = f(12.0, 7.0);
    icons::paint(
        &clip,
        Icon::Hash,
        pos2(x, y + head.line_px() / 2.0 - hicon / 2.0),
        hicon,
        c.text,
    );
    Block::single(ui, "allgemein", head, c.text).paint(&clip, pos2(x + hicon + 4.0, y));
    y += head.line_px();
    if !small {
        y += 4.0;
        Block::single(ui, "Allgemeiner Chat", style(10.0, 400.0), c.muted).paint(&clip, pos2(x, y));
        y += 15.0;
    }
    y += f(16.0, 7.0);
    let msgs: &[(&str, &str, &str, &str)] = if small {
        &[("MA", "Mara", "18:42", "Wer ist heute Abend dabei?")]
    } else {
        &[
            ("MA", "Mara", "18:42", "Wer ist heute Abend dabei?"),
            ("DU", "Du", "18:43", "Bin im Wohnzimmer."),
        ]
    };
    let msg = style(f(11.0, 6.0), 400.0);
    let ava = style(f(9.0, 5.0), 400.0);
    let apad = f(7.0, 4.0);
    for (initials, name, time, body) in msgs {
        let ab = Block::single(ui, initials, ava, c.text);
        let abox = Rect::from_min_size(
            pos2(x, y),
            vec2(ab.width() + 2.0 * apad, ava.line_px() + 2.0 * apad),
        );
        clip.rect_filled(abox, corner(t.radius.md), c.surface);
        ab.paint(&clip, pos2(x + apad, y + apad));
        let tx = abox.right() + f(8.0, 4.0);
        let nb = Block::single(ui, name, msg.weight(700.0), c.text);
        nb.paint(&clip, pos2(tx, y));
        if !small {
            let tb = Block::single(ui, time, style(9.0, 400.0), c.muted);
            tb.paint(&clip, pos2(tx + nb.width() + 3.0, y + 2.0));
        }
        Block::single(ui, body, msg, c.text).paint(&clip, pos2(tx, y + msg.line_px() + 4.0));
        y += 2.0 * msg.line_px() + 4.0 + f(16.0, 7.0);
    }
    // Composer at the bottom.
    let comp = style(f(10.0, 5.0), 400.0);
    let cpad = f(9.0, 5.0);
    let ch = comp.line_px() + 2.0 * cpad + 2.0;
    let crect = Rect::from_min_size(
        pos2(x, chat.bottom() - pad - ch),
        vec2(chat.width() - 2.0 * pad, ch),
    );
    clip.rect(
        crect,
        corner(t.radius.sm),
        c.surface,
        Stroke::new(1.0, c.border),
        StrokeKind::Inside,
    );
    Block::single(ui, "Nachricht an #allgemein", comp, c.muted).paint(
        &clip,
        pos2(crect.left() + 1.0 + cpad, crect.top() + 1.0 + cpad),
    );
    let send = Block::single(ui, "Senden", comp.weight(700.0), c.accent_text);
    send.paint(
        &clip,
        pos2(
            crect.right() - 1.0 - cpad - send.width(),
            crect.top() + 1.0 + cpad,
        ),
    );
    if !small {
        let foot = Rect::from_min_max(pos2(inner.left(), side.bottom()), inner.max);
        painter.rect_filled(
            foot,
            egui::CornerRadius {
                nw: 0,
                ne: 0,
                sw: (radius - 1.0) as u8,
                se: (radius - 1.0) as u8,
            },
            c.panel,
        );
        crate::widgets::rule_h(&painter, c.border, foot.x_range(), foot.top());
        let fs = style(10.0, 400.0);
        let cy = foot.top() + 1.0 + 10.0 + 12.5;
        painter.circle_filled(pos2(foot.left() + 10.0 + 3.0, cy), 3.0, c.online);
        Block::single(ui, "Verbunden · Wohnzimmer", fs, c.text).paint(
            &painter,
            pos2(foot.left() + 10.0 + 6.0 + 4.0, cy - fs.line_px() / 2.0),
        );
        let pill = Block::single(ui, "Mikrofon an", fs.weight(500.0), c.accent_ink);
        let prect = Rect::from_min_size(
            pos2(foot.right() - 10.0 - pill.width() - 16.0, cy - 12.5),
            vec2(pill.width() + 16.0, 25.0),
        );
        painter.rect_filled(prect, corner(t.radius.sm), c.accent);
        pill.paint(&painter, pos2(prect.left() + 8.0, prect.top() + 5.0));
    }
    if !small {
        painter.rect_stroke(
            rect,
            corner(radius),
            Stroke::new(1.0, c.border),
            StrokeKind::Inside,
        );
    }
}

// ----------------------------------------------------------------- panel

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Filter {
    #[default]
    All,
    Light,
    Dark,
    Own,
}

#[derive(Default)]
pub struct PanelState {
    filter: Filter,
    /// The card tried on; `None` follows the account's theme.
    picked: Option<String>,
    draft: Option<gelabber_client::Theme>,
    baseline: Option<gelabber_client::Theme>,
    /// Hex fields while typed (`editing` of ColorField).
    editing: [Option<String>; 8],
    color_open: Option<usize>,
    notice: Status,
    importing: Option<Picked>,
    keep_after_save: Option<&'static str>,
    /// The draft or card to open once "discard?" was answered yes.
    after_discard: Option<Box<Pending2>>,
    /// The workbench's height last frame, for its sticky stop.
    bench_h: f32,
}

/// What waits behind the draft-discard question.
enum Pending2 {
    Open(Box<gelabber_client::Theme>, bool),
    Choose(String),
    Close,
}

impl PanelState {
    pub fn draft_dirty(&self) -> bool {
        match (&self.draft, &self.baseline) {
            (Some(d), Some(b)) => d != b,
            (Some(_), None) => true,
            _ => false,
        }
    }

    pub fn drop_draft(&mut self) {
        self.draft = None;
        self.baseline = None;
    }
}

const COLOR_LABELS: [&str; 8] = [
    "Hintergrund",
    "Seitenleiste",
    "Flächen",
    "Serverleiste",
    "Text",
    "Sekundärtext",
    "Akzent",
    "Rahmen",
];

fn color_slot(c: &mut ThemeColors, i: usize) -> &mut String {
    match i {
        0 => &mut c.background,
        1 => &mut c.panel,
        2 => &mut c.surface,
        3 => &mut c.rail,
        4 => &mut c.text,
        5 => &mut c.muted,
        6 => &mut c.accent,
        _ => &mut c.border,
    }
}

fn copy_theme(d: &Definition) -> gelabber_client::Theme {
    let mut t = stored_of(d);
    t.id = format!("custom-{}", uuid::Uuid::new_v4());
    let name: String = d.name.chars().take(48).collect();
    t.name = format!("{name} – Eigene");
    t
}

/// `importTheme`: the strict format of `isTheme`, with a fresh id.
fn import_theme(bytes: &[u8]) -> Result<gelabber_client::Theme, String> {
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err("Die Datei darf höchstens 64 KiB groß sein.".into());
    }
    let invalid = || "Das Theme-Format ist ungültig oder wird noch nicht unterstützt.".to_owned();
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|_| "Die Datei enthält kein gültiges JSON.".to_owned())?;
    let obj = value.as_object().ok_or_else(invalid)?;
    let keys = ["version", "id", "name", "mode", "style", "colors"];
    if obj.len() != keys.len() || !keys.iter().all(|k| obj.contains_key(*k)) {
        return Err(invalid());
    }
    let mut t: gelabber_client::Theme =
        serde_json::from_value(value.clone()).map_err(|_| invalid())?;
    let name_ok = !t.name.trim().is_empty()
        && t.name.chars().count() <= 60
        && !t
            .name
            .chars()
            .any(|c| (c as u32) < 32 || (127..=159).contains(&(c as u32)));
    let colors_ok = obj["colors"].as_object().is_some_and(|c| c.len() == 8);
    if t.version != 1
        || t.id.len() > 80
        || !name_ok
        || !["dark", "light"].contains(&t.mode.as_str())
        || !["clear", "soft", "terminal"].contains(&t.style.as_str())
        || !colors_ok
        || definition_of(&t).is_none()
    {
        return Err(invalid());
    }
    t.id = format!("custom-{}", uuid::Uuid::new_v4());
    Ok(t)
}

fn export_theme(ctx: &egui::Context, t: &gelabber_client::Theme) {
    let name: String = t
        .name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let file = format!("{name}.gelabber-theme.json");
    let json = serde_json::to_string_pretty(t).unwrap_or_default();
    controls::save_file(ctx, "Theme exportieren", &file, json.into_bytes());
}

/// Settings area "Themes".
pub fn area(ui: &mut Ui, app: &mut AppCtx, s: &mut PanelState, viewport: Rect) {
    let ctx = ui.ctx().clone();
    let theme = Theme::of(&ctx);
    let c = &theme.c;
    let sync_view = sync_state(&ctx, app);
    let doc = sync_view.doc.clone();
    let active = doc.active.clone();
    let selected = s.picked.clone().unwrap_or_else(|| active.clone());
    let all: Vec<Definition> = presets()
        .into_iter()
        .chain(doc.custom_themes.iter().filter_map(definition_of))
        .collect();
    let chosen = if selected == "system" {
        preset(if system_dark(&ctx) { "dark" } else { "light" })
    } else {
        all.iter()
            .find(|t| t.id == selected)
            .cloned()
            .unwrap_or_else(Definition::dark)
    };
    let blocked = sync_view.saving || !sync_view.ready;
    let can_create = doc.custom_themes.len() < MAX_THEMES;

    // Answers and finished work.
    if let Some(slot) = &s.importing {
        let done = slot.lock().take();
        if let Some(file) = done {
            s.importing = None;
            if let Some((bytes, _)) = file {
                match import_theme(&bytes) {
                    Ok(t) => open_draft(s, t, true),
                    Err(e) => s.notice.error(ui, e),
                }
            }
        }
    }
    if let Some(yes) = controls::answer(&ctx, "theme-discard")
        && let Some(next) = s.after_discard.take().filter(|_| yes)
    {
        s.drop_draft();
        match *next {
            Pending2::Open(t, new) => {
                s.baseline = (!new).then(|| (*t).clone());
                s.draft = Some(*t);
                s.editing = Default::default();
            }
            Pending2::Choose(id) => s.picked = (id != active).then_some(id),
            Pending2::Close => {}
        }
    }
    if let Some(true) = controls::answer(&ctx, "theme-delete")
        && change(&ctx, app, Op::Delete(chosen.id.clone()))
    {
        s.picked = None;
        s.keep_after_save = Some("Theme gelöscht.");
    }
    if let Some(true) = controls::answer(&ctx, "theme-account") {
        discard_pending(&ctx, app);
    }
    if let Some(message) = s.keep_after_save
        && !sync_view.saving
    {
        s.keep_after_save = None;
        if sync_view.error.is_none() {
            s.notice.success(ui, message);
        } else {
            s.notice.error(ui, "Nicht gespeichert – erneut versuchen");
        }
    }

    // The gallery tries a theme on across the app.
    let previewing = s.draft.is_none() && s.picked.as_ref().is_some_and(|p| *p != active);
    if previewing {
        set_preview(&ctx, Some(chosen.clone()));
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !controls::asking(&ctx) {
            s.picked = None;
            s.notice.clear();
        }
    }

    let width = ui.available_width();
    // Toolbar.
    controls::row(ui, 8.0, 32.0, |ui| {
        if ui
            .add(
                Button::new("+ Eigenes Theme")
                    .small()
                    .disabled(!can_create || blocked),
            )
            .clicked()
        {
            let t = copy_theme(&chosen);
            ask_open(&ctx, s, t, true);
        }
        if ui
            .add(
                Button::new("Importieren")
                    .small()
                    .disabled(!can_create || blocked),
            )
            .clicked()
            && s.importing.is_none()
        {
            s.importing = Some(controls::pick_file(
                &ctx,
                "Theme-Datei importieren",
                &["*.json"],
            ));
        }
    });
    // Sync box.
    if sync_view.saving || sync_view.error.is_some() {
        ui.add_space(16.0);
        let msg = if sync_view.saving {
            "Wird mit deinem Account synchronisiert …".to_owned()
        } else {
            sync_view.error.clone().unwrap_or_default()
        };
        let block = Block::wrapped(
            ui,
            &msg,
            TextStyle::new(13.0, 400.0, 1.5),
            c.text,
            width - 30.0,
        );
        let buttons_h = if sync_view.saving { 0.0 } else { 10.0 + 32.0 };
        let h = 24.0 + 2.0 + block.height() + buttons_h;
        let top = ui.cursor().min;
        let rect = Rect::from_min_size(top, vec2(width, h));
        ui.painter().rect(
            rect,
            corner(theme.radius.sm),
            c.panel,
            Stroke::new(1.0, c.border),
            StrokeKind::Inside,
        );
        block.paint(ui.painter(), pos2(rect.left() + 15.0, rect.top() + 13.0));
        let status = ui.interact(
            Rect::from_min_size(pos2(rect.left() + 15.0, rect.top() + 13.0), block.size()),
            Id::unique("gelabber-theme-sync-status"),
            Sense::hover(),
        );
        crate::widgets::announce(&status, egui::Role::Status, &msg);
        if !sync_view.saving {
            let brect = Rect::from_min_size(
                pos2(
                    rect.left() + 15.0,
                    rect.top() + 13.0 + block.height() + 10.0,
                ),
                vec2(width - 30.0, 32.0),
            );
            controls::place(
                ui,
                UiBuilder::new()
                    .max_rect(brect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let label = if sync_view.pending {
                        "Erneut anwenden"
                    } else {
                        "Erneut versuchen"
                    };
                    if ui.add(Button::new(label).small()).clicked() {
                        retry(&ctx, app);
                    }
                    if sync_view.pending
                        && ui.add(Button::new("Account-Stand laden").small()).clicked()
                    {
                        controls::ask(
                            &ctx,
                            Confirm::danger(
                                "theme-account",
                                "Lokale Änderung verwerfen?",
                                Some("Gelabber lädt dann wieder den Theme-Stand deines Accounts."),
                                "Account-Stand laden",
                            ),
                        );
                    }
                },
            );
        }
        ui.allocate_exact_size(vec2(width, h), Sense::hover());
    }
    if s.notice.current(ui).is_some() {
        ui.add_space(12.0);
        s.notice.ui(ui, width);
    }
    ui.add_space(20.0);
    let filters = ["Alle", "Hell", "Dunkel", "Eigene"];
    let order = [Filter::All, Filter::Light, Filter::Dark, Filter::Own];
    let current = order.iter().position(|f| *f == s.filter).unwrap_or(0);
    if let Some(i) = controls::chips(ui, "theme-filter", &filters, current, 32.0, None) {
        s.filter = order[i];
    }
    ui.add_space(16.0);

    // Gallery and workbench.
    let top = ui.cursor().min;
    let bench_w = 340.0_f32.min((width - 24.0) / 2.0).max(280.0);
    let gallery_w = width - 24.0 - bench_w;
    let card_w = (gallery_w - 12.0) / 2.0;
    let card_h = 207.5;
    let mut options: Vec<(String, Option<Definition>)> = Vec::new();
    if s.filter == Filter::All {
        options.push(("system".into(), None));
    }
    for t in &all {
        let keep = match s.filter {
            Filter::All => true,
            Filter::Own => t.id.starts_with("custom-"),
            Filter::Light => t.mode == Mode::Light,
            Filter::Dark => t.mode == Mode::Dark,
        };
        if keep {
            options.push((t.id.clone(), Some(t.clone())));
        }
    }
    let mut focused = None;
    for (i, (id, def)) in options.iter().enumerate() {
        let col = (i % 2) as f32;
        let row = (i / 2) as f32;
        let rect = Rect::from_min_size(
            pos2(top.x + col * (card_w + 12.0), top.y + row * (card_h + 12.0)),
            vec2(card_w, card_h),
        );
        let wid = Id::unique(("gelabber-theme-card", id));
        let response = ui.interact(rect, wid, Sense::click());
        if response.has_focus() {
            focused = Some(i);
        }
        let is_sel = *id == selected;
        let fill = eased(&ctx, wid, response.hovered(), c.panel, c.hover);
        let painter = ui.painter();
        if response.has_focus() && keyboard_focus(&ctx) {
            focus_ring(painter, rect, theme.radius.md, &theme);
        }
        painter.rect(
            rect,
            corner(theme.radius.md),
            fill,
            Stroke::NONE,
            StrokeKind::Inside,
        );
        match def {
            Some(d) => {
                let prev =
                    Rect::from_min_size(rect.min + vec2(1.0, 1.0), vec2(card_w - 2.0, 115.0));
                paint_preview_clipped(ui, prev, d, rect);
                let desc_top = prev.bottom();
                crate::widgets::rule_h(
                    ui.painter(),
                    c.border_strong,
                    rect.left() + 1.0..=rect.right() - 1.0,
                    desc_top,
                );
                let x = rect.left() + 13.0;
                Block::single(ui, &d.name, TextStyle::new(13.0, 700.0, 1.5), c.text)
                    .paint(ui.painter(), pos2(x, desc_top + 13.0));
                let mode = if d.mode == Mode::Dark {
                    "Dunkel"
                } else {
                    "Hell"
                };
                let style = match d.style {
                    Style::Clear => "Klar",
                    Style::Soft => "Weich",
                    Style::Terminal => "Terminal",
                };
                Block::single(ui, &format!("{mode} · {style}"), controls::HINT, c.muted)
                    .paint(ui.painter(), pos2(x, desc_top + 13.0 + 19.5 + 4.0));
                let sy = desc_top + 13.0 + 19.5 + 4.0 + 18.0 + 10.0;
                for (k, col) in [
                    d.colors.background,
                    d.colors.surface,
                    d.colors.text,
                    d.colors.accent,
                ]
                .iter()
                .enumerate()
                {
                    let center = pos2(x + 7.0 + k as f32 * 19.0, sy + 7.0);
                    ui.painter().circle_filled(center, 7.0, c.border_strong);
                    ui.painter().circle_filled(center, 6.0, *col);
                }
            }
            None => {
                let content_h = 32.0 + 9.0 + 24.0 + 9.0 + 18.0;
                let y = rect.top() + 1.0 + (card_h - 2.0 - content_h) / 2.0;
                let x = rect.left() + 19.0;
                icons::paint(ui.painter(), Icon::Screen, pos2(x, y), 32.0, c.accent_text);
                Block::single(ui, "System", TextStyle::new(16.0, 700.0, 1.5), c.text)
                    .paint(ui.painter(), pos2(x, y + 41.0));
                Block::single(ui, "Folgt dem Gerät", controls::HINT, c.muted)
                    .paint(ui.painter(), pos2(x, y + 74.0));
            }
        }
        let painter = ui.painter();
        let border = if is_sel { c.accent } else { c.border_strong };
        painter.rect_stroke(
            rect,
            corner(theme.radius.md),
            Stroke::new(1.0, border),
            StrokeKind::Inside,
        );
        if is_sel {
            painter.rect_stroke(
                rect.shrink(1.0),
                corner(theme.radius.md - 1.0),
                Stroke::new(1.0, c.accent),
                StrokeKind::Inside,
            );
            let check = Rect::from_min_size(
                pos2(rect.right() - 1.0 - 8.0 - 22.0, rect.top() + 1.0 + 8.0),
                vec2(22.0, 22.0),
            );
            painter.circle_filled(check.center(), 11.0, c.accent);
            icons::paint_centered(painter, Icon::Check, check, 14.0, c.accent_ink);
        }
        if *id == active {
            let b = Block::single(ui, "Aktiv", TextStyle::new(11.0, 600.0, 1.5), c.accent_text);
            b.paint(
                painter,
                pos2(
                    rect.right() - 11.0 - b.width(),
                    rect.bottom() - 1.0 - 12.0 - 16.5,
                ),
            );
        }
        if response.hovered() {
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.clicked() {
            choose(&ctx, s, id.clone(), &active);
        }
    }
    if let Some(i) = focused {
        let (prev, next) = ctx.input(|inp| {
            (
                inp.key_pressed(egui::Key::ArrowLeft) || inp.key_pressed(egui::Key::ArrowUp),
                inp.key_pressed(egui::Key::ArrowRight) || inp.key_pressed(egui::Key::ArrowDown),
            )
        });
        let n = options.len();
        let to = if prev {
            Some((i + n - 1) % n)
        } else if next {
            Some((i + 1) % n)
        } else {
            None
        };
        if let Some(to) = to {
            ui.memory_mut(|m| m.request_focus(Id::unique(("gelabber-theme-card", &options[to].0))));
            choose(&ctx, s, options[to].0.clone(), &active);
        }
    }
    let rows = options.len().div_ceil(2) as f32;
    let mut gallery_h = if rows > 0.0 {
        rows * card_h + (rows - 1.0) * 12.0
    } else {
        0.0
    };
    if s.filter == Filter::Own && doc.custom_themes.is_empty() {
        let b = Block::wrapped(
            ui,
            "Keine eigenen Themes.",
            TextStyle::new(13.0, 400.0, 1.6),
            c.muted,
            gallery_w,
        );
        b.paint(ui.painter(), top);
        gallery_h = b.height();
    }

    // Workbench: sticky at the top of the view.
    let bench_x = top.x + gallery_w + 24.0;
    // Sticky inside the layout row: it stops where the gallery ends.
    let last_h = s.bench_h;
    let layout_end = top.y + gallery_h.max(last_h);
    let bench_y = top
        .y
        .max(viewport.top())
        .min(layout_end - last_h)
        .max(top.y);
    let bench_rect = Rect::from_min_size(pos2(bench_x, bench_y), vec2(bench_w, 2000.0));
    let mut bench = ui.new_child(
        UiBuilder::new()
            .max_rect(bench_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    bench.spacing_mut().item_spacing = egui::Vec2::ZERO;
    workbench(&mut bench, app, s, &chosen, &selected, blocked, can_create);
    let bench_h = bench.min_rect().height();
    if (bench_h - s.bench_h).abs() > 0.5 {
        s.bench_h = bench_h;
        ctx.request_repaint();
    }
    let layout_h = gallery_h.max(bench_h);
    ui.advance_cursor_after_rect(Rect::from_min_size(top, vec2(width, layout_h)));

    if previewing {
        ui.add_space(20.0);
        let natural = ui.cursor().min;
        let name = theme_name(&doc, &selected);
        let back = format!("Zurück zu {}", theme_name(&doc, &active));
        let h = 2.0 + 24.0 + 36.0;
        let y = natural
            .y
            .min(viewport.bottom() - h)
            .max(natural.y.min(viewport.top()));
        let rect = Rect::from_min_size(pos2(natural.x, y), vec2(width, h));
        let painter = ui.painter();
        painter.add(theme.shadow_2.as_shape(rect, corner(theme.radius.lg)));
        painter.rect(
            rect,
            corner(theme.radius.lg),
            c.elevated,
            Stroke::new(1.0, c.border),
            StrokeKind::Inside,
        );
        let rich = text::Rich::new(text::UI)
            .push(&format!("„{name}“"), text::UI.weight(700.0), c.text)
            .push(" wird angezeigt", text::UI, c.text)
            .layout(ui);
        rich.paint(
            ui.painter(),
            pos2(
                rect.left() + 17.0,
                rect.center().y - text::UI.line_px() / 2.0,
            ),
        );
        let buttons = Rect::from_min_max(
            pos2(rect.left() + 17.0, rect.top() + 13.0),
            pos2(rect.right() - 17.0, rect.bottom() - 13.0),
        );
        controls::place(
            ui,
            UiBuilder::new()
                .max_rect(buttons)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if ui.add(Button::new(&back).ghost()).clicked() {
                    s.picked = None;
                    s.notice.clear();
                }
                if ui
                    .add(Button::new("Behalten").primary().disabled(blocked))
                    .clicked()
                {
                    let id = selected.clone();
                    let message = format!("„{}“ angewendet.", theme_name(&doc, &id));
                    if change(&ctx, app, Op::Apply(id)) {
                        s.picked = None;
                        s.notice.success(ui, message);
                    } else {
                        s.notice.error(ui, "Nicht gespeichert – erneut versuchen");
                    }
                }
            },
        );
        ui.advance_cursor_after_rect(Rect::from_min_size(natural, vec2(width, h)));
    }

    // Footnote.
    ui.add_space(24.0);
    let foot = TextStyle::new(13.0, 400.0, 1.6);
    let lead = if sync_view.ready && sync_view.error.is_none() {
        "Mit deinem Account synchronisiert."
    } else {
        "Deine Themes gehören zu deinem Account."
    };
    let rich = text::Rich::new(foot)
        .push(&format!("{lead} Paletten nach "), foot, c.muted)
        .push("Omarchy", foot, c.muted)
        .push(", für Gelabber angepasst.", foot, c.muted)
        .wrap(width)
        .layout(ui);
    let at = ui.cursor().min;
    text::show(ui, &rich);
    if let Some(r) = rich.section_rect(at, 1) {
        // Under the baseline, not at the bottom of the 1.6 line box.
        link::underline(ui.painter(), foot, r.top(), r.left(), r.right(), c.muted);
        if Link::interact(ui, r, Id::unique("gelabber-omarchy"), "Omarchy").clicked() {
            ctx.open_url(egui::OpenUrl::new_tab(
                "https://github.com/omacom/omarchy/tree/quattro/themes",
            ));
        }
    }
}

fn paint_preview_clipped(ui: &Ui, prev: Rect, d: &Definition, card: Rect) {
    paint_preview(
        ui,
        prev,
        d,
        true,
        card.shrink(1.0).intersect(ui.clip_rect()),
    );
}

fn choose(ctx: &egui::Context, s: &mut PanelState, id: String, active: &str) {
    if s.draft_dirty() {
        s.after_discard = Some(Box::new(Pending2::Choose(id)));
        ask_discard(ctx);
        return;
    }
    s.drop_draft();
    s.picked = (id != active).then_some(id);
    s.notice.clear();
}

fn ask_discard(ctx: &egui::Context) {
    controls::ask(
        ctx,
        Confirm::danger(
            "theme-discard",
            "Ungespeicherten Theme-Entwurf verwerfen?",
            Some("Deine Änderungen an diesem Theme gehen verloren."),
            "Verwerfen",
        ),
    );
}

fn ask_open(ctx: &egui::Context, s: &mut PanelState, t: gelabber_client::Theme, new: bool) {
    if s.draft_dirty() {
        s.after_discard = Some(Box::new(Pending2::Open(Box::new(t), new)));
        ask_discard(ctx);
    } else {
        open_draft(s, t, new);
    }
}

fn open_draft(s: &mut PanelState, t: gelabber_client::Theme, new: bool) {
    s.baseline = (!new).then(|| t.clone());
    s.draft = Some(t);
    s.editing = Default::default();
    s.notice.clear();
}

const EYEBROW: TextStyle = text::CAPS;

fn workbench(
    ui: &mut Ui,
    app: &mut AppCtx,
    s: &mut PanelState,
    chosen: &Definition,
    selected: &str,
    blocked: bool,
    can_create: bool,
) {
    let ctx = ui.ctx().clone();
    let theme = Theme::of(&ctx);
    let c = &theme.c;
    let width = ui.available_width();
    let top = ui.cursor().min;
    let eyebrow = if s.draft.is_some() {
        "DEIN ENTWURF"
    } else {
        "VORSCHAU"
    };
    Block::single(ui, eyebrow, EYEBROW, c.muted).paint(ui.painter(), pos2(top.x, top.y + 6.85));
    let title = match &s.draft {
        Some(d) if d.name.trim().is_empty() => "Neues Theme".to_owned(),
        Some(d) => d.name.clone(),
        None if selected == "system" => "Wie dein Gerät".to_owned(),
        None => chosen.name.clone(),
    };
    ui.add_space(24.0 + 4.0);
    let w = ui.available_width();
    controls::paragraph(ui, &title, text::TITLE_SM, c.text, w);
    ui.add_space(16.0);
    let draft_def = s.draft.as_ref().and_then(definition_of);
    let shown = draft_def.clone().unwrap_or_else(|| chosen.clone());
    let prev = Rect::from_min_size(ui.cursor().min, vec2(width, 270.0));
    paint_preview(ui, prev, &shown, false, ui.clip_rect());
    ui.allocate_exact_size(vec2(width, 270.0), Sense::hover());
    let issues = theme::contrast_issues(&shown);
    if !issues.is_empty() {
        ui.add_space(16.0);
        let st = TextStyle::new(13.0, 400.0, 1.5);
        let boxed_top = ui.cursor().min;
        let mut child = ui.new_child(UiBuilder::new().max_rect(Rect::from_min_size(
            boxed_top + vec2(15.0, 13.0),
            vec2(width - 30.0, 1000.0),
        )));
        child.spacing_mut().item_spacing = egui::Vec2::ZERO;
        let painter = ui.painter().clone();
        let backdrop = painter.add(egui::Shape::Noop);
        controls::paragraph(
            &mut child,
            "Kontrast prüfen",
            st.weight(700.0),
            c.text,
            width - 30.0,
        );
        for issue in &issues {
            child.add_space(6.0);
            controls::paragraph(&mut child, issue, st, c.muted, width - 30.0);
            child.add_space(6.0);
        }
        if let Some(d) = &draft_def
            && child
                .add(Button::new("Kontraste verbessern").small())
                .clicked()
        {
            let fixed = theme::fixed(d.clone());
            let mut t = stored_of(&fixed);
            if let Some(old) = &s.draft {
                t.id = old.id.clone();
                t.name = old.name.clone();
            }
            s.draft = Some(t);
            s.editing = Default::default();
        }
        let h = child.min_rect().height() + 26.0;
        let rect = Rect::from_min_size(boxed_top, vec2(width, h));
        painter.set(
            backdrop,
            egui::epaint::RectShape::new(
                rect,
                corner(theme.radius.sm),
                c.panel,
                Stroke::new(1.0, c.border),
                StrokeKind::Inside,
            ),
        );
        ui.allocate_exact_size(vec2(width, h), Sense::hover());
    }
    ui.add_space(16.0);
    if s.draft.is_some() {
        editor(ui, app, s, blocked);
        return;
    }
    controls::row(ui, 8.0, 32.0, |ui| {
        if ui
            .add(
                Button::new("Als Vorlage verwenden")
                    .small()
                    .disabled(!can_create || blocked),
            )
            .clicked()
        {
            let t = copy_theme(chosen);
            open_draft(s, t, true);
        }
        if chosen.id.starts_with("custom-") && selected != "system" {
            if ui
                .add(Button::new("Bearbeiten").small().disabled(blocked))
                .clicked()
            {
                open_draft(s, stored_of(chosen), false);
            }
            if ui
                .add(
                    Button::new("Löschen")
                        .small()
                        .variant(ButtonVariant::DangerOutline)
                        .disabled(blocked),
                )
                .clicked()
            {
                controls::ask(
                    &ctx,
                    Confirm::danger(
                        "theme-delete",
                        format!("Theme „{}“ löschen?", chosen.name),
                        None,
                        "Löschen",
                    ),
                );
            }
        }
        if ui.add(Button::new("Exportieren").small()).clicked() {
            export_theme(&ctx, &stored_of(chosen));
        }
    });
}

/// A 40px editor input on the panel colour (`.theme-editor input`).
fn editor_field(
    ui: &mut Ui,
    id: &str,
    value: &mut String,
    mono: bool,
    limit: usize,
) -> egui::Response {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let width = ui.available_width();
    let rect = Rect::from_min_size(ui.cursor().min, vec2(width, 40.0));
    let fid = crate::widgets::field::field_id(id);
    let focused = ui.memory(|m| m.has_focus(fid));
    if focused {
        focus_ring(ui.painter(), rect, theme.radius.sm, &theme);
    }
    ui.painter().rect(
        rect,
        corner(theme.radius.sm),
        c.panel,
        Stroke::new(1.0, if focused { c.focus } else { c.border_strong }),
        StrokeKind::Inside,
    );
    let st = if mono {
        TextStyle::new(11.0, 400.0, 1.5)
    } else {
        TextStyle::new(12.0, 400.0, 1.5)
    };
    let color = c.text;
    let mut layouter = move |ui: &Ui, buf: &dyn egui::TextBuffer, _w: f32| {
        let mut format = st.format(color);
        if mono {
            format.font_id = egui::FontId::monospace(11.0);
        }
        let job = egui::text::LayoutJob::single_section(buf.as_str().to_owned(), format);
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let pad = if mono { 6 } else { 9 };
    let edit = egui::TextEdit::singleline(value)
        .id(fid)
        .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(pad, 11)))
        .desired_width(width - 2.0)
        .char_limit(limit)
        .layouter(&mut layouter);
    let inner = rect.shrink(1.0);
    let r = ui
        .scope_builder(UiBuilder::new().max_rect(inner), |ui| ui.add(edit))
        .inner;
    ui.advance_cursor_after_rect(rect);
    r
}

fn editor(ui: &mut Ui, app: &mut AppCtx, s: &mut PanelState, blocked: bool) {
    let ctx = ui.ctx().clone();
    let theme = Theme::of(&ctx);
    let c = &theme.c;
    let width = ui.available_width();
    let label = TextStyle::new(12.0, 400.0, 1.5);
    let Some(mut draft) = s.draft.take() else {
        return;
    };
    controls::paint(ui, ui.cursor().min, "Name", label, c.text);
    ui.add_space(18.0 + 6.0);
    let mut name = draft.name.clone();
    if editor_field(ui, "theme-name", &mut name, false, 60).changed() {
        draft.name = name;
    }
    ui.add_space(16.0);
    let half = (width - 10.0) / 2.0;
    let top = ui.cursor().min;
    for (i, (title, options)) in [
        ("Modus", vec![("dark", "Dunkel"), ("light", "Hell")]),
        (
            "Stil",
            vec![
                ("clear", "Klar"),
                ("soft", "Weich"),
                ("terminal", "Terminal"),
            ],
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let rect = Rect::from_min_size(
            pos2(top.x + i as f32 * (half + 10.0), top.y),
            vec2(half, 70.0),
        );
        let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
        child.spacing_mut().item_spacing = egui::Vec2::ZERO;
        controls::paint(&child, rect.min, title, label, c.text);
        child.add_space(18.0 + 6.0);
        let opts: Vec<(String, String)> = options
            .iter()
            .map(|(k, l)| ((*k).to_owned(), (*l).to_owned()))
            .collect();
        let value = if i == 0 {
            &mut draft.mode
        } else {
            &mut draft.style
        };
        controls::select(&mut child, ("theme-editor", i), &opts, value, c.panel);
    }
    ui.advance_cursor_after_rect(Rect::from_min_size(top, vec2(width, 18.0 + 6.0 + 37.0)));
    ui.add_space(16.0);
    controls::paint(
        ui,
        ui.cursor().min,
        "Farben",
        TextStyle::new(13.0, 600.0, 1.5),
        c.text,
    );
    ui.add_space(19.5 + 10.0);
    let grid_top = ui.cursor().min;
    let cell_h = 18.0 + 6.0 + 40.0;
    for (i, name) in COLOR_LABELS.iter().enumerate() {
        let col = (i % 2) as f32;
        let row = (i / 2) as f32;
        let rect = Rect::from_min_size(
            pos2(
                grid_top.x + col * (half + 10.0),
                grid_top.y + row * (cell_h + 10.0),
            ),
            vec2(half, cell_h),
        );
        let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
        child.spacing_mut().item_spacing = egui::Vec2::ZERO;
        controls::paint(&child, rect.min, name, label, c.text);
        child.add_space(18.0 + 6.0);
        let slot = color_slot(&mut draft.colors, i);
        let current = parse_hex(slot).unwrap_or(Color32::BLACK);
        let sw = Rect::from_min_size(child.cursor().min + vec2(0.0, 2.0), vec2(32.0, 36.0));
        let sid = Id::unique(("gelabber-theme-swatch", i));
        let resp = child.interact(sw, sid, Sense::click());
        child.painter().rect(
            sw,
            corner(5.0),
            c.panel,
            Stroke::new(1.0, c.border_strong),
            StrokeKind::Inside,
        );
        child
            .painter()
            .rect_filled(sw.shrink2(vec2(5.0, 7.0)), 0.0, current);
        if resp.hovered() {
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            s.color_open = if s.color_open == Some(i) {
                None
            } else {
                Some(i)
            };
        }
        let field_rect = Rect::from_min_max(
            pos2(sw.right() + 5.0, sw.top() - 2.0),
            pos2(rect.right(), sw.bottom() + 2.0),
        );
        let mut fchild = child.new_child(UiBuilder::new().max_rect(field_rect));
        let mut text_value = s.editing[i].clone().unwrap_or_else(|| slot.clone());
        let r = editor_field(
            &mut fchild,
            &format!("theme-hex-{i}"),
            &mut text_value,
            true,
            7,
        );
        if r.changed() {
            if parse_hex(&text_value).is_some() {
                *slot = text_value.to_lowercase();
            }
            s.editing[i] = Some(text_value);
        }
        if r.lost_focus() {
            s.editing[i] = None;
        }
        if s.color_open == Some(i) {
            let mut col = current;
            let shown = crate::widgets::Popover::new(("theme-color", i), sw).show(&ctx, |ui| {
                egui::widgets::color_picker::color_picker_color32(
                    ui,
                    &mut col,
                    egui::widgets::color_picker::Alpha::Opaque,
                )
            });
            match shown {
                Some(true) => {
                    *slot = hex_of(col);
                    s.editing[i] = None;
                }
                Some(false) => {}
                None => s.color_open = None,
            }
        }
    }
    ui.advance_cursor_after_rect(Rect::from_min_size(
        grid_top,
        vec2(width, 4.0 * cell_h + 3.0 * 10.0),
    ));
    ui.add_space(16.0);
    let mut save = false;
    let mut cancel = false;
    controls::row(ui, 8.0, 36.0, |ui| {
        save = ui
            .add(
                Button::new("Speichern und anwenden")
                    .primary()
                    .disabled(blocked || draft.name.trim().is_empty()),
            )
            .clicked();
        cancel = ui.add(Button::new("Abbrechen").ghost()).clicked();
    });
    let dirty_now = match &s.baseline {
        Some(b) => *b != draft,
        None => true,
    };
    if save {
        let mut t = draft.clone();
        t.name = t.name.trim().to_owned();
        if change(&ctx, app, Op::Save(Box::new(t))) {
            s.picked = None;
            s.baseline = None;
            s.keep_after_save = Some("Theme gespeichert und angewendet.");
            return;
        }
    }
    if cancel {
        if dirty_now {
            s.draft = Some(draft);
            s.after_discard = Some(Box::new(Pending2::Close));
            ask_discard(&ctx);
        } else {
            s.baseline = None;
        }
        return;
    }
    s.draft = Some(draft);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_round_trip_and_ops() {
        let all = presets();
        assert_eq!(all.len(), 14);
        for d in &all {
            assert_eq!(definition_of(&stored_of(d)).as_ref(), Some(d));
        }
        let doc = ThemeDocument::default();
        let custom = copy_theme(&preset("nord"));
        assert!(custom.name.ends_with(" – Eigene"));
        let saved = apply_op(&doc, &Op::Save(Box::new(custom.clone()))).unwrap();
        assert_eq!(saved.active, custom.id);
        let deleted = apply_op(&saved, &Op::Delete(custom.id.clone())).unwrap();
        assert_eq!(deleted.active, "dark");
        assert!(apply_op(&doc, &Op::Apply("nope".into())).is_err());
    }

    /// A theme change the server refuses, then a document it cannot read
    /// (theme/account.ts): the sync box says so and offers the retry.
    #[test]
    fn failed_theme_saves_and_unknown_documents() {
        use crate::app::testkit::{FakeApi, Reply, launcher, wait_for_label};
        use egui::accesskit::Role;
        use egui_kittest::kittest::Queryable;
        let _config = crate::platform::paths::testing::TempConfig::new("themes-failed");
        let garbage = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let serve = garbage.clone();
        let api = FakeApi::start(true, move |method, path, _| match (method, path) {
            ("PUT", "/me/themes") => Some(Reply::error(500, "internal")),
            ("GET", "/me/themes") if serve.load(std::sync::atomic::Ordering::SeqCst) => {
                Some(Reply::json(serde_json::json!({"active": 7})))
            }
            _ => None,
        });
        let mut harness = launcher(&api, "/settings/darstellung");
        wait_for_label(&mut harness, "Hell");
        // The card takes the click once the account's document is loaded.
        let start = std::time::Instant::now();
        while Theme::of(&harness.ctx).is_dark() {
            assert!(start.elapsed().as_secs() < 15, "the theme did not change");
            harness
                .get_by_role_and_label(Role::RadioButton, "Hell")
                .click();
            harness.run_steps(2);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let unsynced = "Noch nicht synchronisiert. Deine Änderung ist hier gespeichert.";
        wait_for_label(&mut harness, "Nicht gespeichert – erneut versuchen");
        // The Themes area holds the sync box with the account's state.
        harness
            .state_mut()
            .app()
            .expect("the app")
            .navigate("/settings/themes");
        wait_for_label(&mut harness, unsynced);
        // The local change shows meanwhile, as in the web.
        assert!(!Theme::of(&harness.ctx).is_dark());
        garbage.store(true, std::sync::atomic::Ordering::SeqCst);
        harness
            .get_by_role_and_label(Role::Button, "Erneut anwenden")
            .click();
        wait_for_label(&mut harness, "Unbekanntes Theme-Format vom Server.");
        // An operation that cannot apply says why, or the generic line.
        let doc = ThemeDocument::default();
        assert_eq!(
            apply_op(&doc, &Op::Apply("nope".into())).unwrap_err(),
            "Das ausgewählte Theme ist nicht mehr verfügbar."
        );
    }

    /// "System" follows the OS while the app runs, as prefers-color-scheme.
    #[test]
    fn system_follows_the_os_live() {
        // apply_theme stores the theme for the next start: not in the real config.
        let _config = crate::platform::paths::testing::TempConfig::new("themes-system");
        let ctx = egui::Context::default();
        let client = Client::new("http://127.0.0.1:9").unwrap();
        sync(&ctx).lock().local = Some(ThemeDocument {
            active: "system".into(),
            ..ThemeDocument::default()
        });
        let frame = |system: egui::Theme| {
            let input = egui::RawInput {
                system_theme: Some(system),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| apply_theme(ui.ctx(), &client));
            out.textures_delta.clear();
            Theme::of(&ctx).is_dark()
        };
        assert!(!frame(egui::Theme::Light));
        assert!(frame(egui::Theme::Dark));
        assert!(!frame(egui::Theme::Light));
        // Linux: the portal's preference, where winit reports none.
        crate::theme::set_os_dark(&ctx, true);
        assert!(frame(egui::Theme::Light));
    }

    #[test]
    fn import_is_strict() {
        let t = stored_of(&preset("nord"));
        let json = serde_json::to_vec(&t).unwrap();
        let imported = import_theme(&json).unwrap();
        assert!(imported.id.starts_with("custom-"));
        assert!(import_theme(b"{").is_err());
        assert!(import_theme(br#"{"version":1}"#).is_err());
    }
}

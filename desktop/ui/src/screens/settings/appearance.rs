//! "Darstellung" (pages/AppearanceSettings.tsx): the account's language,
//! light, dark or system, text size, compact voice rooms and reduced
//! motion.

use egui::{Id, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use gelabber_client::OpState;
use gelabber_i18n::{Locale, Text, common, language, settings as texts};

use super::controls::{self, Scope, Status};
use super::prefs::{self, UiScale};
use super::themes::{self, preset};
use crate::app::AppCtx;
use crate::text::{self, Block};
use crate::theme::{BaseColors, Theme, corner};
use crate::widgets::{Link, eased, focus_ring, keyboard_focus, link};

/// Each mode: id, label, hint. The presets' names are names, the same in
/// every language.
const MODES: [(&str, Text, Text); 3] = [
    ("light", texts::appearance_mode_light, || "Gelabber Hell"), // i18n-ok
    ("dark", texts::appearance_mode_dark, || "Gelabber Dunkel"), // i18n-ok
    (
        "system",
        texts::appearance_mode_system,
        texts::appearance_mode_system_hint,
    ),
];

#[derive(Default)]
pub struct State {
    status: Status,
    waiting: bool,
    /// "Weitere Themes" was chosen; the page navigates after the draw.
    pub(super) navigate: Option<&'static str>,
    /// The language section's own status line.
    language_status: Status,
    /// The language chosen last, until the server answers: for the status
    /// line only; [`settle_language`] goes back on a failure.
    language_saving: Option<Locale>,
}

/// `ModeSwatch`: page, panel strip, a text bar and an accent bar.
fn swatch(ui: &Ui, rect: Rect, colors: &BaseColors, radius: egui::CornerRadius, edges: [bool; 4]) {
    let painter = ui.painter();
    painter.rect_filled(rect, radius, colors.background);
    let inner = Rect::from_min_max(
        pos2(
            rect.left() + if edges[3] { 1.0 } else { 0.0 },
            rect.top() + 1.0,
        ),
        pos2(
            rect.right() - if edges[1] { 1.0 } else { 0.0 },
            rect.bottom() - 1.0,
        ),
    );
    let strip = Rect::from_min_size(inner.min, vec2(inner.width() * 0.3, inner.height()));
    painter
        .with_clip_rect(rect)
        .rect_filled(strip, egui::CornerRadius::ZERO, colors.panel);
    let bar = Rect::from_min_size(
        pos2(inner.left() + inner.width() * 0.4, inner.top() + 14.0),
        vec2(inner.width() * 0.4, 6.0),
    );
    painter.rect_filled(bar, corner(3.0), colors.text.gamma_multiply(0.8));
    let accent = Rect::from_min_size(
        pos2(
            inner.left() + inner.width() * 0.4,
            inner.bottom() - 12.0 - 10.0,
        ),
        vec2(inner.width() * 0.24, 10.0),
    );
    painter.rect_filled(accent, corner(3.0), colors.accent);
    // The border, only on the sides that keep it.
    let border = colors.border;
    if edges[0] {
        crate::widgets::rule_h(painter, border, rect.x_range(), rect.top());
    }
    if edges[2] {
        crate::widgets::rule_h(painter, border, rect.x_range(), rect.bottom() - 1.0);
    }
    if edges[1] {
        crate::widgets::rule_v(painter, border, rect.right() - 1.0, rect.y_range());
    }
    if edges[3] {
        crate::widgets::rule_v(painter, border, rect.left(), rect.y_range());
    }
}

pub fn area(ui: &mut Ui, app: &mut AppCtx, s: &mut State) {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let sync = themes::sync_state(ui.ctx(), app);
    if s.waiting && !sync.saving {
        s.waiting = false;
        match &sync.error {
            None => s.status.success(ui, common::saved()),
            Some(_) => s.status.error(ui, common::not_saved()),
        }
    }
    let active = sync.doc.active.clone();
    let gap = 12.0;
    let width = ui.available_width().min(640.0);
    let card_w = (width - 2.0 * gap) / 3.0;
    language_section(ui, app, s, card_w);
    controls::group_legend(ui, texts::appearance_color_scheme(), Some(Scope::Account));
    ui.add_space(4.0);
    let top = ui.cursor().min;
    let card_h = 126.3;
    let mut arrowed = None;
    for (i, (id, label, hint)) in MODES.iter().enumerate() {
        let (label, hint) = (label(), hint());
        let rect = Rect::from_min_size(
            pos2(top.x + i as f32 * (card_w + gap), top.y),
            vec2(card_w, card_h),
        );
        let wid = Id::unique(("gelabber-mode", *id));
        let checked = active == *id;
        // Radios of one name: the checked one is the group's Tab stop
        // (all of them while none is), the arrows choose the next.
        let none_checked = !MODES.iter().any(|m| m.0 == active);
        let stop = checked || none_checked || ui.memory(|m| m.has_focus(wid));
        let response = ui.interact(rect, wid, if stop { Sense::click() } else { Sense::CLICK });
        if response.has_focus() {
            // The arrows are the group's, not egui's spatial focus moves.
            let arrows = egui::EventFilter {
                horizontal_arrows: true,
                vertical_arrows: true,
                ..Default::default()
            };
            ui.memory_mut(|m| m.set_focus_lock_filter(wid, arrows));
            let step = ui.input(|i| {
                if i.key_pressed(egui::Key::ArrowRight) || i.key_pressed(egui::Key::ArrowDown) {
                    1
                } else if i.key_pressed(egui::Key::ArrowLeft) || i.key_pressed(egui::Key::ArrowUp) {
                    MODES.len() - 1
                } else {
                    0
                }
            });
            if step > 0 {
                arrowed = Some((i + step) % MODES.len());
            }
        }
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::RadioButton, true, checked, label)
        });
        let fill = eased(ui.ctx(), wid, response.hovered(), c.panel, c.hover);
        let painter = ui.painter();
        if response.has_focus() && keyboard_focus(ui.ctx()) {
            focus_ring(painter, rect, theme.radius.md, &theme);
        }
        painter.rect(
            rect,
            corner(theme.radius.md),
            fill,
            Stroke::new(1.0, if checked { c.accent } else { c.border_strong }),
            StrokeKind::Inside,
        );
        if checked {
            painter.rect_stroke(
                rect.shrink(1.0),
                corner(theme.radius.md - 1.0),
                Stroke::new(1.0, c.accent),
                StrokeKind::Inside,
            );
        }
        let sw = Rect::from_min_size(
            pos2(rect.left() + 13.0, rect.top() + 11.0),
            vec2(card_w - 26.0, 56.0),
        );
        let r = theme.radius.sm.round() as u8;
        if *id == "system" {
            let (l, d) = (preset("light").colors, preset("dark").colors);
            let half = sw.width() / 2.0;
            let left = Rect::from_min_size(sw.min, vec2(half, sw.height()));
            let right =
                Rect::from_min_size(pos2(sw.left() + half, sw.top()), vec2(half, sw.height()));
            swatch(
                ui,
                left,
                &l,
                egui::CornerRadius {
                    nw: r,
                    sw: r,
                    ne: 0,
                    se: 0,
                },
                [true, false, true, true],
            );
            swatch(
                ui,
                right,
                &d,
                egui::CornerRadius {
                    nw: 0,
                    sw: 0,
                    ne: r,
                    se: r,
                },
                [true, true, true, false],
            );
        } else {
            swatch(
                ui,
                sw,
                &preset(id).colors,
                corner(theme.radius.sm),
                [true; 4],
            );
        }
        controls::radio_mark(ui, pos2(rect.left() + 13.0, rect.top() + 78.0), checked);
        let x = rect.left() + 43.0;
        Block::single(ui, label, text::UI, c.text).paint(ui.painter(), pos2(x, rect.top() + 77.0));
        Block::single(ui, hint, text::META, c.muted).paint(
            ui.painter(),
            pos2(x, rect.top() + 77.0 + text::UI.line_px() + 2.0),
        );
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.clicked() && !checked && !sync.saving && sync.ready {
            themes::change(ui.ctx(), app, themes::Op::Apply(id.to_string()));
            s.waiting = true;
        }
    }
    if let Some(next) = arrowed {
        let id = MODES[next].0;
        ui.memory_mut(|m| m.request_focus(Id::unique(("gelabber-mode", id))));
        if active != id && !sync.saving && sync.ready {
            themes::change(ui.ctx(), app, themes::Op::Apply(id.to_string()));
            s.waiting = true;
        }
    }
    ui.allocate_exact_size(vec2(ui.available_width(), card_h), Sense::hover());
    if let Some(error) = sync.error.as_deref().filter(|_| !sync.ready) {
        ui.add_space(6.0);
        let w = ui.available_width();
        controls::paragraph(ui, error, text::META, c.danger, w);
        // The retry the Themes area offers, here too.
        if !sync.saving {
            ui.add_space(8.0);
            let label = if sync.pending {
                texts::appearance_retry_apply()
            } else {
                common::try_again()
            };
            if ui.add(crate::widgets::Button::new(label).small()).clicked() {
                themes::retry(ui.ctx(), app);
            }
        }
    }
    ui.add_space(10.0);
    // "Gerade aktiv: …" + the link to Themes.
    let other =
        (!MODES.iter().any(|m| m.0 == active)).then(|| themes::theme_name(&sync.doc, &active));
    let lead = other
        .map(|n| format!("{} ", texts::appearance_active_theme(&n)))
        .unwrap_or_default();
    let rich = text::Rich::new(controls::HINT)
        .push(&lead, controls::HINT, c.muted)
        .push(
            texts::appearance_more_themes(),
            controls::HINT,
            c.accent_text,
        )
        .layout(ui);
    let link_top = ui.cursor().min;
    text::show(ui, &rich);
    if let Some(r) = rich.section_rect(link_top, 1) {
        // `.settings-link`: underline 2px under the baseline, not at the line box.
        link::underline(
            ui.painter(),
            controls::HINT,
            r.top(),
            r.left(),
            r.right(),
            c.accent_text,
        );
        let id = Id::unique("gelabber-more-themes");
        if Link::interact(ui, r, id, texts::appearance_more_themes()).clicked() {
            s.navigate = Some("/settings/themes");
        }
    }
    let w = ui.available_width();
    s.status.ui(ui, w);
    ui.add_space(24.0 + 32.0);

    let mut p = prefs::get(ui.ctx());
    let before = p.clone();
    controls::group_legend(ui, texts::appearance_view(), Some(Scope::Device));
    // Stacked row: label, hint, chips.
    let row_top = ui.cursor().min;
    ui.add_space(12.0);
    controls::paint(
        ui,
        ui.cursor().min,
        texts::appearance_text_size(),
        text::UI,
        c.text,
    );
    ui.add_space(text::UI.line_px() + 4.0);
    controls::paint(
        ui,
        ui.cursor().min,
        texts::appearance_text_size_hint(),
        controls::HINT,
        c.muted,
    );
    ui.add_space(18.0 + 10.0);
    let labels: Vec<&str> = UiScale::ALL.iter().map(|s| s.label()).collect();
    let current = UiScale::ALL
        .iter()
        .position(|s| *s == p.ui_scale)
        .unwrap_or(0);
    if let Some(i) = controls::chips(ui, "ui-scale", &labels, current, 32.0, None) {
        p.ui_scale = UiScale::ALL[i];
    }
    ui.add_space(12.0);
    let bottom = ui.cursor().min.y;
    crate::widgets::rule_h(
        ui.painter(),
        c.border,
        row_top.x..=row_top.x + ui.available_width(),
        bottom,
    );
    ui.add_space(1.0);
    controls::switch_row(
        ui,
        "compact-rooms",
        texts::appearance_compact_rooms(),
        Some(texts::appearance_compact_rooms_hint()),
        &mut p.compact_rooms,
        false,
    );
    controls::switch_row(
        ui,
        "reduced-motion",
        texts::appearance_reduced_motion(),
        Some(texts::appearance_reduced_motion_hint()),
        &mut p.reduced_motion,
        false,
    );
    if p != before {
        prefs::update(ui.ctx(), |x| *x = p);
    }
}

/// The language a save is under way for (`changeAccountLocale`'s `next`).
/// It lives beside the app, not in the page's state, so a failure goes
/// back even after the page was left.
fn language_saving_id() -> Id {
    Id::unique("gelabber-language-saving")
}

/// Shows `next` at once and saves it with the account.
fn choose_language(ctx: &egui::Context, client: &gelabber_client::Client, next: Locale) {
    prefs::show_locale(ctx, next);
    client.set_locale(next.as_str());
    ctx.data_mut(|d| d.insert_temp(language_saving_id(), next));
}

/// Run by the app each frame, whatever it shows: once the newest language
/// save failed, shows the language the server last confirmed (`confirmed`),
/// only while nothing else chose a language meanwhile.
pub fn settle_language(
    ctx: &egui::Context,
    client: &gelabber_client::Client,
    confirmed: impl FnOnce() -> Option<String>,
) {
    let op = client.with_state(|st| st.op("locale").cloned());
    settle_on(ctx, op.as_ref(), confirmed);
}

/// [`settle_language`] for the save's state `op`, read once by the caller.
fn settle_on(
    ctx: &egui::Context,
    op: Option<&OpState>,
    confirmed: impl FnOnce() -> Option<String>,
) {
    let Some(chosen) = ctx.data(|d| d.get_temp::<Locale>(language_saving_id())) else {
        return;
    };
    let failed = match op {
        Some(OpState::Pending) => return,
        Some(OpState::Failed(_)) => true,
        Some(OpState::Done) | None => false,
    };
    ctx.data_mut(|d| d.remove::<Locale>(language_saving_id()));
    if failed
        && gelabber_i18n::get() == chosen
        && let Some(saved) = confirmed().as_deref().and_then(Locale::parse)
    {
        prefs::show_locale(ctx, saved);
    }
}

/// "Sprache" (`LanguageSettings`): the account's language. Each option is
/// named in its own language, with its name in the interface's language
/// below when that differs. A choice shows at once and is saved with the
/// account; a failed save goes back to the saved language
/// ([`settle_language`]). Hidden against a server older than v0.10, which
/// keeps no language.
fn language_section(ui: &mut Ui, app: &mut AppCtx, s: &mut State, card_w: f32) {
    let Some((_, confirmed)) = crate::app::account_locale(app.session, app.client) else {
        return;
    };
    // Again here, on the same reading as the status line: an answer that
    // came after the app's call this frame goes back before the status line
    // takes its words.
    let op = app.client.with_state(|st| st.op("locale").cloned());
    settle_on(ui.ctx(), op.as_ref(), || Some(confirmed));
    if s.language_saving.is_some() {
        match op {
            Some(OpState::Done) => {
                s.language_saving = None;
                s.language_status.success(ui, common::saved());
            }
            Some(OpState::Failed(_)) => {
                s.language_saving = None;
                s.language_status.error(ui, common::not_saved());
            }
            _ => {}
        }
    }
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let legend = ui.cursor().min;
    controls::group_legend(ui, language::section_title(), Some(Scope::Account));
    // The fieldset's legend names the group for screen readers.
    let title = Rect::from_min_size(legend, vec2(card_w, 20.8));
    ui.interact(
        title,
        Id::unique("gelabber-language-legend"),
        Sense::hover(),
    )
    .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, language::section_title()));
    ui.add_space(4.0);
    let top = ui.cursor().min;
    // RadioCard: 1px border and 12px padding around the taller option, a
    // label with its name below (`gap: 2px`); the grid row gives both cards
    // that height.
    let gap = 12.0;
    let card_h = 2.0 * 13.0 + text::UI.line_px() + 2.0 + text::META.line_px();
    let current = gelabber_i18n::get();
    let mut chosen = None;
    let mut arrowed = None;
    for (i, locale) in Locale::ALL.iter().copied().enumerate() {
        let rect = Rect::from_min_size(
            pos2(top.x + i as f32 * (card_w + gap), top.y),
            vec2(card_w, card_h),
        );
        let wid = Id::unique(("gelabber-language", locale.as_str()));
        let checked = current == locale;
        // One Tab stop for the group, the checked radio; the arrows choose.
        let stop = checked || ui.memory(|m| m.has_focus(wid));
        let response = ui.interact(rect, wid, if stop { Sense::click() } else { Sense::CLICK });
        if response.has_focus() {
            let arrows = egui::EventFilter {
                horizontal_arrows: true,
                vertical_arrows: true,
                ..Default::default()
            };
            ui.memory_mut(|m| m.set_focus_lock_filter(wid, arrows));
            let step = ui.input(|i| {
                [
                    egui::Key::ArrowRight,
                    egui::Key::ArrowDown,
                    egui::Key::ArrowLeft,
                    egui::Key::ArrowUp,
                ]
                .iter()
                .any(|k| i.key_pressed(*k))
            });
            if step {
                // Two options: every arrow moves to the other one.
                arrowed = Some(Locale::ALL[(i + 1) % Locale::ALL.len()]);
            }
        }
        let label = language::option(locale);
        let name = language::name(locale);
        let description = (name != label).then_some(name);
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::RadioButton, true, checked, label)
        });
        let fill = eased(ui.ctx(), wid, response.hovered(), c.panel, c.hover);
        let painter = ui.painter();
        if response.has_focus() && keyboard_focus(ui.ctx()) {
            focus_ring(painter, rect, theme.radius.md, &theme);
        }
        painter.rect(
            rect,
            corner(theme.radius.md),
            fill,
            Stroke::new(1.0, if checked { c.accent } else { c.border_strong }),
            StrokeKind::Inside,
        );
        if checked {
            painter.rect_stroke(
                rect.shrink(1.0),
                corner(theme.radius.md - 1.0),
                Stroke::new(1.0, c.accent),
                StrokeKind::Inside,
            );
        }
        // Radio and text centred in the card (`align-items: center`).
        let text_h = text::UI.line_px() + description.map_or(0.0, |_| 2.0 + text::META.line_px());
        controls::radio_mark(
            ui,
            pos2(rect.left() + 13.0, rect.center().y - 10.0),
            checked,
        );
        // Border, padding, the 20px radio and the 12px gap.
        let x = rect.left() + 45.0;
        let y = rect.center().y - text_h / 2.0;
        Block::single(ui, label, text::UI, c.text).paint(ui.painter(), pos2(x, y));
        if let Some(description) = description {
            Block::single(ui, description, text::META, c.muted)
                .paint(ui.painter(), pos2(x, y + text::UI.line_px() + 2.0));
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.clicked() && !checked {
            chosen = Some(locale);
        }
    }
    if let Some(next) = arrowed {
        ui.memory_mut(|m| m.request_focus(Id::unique(("gelabber-language", next.as_str()))));
        if next != current {
            chosen = Some(next);
        }
    }
    ui.allocate_exact_size(vec2(ui.available_width(), card_h), Sense::hover());
    // Not held back while a choice saves: the newest choice is the one kept.
    if let Some(next) = chosen {
        choose_language(ui.ctx(), app.client, next);
        s.language_saving = Some(next);
        s.language_status.clear();
    }
    ui.add_space(10.0);
    controls::hint(ui, language::hint());
    let w = ui.available_width();
    s.language_status.ui(ui, w);
    ui.add_space(24.0 + 32.0);
}

#[cfg(test)]
mod tab_tests {
    use egui_kittest::kittest::Queryable;
    use gelabber_i18n::Locale;
    use serde_json::json;

    use crate::app::testkit::{
        FakeApi, Reply, focused_label, launcher, lena, wait_for, wait_for_label, workspace,
    };
    use crate::platform::paths::testing::TempConfig;

    /// Lena on a v0.10 server, German; `PUT /me/locale` saves or fails.
    fn language_api(saves: bool) -> FakeApi {
        let mut user = lena();
        user["locale"] = "de".into();
        FakeApi::start(true, move |method, path, body| match (method, path) {
            ("GET", "/auth/session") => {
                Some(Reply::json(json!({"user": user, "csrf_token": "csrf"})))
            }
            ("GET", "/me") => Some(Reply::json(user.clone())),
            ("PUT", "/me/locale") if saves => {
                let body: serde_json::Value = serde_json::from_str(body).unwrap();
                Some(Reply::json(json!({"locale": body["locale"]})))
            }
            ("PUT", "/me/locale") => Some(Reply::error(500, "internal")),
            _ => workspace(method, path),
        })
    }

    /// English shows at once and is saved with the account and this device.
    #[test]
    fn a_language_shows_at_once_and_is_saved() {
        let _locale = gelabber_i18n::on_this_thread(Locale::De);
        let _config = TempConfig::new("language-saved");
        let api = language_api(true);
        let mut harness = launcher(&api, "/settings/darstellung");
        wait_for_label(&mut harness, "Sprache");
        harness.get_by_label("English").click();
        harness.run_steps(2);
        assert_eq!(gelabber_i18n::get(), Locale::En);
        wait_for_label(&mut harness, "Saved.");
        assert!(api.saw("PUT /me/locale"));
        assert!(harness.query_by_label("Language").is_some());
        // The settings navigation names the area in English too.
        assert!(harness.query_by_label("Appearance").is_some());
        assert!(harness.query_by_label("Darstellung").is_none());
        assert_eq!(gelabber_i18n::get(), Locale::En);
        assert_eq!(
            super::prefs::get(&harness.ctx).locale,
            Some(Locale::En),
            "the next start shows it too"
        );
    }

    /// A failed save goes back to the account's language and says so.
    #[test]
    fn a_failed_language_save_goes_back() {
        let _locale = gelabber_i18n::on_this_thread(Locale::De);
        let _config = TempConfig::new("language-failed");
        let api = language_api(false);
        let mut harness = launcher(&api, "/settings/darstellung");
        wait_for_label(&mut harness, "Sprache");
        harness.get_by_label("English").click();
        // The fake server may answer within the click's frames, so the
        // switch itself is not asserted here (see the test above).
        wait_for(&mut harness, "the save", |_| api.saw("PUT /me/locale"));
        wait_for_label(&mut harness, "Nicht gespeichert – erneut versuchen");
        assert_eq!(gelabber_i18n::get(), Locale::De);
        assert!(harness.query_by_label("Sprache").is_some());
    }

    /// A save that fails after the page was left still goes back, as the
    /// web's `changeAccountLocale` does in its own catch.
    #[test]
    fn a_language_save_that_fails_elsewhere_goes_back() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        let _locale = gelabber_i18n::on_this_thread(Locale::De);
        let _config = TempConfig::new("language-failed-elsewhere");
        let mut user = lena();
        user["locale"] = "de".into();
        let release = Arc::new(AtomicBool::new(false));
        let held = release.clone();
        let api = FakeApi::start(true, move |method, path, _| match (method, path) {
            ("GET", "/auth/session") => {
                Some(Reply::json(json!({"user": user, "csrf_token": "csrf"})))
            }
            ("GET", "/me") => Some(Reply::json(user.clone())),
            ("PUT", "/me/locale") => {
                while !held.load(Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Some(Reply::error(500, "internal"))
            }
            _ => workspace(method, path),
        });
        let mut harness = launcher(&api, "/settings/darstellung");
        wait_for_label(&mut harness, "Sprache");
        harness.get_by_label("English").click();
        wait_for(&mut harness, "the save", |_| api.saw("PUT /me/locale"));
        assert_eq!(gelabber_i18n::get(), Locale::En);
        // Another area, in English, while the save is under way.
        harness.get_by_label("Audio").click();
        wait_for(&mut harness, "the Audio area", |h| {
            h.query_by_label("Language").is_none() && h.query_by_label("Audio").is_some()
        });
        release.store(true, Ordering::Relaxed);
        wait_for(&mut harness, "German again", |_| {
            gelabber_i18n::get() == Locale::De
        });
        assert_eq!(super::prefs::get(&harness.ctx).locale, Some(Locale::De));
    }

    /// A server older than v0.10 keeps no language: no section.
    #[test]
    fn an_older_server_shows_no_language_section() {
        let _locale = gelabber_i18n::on_this_thread(Locale::De);
        let _config = TempConfig::new("language-older");
        let api = FakeApi::start(true, |method, path, _| workspace(method, path));
        let mut harness = launcher(&api, "/settings/darstellung");
        wait_for_label(&mut harness, "Weitere Themes");
        assert!(harness.query_by_label("Sprache").is_none());
        assert!(harness.query_by_label("Deutsch").is_none());
    }

    /// The area's heading holds the focus on arrival, then the controls
    /// are named Tab stops, each radio group once (the checked radio, the
    /// chosen chip), as the browser has them; the radio marks inside the
    /// cards used to swallow the Tab after the first card.
    #[test]
    fn tab_reaches_every_control_once() {
        let _config = TempConfig::new("appearance-tab");
        let api = FakeApi::start(true, |method, path, _| workspace(method, path));
        let mut harness = launcher(&api, "/settings/darstellung");
        // The account's document decides which radio is checked.
        wait_for(&mut harness, "the theme document", |_| {
            api.saw("GET /me/themes")
        });
        harness.run_steps(20);
        let mut order = Vec::new();
        for _ in 0..30 {
            harness.key_press(egui::Key::Tab);
            harness.run_steps(2);
            let label = focused_label(&harness).unwrap_or_default();
            if label == "Zum Inhalt springen" && !order.is_empty() {
                break;
            }
            order.push(label);
        }
        assert_eq!(
            order,
            [
                "Dunkel",
                "Weitere Themes",
                "Kompakt",
                "Kompakte Sprachkanäle",
                "Bewegung reduzieren"
            ]
        );
    }
}

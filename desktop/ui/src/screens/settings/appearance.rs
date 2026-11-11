//! "Darstellung" (pages/AppearanceSettings.tsx): the account's language,
//! light, dark or system, text size, compact voice rooms and reduced
//! motion.

use egui::{Id, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use gelabber_client::OpState;
use gelabber_i18n::{Locale, common, language};

use super::controls::{self, Scope, Status};
use super::prefs::{self, UiScale};
use super::themes::{self, preset};
use crate::app::AppCtx;
use crate::text::{self, Block};
use crate::theme::{BaseColors, Theme, corner};
use crate::widgets::{Link, eased, focus_ring, keyboard_focus, link};

const MODES: [(&str, &str, &str); 3] = [
    ("light", "Hell", "Gelabber Hell"),
    ("dark", "Dunkel", "Gelabber Dunkel"),
    ("system", "System", "Folgt dem Gerät"),
];

#[derive(Default)]
pub struct State {
    status: Status,
    waiting: bool,
    /// "Weitere Themes" was chosen; the page navigates after the draw.
    pub(super) navigate: Option<&'static str>,
    /// The language section's own status line.
    language_status: Status,
    /// The language chosen last, until the server answers.
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
            None => s.status.success(ui, "Gespeichert."),
            Some(_) => s.status.error(ui, "Nicht gespeichert – erneut versuchen"),
        }
    }
    let active = sync.doc.active.clone();
    let gap = 12.0;
    let width = ui.available_width().min(640.0);
    let card_w = (width - 2.0 * gap) / 3.0;
    language_section(ui, app, s, card_w);
    controls::group_legend(ui, "Farbschema", Some(Scope::Account));
    ui.add_space(4.0);
    let top = ui.cursor().min;
    let card_h = 126.3;
    let mut arrowed = None;
    for (i, (id, label, hint)) in MODES.iter().enumerate() {
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
            egui::WidgetInfo::selected(egui::Role::RadioButton, true, checked, *label)
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
                "Erneut anwenden"
            } else {
                "Erneut versuchen"
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
        .map(|n| format!("Gerade aktiv: {n}. "))
        .unwrap_or_default();
    let rich = text::Rich::new(controls::HINT)
        .push(&lead, controls::HINT, c.muted)
        .push("Weitere Themes", controls::HINT, c.accent_text)
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
        if Link::interact(ui, r, id, "Weitere Themes").clicked() {
            s.navigate = Some("/settings/themes");
        }
    }
    let w = ui.available_width();
    s.status.ui(ui, w);
    ui.add_space(24.0 + 32.0);

    let mut p = prefs::get(ui.ctx());
    let before = p.clone();
    controls::group_legend(ui, "Ansicht", Some(Scope::Device));
    // Stacked row: label, hint, chips.
    let row_top = ui.cursor().min;
    ui.add_space(12.0);
    controls::paint(ui, ui.cursor().min, "Textgröße", text::UI, c.text);
    ui.add_space(text::UI.line_px() + 4.0);
    controls::paint(
        ui,
        ui.cursor().min,
        "Gilt für die ganze Oberfläche.",
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
        "Kompakte Sprachkanäle",
        Some("Kleinere Avatare und weniger Abstand."),
        &mut p.compact_rooms,
        false,
    );
    controls::switch_row(
        ui,
        "reduced-motion",
        "Bewegung reduzieren",
        Some("Berücksichtigt auch die Einstellung deines Geräts."),
        &mut p.reduced_motion,
        false,
    );
    if p != before {
        prefs::update(ui.ctx(), |x| *x = p);
    }
}

/// "Sprache" (`LanguageSettings`): the account's language. Each option is
/// named in its own language, with its name in the interface's language
/// below when that differs. A choice shows at once and is saved with the
/// account; a failed save goes back to the saved language. Hidden against
/// a server older than v0.10, which keeps no language.
fn language_section(ui: &mut Ui, app: &mut AppCtx, s: &mut State, card_w: f32) {
    let Some((_, saved)) = crate::app::account_locale(app.session, app.client) else {
        return;
    };
    if let Some(chosen) = s.language_saving {
        match app.client.with_state(|st| st.op("locale").cloned()) {
            Some(OpState::Done) => {
                s.language_saving = None;
                s.language_status.success(ui, common::saved());
            }
            Some(OpState::Failed(_)) => {
                s.language_saving = None;
                // Only while nothing else chose a language meanwhile.
                if let Some(saved) = Locale::parse(&saved)
                    && gelabber_i18n::get() == chosen
                {
                    prefs::show_locale(ui.ctx(), saved);
                }
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
    let (gap, card_h) = (12.0, 58.0);
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
        let x = rect.left() + 43.0;
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
        prefs::show_locale(ui.ctx(), next);
        app.client.set_locale(next.as_str());
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
        harness.run_steps(2);
        assert_eq!(gelabber_i18n::get(), Locale::En);
        wait_for_label(&mut harness, "Nicht gespeichert – erneut versuchen");
        assert_eq!(gelabber_i18n::get(), Locale::De);
        assert!(harness.query_by_label("Sprache").is_some());
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

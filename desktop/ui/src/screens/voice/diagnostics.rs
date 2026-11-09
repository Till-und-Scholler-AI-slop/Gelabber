//! The connection views (voice/ConnectionStatus.tsx, connectionSummary.ts,
//! VoiceDiagnostics.tsx, connection.css): the status block of the call
//! bar's popover and of the "Diagnose" tab, and the tab itself with what
//! is sent and received, the microphone, the raw figures under "Für die
//! Fehlersuche" and the JSON export.

use egui::{Color32, Rect, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2};
use gelabber_call::diagnostics::{self as diag, Diagnostics, FlowGroup};
use gelabber_call::{CallState, CallStatus, Connection, MicStatus, logic};

use crate::app::AppCtx;
use crate::text::{self, Block, TextStyle};
use crate::theme::{Theme, corner, mix};
use crate::widgets::Button;

/// `CALL_QUALITY_LIMITS`.
const RTT_LIMITS: [f64; 3] = [150.0, 300.0, 500.0];
const LOSS_LIMITS: [f64; 3] = [0.02, 0.05, 0.1];
/// The figures' numbers (`600 22px / 1.25`, 20px in the popover).
const FIGURE: TextStyle = TextStyle::new(22.0, 600.0, 1.25);
const FIGURE_COMPACT: TextStyle = TextStyle::new(20.0, 600.0, 1.25);

/// `QUALITY_LABEL`.
pub fn quality_label(level: u8) -> &'static str {
    match level {
        4 => "gut",
        3 => "ordentlich",
        2 => "schwach",
        _ => "schlecht",
    }
}

/// `callQuality`'s level of a measurement; 0 before anything was measured.
pub fn level(connection: &Connection) -> u8 {
    logic::quality_level(connection.rtt_ms, connection.loss)
}

/// `callQualitySummary`: "Verbindung: gut · 38 ms · 0 % Verlust".
pub fn summary(connection: &Connection) -> Option<String> {
    let level = level(connection);
    if level == 0 {
        return None;
    }
    let mut parts = vec![format!("Verbindung: {}", quality_label(level))];
    if let Some(rtt) = connection.rtt_ms {
        parts.push(format!("{} ms", rtt.round() as i64));
    }
    if let Some(loss) = connection.loss {
        parts.push(format!(
            "{} % Verlust",
            diag::german((loss * 100.0).round(), 0, false)
        ));
    }
    Some(parts.join(" · "))
}

/// `callQualityDetail`: "Direkt verbunden, kaum Verlust."
pub fn detail(connection: &Connection) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    match connection.relay {
        Some(false) => parts.push("Direkt verbunden".into()),
        Some(true) => parts.push("Über einen Relay-Server verbunden".into()),
        None => {}
    }
    if let Some(loss) = connection.loss {
        let pct = diag::german((loss * 100.0).round(), 0, false);
        parts.push(if loss < LOSS_LIMITS[0] {
            "kaum Verlust".into()
        } else if loss < LOSS_LIMITS[1] {
            format!("etwas Verlust ({pct} %)")
        } else {
            format!("hoher Verlust ({pct} %)")
        });
    }
    if let Some(rtt) = connection.rtt_ms
        && rtt >= RTT_LIMITS[1]
    {
        parts.push(format!("hohe Verzögerung ({} ms)", rtt.round() as i64));
    }
    if parts.is_empty() {
        return None;
    }
    let mut out = parts.join(", ");
    if let Some(first) = out.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    Some(out + ".")
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    Good,
    Weak,
    Bad,
}

fn tone(value: f64, limits: [f64; 3]) -> Tone {
    if value >= limits[2] {
        Tone::Bad
    } else if value >= limits[1] {
        Tone::Weak
    } else {
        Tone::Good
    }
}

pub fn ping_tone(ms: f64) -> Tone {
    tone(ms, RTT_LIMITS)
}

pub fn loss_tone(ratio: f64) -> Tone {
    tone(ratio, LOSS_LIMITS)
}

/// The figure's colour for a tone (`--lr-warning` a step towards the text).
fn tone_color(theme: &Theme, tone: Option<Tone>) -> Color32 {
    match tone {
        Some(Tone::Weak) => mix(theme.c.warning, theme.c.text, 0.3),
        Some(Tone::Bad) => theme.c.danger,
        _ => theme.c.text,
    }
}

/// The microphone's processing in words (`useAudioProcessing().message`,
/// the desktop app's `nativeInfo`), or why there is none; `chosen` is the
/// microphone picked in the settings.
pub fn processing_message(state: &CallState, mode: &str, chosen: &str) -> String {
    if state.channel.is_none() {
        return "Mikrofon inaktiv".into();
    }
    if state.mic != MicStatus::Live {
        return logic::MIC_LISTEN_ONLY.into();
    }
    processing_text(mode, state.mic_input.as_deref(), chosen)
}

/// `nativeInfo`'s message for a capture on `captured`: the mode, and
/// "; Standardmikrofon" where the chosen microphone did not open.
pub fn processing_text(mode: &str, captured: Option<&str>, chosen: &str) -> String {
    let words = match mode {
        "browser" => "Desktop-App: WebRTC-Rauschfilter im nativen Kern",
        "original" => "Desktop-App: Originalton (Stereo, ohne Filter)",
        _ => "Desktop-App: RNNoise im nativen Kern",
    };
    if captured.is_some_and(|c| c != chosen) {
        format!("{words}; Standardmikrofon")
    } else {
        words.into()
    }
}

/// A measurement runs (`polling.voice`): connected in a call.
pub fn measuring(state: &CallState) -> bool {
    state.channel.is_some() && state.status == CallStatus::Connected
}

/// A figure: its label, the value with its unit, the value's tone.
type Figure<'a> = (&'a str, Option<(String, &'a str)>, Option<Tone>);

/// `SignalBars` at `rect.min`: four bars of `width`, `gap` apart, the
/// heights from the bottom.
fn signal(ui: &Ui, origin: egui::Pos2, level: u8, heights: [f32; 4], width: f32, gap: f32) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let color = match level {
        0 => c.muted,
        1 => c.danger,
        2 => c.warning,
        _ => c.online,
    };
    let bottom = origin.y + heights[3];
    for (i, h) in heights.iter().enumerate() {
        let x = origin.x + i as f32 * (width + gap);
        let lit = (i as u8) < level;
        ui.painter().rect_filled(
            Rect::from_min_max(pos2(x, bottom - h), pos2(x + width, bottom)),
            corner(1.0),
            if lit {
                color
            } else {
                color.gamma_multiply(0.28)
            },
        );
    }
}

/// `ConnectionStatus`: drawn from `ui.cursor()` over `width`; the full
/// block for the tab, `compact` for the popover. Allocates its height.
pub fn status(
    ui: &mut Ui,
    connection: Option<&Connection>,
    measuring: bool,
    pings: &[Option<f64>],
    compact: bool,
    width: f32,
) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let quality = connection
        .filter(|_| measuring)
        .filter(|conn| level(conn) > 0);
    let lvl = quality.map_or(0, level);
    let top = ui.cursor().min;
    let painter = ui.painter().clone();
    let (pad_t, pad_x, pad_b, gap) = if compact {
        (0.0, 0.0, 0.0, 12.0)
    } else {
        (18.0, 20.0, 16.0, 16.0)
    };
    let inner_left = top.x + 1.0 * f32::from(!compact) + pad_x;
    let inner_w = width - 2.0 * (pad_x + f32::from(!compact));
    let mut y = top.y + f32::from(!compact) + pad_t;

    // Head: bars, verdict, meta.
    let heading = match quality {
        Some(q) => format!("Verbindung {}", quality_label(level(q))),
        None if measuring => "Verbindung wird gemessen …".into(),
        None => "Keine laufende Messung".into(),
    };
    let (bars_w, bars_h) = if compact {
        (4.0 * 4.0 + 3.0 * 2.0, 20.0)
    } else {
        (4.0 * 5.0 + 3.0 * 3.0, 28.0)
    };
    let title_style = if compact {
        text::UI.weight(600.0)
    } else {
        text::TITLE_SM
    };
    let meta = if compact {
        quality.and_then(|q| q.relay).map(|relay| {
            if relay {
                "über Relay"
            } else {
                "direkt verbunden"
            }
            .to_owned()
        })
    } else {
        measuring.then(|| format!("misst alle {} s", diag::INTERVAL_MS / 1000))
    };
    let meta_block = meta.map(|m| Block::single(ui, &m, text::META, c.subtle));
    let meta_w = meta_block
        .as_ref()
        .map_or(0.0, |b| b.width() + if compact { 0.0 } else { 13.0 });
    let text_x = inner_left + bars_w + if compact { 12.0 } else { 14.0 };
    let text_w = (inner_left + inner_w - text_x - meta_w - 14.0).max(20.0);
    let title = Block::wrapped(ui, &heading, title_style, c.text, text_w);
    let verdict = (!compact)
        .then(|| quality.and_then(detail))
        .flatten()
        .map(|d| Block::wrapped(ui, &d, text::BODY, c.muted, text_w));
    let text_h = title.height() + verdict.as_ref().map_or(0.0, |v| 2.0 + v.height());
    let head_h = text_h.max(bars_h);
    if compact {
        let cy = y + head_h / 2.0;
        signal(
            ui,
            pos2(inner_left, cy - 10.0),
            lvl,
            [6.0, 10.0, 15.0, 20.0],
            4.0,
            2.0,
        );
        title.paint(&painter, pos2(text_x, cy - title.height() / 2.0));
        if let Some(m) = &meta_block {
            m.paint(
                &painter,
                pos2(inner_left + inner_w - m.width(), cy - m.height() / 2.0),
            );
        }
    } else {
        signal(
            ui,
            pos2(inner_left, y + 1.0),
            lvl,
            [8.0, 14.0, 20.0, 27.0],
            5.0,
            3.0,
        );
        title.paint(&painter, pos2(text_x, y));
        if let Some(v) = &verdict {
            v.paint(&painter, pos2(text_x, y + title.height() + 2.0));
        }
        if let Some(m) = &meta_block {
            let mx = inner_left + inner_w - m.width();
            m.paint(&painter, pos2(mx, y + 3.0));
            painter.circle_filled(
                pos2(mx - 7.0 - 3.0, y + 3.0 + m.height() / 2.0),
                3.0,
                c.online,
            );
        }
    }
    y += head_h + gap;

    // Figures.
    if !compact {
        crate::widgets::rule_h(&painter, c.border, inner_left..=inner_left + inner_w, y);
        y += 1.0 + 14.0;
    }
    let cols = if compact { 2 } else { 3 };
    let col_w = (inner_w - 12.0 * (cols as f32 - 1.0)) / cols as f32;
    let number = if compact { FIGURE_COMPACT } else { FIGURE };
    let rtt = quality.and_then(|q| q.rtt_ms);
    let loss = quality.and_then(|q| q.loss);
    let mut figures: Vec<Figure> = vec![
        (
            "Ping",
            rtt.map(|ms| (format!("{}", ms.round() as i64), "ms")),
            rtt.map(ping_tone),
        ),
        (
            "Paketverlust",
            loss.map(|l| (diag::german(l * 100.0, 1, true), "%")),
            loss.map(loss_tone),
        ),
    ];
    if !compact {
        let path = quality
            .and_then(|q| q.relay)
            .map(|r| (if r { "Relay" } else { "Direkt" }.to_owned(), ""));
        figures.push(("Verbunden", path, None));
    }
    let mut fig_h: f32 = 0.0;
    for (i, (label, value, tone)) in figures.iter().enumerate() {
        let x = inner_left + i as f32 * (col_w + 12.0);
        let dt = Block::single(ui, label, text::META, c.subtle);
        dt.paint(&painter, pos2(x, y));
        let vy = y + dt.height() + 2.0;
        let color = tone_color(&theme, *tone);
        let dd = match value {
            Some((v, unit)) if !unit.is_empty() => text::Rich::new(number)
                .push(v, number, color)
                .push(&format!(" {unit}"), text::UI, c.subtle)
                .layout(ui),
            Some((v, _)) => Block::single(ui, v, number, color),
            None => Block::single(ui, "–", number, c.text),
        };
        dd.paint(&painter, pos2(x, vy));
        fig_h = fig_h.max(dt.height() + 2.0 + number.line_px());
    }
    y += fig_h + gap;

    // Ping trace.
    let seconds = pings.len() as u64 * diag::INTERVAL_MS / 1000;
    let known: Vec<f64> = pings.iter().flatten().copied().collect();
    if !compact {
        let caption = Block::single(
            ui,
            &format!("Ping der letzten {seconds} s"),
            text::META,
            c.subtle,
        );
        caption.paint(&painter, pos2(inner_left, y));
        if !known.is_empty() {
            let low = known.iter().copied().fold(f64::INFINITY, f64::min).round() as i64;
            let high = known.iter().copied().fold(0.0, f64::max).round() as i64;
            let range = if low == high {
                format!("{low} ms")
            } else {
                format!("{low}–{high} ms")
            };
            let r = Block::single(ui, &range, text::META, c.subtle);
            r.paint(&painter, pos2(inner_left + inner_w - r.width(), y));
        }
        y += caption.height() + 8.0;
    }
    let (bars_area, bar_gap) = if compact { (32.0, 2.0) } else { (46.0, 3.0) };
    let topmost = known.iter().copied().fold(100.0, f64::max);
    let n = pings.len().max(1) as f32;
    let bar_w = (inner_w - bar_gap * (n - 1.0)) / n;
    for (i, ms) in pings.iter().enumerate() {
        let Some(ms) = ms else { continue };
        let h = (bars_area * (ms / topmost).max(0.08) as f32).min(bars_area);
        let x = inner_left + i as f32 * (bar_w + bar_gap);
        let (color, alpha) = match ping_tone(*ms) {
            Tone::Good => (c.online, 0.7),
            Tone::Weak => (c.warning, 0.9),
            Tone::Bad => (c.danger, 1.0),
        };
        painter.rect_filled(
            Rect::from_min_max(pos2(x, y + bars_area - h), pos2(x + bar_w, y + bars_area)),
            egui::CornerRadius {
                nw: 1,
                ne: 1,
                sw: 0,
                se: 0,
            },
            color.gamma_multiply(alpha),
        );
    }
    y += bars_area;
    crate::widgets::rule_h(&painter, c.border, inner_left..=inner_left + inner_w, y);
    y += 1.0 + 5.0;
    let from = Block::single(ui, &format!("vor {seconds} s"), text::META, c.subtle);
    let now = Block::single(ui, "jetzt", text::META, c.subtle);
    from.paint(&painter, pos2(inner_left, y));
    now.paint(&painter, pos2(inner_left + inner_w - now.width(), y));
    y += from.height();
    let bottom = y + pad_b + f32::from(!compact);
    if !compact {
        let rect = Rect::from_min_max(top, pos2(top.x + width, bottom));
        painter.rect(
            rect,
            corner(theme.radius.md),
            Color32::TRANSPARENT,
            Stroke::new(1.0, c.border),
            StrokeKind::Inside,
        );
    }
    ui.allocate_rect(
        Rect::from_min_max(top, pos2(top.x + width, bottom)),
        Sense::hover(),
    );
}

/// A section head (`.connection-block-head`): the title, an optional note.
fn block_head(ui: &mut Ui, title: &str, note: Option<&str>, width: f32) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let top = ui.cursor().min;
    let t = Block::single(ui, title, text::UI.weight(600.0), c.text);
    t.paint(ui.painter(), top);
    if let Some(n) = note {
        let b = Block::single(ui, n, text::META, c.subtle);
        b.paint(
            ui.painter(),
            pos2(top.x + width - b.width(), top.y + t.height() - b.height()),
        );
    }
    ui.allocate_rect(
        Rect::from_min_size(top, vec2(width, t.height() + 10.0)),
        Sense::hover(),
    );
}

/// `.connection-facts`: label left, value right, two to a row (`wide`
/// rows across), a hairline under each. `style` is the rows' font.
fn facts(ui: &mut Ui, rows: &[(&str, String, bool)], width: f32, style: TextStyle, pad: f32) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let col_w = (width - 28.0) / 2.0;
    let mut y = ui.cursor().min.y;
    let left = ui.cursor().min.x;
    let mut col = 0;
    let mut row_h: f32 = 0.0;
    let mut cells: Vec<(f32, f32, &str, &str)> = Vec::new();
    let flush = |ui: &mut Ui, y: &mut f32, cells: &mut Vec<(f32, f32, &str, &str)>, row_h: f32| {
        for (x, w, dt, dd) in cells.drain(..) {
            let label = Block::single(ui, dt, style, c.subtle);
            let value = Block::wrapped(
                ui,
                dd,
                style.weight(500.0),
                c.text,
                (w - label.width() - 12.0).max(20.0),
            );
            label.paint(ui.painter(), pos2(x, *y + pad));
            value.paint(ui.painter(), pos2(x + w - value.width(), *y + pad));
            crate::widgets::rule_h(ui.painter(), c.border, x..=x + w, *y + row_h - 1.0);
        }
        *y += row_h;
    };
    for (dt, dd, wide) in rows {
        if *wide && col == 1 {
            flush(ui, &mut y, &mut cells, row_h);
            col = 0;
        }
        let w = if *wide { width } else { col_w };
        let x = left + if col == 1 { col_w + 28.0 } else { 0.0 };
        let value = Block::wrapped(ui, dd, style.weight(500.0), c.text, w * 0.6);
        row_h = row_h.max(2.0 * pad + value.height().max(style.line_px()) + 1.0);
        if col == 0 && cells.is_empty() {
            row_h = 2.0 * pad + value.height().max(style.line_px()) + 1.0;
        }
        cells.push((x, w, dt, dd.as_str()));
        if *wide || col == 1 {
            flush(ui, &mut y, &mut cells, row_h);
            col = 0;
        } else {
            col = 1;
        }
    }
    if !cells.is_empty() {
        flush(ui, &mut y, &mut cells, row_h);
    }
    let top = ui.cursor().min.y;
    ui.allocate_rect(
        Rect::from_min_max(pos2(left, top), pos2(left + width, y)),
        Sense::hover(),
    );
}

fn on_off(value: Option<bool>) -> String {
    match value {
        Some(true) => "an".into(),
        Some(false) => "aus".into(),
        None => "–".into(),
    }
}

fn yes_no(value: bool) -> String {
    if value { "ja" } else { "nein" }.into()
}

/// A figure the core did not report reads as a dash.
fn known(text: String) -> String {
    if text == "unbekannt" {
        "–".into()
    } else {
        text
    }
}

/// A dashed rounded box (`.connection-empty`).
fn dashed_box(ui: &Ui, rect: Rect, color: Color32) {
    let pts = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
        rect.left_top(),
    ];
    ui.painter()
        .extend(Shape::dashed_line(&pts, Stroke::new(1.0, color), 3.0, 3.0));
}

fn disclosure_id() -> egui::Id {
    egui::Id::unique("gelabber-voice-diagnostics-debug")
}

/// The "Diagnose" tab (`VoiceDiagnostics`) over `width`.
pub fn tab(ui: &mut Ui, app: &mut AppCtx, width: f32) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let state = app.call.state();
    let data = app.call.diagnostics();
    let measuring = measuring(&state);
    let p = crate::screens::settings::prefs::get(ui.ctx());
    ui.spacing_mut().item_spacing.y = 0.0;

    // The status block on its panel.
    let top = ui.cursor().min;
    let bg = ui.painter().add(Shape::Noop);
    status(
        ui,
        state.connection.as_ref(),
        measuring,
        &data.pings(45),
        false,
        width,
    );
    let block = Rect::from_min_max(top, pos2(top.x + width, ui.cursor().min.y));
    ui.painter().set(
        bg,
        Shape::rect_filled(block, corner(theme.radius.md), c.panel),
    );
    upload_tip(ui, &state, &data, width, p.economy_mode);
    ui.add_space(28.0);

    // Übertragung.
    let groups = data
        .latest()
        .map(|s| diag::flow_groups(&s.flows))
        .unwrap_or_default();
    block_head(
        ui,
        "Übertragung",
        (!groups.is_empty() && !measuring).then_some("letzte Messung"),
        width,
    );
    if groups.is_empty() {
        let text_s = if measuring {
            "Noch nichts gemessen. Die ersten Werte kommen nach zwei Sekunden."
        } else {
            "Keine laufende Messung. Werte gibt es, sobald du in einem Anruf bist."
        };
        let b = Block::wrapped(ui, text_s, text::BODY, c.muted, width - 30.0);
        let top = ui.cursor().min;
        let rect = Rect::from_min_size(top, vec2(width, b.height() + 30.0));
        dashed_box(ui, rect.shrink(0.5), c.border);
        b.paint(ui.painter(), pos2(top.x + 15.0, top.y + 15.0));
        ui.allocate_rect(rect, Sense::hover());
    } else {
        flows_table(ui, &groups, width);
    }
    ui.add_space(10.0);
    let limits: Vec<String> = [
        p.economy_mode.then(|| "Sprache 64 kbit/s".to_owned()),
        (p.video_upload_limit > 0.0 || p.economy_mode).then(|| {
            let budget = if p.video_upload_limit > 0.0 {
                p.video_upload_limit
            } else {
                logic::VIDEO_SEND_BUDGET as f64
            };
            format!("Video {}", diag::format_bps(Some(budget)))
        }),
    ]
    .into_iter()
    .flatten()
    .collect();
    let note = if limits.is_empty() {
        "Gelabber setzt keine Bitratengrenze.".to_owned()
    } else {
        format!(
            "Obergrenze{}: {}.",
            if limits.len() > 1 { "n" } else { "" },
            limits.join(", ")
        )
    };
    let note = format!("{note} Video höchstens {} FPS.", logic::VIDEO_MAX_FPS);
    let b = Block::wrapped(ui, &note, text::META, c.subtle, width);
    let (r, _) = ui.allocate_exact_size(vec2(width, b.height()), Sense::hover());
    b.paint(ui.painter(), r.min);
    ui.add_space(28.0);

    // Mikrofon.
    block_head(ui, "Mikrofon", None, width);
    let in_call = state.channel.is_some() && state.mic == MicStatus::Live;
    let mode = p.processing_mode.as_str();
    let format = if in_call {
        format!(
            "48 kHz, {}",
            if mode == "original" { "Stereo" } else { "Mono" }
        )
    } else {
        "–".into()
    };
    let rows: Vec<(&str, String, bool)> = vec![
        (
            "Verarbeitung",
            processing_message(&state, mode, &p.audio_input_id),
            true,
        ),
        ("Format", format, false),
        (
            "Echo-Unterdrückung",
            on_off(in_call.then_some(p.echo_cancellation)),
            false,
        ),
        (
            "Rauschfilter (WebRTC)",
            on_off(in_call.then_some(mode == "browser" && p.noise_suppression)),
            false,
        ),
        (
            "Auto-Gain (WebRTC)",
            on_off(in_call.then_some(mode != "original" && p.auto_gain_control)),
            false,
        ),
        (
            "Mikrofonverstärkung",
            format!("{} %", (p.input_gain * 100.0).round() as i64),
            false,
        ),
        (
            "Filter-Puffer",
            if in_call {
                format!("{} ms", if mode == "enhanced" { 10 } else { 0 })
            } else {
                "–".into()
            },
            false,
        ),
    ];
    facts(ui, &rows, width, text::BODY, 9.0);
    ui.add_space(28.0);

    debug(ui, &state, &data, width);
    ui.add_space(28.0);

    // The export, right.
    let label = "Diagnose herunterladen";
    let bw = Block::single(ui, label, text::UI.weight(600.0), c.text).width() + 26.0;
    let top = ui.cursor().min;
    let rect = Rect::from_min_size(pos2(top.x + width - bw, top.y), vec2(bw, 32.0));
    let response = ui
        .put(rect, Button::new(label).ghost().small())
        .on_hover_text("Speichert die Messwerte als JSON-Datei, etwa für einen Fehlerbericht.");
    if response.clicked() {
        let at = diag::now_iso();
        let processing = serde_json::json!({
            "requested": mode,
            "actual": mode,
            "message": processing_message(&state, mode, &p.audio_input_id),
            "sampleRate": in_call.then_some(48_000),
            "channels": in_call.then_some(if mode == "original" { 2 } else { 1 }),
            "inputGain": p.input_gain,
        });
        let payload = diag::export(
            &data,
            &app.call.settings(),
            &processing,
            env!("CARGO_PKG_VERSION"),
            &at,
        );
        let bytes = serde_json::to_vec_pretty(&payload).unwrap_or_default();
        crate::screens::settings::controls::save_file(
            ui.ctx(),
            "Diagnose speichern",
            &diag::export_name(&at),
            bytes,
        );
    }
    ui.allocate_exact_size(vec2(width, 32.0), Sense::hover());
}

/// `UploadTip`: loss on the way out of a weak call points to the economy
/// mode on the video tab.
fn upload_tip(ui: &mut Ui, state: &CallState, data: &Diagnostics, width: f32, economy: bool) {
    let Some(connection) = state.connection.as_ref().filter(|_| measuring(state)) else {
        return;
    };
    let lvl = level(connection);
    let send_loss = data
        .latest()
        .map(|s| {
            s.flows
                .iter()
                .filter(|f| f.send)
                .filter_map(|f| f.loss)
                .fold(0.0, f64::max)
        })
        .unwrap_or(0.0);
    if economy || lvl == 0 || lvl > 2 || send_loss < LOSS_LIMITS[0] {
        return;
    }
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    ui.add_space(16.0);
    let b = Block::wrapped(
        ui,
        "Viel Verlust beim Senden? Der Sparmodus begrenzt deinen Upload.",
        text::BODY,
        c.text,
        width - 26.0 - 140.0,
    );
    let top = ui.cursor().min;
    let rect = Rect::from_min_size(top, vec2(width, b.height().max(32.0) + 20.0));
    ui.painter().rect_filled(
        rect,
        corner(theme.radius.md),
        c.warning.gamma_multiply(0.12),
    );
    b.paint(
        ui.painter(),
        pos2(top.x + 14.0, rect.center().y - b.height() / 2.0),
    );
    let label = "Zum Sparmodus";
    let bw = Block::single(ui, label, text::UI.weight(600.0), c.text).width() + 26.0;
    let br = Rect::from_min_size(
        pos2(rect.right() - 12.0 - bw, rect.center().y - 16.0),
        vec2(bw, 32.0),
    );
    if ui.put(br, Button::new(label).small()).clicked() {
        super::dialogs::open_settings(ui.ctx(), super::dialogs::Tab::Video);
    }
    ui.allocate_rect(rect, Sense::hover());
}

/// The table of what is sent and received (`.connection-flows`).
fn flows_table(ui: &mut Ui, groups: &[FlowGroup], width: f32) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let top = ui.cursor().min;
    let painter = ui.painter().clone();
    let cols = [
        width * 0.40,
        width * 0.15,
        width * 0.17,
        width * 0.14,
        width * 0.14,
    ];
    let heads = ["Spur", "Codec", "Bitrate", "Verlust", "Jitter"];
    let head_h = 7.0 + 16.8 + 7.0;
    painter.rect_filled(
        Rect::from_min_size(top, vec2(width, head_h)),
        egui::CornerRadius {
            nw: theme.radius.md as u8,
            ne: theme.radius.md as u8,
            sw: 0,
            se: 0,
        },
        c.panel,
    );
    let mut x = top.x;
    for (i, h) in heads.iter().enumerate() {
        let b = Block::single(ui, h, text::META, c.subtle);
        let bx = if i >= 2 {
            x + cols[i] - 14.0 - b.width()
        } else {
            x + 14.0
        };
        b.paint(&painter, pos2(bx, top.y + 7.0));
        x += cols[i];
    }
    let mut y = top.y + head_h;
    for (row, g) in groups.iter().enumerate() {
        if row > 0 {
            crate::widgets::rule_h(&painter, c.border, top.x..=top.x + width, y);
            y += 1.0;
        }
        let count = if g.count < 2 {
            None
        } else if g.send && !g.audio {
            Some(format!("{} Schichten", g.count))
        } else {
            Some(format!("×{}", g.count))
        };
        let mut name = text::Rich::new(text::BODY)
            .push(if g.send { "↑ " } else { "↓ " }, text::BODY, c.accent_text)
            .push(g.name, text::BODY.weight(600.0), c.text);
        if let Some(cnt) = &count {
            name = name.push(&format!(" {cnt}"), text::META, c.subtle);
        }
        let name = name.layout(ui);
        let mut sub: Vec<String> = Vec::new();
        if !g.audio {
            if let (Some(w), Some(h)) = (g.width, g.height) {
                sub.push(format!("{w}×{h}"));
            }
            if let Some(fps) = g.fps {
                sub.push(format!("{} FPS", diag::german(fps, 1, false)));
            }
        }
        if let Some(l) = &g.limited_by {
            sub.push(format!("gebremst durch {}", limit_label(l)));
        }
        let sub =
            (!sub.is_empty()).then(|| Block::single(ui, &sub.join(" · "), text::META, c.muted));
        let row_h = 10.0 + name.height() + sub.as_ref().map_or(0.0, |s| 3.0 + s.height()) + 10.0;
        name.paint(&painter, pos2(top.x + 14.0, y + 10.0));
        if let Some(s) = &sub {
            s.paint(
                &painter,
                pos2(top.x + 14.0 + 16.0, y + 10.0 + name.height() + 3.0),
            );
        }
        let cells = [
            g.codec.clone().unwrap_or_else(|| "–".into()),
            known(diag::format_bps(g.bitrate_bps)),
            known(diag::format_ratio(g.loss)),
            known(diag::format_ms(g.jitter_ms)),
        ];
        let mut x = top.x + cols[0];
        for (i, cell) in cells.iter().enumerate() {
            let color = if i == 2 {
                tone_color(&theme, g.loss.map(loss_tone))
            } else {
                c.text
            };
            let b = Block::single(ui, cell, text::BODY, color);
            let bx = if i >= 1 {
                x + cols[i + 1] - 14.0 - b.width()
            } else {
                x + 14.0
            };
            b.paint(&painter, pos2(bx, y + 10.0));
            x += cols[i + 1];
        }
        y += row_h;
    }
    let rect = Rect::from_min_max(top, pos2(top.x + width, y));
    painter.rect(
        rect,
        corner(theme.radius.md),
        Color32::TRANSPARENT,
        Stroke::new(1.0, c.border),
        StrokeKind::Inside,
    );
    ui.allocate_rect(rect, Sense::hover());
}

/// `formatQualityLimit`.
fn limit_label(reason: &str) -> &'static str {
    match reason {
        "cpu" => "CPU",
        "bandwidth" => "Bandbreite",
        "other" => "sonstige",
        "none" => "keine",
        _ => "unbekannt",
    }
}

/// "Für die Fehlersuche" (`media-disclosure`): closed, a summary row; open,
/// the video facts, the connection, every flow, the history and events.
fn debug(ui: &mut Ui, state: &CallState, data: &Diagnostics, width: f32) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let ctx = ui.ctx().clone();
    let open: bool = ctx.data(|d| d.get_temp(disclosure_id())).unwrap_or(false);
    let top = ui.cursor().min;
    let summary_rect = Rect::from_min_size(top, vec2(width, 44.0));
    let response = ui.interact(
        summary_rect,
        disclosure_id().with("summary"),
        Sense::click(),
    );
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Für die Fehlersuche"));
    if response.hovered() {
        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if response.clicked() {
        ctx.data_mut(|d| d.insert_temp(disclosure_id(), !open));
    }
    let painter = ui.painter().clone();
    let cy = summary_rect.center().y;
    // The chevron: two 2px strokes of a 6px box, turned.
    let cx = top.x + 1.0 + 14.0 + 4.0;
    let (a, b, d) = if open {
        (
            pos2(cx - 4.0, cy - 2.0),
            pos2(cx, cy + 2.0),
            pos2(cx + 4.0, cy - 2.0),
        )
    } else {
        (
            pos2(cx - 2.0, cy - 4.0),
            pos2(cx + 2.0, cy),
            pos2(cx - 2.0, cy + 4.0),
        )
    };
    painter.line(vec![a, b, d], Stroke::new(2.0, c.text));
    let title = Block::single(ui, "Für die Fehlersuche", text::UI.weight(600.0), c.text);
    let tx = top.x + 1.0 + 14.0 + 8.0 + 12.0;
    common_paint_mid(&painter, &title, tx, cy);
    let small = Block::single(
        ui,
        "Codecs, Prioritäten, Verlauf, Ereignisse",
        text::META,
        c.muted,
    );
    common_paint_mid(&painter, &small, tx + title.width() + 12.0, cy);
    ui.allocate_rect(summary_rect, Sense::hover());
    if open {
        ui.add_space(16.0);
        let inner_w = width - 28.0;
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(
            ui.cursor().min + vec2(14.0, 0.0),
            vec2(inner_w, f32::INFINITY),
        )));
        child.spacing_mut().item_spacing.y = 0.0;
        debug_body(&mut child, state, data, inner_w);
        let h = child.min_rect().height();
        ui.allocate_exact_size(vec2(width, h + 6.0), Sense::hover());
    }
    let rect = Rect::from_min_max(top, pos2(top.x + width, ui.cursor().min.y));
    painter.rect(
        rect,
        corner(theme.radius.md),
        Color32::TRANSPARENT,
        Stroke::new(1.0, c.border),
        StrokeKind::Inside,
    );
}

fn common_paint_mid(painter: &egui::Painter, block: &Block, x: f32, cy: f32) {
    block.paint(painter, pos2(x, cy - block.height() / 2.0));
}

fn h4(ui: &mut Ui, s: &str, width: f32) {
    let theme = Theme::of(ui.ctx());
    let b = Block::single(ui, s, text::META.weight(600.0), theme.c.subtle);
    let (r, _) = ui.allocate_exact_size(vec2(width, b.height() + 6.0), Sense::hover());
    b.paint(ui.painter(), r.min);
}

fn lines(ui: &mut Ui, items: &[String], width: f32) {
    let theme = Theme::of(ui.ctx());
    for item in items {
        let b = Block::wrapped(ui, item, text::META, theme.c.text, width);
        let (r, _) = ui.allocate_exact_size(vec2(width, b.height() + 10.0), Sense::hover());
        b.paint(ui.painter(), r.min + vec2(0.0, 5.0));
        crate::widgets::rule_h(
            ui.painter(),
            theme.c.border,
            r.left()..=r.right(),
            r.bottom() - 1.0,
        );
    }
}

fn paragraph(ui: &mut Ui, s: &str, width: f32, color: Color32) {
    let b = Block::wrapped(ui, s, text::META, color, width);
    let (r, _) = ui.allocate_exact_size(vec2(width, b.height()), Sense::hover());
    b.paint(ui.painter(), r.min);
}

fn clock(iso: &str) -> String {
    // "2026-10-28T20:00:00.000Z" → "20:00:00" (UTC; the export keeps the date).
    iso.get(11..19).unwrap_or(iso).to_owned()
}

fn debug_body(ui: &mut Ui, state: &CallState, data: &Diagnostics, width: f32) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    if let Some(video) = &data.video {
        h4(ui, "Video auf diesem Gerät", width);
        let codecs = video.room_codecs.as_ref().map_or("–".to_owned(), |list| {
            list.iter()
                .map(|c| c.trim_start_matches("video/").to_owned())
                .collect::<Vec<_>>()
                .join(", ")
        });
        let rows: Vec<(&str, String, bool)> = vec![
            ("Raum-Codecs", codecs, true),
            ("AV1 in Hardware", yes_no(video.encoders.av1), false),
            ("H.264 in Hardware", yes_no(video.encoders.h264), false),
            ("VP9 in Schichten", yes_no(video.encoders.vp9_svc), false),
            ("AV1 empfangen", yes_no(video.decodes_av1), false),
            (
                "Desktop-App",
                format!("v{}", env!("CARGO_PKG_VERSION")),
                false,
            ),
        ];
        facts(ui, &rows, width, text::META, 6.0);
        ui.add_space(20.0);
    }
    h4(ui, "Verbindungen", width);
    let mut rows: Vec<(&str, String, bool)> = Vec::new();
    if let Some(sample) = data.latest() {
        let t = &sample.transport;
        let path = match t.relay {
            Some(false) => "direkt",
            Some(true) => "TURN",
            None => "–",
        };
        rows.push((
            "Sprache",
            format!(
                "{path} · RTT {} · Bandbreite senden {}, empfangen {}{}",
                known(diag::format_ms(t.rtt_ms)),
                known(diag::format_bps(t.available_out_bps)),
                known(diag::format_bps(t.available_in_bps)),
                if measuring(state) {
                    ""
                } else {
                    " · letzte Messung"
                }
            ),
            true,
        ));
    }
    rows.push(("Audio-Graph", "ohne Graph".into(), true));
    facts(ui, &rows, width, text::META, 6.0);
    ui.add_space(20.0);

    h4(ui, "Spuren einzeln", width);
    let flows = data.latest().map(|s| s.flows.clone()).unwrap_or_default();
    if flows.is_empty() {
        paragraph(ui, "Keine RTP-Statistik.", width, c.text);
    } else {
        let items: Vec<String> = flows
            .iter()
            .map(|f| {
                let name = match f.source {
                    "voice" => "Sprache",
                    "camera" => "Kamera",
                    "screen" => "Live",
                    "screen-audio" => "Live-Ton",
                    _ => "Video",
                };
                let mut line = format!(
                    "{name}{} · {}\nCodec: {} · Bitrate: {}{}\nPaketverlust: {}{} · Jitter: {} · RTT: {}",
                    if f.audio { "" } else { " (Video)" },
                    if f.send { "Senden" } else { "Empfangen" },
                    f.codec.as_deref().unwrap_or("unbekannt"),
                    diag::format_bps(f.bitrate_bps),
                    if f.send {
                        " · keine Gelabber-Bitratengrenze"
                    } else {
                        ""
                    },
                    diag::format_ratio(f.loss),
                    f.packets_lost
                        .map(|n| format!(" ({n} Pakete)"))
                        .unwrap_or_default(),
                    diag::format_ms(f.jitter_ms),
                    diag::format_ms(f.rtt_ms),
                );
                if !f.audio {
                    line.push_str(&format!(
                        "\n{} · {} · Qualitätslimit: {}",
                        match (f.width, f.height) {
                            (Some(w), Some(h)) => format!("{w}×{h}"),
                            _ => "Auflösung unbekannt".into(),
                        },
                        f.fps.map_or("FPS unbekannt".into(), |v| {
                            format!("{} FPS", diag::german(v, 1, false))
                        }),
                        f.limited_by.as_deref().map_or("keine", limit_label),
                    ));
                }
                line
            })
            .collect();
        lines(ui, &items, width);
    }
    ui.add_space(6.0);
    paragraph(
        ui,
        "Obergrenze meint die eingestellte Grenze, nicht die gemessene Bitrate. Prioritätswunsch: Audio hoch, Video normal. Browser und Netzwerk können diese Hinweise ignorieren; das garantiert keine Priorisierung auf dem SFU oder im Internet.",
        width,
        c.muted,
    );
    ui.add_space(20.0);

    h4(ui, "Verlauf", width);
    if data.phases.is_empty() {
        paragraph(ui, "noch kein Verlauf", width, c.text);
    } else {
        let items: Vec<String> = data
            .phases
            .iter()
            .map(|(at, phase)| {
                let label = match *phase {
                    "voice-only" => "nur Sprache",
                    "stream-on" => "Stream an",
                    "stream-off" => "Stream aus",
                    _ => "Watch",
                };
                format!("{}   {label}", clock(at))
            })
            .collect();
        lines(ui, &items, width);
    }
    ui.add_space(20.0);
    h4(ui, "Ereignisse", width);
    if data.events.is_empty() {
        paragraph(ui, "noch keine Ereignisse", width, c.text);
    } else {
        let items: Vec<String> = data
            .events
            .iter()
            .rev()
            .take(8)
            .rev()
            .map(|e| {
                let kind = match e.kind {
                    "stream-start" => "Streamstart",
                    "stream-stop" => "Streamstopp",
                    "device-change" => "Gerätewechsel",
                    "recovery" => "Recovery",
                    other => other,
                };
                format!("{}   {kind} · Sprache · {}", clock(&e.at), e.detail)
            })
            .collect();
        lines(ui, &items, width);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_words() {
        let good = Connection {
            rtt_ms: Some(38.0),
            loss: Some(0.0),
            relay: Some(false),
            ..Connection::default()
        };
        assert_eq!(level(&good), 4);
        assert_eq!(
            summary(&good).as_deref(),
            Some("Verbindung: gut · 38 ms · 0 % Verlust")
        );
        assert_eq!(
            detail(&good).as_deref(),
            Some("Direkt verbunden, kaum Verlust.")
        );
        let weak = Connection {
            rtt_ms: Some(14.0),
            loss: Some(0.06),
            relay: Some(false),
            ..Connection::default()
        };
        assert_eq!(quality_label(level(&weak)), "schwach");
        assert_eq!(
            detail(&weak).as_deref(),
            Some("Direkt verbunden, hoher Verlust (6 %).")
        );
        assert_eq!(summary(&Connection::default()), None);
        assert_eq!(ping_tone(320.0), Tone::Weak);
        assert_eq!(loss_tone(0.12), Tone::Bad);
    }

    #[test]
    fn processing_follows_the_call() {
        let mut state = CallState::default();
        assert_eq!(
            processing_message(&state, "enhanced", ""),
            "Mikrofon inaktiv"
        );
        state.channel = Some(uuid::Uuid::nil());
        state.mic_input = Some("usb".into());
        assert_eq!(
            processing_message(&state, "enhanced", "usb"),
            "Desktop-App: RNNoise im nativen Kern"
        );
        // The chosen microphone did not open: the call runs on the system's.
        state.mic_input = Some(String::new());
        assert_eq!(
            processing_message(&state, "browser", "usb"),
            "Desktop-App: WebRTC-Rauschfilter im nativen Kern; Standardmikrofon"
        );
        assert_eq!(
            processing_message(&state, "enhanced", ""),
            "Desktop-App: RNNoise im nativen Kern"
        );
        state.mic = MicStatus::Unavailable;
        assert_eq!(
            processing_message(&state, "enhanced", "usb"),
            "Mikrofon konnte nicht aktiviert werden; du hörst weiter zu"
        );
    }
}

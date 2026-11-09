//! A message's text with its markdown (messages/MessageContent.tsx,
//! chat.css `.lr-message-text`, `.lr-md-*`): paragraphs, quotes and code
//! blocks laid out as CSS boxes, inline chips (code, mentions, spoilers)
//! painted under the glyphs, links and spoilers clickable.

use std::collections::{HashMap, HashSet};

use egui::text::LayoutJob;
use egui::{Color32, CornerRadius, FontFamily, FontId, Pos2, Rect, Sense, Stroke, Ui, pos2};
use uuid::Uuid;

use super::markdown::{Block as MdBlock, Inline, parse_message};
use crate::icons::{self, Icon};
use crate::text::{Block, TextStyle};
use crate::theme::{Theme, corner, mix};

/// `.lr-message-text`: 14.5px / 1.5.
pub const BODY: TextStyle = TextStyle::new(14.5, 400.0, 1.5);
/// `0.88em` of the body.
const MONO_SIZE: f32 = 12.76;
/// `pre`: 14.5px strut × 1.45, in Blink's 1/64 px layout units.
const PRE_LINE: f32 = 1345.0 / 64.0;
/// A search hit's text (`.lr-search-results`): 14px / 1.5.
const HIT_BODY: TextStyle = TextStyle::new(14.0, 400.0, 1.5);
/// A hit's code line: the 21px strut plus the mono run hanging below it,
/// as Blink measures it (1/64 px).
const HIT_PRE_LINE: f32 = 1374.0 / 64.0;
/// `span.lr-md-codeblock`: `max-height: 8em` of 14px, overflow hidden.
const HIT_CODE_MAX: f32 = 112.0;

/// Who the mentions of one message name and whether they reached me.
pub struct Mentions<'a> {
    /// Lower-cased user id → name.
    pub names: &'a HashMap<String, String>,
    pub me: Option<Uuid>,
    pub everyone: bool,
    pub mentions: &'a [Uuid],
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Span {
    Plain,
    /// A searched word in a hit (`<mark class="lr-search-mark">`).
    Match,
    Code,
    Mention {
        me: bool,
    },
    Link(usize),
    Edited,
}

#[derive(Clone, Copy, Debug)]
struct SectionInfo {
    span: Span,
    /// The spoiler this run is inside, with whether it is revealed.
    spoiler: Option<(usize, bool)>,
    chars: (usize, usize),
}

/// One laid-out paragraph or quote.
pub struct TextPart {
    pub block: Block,
    sections: Vec<SectionInfo>,
    links: Vec<String>,
    /// Left inset (quote bar and padding).
    inset: f32,
}

pub struct CodePart {
    lang: Option<Block>,
    lines: Block,
    pub text: String,
    /// Index among the message's code blocks (copy feedback).
    pub index: usize,
}

pub enum PartKind {
    Text(TextPart),
    Code(CodePart),
}

pub struct Part {
    /// Top of the border box, relative to the content top.
    pub y: f32,
    pub height: f32,
    pub kind: PartKind,
}

/// A laid-out `.lr-message-text`.
pub struct Laid {
    pub parts: Vec<Part>,
    /// Height of the content box (margins not included).
    pub height: f32,
    /// The collapsed margins at the top and bottom edge (CSS margin collapsing
    /// through the div: its own 1px top, the first/last block's margins).
    pub margin_top: f32,
    pub margin_bottom: f32,
    /// The text column width it was laid out for.
    pub width: f32,
    body: TextStyle,
    /// A search hit: code blocks without their bar.
    hit: bool,
}

/// What a click inside the content asks for.
pub enum ContentAction {
    OpenLink(String),
    RevealSpoiler(usize),
    CopyCode(usize, String),
}

/// What [`Laid::paint`] gave back.
pub struct Painted {
    /// What a click on a link, spoiler or copy button asked for.
    pub action: Option<ContentAction>,
    /// All of its selectable text: a click or right click on the text
    /// lands here, not on what lies under it.
    pub text: Option<egui::Response>,
}

struct Builder<'a> {
    job: LayoutJob,
    sections: Vec<SectionInfo>,
    links: Vec<String>,
    chars: usize,
    pending_space: f32,
    spoilers: usize,
    revealed: &'a HashSet<usize>,
    mentions: &'a Mentions<'a>,
    hereish: bool,
    color: Color32,
    theme: &'a Theme,
    body: TextStyle,
    /// Words to mark, lower-cased (`search::search_terms`).
    terms: &'a [String],
}

#[derive(Clone, Copy, Default)]
struct Marks {
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    spoiler: Option<(usize, bool)>,
}

impl Builder<'_> {
    fn style(&self, marks: Marks) -> TextStyle {
        let mut style = self.body;
        if marks.bold {
            style = style.weight(700.0);
        }
        if marks.italic {
            style = style.italic();
        }
        style
    }

    fn push(&mut self, text: &str, mut format: egui::TextFormat, span: Span, marks: Marks) {
        if text.is_empty() {
            return;
        }
        format.line_height = Some(self.body.line_px());
        if marks.underline {
            format.underline = Stroke::new(1.0, format.color);
        }
        if marks.strike {
            format.strikethrough = Stroke::new(1.0, format.color);
        }
        if marks.spoiler.is_some_and(|(_, shown)| !shown) {
            format.color = Color32::TRANSPARENT;
            format.underline = Stroke::NONE;
            format.strikethrough = Stroke::NONE;
        }
        let pad = match span {
            Span::Code => 5.0,
            Span::Mention { .. } => 2.0,
            Span::Edited => 4.0,
            _ => 0.0,
        };
        let leading = std::mem::take(&mut self.pending_space) + pad;
        self.pending_space = if span == Span::Edited { 0.0 } else { pad };
        let len = text.chars().count();
        self.sections.push(SectionInfo {
            span,
            spoiler: marks.spoiler,
            chars: (self.chars, self.chars + len),
        });
        self.chars += len;
        self.job.append(text, leading, format);
    }

    fn inline(&mut self, nodes: &[Inline], marks: Marks) {
        let c = self.theme.c;
        for node in nodes {
            match node {
                Inline::Text(t) => {
                    let format = self.style(marks).format(self.color);
                    if self.terms.is_empty() || marks.spoiler.is_some() {
                        self.push(t, format, Span::Plain, marks);
                    } else {
                        for (run, matched) in super::search::mark_matches(t, self.terms) {
                            let span = if matched { Span::Match } else { Span::Plain };
                            self.push(run, format.clone(), span, marks);
                        }
                    }
                }
                Inline::Strong(children) => self.inline(
                    children,
                    Marks {
                        bold: true,
                        ..marks
                    },
                ),
                Inline::Em(children) => self.inline(
                    children,
                    Marks {
                        italic: true,
                        ..marks
                    },
                ),
                Inline::Underline(children) => self.inline(
                    children,
                    Marks {
                        underline: true,
                        ..marks
                    },
                ),
                Inline::Strike(children) => self.inline(
                    children,
                    Marks {
                        strike: true,
                        ..marks
                    },
                ),
                Inline::Spoiler(children) => {
                    let id = self.spoilers;
                    self.spoilers += 1;
                    let shown = self.revealed.contains(&id);
                    self.inline(
                        children,
                        Marks {
                            spoiler: Some((id, shown)),
                            ..marks
                        },
                    );
                }
                Inline::Code(code) => {
                    let mut format = self.style(marks).format(self.color);
                    format.font_id = FontId::new(mono_size(self.body), FontFamily::Monospace);
                    format.coords = Default::default();
                    self.push(code, format, Span::Code, marks);
                }
                Inline::Link { href, text, .. } => {
                    let index = self.links.len();
                    self.links.push(href.clone());
                    let format = self.style(marks).format(c.accent_text);
                    self.push(text, format, Span::Link(index), marks);
                }
                Inline::Mention(id) => {
                    let m = self.mentions;
                    let mentions_me = m.me.is_some_and(|me| m.mentions.contains(&me));
                    let own =
                        m.me.is_some_and(|me| me.to_string() == *id) && (m.everyone || mentions_me);
                    let name = m.names.get(id).map(String::as_str).unwrap_or("Unbekannt");
                    let color = if own { c.text } else { c.accent_text };
                    let format = self.style(marks).weight(600.0).format(color);
                    self.push(
                        &format!("@{name}"),
                        format,
                        Span::Mention { me: own },
                        marks,
                    );
                }
                Inline::Everyone | Inline::Here => {
                    let m = self.mentions;
                    let word = if matches!(node, Inline::Everyone) {
                        "@everyone"
                    } else {
                        "@here"
                    };
                    let chip = if matches!(node, Inline::Everyone) {
                        m.everyone
                    } else {
                        self.hereish
                    };
                    if chip {
                        let own = m.everyone || m.me.is_some_and(|me| m.mentions.contains(&me));
                        let color = if own { c.text } else { c.accent_text };
                        let format = self.style(marks).weight(600.0).format(color);
                        self.push(word, format, Span::Mention { me: own }, marks);
                    } else {
                        let format = self.style(marks).format(self.color);
                        self.push(word, format, Span::Plain, marks);
                    }
                }
            }
        }
    }
}

fn named_tokens(blocks: &[MdBlock]) -> HashSet<String> {
    fn walk(nodes: &[Inline], out: &mut HashSet<String>) {
        for node in nodes {
            match node {
                Inline::Mention(id) => {
                    out.insert(id.clone());
                }
                Inline::Strong(c)
                | Inline::Em(c)
                | Inline::Underline(c)
                | Inline::Strike(c)
                | Inline::Spoiler(c) => walk(c, out),
                _ => {}
            }
        }
    }
    let mut out = HashSet::new();
    for block in blocks {
        if let MdBlock::Paragraph(n) | MdBlock::Quote(n) = block {
            walk(n, &mut out);
        }
    }
    out
}

/// The body text colour: mix(text 90%, muted).
pub fn body_color(theme: &Theme) -> Color32 {
    mix(theme.c.muted, theme.c.text, 0.9)
}

/// `0.88em` of the text.
fn mono_size(body: TextStyle) -> f32 {
    if body.size == BODY.size {
        MONO_SIZE
    } else {
        body.size * 0.88
    }
}

/// Lays out `content` for a text column `width` wide. `edited` appends the
/// "(bearbeitet)" marker after the last glyph.
pub fn layout(
    ui: &Ui,
    content: &str,
    width: f32,
    mentions: &Mentions,
    revealed: &HashSet<usize>,
    edited: bool,
) -> Laid {
    layout_as(ui, content, width, mentions, revealed, edited, None)
}

/// A search hit's content (`MessageContent interactive={false}` in
/// `.lr-search-hit`): 14px in the text colour, code blocks without their bar.
/// `terms` are the searched words, marked in the text (`highlight`).
pub fn layout_hit(
    ui: &Ui,
    content: &str,
    width: f32,
    mentions: &Mentions,
    terms: &[String],
) -> Laid {
    layout_as(
        ui,
        content,
        width,
        mentions,
        &HashSet::new(),
        false,
        Some(terms),
    )
}

fn layout_as(
    ui: &Ui,
    content: &str,
    width: f32,
    mentions: &Mentions,
    revealed: &HashSet<usize>,
    edited: bool,
    // A search hit, with the words to mark.
    hit: Option<&[String]>,
) -> Laid {
    let theme = Theme::of(ui.ctx());
    let terms = hit.unwrap_or_default();
    let hit = hit.is_some();
    let body = if hit { HIT_BODY } else { BODY };
    let blocks = parse_message(content);
    let named = named_tokens(&blocks);
    let hereish = mentions.everyone
        || mentions
            .mentions
            .iter()
            .any(|id| !named.contains(&id.to_string()));
    let color = if hit {
        theme.c.text
    } else {
        body_color(&theme)
    };
    let mut parts: Vec<Part> = Vec::new();
    let mut spoilers = 0;
    let mut codes = 0;
    // (height, margin top, margin bottom) per block, stacked with collapsing.
    let mut boxes: Vec<(f32, f32, f32, PartKind)> = Vec::new();
    let count = blocks.len();
    for (n, block) in blocks.iter().enumerate() {
        let last = n + 1 == count;
        match block {
            MdBlock::Paragraph(nodes) | MdBlock::Quote(nodes) => {
                let quote = matches!(block, MdBlock::Quote(_));
                let inset = if quote { 13.0 } else { 0.0 };
                let mut b = Builder {
                    job: LayoutJob::default(),
                    sections: Vec::new(),
                    links: Vec::new(),
                    chars: 0,
                    pending_space: 0.0,
                    spoilers,
                    revealed,
                    mentions,
                    hereish,
                    color,
                    theme: &theme,
                    body,
                    terms,
                };
                b.inline(nodes, Marks::default());
                if edited && last {
                    push_edited(&mut b, &theme);
                }
                spoilers = b.spoilers;
                b.job.wrap.max_width = (width - inset).max(1.0);
                b.job.wrap.break_anywhere = false;
                let block = hide_spoilers(Block::new(ui, b.job, body), &b.sections);
                let height = block.height();
                let margin = if quote { 2.0 } else { 0.0 };
                boxes.push((
                    height,
                    margin,
                    margin,
                    PartKind::Text(TextPart {
                        block,
                        sections: b.sections,
                        links: b.links,
                        inset,
                    }),
                ));
            }
            MdBlock::CodeBlock { lang, text: code } => {
                let lang_block = lang
                    .as_deref()
                    .map(|l| Block::single(ui, l, TextStyle::new(11.0, 400.0, 1.5), theme.c.muted));
                let mut job = LayoutJob::default();
                let line = if hit { HIT_PRE_LINE } else { PRE_LINE };
                let mut format = egui::TextFormat {
                    font_id: FontId::new(mono_size(body), FontFamily::Monospace),
                    line_height: Some(line),
                    color,
                    valign: egui::Align::Center,
                    ..Default::default()
                };
                format.extra_letter_spacing = 0.0;
                job.append(code, 0.0, format);
                job.wrap.max_width = f32::INFINITY;
                let lines = Block::new(ui, job, body.line_height(line / body.size));
                let height = if hit {
                    // Border, padding 6 … 6, at most 8em of lines.
                    2.0 + 12.0 + (lines.rows() as f32 * line).min(HIT_CODE_MAX)
                } else {
                    // Border, bar (2 + 20.5 button), pre padding 2 … 8.
                    2.0 + 22.5 + 2.0 + lines.rows() as f32 * PRE_LINE + 8.0
                };
                boxes.push((
                    height,
                    4.0,
                    4.0,
                    PartKind::Code(CodePart {
                        lang: lang_block,
                        lines,
                        text: code.clone(),
                        index: codes,
                    }),
                ));
                codes += 1;
            }
        }
    }
    if edited
        && !matches!(
            blocks.last(),
            Some(MdBlock::Paragraph(_) | MdBlock::Quote(_))
        )
    {
        // After a code block (or alone) the marker gets a line of its own.
        let mut b = Builder {
            job: LayoutJob::default(),
            sections: Vec::new(),
            links: Vec::new(),
            chars: 0,
            pending_space: 0.0,
            spoilers,
            revealed,
            mentions,
            hereish,
            color,
            theme: &theme,
            body,
            terms,
        };
        push_edited(&mut b, &theme);
        b.job.wrap.max_width = width.max(1.0);
        let block = hide_spoilers(Block::new(ui, b.job, body), &b.sections);
        let height = block.height();
        boxes.push((
            height,
            0.0,
            0.0,
            PartKind::Text(TextPart {
                block,
                sections: b.sections,
                links: b.links,
                inset: 0.0,
            }),
        ));
    }
    // `.lr-message-text` has a 1px top margin, a hit's text paragraph 2px.
    let floor = if hit { 2.0 } else { 1.0 };
    let margin_top = boxes.first().map_or(floor, |b| b.1.max(floor));
    let margin_bottom = boxes.last().map_or(0.0, |b| b.2);
    let mut y = 0.0;
    let mut previous_bottom: Option<f32> = None;
    for (height, mt, mb, kind) in boxes {
        if let Some(prev) = previous_bottom {
            y += prev.max(mt);
        }
        parts.push(Part { y, height, kind });
        y += height;
        previous_bottom = Some(mb);
    }
    Laid {
        parts,
        height: y,
        margin_top,
        margin_bottom,
        width,
        body,
        hit,
    }
}

fn push_edited(b: &mut Builder, theme: &Theme) {
    let format = TextStyle::new(11.0, 400.0, 1.5).format(theme.c.muted);
    b.push("(bearbeitet)", format, Span::Edited, Marks::default());
}

/// Per-row x extents of the chars `range` in `block`'s galley, relative to
/// the galley, with the row index.
fn extents(block: &Block, range: (usize, usize)) -> Vec<(usize, f32, f32)> {
    let mut out = Vec::new();
    let mut index = 0;
    for (r, row) in block.galley.rows.iter().enumerate() {
        let mut lo = f32::INFINITY;
        let mut hi = f32::NEG_INFINITY;
        for glyph in &row.glyphs {
            if index >= range.0 && index < range.1 {
                lo = lo.min(row.pos.x + glyph.pos.x);
                hi = hi.max(row.pos.x + glyph.pos.x + glyph.advance_width);
            }
            index += 1;
        }
        if row.ends_with_newline {
            index += 1;
        }
        if lo.is_finite() {
            out.push((r, lo, hi));
        }
    }
    out
}

/// Hidden spoilers stay out of what a selection copies and announces,
/// as the web's `aria-hidden` spoiler text does.
fn hide_spoilers(mut block: Block, sections: &[SectionInfo]) -> Block {
    for section in sections {
        if section.spoiler.is_some_and(|(_, shown)| !shown) {
            block.hide(section.chars);
        }
    }
    block
}

impl Laid {
    /// Paints at `origin` (content box top-left), its text selectable;
    /// returns what a click asked for.
    pub fn paint(
        &self,
        ui: &mut Ui,
        origin: Pos2,
        id: egui::Id,
        interactive: bool,
        copied: Option<(usize, bool)>,
    ) -> Painted {
        let theme = Theme::of(ui.ctx());
        let c = theme.c;
        let mut action = None;
        let mut text: Option<egui::Response> = None;
        let ppp = ui.ctx().pixels_per_point();
        for (p, part) in self.parts.iter().enumerate() {
            let top = origin.y + part.y;
            match &part.kind {
                PartKind::Text(t) => {
                    let left = origin.x + t.inset;
                    // Under the spoilers and links sensed below.
                    let selection = t.block.select(ui, id.with(("text", p)), pos2(left, top));
                    crate::text::join(&mut text, selection.clone());
                    if t.inset > 0.0 {
                        ui.painter().rect_filled(
                            crate::widgets::snap(
                                ui.ctx(),
                                Rect::from_min_max(
                                    pos2(origin.x, top),
                                    pos2(origin.x + 3.0, top + part.height),
                                ),
                            ),
                            0.0,
                            Color32::from_rgba_unmultiplied(c.text.r(), c.text.g(), c.text.b(), 64),
                        );
                    }
                    let body = self.body;
                    let line = body.line_px();
                    let baseline =
                        |r: usize| ((top + r as f32 * line + body.baseline()) * ppp).round() / ppp;
                    // Chips under the glyphs.
                    for (s, section) in t.sections.iter().enumerate() {
                        let (fill, stroke, radius, up, down, pad) = match section.span {
                            Span::Code => {
                                (c.panel, Stroke::new(1.0, c.border), 5.0, 14.0, 5.0, 5.0)
                            }
                            Span::Mention { me } => {
                                let base = if me { c.warning } else { c.accent };
                                let alpha = if me { 66 } else { 46 };
                                (
                                    Color32::from_rgba_unmultiplied(
                                        base.r(),
                                        base.g(),
                                        base.b(),
                                        alpha,
                                    ),
                                    Stroke::NONE,
                                    4.0,
                                    14.0,
                                    3.5,
                                    2.0,
                                )
                            }
                            // `.lr-search-mark`: warning at 30 %.
                            Span::Match => (
                                Color32::from_rgba_unmultiplied(
                                    c.warning.r(),
                                    c.warning.g(),
                                    c.warning.b(),
                                    77,
                                ),
                                Stroke::NONE,
                                3.0,
                                13.5,
                                3.5,
                                0.0,
                            ),
                            _ => continue,
                        };
                        let _ = s;
                        for (r, lo, hi) in extents(&t.block, section.chars) {
                            let b = baseline(r);
                            let rect = Rect::from_min_max(
                                pos2(left + lo - pad, b - up),
                                pos2(left + hi + pad, b + down),
                            );
                            let rect = crate::widgets::snap(ui.ctx(), rect);
                            ui.painter().rect(
                                rect,
                                corner(radius),
                                fill,
                                stroke,
                                egui::StrokeKind::Inside,
                            );
                        }
                    }
                    // Spoilers: one box per spoiler and row.
                    // In spoiler order, as screen readers meet them.
                    let mut spoiler_rects: std::collections::BTreeMap<usize, (bool, Vec<Rect>)> =
                        std::collections::BTreeMap::new();
                    for section in &t.sections {
                        if let Some((sid, shown)) = section.spoiler {
                            for (r, lo, hi) in extents(&t.block, section.chars) {
                                let b = baseline(r);
                                let rect = Rect::from_min_max(
                                    pos2(left + lo, b - 14.0),
                                    pos2(left + hi, b + 3.5),
                                );
                                let entry = spoiler_rects.entry(sid).or_insert((shown, Vec::new()));
                                match entry
                                    .1
                                    .iter_mut()
                                    .find(|e| (e.top() - rect.top()).abs() < 0.5)
                                {
                                    Some(e) => *e = e.union(rect),
                                    None => entry.1.push(rect),
                                }
                            }
                        }
                    }
                    // Shown spoilers tint under the glyphs; hidden ones cover
                    // them, so a selection cannot reveal what they hide.
                    let mut covers = Vec::new();
                    let mut spoiler_hot = false;
                    for (sid, (shown, rects)) in &spoiler_rects {
                        for (n, rect) in rects.iter().enumerate() {
                            if *shown {
                                let fill = Color32::from_rgba_unmultiplied(
                                    c.text.r(),
                                    c.text.g(),
                                    c.text.b(),
                                    20,
                                );
                                ui.painter().rect_filled(*rect, CornerRadius::same(5), fill);
                                continue;
                            }
                            covers.push(*rect);
                            if interactive {
                                let response = ui.interact(
                                    *rect,
                                    id.with(("spoiler", p, *sid, rect.top() as i32)),
                                    Sense::click(),
                                );
                                // `role="button" aria-label="Spoiler anzeigen"`,
                                // once per spoiler.
                                if n == 0 {
                                    response.widget_info(|| {
                                        egui::WidgetInfo::labeled(
                                            egui::Role::Button,
                                            true,
                                            "Spoiler anzeigen",
                                        )
                                    });
                                }
                                spoiler_hot |= response.hovered();
                                if response.clicked() {
                                    action = Some(ContentAction::RevealSpoiler(*sid));
                                }
                            }
                        }
                    }
                    t.block.paint_selectable(ui, &selection, pos2(left, top));
                    for rect in covers {
                        ui.painter().rect_filled(
                            rect,
                            CornerRadius::same(5),
                            mix(c.bg, c.text, 0.22),
                        );
                    }
                    if spoiler_hot {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    // Links: underline in alpha accent, currentColor on hover.
                    for section in &t.sections {
                        let Span::Link(index) = section.span else {
                            continue;
                        };
                        if section.spoiler.is_some_and(|(_, shown)| !shown) {
                            continue;
                        }
                        let rows = extents(&t.block, section.chars);
                        let rects: Vec<Rect> = rows
                            .iter()
                            .map(|(r, lo, hi)| {
                                Rect::from_min_max(
                                    pos2(left + lo, top + *r as f32 * line),
                                    pos2(left + hi, top + (*r + 1) as f32 * line),
                                )
                            })
                            .collect();
                        let mut hovered = false;
                        let mut clicked = false;
                        if interactive {
                            for (n, rect) in rects.iter().enumerate() {
                                let response = ui.interact(
                                    *rect,
                                    id.with(("link", p, index, n)),
                                    Sense::click(),
                                );
                                hovered |= response.hovered();
                                clicked |= response.clicked();
                            }
                        }
                        if hovered {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        let underline = if hovered {
                            c.accent_text
                        } else {
                            Color32::from_rgba_unmultiplied(
                                c.accent.r(),
                                c.accent.g(),
                                c.accent.b(),
                                115,
                            )
                        };
                        for (r, lo, hi) in rows {
                            crate::widgets::link::underline(
                                ui.painter(),
                                body,
                                top + r as f32 * line,
                                left + lo,
                                left + hi,
                                underline,
                            );
                        }
                        if clicked {
                            action = Some(ContentAction::OpenLink(t.links[index].clone()));
                        }
                    }
                }
                PartKind::Code(code) => {
                    let rect = Rect::from_min_max(
                        pos2(origin.x, top),
                        pos2(origin.x + self.width, top + part.height),
                    );
                    let rect = crate::widgets::snap(ui.ctx(), rect);
                    ui.painter().rect(
                        rect,
                        CornerRadius::same(5),
                        c.panel,
                        Stroke::new(1.0, c.border),
                        egui::StrokeKind::Inside,
                    );
                    if self.hit {
                        // `span.lr-md-codeblock`: padding 6 8, clipped at 8em.
                        let lines = crate::text::selectable_in(
                            ui,
                            &code.lines,
                            id.with(("code", code.index)),
                            pos2(rect.left() + 9.0, rect.top() + 1.0 + 6.0),
                            rect.shrink(1.0),
                        );
                        crate::text::join(&mut text, lines);
                        continue;
                    }
                    if let Some(lang) = &code.lang {
                        lang.paint(
                            ui.painter(),
                            pos2(rect.left() + 11.0, rect.top() + 1.0 + 2.0 + 2.0),
                        );
                    }
                    // The copy button: padding 2 6, gap 4, icon 13, 11px label.
                    let state = copied.filter(|(i, _)| *i == code.index).map(|(_, ok)| ok);
                    let (icon, label, icon_color) = match state {
                        Some(true) => (Some(Icon::Check), "Kopiert", Some(c.online_text)),
                        Some(false) => (None, "Kopieren nicht möglich", None),
                        None => (Some(Icon::Copy), "Kopieren", None),
                    };
                    let style = TextStyle::new(11.0, 400.0, 1.5);
                    let label_block = Block::single(ui, label, style, c.muted);
                    let inner = label_block.width() + if icon.is_some() { 13.0 + 4.0 } else { 0.0 };
                    let button = Rect::from_min_size(
                        pos2(
                            rect.right() - 1.0 - 6.0 - inner - 12.0,
                            rect.top() + 1.0 + 2.0,
                        ),
                        egui::vec2(inner + 12.0, 20.5),
                    );
                    // The code's text first, so the button lies on top of it.
                    let pre_top = rect.top() + 1.0 + 22.5 + 2.0;
                    let lines_at = pos2(rect.left() + 11.0, pre_top - 3.0);
                    let lines_ui = crate::text::clipped(ui, rect.shrink(1.0));
                    let lines =
                        code.lines
                            .select(&lines_ui, id.with(("code", code.index)), lines_at);
                    let mut hovered = false;
                    if interactive {
                        let response =
                            ui.interact(button, id.with(("copy", code.index)), Sense::click());
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::Role::Button, true, label)
                        });
                        hovered = response.hovered();
                        if hovered {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if response.clicked() {
                            action = Some(ContentAction::CopyCode(code.index, code.text.clone()));
                        }
                    }
                    let fg = if hovered { c.text } else { c.muted };
                    if hovered {
                        ui.painter()
                            .rect_filled(button, CornerRadius::same(5), c.hover);
                    }
                    let mut x = button.left() + 6.0;
                    if let Some(icon) = icon {
                        icons::paint(
                            ui.painter(),
                            icon,
                            pos2(x, button.center().y - 6.5),
                            13.0,
                            icon_color.unwrap_or(fg),
                        );
                        x += 13.0 + 4.0;
                    }
                    label_block.paint_tinted(ui.painter(), pos2(x, button.top() + 2.0), Some(fg));
                    code.lines.paint_selectable(&lines_ui, &lines, lines_at);
                    crate::text::join(&mut text, lines);
                }
            }
        }
        Painted { action, text }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use egui::{Event, Modifiers, OutputCommand, PointerButton, Vec2};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable as _;

    use super::*;

    /// The revealed spoilers and what the clicks asked for.
    #[derive(Default)]
    struct State {
        revealed: HashSet<usize>,
        asked: Vec<String>,
    }

    /// `content` as a message row paints it, its actions noted.
    fn harness(content: String) -> Harness<'static, State> {
        Harness::builder()
            .with_size(egui::vec2(700.0, 300.0))
            .build_ui_state(
                move |ui, state: &mut State| {
                    crate::fonts::install(ui.ctx());
                    crate::theme::ensure(ui.ctx());
                    let names = HashMap::new();
                    let mentions = Mentions {
                        names: &names,
                        me: None,
                        everyone: false,
                        mentions: &[],
                    };
                    let laid = layout(ui, &content, 600.0, &mentions, &state.revealed, false);
                    let id = egui::Id::unique("message");
                    match laid.paint(ui, pos2(10.0, 10.0), id, true, None).action {
                        Some(ContentAction::OpenLink(href)) => state.asked.push(href),
                        Some(ContentAction::RevealSpoiler(n)) => {
                            state.revealed.insert(n);
                        }
                        Some(ContentAction::CopyCode(_, text)) => state.asked.push(text),
                        None => {}
                    }
                },
                State::default(),
            )
    }

    /// What the last pass handed to the clipboards: (Ctrl+C, selection).
    fn copied(harness: &Harness<'_, State>) -> (Vec<String>, Vec<String>) {
        let mut copy = Vec::new();
        let mut settled = Vec::new();
        for command in &harness.output().platform_output.commands {
            match command {
                OutputCommand::CopyText(text) => copy.push(text.clone()),
                OutputCommand::TextSelectionSettled(text) => settled.push(text.clone()),
                _ => {}
            }
        }
        (copy, settled)
    }

    fn press(harness: &mut Harness<'_, State>, pos: Pos2, pressed: bool) {
        harness.event(Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }

    /// Drags from `from` to `to` and copies: what Ctrl+C and the
    /// selection buffer got.
    fn drag_and_copy(harness: &mut Harness<'_, State>, from: Pos2, to: Pos2) -> [Vec<String>; 2] {
        harness.hover_at(from);
        harness.drag_at(from);
        harness.hover_at(from.lerp(to, 0.5));
        harness.hover_at(to);
        press(harness, to, false);
        harness.step();
        let (_, mut settled) = copied(harness);
        harness.step();
        settled.extend(copied(harness).1);
        harness.event(Event::Copy);
        harness.step();
        [copied(harness).0, settled]
    }

    fn mentions_secret(harness: &Harness<'_, State>) -> bool {
        harness
            .query_all_by(|node| {
                [node.label(), node.value()]
                    .into_iter()
                    .flatten()
                    .any(|text| text.contains("geheim"))
            })
            .next()
            .is_some()
    }

    /// A hidden spoiler stays out of what a selection copies and of the
    /// accessibility tree (`aria-hidden` on the web); revealed, it is in.
    #[test]
    fn hidden_spoilers_stay_out_of_a_selection() {
        let mut harness = harness("Vorher ||geheim|| nachher.\n\nZweite Zeile.".into());
        harness.run_steps(2);
        let text = harness
            .get_by_label("Vorher  nachher.\n\nZweite Zeile.")
            .rect();
        harness.get_by_label("Spoiler anzeigen");
        assert!(!mentions_secret(&harness));
        let (from, to) = (text.left_top() + Vec2::new(1.0, 4.0), text.right_bottom());
        let [copy, settled] = drag_and_copy(&mut harness, from, to);
        let expected = vec!["Vorher  nachher.\n\nZweite Zeile.".to_owned()];
        assert_eq!(copy, expected);
        assert_eq!(settled, expected);

        harness.get_by_label("Spoiler anzeigen").click();
        harness.run_steps(2);
        assert!(harness.query_by_label("Spoiler anzeigen").is_none());
        assert!(mentions_secret(&harness));
        let text = harness
            .get_by_label("Vorher geheim nachher.\n\nZweite Zeile.")
            .rect();
        let (from, to) = (text.left_top() + Vec2::new(1.0, 4.0), text.right_bottom());
        let [copy, _] = drag_and_copy(&mut harness, from, to);
        assert_eq!(
            copy,
            vec!["Vorher geheim nachher.\n\nZweite Zeile.".to_owned()]
        );
    }

    /// A plain click on text selects nothing to copy: no whole paragraph
    /// for Ctrl+C or the middle-click buffer, as in the browser.
    #[test]
    fn a_click_on_text_copies_nothing() {
        let mut harness = harness("Film oder Spiel heute?".into());
        harness.run_steps(2);
        harness.get_by_label("Film oder Spiel heute?").click();
        for _ in 0..3 {
            harness.step();
            assert_eq!(copied(&harness), (vec![], vec![]));
        }
        harness.event(Event::Copy);
        harness.step();
        assert_eq!(copied(&harness), (vec![], vec![]));
    }

    /// A link in selectable text still opens on a click.
    #[test]
    fn a_link_in_the_text_still_opens() {
        let mut harness = harness("https://example.org/plan".into());
        harness.run_steps(2);
        harness.get_by_label("https://example.org/plan").click();
        harness.run_steps(2);
        assert_eq!(harness.state().asked, vec!["https://example.org/plan"]);
    }

    /// A code block's copy button lies over its selectable text, also on
    /// its bottom edge where the text box starts.
    #[test]
    fn the_code_copy_button_lies_over_the_code() {
        let line = "x".repeat(120);
        let mut harness = harness(format!("```\n{line}\n```"));
        harness.run_steps(2);
        let button = harness.get_by_label("Kopieren").rect();
        assert!(harness.get_by_label(&line).rect().right() > button.right());
        let at = pos2(button.center().x, button.bottom() - 0.5);
        harness.hover_at(at);
        press(&mut harness, at, true);
        press(&mut harness, at, false);
        harness.run_steps(2);
        assert_eq!(harness.state().asked, vec![line]);
    }

    /// `MessageContent highlight`: the searched word is marked in plain
    /// text, bold included, but not in a spoiler, code, a link or a longer
    /// word.
    #[test]
    fn a_hit_marks_only_its_plain_text() {
        let marked = std::cell::RefCell::new(Vec::new());
        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            crate::fonts::install(ui.ctx());
            let names = HashMap::new();
            let mentions = Mentions {
                names: &names,
                me: None,
                everyone: false,
                mentions: &[],
            };
            let laid = layout_hit(
                ui,
                "**Snacks** ||snacks|| `snacks` https://snacks.example snacksbar",
                600.0,
                &mentions,
                &["snacks".to_owned()],
            );
            let mut out = Vec::new();
            for part in &laid.parts {
                if let PartKind::Text(text) = &part.kind {
                    let chars: Vec<char> = text.block.galley.text().chars().collect();
                    for s in text.sections.iter().filter(|s| s.span == Span::Match) {
                        out.push(chars[s.chars.0..s.chars.1].iter().collect::<String>());
                    }
                }
            }
            *marked.borrow_mut() = out;
        });
        harness.run();
        drop(harness);
        assert_eq!(marked.into_inner(), ["Snacks"]);
    }
}

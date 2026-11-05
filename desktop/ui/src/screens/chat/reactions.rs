//! The reaction chips under a message (components/ReactionBar.tsx,
//! messages/reactions.ts, chat.css `.lr-reactions`, `.lr-reaction`).

use egui::{Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2};
use uuid::Uuid;

use crate::text::{Block, TextStyle};
use crate::theme::{Theme, corner, mix};
use crate::widgets::tooltip::{Side, Tooltip};

const CHIP_H: f32 = 26.0;
const GAP: f32 = 6.0;
const STYLE: TextStyle = TextStyle::new(13.0, 400.0, 1.5);

/// One reaction as shown: the server's list with my pending vote applied.
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    pub emoji: String,
    pub users: Vec<Uuid>,
}

/// `displayedReactions`: only my own vote is optimistic.
pub fn displayed(
    reactions: &[gelabber_client::Reaction],
    me: Uuid,
    intent: Option<(&str, bool)>,
) -> Vec<Shown> {
    let mut out: Vec<Shown> = reactions
        .iter()
        .map(|r| {
            let mut users = Vec::new();
            for u in &r.user_ids {
                if !users.contains(u) {
                    users.push(*u);
                }
            }
            Shown {
                emoji: r.emoji.clone(),
                users,
            }
        })
        .collect();
    if let Some((emoji, add)) = intent {
        if !out.iter().any(|r| r.emoji == emoji) && add {
            out.push(Shown {
                emoji: emoji.to_owned(),
                users: Vec::new(),
            });
        }
        if let Some(row) = out.iter_mut().find(|r| r.emoji == emoji) {
            row.users.retain(|u| *u != me);
            if add {
                row.users.push(me);
            }
        }
    }
    out.retain(|r| !r.users.is_empty());
    out
}

/// "Rafi, Lena und 3 weitere"; I am "du".
pub fn reactor_names(users: &[Uuid], name_of: impl Fn(Uuid) -> Option<String>, me: Uuid) -> String {
    let names: Vec<String> = users
        .iter()
        .filter_map(|u| {
            if *u == me {
                Some("du".into())
            } else {
                name_of(*u)
            }
        })
        .collect();
    let shown = 3;
    let rest = users.len() - names.len().min(shown);
    let listed: Vec<String> = names.into_iter().take(shown).collect();
    let text = match listed.len() {
        0 => return String::new(),
        _ if rest > 0 => format!("{} und {rest} weitere", listed.join(", ")),
        1 => listed[0].clone(),
        n => format!("{} und {}", listed[..n - 1].join(", "), listed[n - 1]),
    };
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => text,
    }
}

/// Laid-out chips, relative to the bar's top-left.
pub struct Chips {
    pub chips: Vec<(Rect, Block, Shown)>,
    pub height: f32,
}

pub fn layout(ui: &Ui, shown: &[Shown], width: f32) -> Chips {
    let theme = Theme::of(ui.ctx());
    let mut chips = Vec::new();
    let (mut x, mut y) = (0.0, 0.0);
    for r in shown {
        let label = format!("{} {}", r.emoji, r.users.len());
        let block = Block::single(ui, &label, STYLE, theme.c.text);
        let w = (block.width() + 18.0 + 2.0).round();
        if x > 0.0 && x + w > width {
            x = 0.0;
            y += CHIP_H + GAP;
        }
        chips.push((
            Rect::from_min_size(pos2(x, y), Vec2::new(w, CHIP_H)),
            block,
            r.clone(),
        ));
        x += w + GAP;
    }
    let height = if chips.is_empty() { 0.0 } else { y + CHIP_H };
    Chips { chips, height }
}

/// Paints the chips at `origin`; returns the emoji clicked and whether it
/// adds my reaction.
pub fn paint(
    ui: &mut Ui,
    chips: &Chips,
    origin: Pos2,
    id: egui::Id,
    me: Uuid,
    can_send: bool,
    name_of: &dyn Fn(Uuid) -> Option<String>,
) -> Option<(String, bool)> {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let mut out = None;
    for (rect, block, shown) in &chips.chips {
        let rect = crate::widgets::snap(ui.ctx(), rect.translate(origin.to_vec2()));
        let mine = shown.users.contains(&me);
        let enabled = mine || can_send;
        let response = ui.interact(
            rect,
            id.with(("reaction", &shown.emoji)),
            if enabled {
                Sense::click()
            } else {
                Sense::hover()
            },
        );
        let hovered = enabled && response.hovered();
        let (fill, border) = if mine {
            (c.selected, mix(c.border, c.accent, 0.6))
        } else if hovered {
            (c.panel, mix(c.border, c.text, 0.22))
        } else {
            (c.panel, c.border)
        };
        let border = if mine && hovered {
            mix(c.border, c.text, 0.22)
        } else {
            border
        };
        let alpha = if enabled { 1.0 } else { 0.5 };
        ui.painter().rect(
            rect,
            corner(13.0),
            fill.gamma_multiply(alpha),
            Stroke::new(1.0, border.gamma_multiply(alpha)),
            StrokeKind::Inside,
        );
        let text_color: Color32 = c.text.gamma_multiply(alpha);
        block.paint_tinted(
            ui.painter(),
            pos2(rect.left() + 10.0, rect.center().y - STYLE.line_px() / 2.0),
            Some(text_color),
        );
        if hovered {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let who = reactor_names(&shown.users, name_of, me);
        if !who.is_empty() {
            Tooltip::new(&who).on_hover(&response, Side::Above);
        }
        if response.clicked() {
            out = Some((shown.emoji.clone(), !mine));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        let me = Uuid::from_u128(1);
        let a = Uuid::from_u128(2);
        let b = Uuid::from_u128(3);
        let name = |u: Uuid| Some(if u == a { "lena" } else { "Tom" }.to_owned());
        assert_eq!(reactor_names(&[a], name, me), "Lena");
        assert_eq!(reactor_names(&[me, a], name, me), "Du und lena");
        assert_eq!(
            reactor_names(&[a, b, me, Uuid::from_u128(4)], name, me),
            "Lena, Tom, du und 1 weitere"
        );
    }
}

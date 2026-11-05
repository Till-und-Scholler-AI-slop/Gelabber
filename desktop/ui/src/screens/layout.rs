//! The workspace grid shared by the shell and the pages (web shell.css,
//! index.css at a 1280×800 desktop): rail, sidebar column, page, members.

use egui::{Context, Id, Rect, pos2};

pub const RAIL_WIDTH: f32 = 64.0;
pub const SIDEBAR_WIDTH: f32 = 224.0;
pub const MEMBERS_WIDTH: f32 = 248.0;
pub const HEADER_HEIGHT: f32 = 52.0;

/// The page column between the sidebar and the member panel.
pub fn page_rect(screen: Rect, members: bool) -> Rect {
    let right = if members {
        screen.right() - MEMBERS_WIDTH
    } else {
        screen.right()
    };
    Rect::from_min_max(
        pos2(screen.left() + RAIL_WIDTH + SIDEBAR_WIDTH, screen.top()),
        pos2(right, screen.bottom()),
    )
}

fn members_id() -> Id {
    Id::unique("gelabber-members-visible")
}

/// Whether the member panel is open (the header's members button).
pub fn members_visible(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp(members_id())).unwrap_or(true)
}

pub fn set_members_visible(ctx: &Context, visible: bool) {
    ctx.data_mut(|d| d.insert_temp(members_id(), visible));
}

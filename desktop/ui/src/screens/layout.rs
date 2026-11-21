//! The workspace grid shared by the shell and the pages (web shell.css,
//! index.css at a 1280×800 desktop): rail, sidebar column, page, members.
//!
//! The channel list folds away and both sidebars take the width the user
//! dragged them to (GEL-34, components/sidebarLayout.ts): kept in the
//! context, stored across restarts by the platform (`native-state.json`).

use egui::{Context, Id, Rect, pos2};
use serde::{Deserialize, Serialize};

pub const RAIL_WIDTH: f32 = 64.0;
pub const SIDEBAR_WIDTH: f32 = 224.0;
pub const MEMBERS_WIDTH: f32 = 248.0;
pub const HEADER_HEIGHT: f32 = 52.0;
/// The narrowest and widest a sidebar gets (`WIDTH_MIN`, `WIDTH_MAX`).
pub const WIDTH_MIN: f32 = 200.0;
pub const WIDTH_MAX: f32 = 360.0;
/// A drag this far below the minimum folds the sidebar away.
pub const COLLAPSE_BELOW: f32 = WIDTH_MIN - 60.0;
/// The page between the sidebars keeps at least this much.
pub const PAGE_MIN: f32 = 440.0;
/// One arrow key press on a sidebar's edge.
pub const KEY_STEP: f32 = 16.0;
/// Where a header line starts its title: after the channel list's button
/// (`.workspace-main .lr-channel-header`, 52 instead of 20).
pub const HEADER_LEAD: f32 = 32.0;
/// The room the folded channel list's user panel takes at the rail's foot
/// (`.is-nav-collapsed .server-rail` padding).
pub const RAIL_PANEL_HEIGHT: f32 = 112.0;

/// What the user chose for the sidebars; `None` is the default width.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Sidebars {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidebar: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub members: Option<f32>,
    /// The channel list is folded away; the rail stays.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub collapsed: bool,
}

impl Sidebars {
    /// A damaged or hand-edited file's widths are brought into range.
    pub fn sane(self) -> Self {
        let width = |w: Option<f32>| w.filter(|w| w.is_finite()).map(clamp_width);
        Self {
            sidebar: width(self.sidebar),
            members: width(self.members),
            collapsed: self.collapsed,
        }
    }
}

/// Which sidebar an edge belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Channels,
    Members,
}

pub fn clamp_width(width: f32) -> f32 {
    width.clamp(WIDTH_MIN, WIDTH_MAX).round()
}

fn sidebars_id() -> Id {
    Id::unique("gelabber-sidebars")
}

pub fn sidebars(ctx: &Context) -> Sidebars {
    ctx.data(|d| d.get_temp(sidebars_id())).unwrap_or_default()
}

pub fn set_sidebars(ctx: &Context, sidebars: Sidebars) {
    ctx.data_mut(|d| d.insert_temp(sidebars_id(), sidebars.sane()));
}

/// A width for one side; `None` goes back to its default.
pub fn set_width(ctx: &Context, side: Side, width: Option<f32>) {
    let mut s = sidebars(ctx);
    let width = width.map(clamp_width);
    match side {
        Side::Channels => s.sidebar = width,
        Side::Members => s.members = width,
    }
    set_sidebars(ctx, s);
}

pub fn collapsed(ctx: &Context) -> bool {
    sidebars(ctx).collapsed
}

pub fn set_collapsed(ctx: &Context, collapsed: bool) {
    let mut s = sidebars(ctx);
    s.collapsed = collapsed;
    set_sidebars(ctx, s);
}

/// The widths the columns get in a window `viewport` wide (`fitWidths`):
/// the chosen ones, unless the page would drop under PAGE_MIN. Then the
/// member list gives way first, then the channel list, each down to
/// WIDTH_MIN. The app has no drawers: every column stays beside the page.
pub fn fit(viewport: f32, sidebars: Sidebars, members_shown: bool) -> (f32, f32) {
    let sidebar = sidebars.sidebar.unwrap_or(SIDEBAR_WIDTH);
    let members = sidebars.members.unwrap_or(MEMBERS_WIDTH);
    let sidebar_takes = !sidebars.collapsed;
    let mut over = RAIL_WIDTH
        + if sidebar_takes { sidebar } else { 0.0 }
        + if members_shown { members } else { 0.0 }
        + PAGE_MIN
        - viewport;
    let mut fitted_members = members;
    let mut fitted_sidebar = sidebar;
    if over > 0.0 && members_shown {
        let give = over.min((members - WIDTH_MIN).max(0.0));
        fitted_members -= give;
        over -= give;
    }
    if over > 0.0 && sidebar_takes {
        let give = over.min((sidebar - WIDTH_MIN).max(0.0));
        fitted_sidebar -= give;
    }
    (fitted_sidebar, fitted_members)
}

/// The shell's columns for one pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Columns {
    /// The channel list's width; 0 while it is folded away.
    pub sidebar: f32,
    /// The member list's width where one is shown.
    pub members: f32,
    /// The sidebar's width as its edge reports it (folded: the one it
    /// comes back with).
    pub sidebar_open: f32,
}

impl Columns {
    /// Where the page starts: after the rail and the channel list.
    pub fn page_left(&self, screen: Rect) -> f32 {
        screen.left() + RAIL_WIDTH + self.sidebar
    }
}

/// The columns in `screen`, with or without a member list beside the page.
pub fn columns(ctx: &Context, screen: Rect, members_shown: bool) -> Columns {
    let s = sidebars(ctx);
    let (sidebar, members) = fit(screen.width(), s, members_shown);
    Columns {
        sidebar: if s.collapsed { 0.0 } else { sidebar },
        members,
        sidebar_open: sidebar,
    }
}

/// The page column between the sidebar and the member panel.
pub fn page_rect(screen: Rect, columns: Columns, members: bool) -> Rect {
    let right = if members {
        screen.right() - columns.members
    } else {
        screen.right()
    };
    Rect::from_min_max(
        pos2(columns.page_left(screen), screen.top()),
        pos2(right, screen.bottom()),
    )
}

/// What a key does on a sidebar's edge (`edgeKey`, the WAI-ARIA window
/// splitter): `Some(Some(width))`, `Some(None)` to fold the side away, or
/// `None` for a key it leaves alone. Arrows move the edge, so on the
/// right-hand member list Left widens it.
pub fn edge_key(side: Side, key: egui::Key, width: f32) -> Option<Option<f32>> {
    use egui::Key;
    let (widen, narrow) = match side {
        Side::Channels => (Key::ArrowRight, Key::ArrowLeft),
        Side::Members => (Key::ArrowLeft, Key::ArrowRight),
    };
    match key {
        k if k == widen => Some(Some(clamp_width(width + KEY_STEP))),
        k if k == narrow => Some(Some(clamp_width(width - KEY_STEP))),
        Key::Home => Some(Some(WIDTH_MIN)),
        Key::End => Some(Some(WIDTH_MAX)),
        Key::Enter => Some(None),
        _ => None,
    }
}

/// The width a drag asks for (`dragWidth`), or `None` once it goes well
/// under the minimum. `delta` is how far the pointer moved to the right.
pub fn drag_width(side: Side, start: f32, delta: f32) -> Option<f32> {
    let raw = match side {
        Side::Channels => start + delta,
        Side::Members => start - delta,
    };
    (raw >= COLLAPSE_BELOW).then(|| clamp_width(raw))
}

fn members_id() -> Id {
    Id::unique("gelabber-members-visible")
}

/// The choice next to pictures (memberPanelState.ts `withVideo`).
fn video_choice_id() -> Id {
    Id::unique("gelabber-members-visible-video")
}

/// The page shows a voice channel whose pictures run.
fn video_id() -> Id {
    Id::unique("gelabber-members-video")
}

/// Whether the member panel is open (the header's members button). Next
/// to pictures it starts closed until chosen there.
pub fn members_visible(ctx: &Context) -> bool {
    if ctx
        .data(|d| d.get_temp::<bool>(video_id()))
        .unwrap_or(false)
    {
        return ctx.data(|d| d.get_temp(video_choice_id())).unwrap_or(false);
    }
    ctx.data(|d| d.get_temp(members_id())).unwrap_or(true)
}

pub fn set_members_visible(ctx: &Context, visible: bool) {
    let video = ctx
        .data(|d| d.get_temp::<bool>(video_id()))
        .unwrap_or(false);
    let id = if video {
        video_choice_id()
    } else {
        members_id()
    };
    ctx.data_mut(|d| d.insert_temp(id, visible));
}

/// The voice page says whether its pictures run (`setMemberPanelVideo`).
pub fn set_members_video(ctx: &Context, video: bool) {
    ctx.data_mut(|d| d.insert_temp(video_id(), video));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chosen(sidebar: f32, members: f32) -> Sidebars {
        Sidebars {
            sidebar: Some(sidebar),
            members: Some(members),
            collapsed: false,
        }
    }

    #[test]
    fn fits_like_the_web() {
        // The same cases as sidebarLayout.test.ts, without drawers.
        assert_eq!(fit(1440.0, Sidebars::default(), true), (224.0, 248.0));
        assert_eq!(fit(1160.0, chosen(360.0, 360.0), true), (360.0, 296.0));
        assert_eq!(fit(1000.0, chosen(360.0, 360.0), true), (296.0, 200.0));
        assert_eq!(fit(1000.0, chosen(360.0, 360.0), false), (360.0, 360.0));
        let folded = Sidebars {
            collapsed: true,
            ..chosen(360.0, 360.0)
        };
        assert_eq!(fit(1000.0, folded, true), (360.0, 360.0));
        // Never under the minimum.
        assert_eq!(fit(600.0, chosen(360.0, 360.0), true), (200.0, 200.0));
    }

    #[test]
    fn keys_and_drags_like_the_web() {
        use egui::Key;
        assert_eq!(
            edge_key(Side::Channels, Key::ArrowRight, 224.0),
            Some(Some(240.0))
        );
        assert_eq!(
            edge_key(Side::Members, Key::ArrowLeft, 248.0),
            Some(Some(264.0))
        );
        assert_eq!(
            edge_key(Side::Members, Key::ArrowRight, 248.0),
            Some(Some(232.0))
        );
        assert_eq!(
            edge_key(Side::Channels, Key::Home, 300.0),
            Some(Some(WIDTH_MIN))
        );
        assert_eq!(
            edge_key(Side::Channels, Key::End, 300.0),
            Some(Some(WIDTH_MAX))
        );
        assert_eq!(edge_key(Side::Channels, Key::Enter, 300.0), Some(None));
        assert_eq!(edge_key(Side::Channels, Key::A, 300.0), None);
        assert_eq!(drag_width(Side::Channels, 224.0, 40.0), Some(264.0));
        assert_eq!(drag_width(Side::Members, 248.0, 40.0), Some(208.0));
        assert_eq!(drag_width(Side::Channels, 224.0, -30.0), Some(WIDTH_MIN));
        assert_eq!(
            drag_width(Side::Channels, 224.0, COLLAPSE_BELOW - 225.0),
            None
        );
    }

    #[test]
    fn repairs_stored_widths() {
        let s = Sidebars {
            sidebar: Some(f32::NAN),
            members: Some(9000.0),
            collapsed: true,
        }
        .sane();
        assert_eq!(s.sidebar, None);
        assert_eq!(s.members, Some(WIDTH_MAX));
        assert!(s.collapsed);
    }
}

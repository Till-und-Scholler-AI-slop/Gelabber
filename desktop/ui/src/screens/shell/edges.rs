//! The channel list's fold button and the sidebars' draggable edges
//! (GEL-34, components/SidebarEdge.tsx, index.css `.sidebar-edge`): drag
//! for the width, double-click for the default, and on a focused edge the
//! arrows, Home, End and Enter (the WAI-ARIA window splitter).
//!
//! Each one is registered where the web has it in the Tab order (the
//! channel list's edge and the button before the page, the member list's
//! edge before the list) and painted once the page has drawn its header
//! line, over it ([`paint`]). An edge senses drags but not clicks, so a
//! message row under its page half still takes the click while the edge
//! takes the drag (egui's thin-handle rule); its double-click is read from
//! the pointer.

use egui::{Color32, CursorIcon, Id, PointerButton, Rect, Sense, Ui, Vec2, pos2};
use gelabber_i18n::shell as texts;

use crate::icons::{self, Icon};
use crate::screens::layout::{self, Columns, HEADER_HEIGHT, Side};
use crate::theme::{Theme, corner};
use crate::widgets::eased;

/// Grab width, centred on the column's line (`.sidebar-edge::before`).
const GRAB: f32 = 8.0;
/// A pointer resting this long lights the edge (`transition-delay`).
const REST: f32 = 0.15;
/// The lit line's width, centred inside [`GRAB`] (`.sidebar-edge::after`,
/// GEL-64): the same for hover, drag and keyboard focus.
const LINE: f32 = 4.0;

fn toggle_id() -> Id {
    Id::unique("gelabber-channel-list-toggle")
}

fn edge_id(side: Side) -> Id {
    Id::unique(("gelabber-sidebar-edge", side))
}

fn painting_id() -> Id {
    Id::unique("gelabber-sidebar-edges-paint")
}

/// What [`paint`] draws this pass.
#[derive(Clone, Copy, Debug, Default)]
struct Paint {
    toggle: Option<ToggleLook>,
    channels: Option<EdgeLook>,
    members: Option<EdgeLook>,
}

#[derive(Clone, Copy, Debug)]
struct ToggleLook {
    rect: Rect,
    open: bool,
    hovered: bool,
    focused: bool,
}

#[derive(Clone, Copy, Debug)]
struct EdgeLook {
    x: f32,
    ys: egui::Rangef,
    near: bool,
    dragged: bool,
    focused: bool,
}

fn record(ui: &Ui, change: impl FnOnce(&mut Paint)) {
    ui.data_mut(|d| change(d.get_temp_mut_or_default::<Paint>(painting_id())));
}

/// The header button at the page's top left (`ChannelListToggle`): lit
/// while the channel list is out, like the member list's button.
pub fn toggle(ui: &mut Ui, page_left: f32, top: f32) {
    let rect = Rect::from_min_size(
        pos2(page_left + 12.0, top + (HEADER_HEIGHT - 32.0) / 2.0),
        Vec2::splat(32.0),
    );
    let open = !layout::collapsed(ui.ctx());
    let title = if open {
        texts::channel_list_hide()
    } else {
        texts::channel_list_show()
    };
    // The web's `title`: a hover text, as browsers draw it.
    let response = ui
        .interact(rect, toggle_id(), Sense::click())
        .on_hover_text(title);
    response.widget_info(|| {
        let mut info =
            egui::WidgetInfo::labeled(egui::Role::Button, true, texts::channel_list_toggle());
        info.selected = Some(open);
        info
    });
    if response.clicked() {
        layout::set_collapsed(ui.ctx(), open);
    }
    let look = ToggleLook {
        rect,
        open: !layout::collapsed(ui.ctx()),
        hovered: response.hovered(),
        focused: response.has_focus() && crate::widgets::keyboard_focus(ui.ctx()),
    };
    record(ui, |p| p.toggle = Some(look));
}

/// The channel list's right edge; `column` is the list's column (folded:
/// none wide, at the rail's edge).
pub fn channels(ui: &mut Ui, column: Rect, cols: Columns, open: bool) {
    let look = edge(
        ui,
        Side::Channels,
        column.right(),
        column.y_range(),
        cols.sidebar_open,
        open,
    );
    record(ui, |p| p.channels = look);
}

/// The member list's left edge in `content` (hidden: at its right end).
pub fn members(ui: &mut Ui, content: Rect, cols: Columns, shown: bool) {
    let x = if shown {
        content.right() - cols.members
    } else {
        content.right()
    };
    let look = edge(ui, Side::Members, x, content.y_range(), cols.members, shown);
    record(ui, |p| p.members = look);
}

#[derive(Clone, Copy)]
struct Drag {
    x: f32,
    width: f32,
}

/// `present`: the sidebar is out. Folded away, its edge goes too, unless
/// a drag still holds it (dragging back out brings the sidebar back).
fn edge(
    ui: &mut Ui,
    side: Side,
    x: f32,
    ys: egui::Rangef,
    width: f32,
    present: bool,
) -> Option<EdgeLook> {
    let id = edge_id(side);
    let drag_id = id.with("drag");
    if !present && ui.data(|d| d.get_temp::<Drag>(drag_id)).is_none() {
        return None;
    }
    let rect = Rect::from_x_y_ranges((x - GRAB / 2.0)..=(x + GRAB / 2.0), ys);
    let response = ui.interact(rect, id, Sense::DRAG | Sense::FOCUSABLE);
    let label = match side {
        Side::Channels => texts::edge_channels_label(),
        Side::Members => texts::edge_members_label(),
    };
    response.widget_info(|| {
        let mut info = egui::WidgetInfo::labeled(egui::Role::Splitter, true, label);
        info.value = Some(f64::from(width));
        info
    });
    let near = ui
        .ctx()
        .pointer_hover_pos()
        .is_some_and(|at| rect.contains(at));

    if response.drag_started()
        && let Some(at) = response.interact_pointer_pos()
    {
        ui.data_mut(|d| d.insert_temp(drag_id, Drag { x: at.x, width }));
    }
    if response.dragged()
        && let Some(start) = ui.data(|d| d.get_temp::<Drag>(drag_id))
        && let Some(at) = ui.ctx().pointer_latest_pos()
    {
        match layout::drag_width(side, start.width, at.x - start.x) {
            None => fold(ui, side, true),
            Some(next) => {
                fold(ui, side, false);
                layout::set_width(ui.ctx(), side, Some(next));
            }
        }
    }
    if response.drag_stopped() || (!response.dragged() && !ui.input(|i| i.pointer.any_down())) {
        ui.data_mut(|d| d.remove::<Drag>(drag_id));
    }
    if near && ui.input(|i| i.pointer.button_double_clicked(PointerButton::Primary)) {
        layout::set_width(ui.ctx(), side, None);
    }
    if response.has_focus() {
        // Left and Right move the edge, not egui's spatial focus.
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    horizontal_arrows: true,
                    ..Default::default()
                },
            )
        });
        let keys = [
            egui::Key::ArrowLeft,
            egui::Key::ArrowRight,
            egui::Key::Home,
            egui::Key::End,
            egui::Key::Enter,
        ];
        for key in keys {
            if !ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, key)) {
                continue;
            }
            match layout::edge_key(side, key, width) {
                Some(Some(next)) => layout::set_width(ui.ctx(), side, Some(next)),
                Some(None) => {
                    fold(ui, side, true);
                    if side == Side::Channels {
                        ui.memory_mut(|m| m.request_focus(toggle_id()));
                    }
                }
                None => {}
            }
        }
    }
    Some(EdgeLook {
        x,
        ys,
        near,
        dragged: response.dragged(),
        focused: response.has_focus() && crate::widgets::keyboard_focus(ui.ctx()),
    })
}

fn fold(ui: &Ui, side: Side, folded: bool) {
    match side {
        Side::Channels => layout::set_collapsed(ui.ctx(), folded),
        Side::Members => layout::set_members_visible(ui.ctx(), !folded),
    }
}

/// Paints what this pass registered, over the page: the button, then the
/// edges' lines; the cursor last, so a row under an edge does not set its own.
pub fn paint(ui: &Ui) {
    let Some(p) = ui.data_mut(|d| d.remove_temp::<Paint>(painting_id())) else {
        return;
    };
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    if let Some(t) = p.toggle {
        let lit = t.open || t.hovered;
        let fill = eased(
            ui.ctx(),
            toggle_id().with("fill"),
            lit,
            Color32::TRANSPARENT,
            c.hover,
        );
        ui.painter().rect_filled(t.rect, corner(7.0), fill);
        if t.focused {
            crate::widgets::focus_ring(ui.painter(), t.rect, 7.0, &theme);
        }
        icons::paint_centered(
            ui.painter(),
            Icon::Sidebar,
            t.rect,
            18.0,
            if lit { c.text } else { c.muted },
        );
        if t.hovered {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
    }
    for (side, look) in [(Side::Channels, p.channels), (Side::Members, p.members)] {
        let Some(e) = look else { continue };
        // The line: lit after a short rest, at once while dragged or
        // focused from the keyboard (in the focus colour then).
        let rest = ui.input(|i| i.pointer.time_since_last_movement()) >= REST;
        let on = e.dragged || e.focused || (e.near && rest);
        let t = ui.ctx().animate_bool_with_time(
            edge_id(side).with("lit"),
            on,
            crate::theme::motion::FAST,
        );
        if e.near && !rest {
            ui.ctx().request_repaint_after_secs(REST);
        }
        if t > 0.0 {
            let color = if e.focused { c.focus } else { c.accent };
            let line = Rect::from_x_y_ranges((e.x - LINE / 2.0)..=(e.x + LINE / 2.0), e.ys);
            ui.painter().rect_filled(line, 0.0, color.gamma_multiply(t));
        }
        if e.near || e.dragged {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::testkit::click;
    use crate::app::testkit::{
        ALLGEMEIN, FakeApi, HOME, launcher, wait_for, wait_for_label, workspace,
    };
    use crate::platform::paths::testing::TempConfig;
    use crate::screens::layout;
    use egui_kittest::kittest::Queryable;

    /// The button folds the channel list away and brings it back; the
    /// rail stays, the edge goes with the list.
    #[test]
    fn the_button_folds_the_channel_list() {
        let _config = TempConfig::new("edges-fold");
        let api = FakeApi::start(true, |method, path, _| workspace(method, path));
        let mut harness = launcher(&api, &format!("/s/{HOME}/c/{ALLGEMEIN}"));
        wait_for_label(&mut harness, "Übersicht");
        assert!(harness.query_by_label("Breite der Kanalliste").is_some());
        click(&mut harness, "Kanalliste");
        wait_for(&mut harness, "the folded list", |h| {
            h.query_by_label("Übersicht").is_none()
        });
        assert!(layout::collapsed(&harness.ctx));
        assert!(harness.query_by_label("Breite der Kanalliste").is_none());
        // The rail and the user panel's settings stay.
        assert!(harness.query_by_label("Direktnachrichten").is_some());
        assert!(harness.query_by_label("Einstellungen").is_some());
        click(&mut harness, "Kanalliste");
        wait_for_label(&mut harness, "Übersicht");
        assert!(!layout::collapsed(&harness.ctx));
    }

    /// On a focused edge the arrows move it, End takes the widest, and
    /// Enter folds the list away with the focus on the button.
    #[test]
    fn keys_on_the_edge() {
        let _config = TempConfig::new("edges-keys");
        let api = FakeApi::start(true, |method, path, _| workspace(method, path));
        let mut harness = launcher(&api, &format!("/s/{HOME}/c/{ALLGEMEIN}"));
        wait_for_label(&mut harness, "Übersicht");
        harness.get_by_label("Breite der Kanalliste").focus();
        harness.run_steps(2);
        harness.key_press(egui::Key::ArrowRight);
        harness.run_steps(2);
        assert_eq!(layout::sidebars(&harness.ctx).sidebar, Some(240.0));
        harness.key_press(egui::Key::End);
        harness.run_steps(2);
        assert_eq!(
            layout::sidebars(&harness.ctx).sidebar,
            Some(layout::WIDTH_MAX)
        );
        harness.key_press(egui::Key::Enter);
        harness.run_steps(3);
        assert!(layout::collapsed(&harness.ctx));
        assert_eq!(
            crate::app::testkit::focused_label(&harness).as_deref(),
            Some("Kanalliste")
        );
    }
}

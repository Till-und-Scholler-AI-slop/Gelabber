//! The menu of a channel, category, server or conversation (web
//! RowMenu.tsx, rowMenuEntries.ts): a right click, Umschalt+F10 or the
//! context-menu key on the row, or its "…" button. "Als gelesen
//! markieren", "Stummschalten" (a submenu of lengths, or "Stummschaltung
//! aufheben"), "Benachrichtigungen" (the levels as radio entries) and
//! "Benachrichtigungseinstellungen…"; a channel adds edit and delete for
//! those who manage channels, a category its own entries. ↑/↓/Pos1/Ende
//! walk a menu, → and ← open and close a submenu, Esc closes the submenu
//! first, then the menu, and focus goes back to the row.

use egui::{Id, Key, Modifiers, Rect, Sense, Ui, Vec2, pos2};
use gelabber_client::{Category, Channel, ChannelKind, NotificationLevel, SettingScope};
use uuid::Uuid;

use super::common::{self, paint_mid};
use super::overlays::{self, Target, notify};
use super::state;
use crate::app::AppCtx;
use crate::icons::{self, Icon};
use crate::text::{self, Block};
use crate::theme::Theme;
use crate::widgets::{Popover, hairline_h, popover};

/// Whose menu it is (state::RowMenu holds it with the anchor).
#[derive(Clone, Debug)]
pub enum Owner {
    Channel(Uuid, Channel),
    Category(Uuid, Category),
    Server(Uuid, String),
    Dm(Uuid, String),
    /// The rail's home tile: every conversation.
    AllDms,
}

impl Owner {
    /// The row it belongs to, to keep its "…" in sight while it is open.
    fn key(&self) -> Uuid {
        match self {
            Owner::Channel(_, ch) => ch.id,
            Owner::Category(_, cat) => cat.id,
            Owner::Server(id, _) | Owner::Dm(id, _) => *id,
            Owner::AllDms => Uuid::nil(),
        }
    }

    /// Its notification target, if it has notifications.
    fn target(&self) -> Option<Target> {
        match self {
            Owner::Channel(server, ch) if ch.kind == ChannelKind::Text => {
                Some(Target::channel(ch.id, *server, format!("#{}", ch.name)))
            }
            Owner::Channel(..) | Owner::Category(..) => None,
            Owner::Server(id, name) => Some(Target::server(*id, name)),
            Owner::Dm(id, name) => Some(Target::dm(*id, name.clone())),
            Owner::AllDms => Some(Target::all_dms()),
        }
    }

    /// The menu's name (`aria-label`).
    fn label(&self) -> String {
        match self {
            Owner::Channel(_, ch) if ch.kind == ChannelKind::Text => format!("#{}", ch.name),
            Owner::Channel(_, ch) => ch.name.clone(),
            Owner::Category(_, cat) => format!("Kategorie {}", cat.name),
            Owner::Server(_, name) | Owner::Dm(_, name) => name.clone(),
            Owner::AllDms => "Direktnachrichten".into(),
        }
    }
}

/// The open menu.
#[derive(Clone, Debug)]
pub struct Open {
    pub owner: Owner,
    /// The row (or its "…") it hangs under.
    pub anchor: Rect,
    /// What takes focus back when it closes.
    pub opener: Option<Id>,
    /// The open submenu, by entry index, and whether focus goes into it.
    pub sub: Option<(usize, bool)>,
    /// Where the menu and its submenu were drawn last.
    pub menu_rect: Option<Rect>,
    pub sub_rect: Option<Rect>,
}

/// Opens `owner`'s menu under `anchor`; `opener` gets the focus back.
pub fn open(ctx: &egui::Context, owner: Owner, anchor: Rect, opener: Option<Id>) {
    state::with(ctx, |s| {
        s.row_menu = Some(Open {
            owner,
            anchor,
            opener,
            sub: None,
            menu_rect: None,
            sub_rect: None,
        });
    });
    ctx.request_repaint();
}

/// Whether the menu of the row `id` (a channel, category, server or
/// conversation) is open.
pub fn is_open_for(ctx: &egui::Context, id: Uuid) -> bool {
    state::with(ctx, |s| {
        s.row_menu.as_ref().is_some_and(|m| m.owner.key() == id)
    })
}

/// Umschalt+F10 or the context-menu key on the focused `response`: its
/// row's menu (`isMenuKey`).
pub fn menu_key(ui: &Ui, response: &egui::Response) -> bool {
    response.has_focus()
        && ui.input_mut(|i| {
            i.consume_key(Modifiers::SHIFT, Key::F10) || i.consume_key(Modifiers::NONE, Key::Menu)
        })
}

/// What an entry does.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Act {
    MarkRead,
    Mute(Option<i64>),
    Unmute,
    /// `None`: as the server.
    Level(Option<NotificationLevel>),
    Settings,
    EditChannel,
    DeleteChannel,
    AddChannel,
    RenameCategory,
    DeleteCategory,
}

#[derive(Clone, Debug)]
enum Leaf {
    Item {
        label: String,
        icon: Option<Icon>,
        /// A second line: how long a mute still holds.
        hint: Option<String>,
        danger: bool,
        act: Act,
    },
    Radio {
        label: String,
        checked: bool,
        act: Act,
    },
    Separator,
}

#[derive(Clone, Debug)]
enum Entry {
    Leaf(Leaf),
    Sub {
        label: &'static str,
        icon: Icon,
        entries: Vec<Leaf>,
    },
}

fn item(label: &str, icon: Icon, act: Act) -> Entry {
    Entry::Leaf(Leaf::Item {
        label: label.to_owned(),
        icon: Some(icon),
        hint: None,
        danger: false,
        act,
    })
}

fn danger(label: &str, act: Act) -> Entry {
    Entry::Leaf(Leaf::Item {
        label: label.to_owned(),
        icon: Some(Icon::Trash),
        hint: None,
        danger: true,
        act,
    })
}

/// `notificationEntries`: what the menu offers for `target` with `own` and
/// (for a channel) its server's setting.
fn notification_entries(
    target: &Target,
    own: notify::Setting,
    server: Option<notify::Setting>,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<Entry> {
    let label = if target.all_dms {
        "Alle als gelesen markieren"
    } else {
        "Als gelesen markieren"
    };
    let mut entries = vec![item(label, Icon::Check, Act::MarkRead)];
    if target.all_dms {
        return entries;
    }
    entries.push(if own.is_muted(now) {
        Entry::Leaf(Leaf::Item {
            label: "Stummschaltung aufheben".into(),
            icon: Some(Icon::Bell),
            hint: Some(notify::muted_line(own.until)),
            danger: false,
            act: Act::Unmute,
        })
    } else {
        Entry::Sub {
            label: "Stummschalten",
            icon: Icon::BellOff,
            entries: notify::MUTES
                .iter()
                .map(|(label, _, minutes)| Leaf::Item {
                    label: (*label).to_owned(),
                    icon: None,
                    hint: None,
                    danger: false,
                    act: Act::Mute(*minutes),
                })
                .collect(),
        }
    });
    // A conversation has no level of its own: every message is for you.
    if !target.dm {
        let mut levels = Vec::new();
        if let Some(server) = server {
            levels.push(Leaf::Radio {
                label: format!(
                    "Wie der Server ({})",
                    notify::level_name(server.level.unwrap_or_default())
                ),
                checked: own.level.is_none(),
                act: Act::Level(None),
            });
        }
        for (level, label, _) in notify::LEVELS {
            levels.push(Leaf::Radio {
                label: label.to_owned(),
                checked: if server.is_some() {
                    own.level == Some(level)
                } else {
                    own.level.unwrap_or_default() == level
                },
                act: Act::Level(Some(level)),
            });
        }
        entries.push(Entry::Sub {
            label: "Benachrichtigungen",
            icon: Icon::Bell,
            entries: levels,
        });
    }
    entries.push(item(
        "Benachrichtigungseinstellungen…",
        Icon::Gear,
        Act::Settings,
    ));
    entries
}

/// Every entry of `owner`'s menu; `manage` whether the user manages
/// channels (edit and delete).
fn entries(app: &AppCtx, owner: &Owner) -> Vec<Entry> {
    let now = app.client.http().server_now();
    let manage = match owner {
        Owner::Channel(server, _) | Owner::Category(server, _) => app.client.with_state(|s| {
            s.server(*server)
                .is_some_and(|me| me.can(gelabber_client::Permission::ManageChannels))
        }),
        _ => false,
    };
    let mut out = match owner.target() {
        Some(target) => {
            let (own, server) = app.client.with_state(|s| {
                (
                    notify::setting_of(s, target.scope, target.id),
                    target
                        .server
                        .map(|id| notify::setting_of(s, SettingScope::Server, id)),
                )
            });
            notification_entries(&target, own, server, now)
        }
        None => Vec::new(),
    };
    match owner {
        Owner::Channel(..) if manage => {
            if !out.is_empty() {
                out.push(Entry::Leaf(Leaf::Separator));
            }
            out.push(item("Kanal bearbeiten", Icon::Pencil, Act::EditChannel));
            out.push(Entry::Leaf(Leaf::Separator));
            out.push(danger("Kanal löschen", Act::DeleteChannel));
        }
        Owner::Category(..) => {
            out.push(item("Kanal hier erstellen", Icon::Plus, Act::AddChannel));
            out.push(item(
                "Kategorie umbenennen",
                Icon::Pencil,
                Act::RenameCategory,
            ));
            out.push(Entry::Leaf(Leaf::Separator));
            out.push(danger("Kategorie löschen", Act::DeleteCategory));
        }
        _ => {}
    }
    out
}

fn item_id(level: usize, index: usize) -> Id {
    Id::unique(("row-menu-item", level, index))
}

/// The focusable entries of a level, by their index in it.
fn focusable(entries: &[Leaf]) -> Vec<usize> {
    entries
        .iter()
        .enumerate()
        .filter(|(_, e)| !matches!(e, Leaf::Separator))
        .map(|(i, _)| i)
        .collect()
}

/// ↑/↓/Pos1/Ende within one level (focus wraps), when focus is on one of
/// its entries; the arrows are the menu's, not egui's spatial moves.
fn walk(ui: &Ui, level: usize, indices: &[usize]) {
    let focused = ui.memory(|m| m.focused());
    let Some(at) = indices
        .iter()
        .position(|i| focused == Some(item_id(level, *i)))
    else {
        return;
    };
    if let Some(id) = focused {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    ..Default::default()
                },
            )
        });
    }
    let n = indices.len();
    let next = ui.input_mut(|i| {
        if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
            Some((at + 1) % n)
        } else if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
            Some((at + n - 1) % n)
        } else if i.consume_key(Modifiers::NONE, Key::Home) {
            Some(0)
        } else if i.consume_key(Modifiers::NONE, Key::End) {
            Some(n - 1)
        } else {
            None
        }
    });
    if let Some(next) = next {
        ui.memory_mut(|m| m.request_focus(item_id(level, indices[next])));
        crate::widgets::set_keyboard_focus(ui.ctx(), true);
    }
}

/// What a frame of the menu asks for.
enum Pick {
    Act(Act),
    /// Open (focus into it) or close a submenu.
    OpenSub(usize, bool),
    CloseSub,
}

/// Draws the open menu, if any, and does what is picked. Call before the
/// pages: it takes Esc and the arrows first.
pub fn show(ui: &mut Ui, app: &mut AppCtx) {
    let ctx = ui.ctx().clone();
    let Some(open) = state::with(&ctx, |s| s.row_menu.clone()) else {
        return;
    };
    let entries = entries(app, &open.owner);
    if entries.is_empty() {
        state::with(&ctx, |s| s.row_menu = None);
        return;
    }
    let focus_in_menu = |ctx: &egui::Context| {
        let focused = ctx.memory(|m| m.focused());
        focused.is_some_and(|f| {
            (0..2).any(|level| (0..entries.len().max(8)).any(|i| item_id(level, i) == f))
        })
    };

    // Esc: the submenu first, then the menu; focus goes back. The menu
    // stays drawn while only its submenu closes (a frame without it would
    // open it anew, with focus on its first entry).
    let mut open = open;
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        match open.sub {
            Some((index, _)) => {
                ctx.memory_mut(|m| m.request_focus(item_id(0, index)));
                open.sub = None;
                state::with(&ctx, |s| {
                    if let Some(m) = s.row_menu.as_mut() {
                        m.sub = None;
                    }
                });
            }
            None => {
                close(&ctx, &open, true);
                return;
            }
        }
    }
    // A submenu opened by a key takes the focus until its first entry has
    // it (a pass may be discarded while the submenu measures itself).
    if let Some((index, true)) = open.sub
        && ctx
            .memory(|m| m.focused())
            .is_some_and(|f| (0..8).any(|i| f == item_id(1, i)))
    {
        open.sub = Some((index, false));
        state::with(&ctx, |s| {
            if let Some(m) = s.row_menu.as_mut() {
                m.sub = Some((index, false));
            }
        });
    }

    let top_leaves: Vec<Leaf> = entries
        .iter()
        .map(|e| match e {
            Entry::Leaf(leaf) => leaf.clone(),
            Entry::Sub { label, .. } => Leaf::Item {
                label: (*label).to_owned(),
                icon: None,
                hint: None,
                danger: false,
                act: Act::Settings,
            },
        })
        .collect();
    walk(ui, 0, &focusable(&top_leaves));
    let sub_entries = open.sub.and_then(|(i, _)| match entries.get(i) {
        Some(Entry::Sub { entries, .. }) => Some(entries.clone()),
        _ => None,
    });
    if let Some(sub) = &sub_entries {
        walk(ui, 1, &focusable(sub));
    }
    // → on a submenu's entry opens it; ← in a submenu goes back.
    let focused = ctx.memory(|m| m.focused());
    let mut pick: Option<Pick> = None;
    for (i, entry) in entries.iter().enumerate() {
        if matches!(entry, Entry::Sub { .. })
            && focused == Some(item_id(0, i))
            && ctx.input_mut(|inp| inp.consume_key(Modifiers::NONE, Key::ArrowRight))
        {
            pick = Some(Pick::OpenSub(i, true));
        }
    }
    if let Some((index, _)) = open.sub
        && sub_entries
            .as_ref()
            .is_some_and(|sub| (0..sub.len()).any(|i| focused == Some(item_id(1, i))))
        && ctx.input_mut(|inp| inp.consume_key(Modifiers::NONE, Key::ArrowLeft))
    {
        ctx.memory_mut(|m| m.request_focus(item_id(0, index)));
        pick = Some(Pick::CloseSub);
    }

    let width = menu_width(ui, &entries);
    let menu_label = open.owner.label();
    let mut trigger_rects = vec![Rect::NOTHING; entries.len()];
    let mut menu_rect = None;
    let shown = Popover::new(("row-menu", open.owner.key()), open.anchor)
        .also_inside(open.sub.and(open.sub_rect))
        .show(&ctx, |ui| {
            menu_rect = Some(ui.max_rect().expand(9.0));
            // `role="menu"` named by its row.
            let named = ui.interact(ui.max_rect(), Id::unique("row-menu-name"), Sense::hover());
            crate::widgets::announce(&named, egui::Role::Menu, &menu_label);
            ui.spacing_mut().item_spacing.y = 2.0;
            let mut picked = None;
            for (i, entry) in entries.iter().enumerate() {
                match entry {
                    Entry::Leaf(leaf) => {
                        let response = leaf_row(ui, item_id(0, i), leaf, width);
                        if let Some(response) = response {
                            if response.hovered() && open.sub.is_some() {
                                picked = Some(Pick::CloseSub);
                            }
                            if response.clicked()
                                && let Some(act) = leaf_act(leaf)
                            {
                                picked = Some(Pick::Act(act));
                            }
                        }
                    }
                    Entry::Sub { label, icon, .. } => {
                        let expanded = open.sub.is_some_and(|(s, _)| s == i);
                        let response = row(
                            ui,
                            item_id(0, i),
                            RowLook {
                                label,
                                icon: Some(*icon),
                                hint: None,
                                danger: false,
                                checked: None,
                                expanded: Some(expanded),
                            },
                            width,
                        );
                        trigger_rects[i] = response.rect;
                        let keyboard = !ui.input(|inp| inp.pointer.any_click());
                        if response.clicked() {
                            picked = Some(if expanded && !keyboard {
                                Pick::CloseSub
                            } else {
                                Pick::OpenSub(i, keyboard)
                            });
                        } else if response.hovered() && !expanded {
                            picked = Some(Pick::OpenSub(i, false));
                        }
                    }
                }
            }
            picked
        });
    let Some(picked) = shown else {
        // A press outside; the right press that opens another row's menu
        // replaces it instead.
        if !ui.input(|i| i.pointer.secondary_pressed()) {
            close(&ctx, &open, false);
        }
        return;
    };
    pick = pick.or(picked);

    // The submenu beside the menu.
    let mut sub_rect = None;
    if let (Some((index, focus)), Some(sub), Some(menu)) = (open.sub, &sub_entries, menu_rect) {
        let anchor = trigger_rects[index];
        let width = leaves_width(ui, sub);
        let first = (0..sub.len()).find(|i| !matches!(sub[*i], Leaf::Separator));
        let sub_shown = Popover::new(("row-submenu", open.owner.key(), index), anchor)
            .beside(menu)
            .takes_focus(false)
            .show(&ctx, |ui| {
                sub_rect = Some(ui.max_rect().expand(9.0));
                let named = ui.interact(
                    ui.max_rect(),
                    Id::unique("row-submenu-name"),
                    Sense::hover(),
                );
                if let Some(Entry::Sub { label, .. }) = entries.get(index) {
                    crate::widgets::announce(&named, egui::Role::Menu, label);
                }
                ui.spacing_mut().item_spacing.y = 2.0;
                let mut picked = None;
                for (i, leaf) in sub.iter().enumerate() {
                    let id = item_id(1, i);
                    if focus && Some(i) == first {
                        ui.memory_mut(|m| m.request_focus(id));
                    }
                    if let Some(response) = leaf_row(ui, id, leaf, width)
                        && response.clicked()
                    {
                        picked = leaf_act(leaf);
                    }
                }
                picked
            });
        if focus {
            crate::widgets::set_keyboard_focus(&ctx, true);
        }
        if let Some(Some(act)) = sub_shown {
            pick = Some(Pick::Act(act));
        }
    }

    state::with(&ctx, |s| {
        if let Some(m) = s.row_menu.as_mut() {
            m.menu_rect = menu_rect;
            m.sub_rect = sub_rect;
        }
    });
    match pick {
        None => {}
        Some(Pick::OpenSub(index, focus)) => state::with(&ctx, |s| {
            if let Some(m) = s.row_menu.as_mut()
                && (m.sub.map(|(i, _)| i) != Some(index) || focus)
            {
                m.sub = Some((index, focus));
            }
        }),
        Some(Pick::CloseSub) => state::with(&ctx, |s| {
            if let Some(m) = s.row_menu.as_mut() {
                m.sub = None;
            }
        }),
        Some(Pick::Act(act)) => {
            let refocus = focus_in_menu(&ctx);
            close(&ctx, &open, refocus);
            run(&ctx, app, &open.owner, act);
        }
    }
    ctx.request_repaint();
}

/// Closes the menu; `refocus` gives the focus back to its opener.
fn close(ctx: &egui::Context, open: &Open, refocus: bool) {
    state::with(ctx, |s| s.row_menu = None);
    if refocus && let Some(opener) = open.opener {
        ctx.memory_mut(|m| m.request_focus(opener));
    }
}

fn leaf_act(leaf: &Leaf) -> Option<Act> {
    match leaf {
        Leaf::Item { act, .. } | Leaf::Radio { act, .. } => Some(*act),
        Leaf::Separator => None,
    }
}

fn run(ctx: &egui::Context, app: &mut AppCtx, owner: &Owner, act: Act) {
    let now = app.client.http().server_now();
    if let Some(target) = owner.target() {
        let own = app
            .client
            .with_state(|s| notify::setting_of(s, target.scope, target.id));
        let to = notify::setting_target(&target);
        let server_scope = target.scope == SettingScope::Server;
        match act {
            Act::MarkRead => return notify::mark_read(app, &target),
            Act::Mute(minutes) => {
                return app
                    .client
                    .set_notifications_from_menu(to, own.mute(minutes));
            }
            Act::Unmute => return app.client.set_notifications_from_menu(to, own.unmute()),
            Act::Level(level) => {
                return app
                    .client
                    .set_notifications_from_menu(to, own.with_level(level, server_scope, now));
            }
            Act::Settings => return overlays::open_notifications(ctx, target),
            _ => {}
        }
    }
    match (owner, act) {
        (Owner::Channel(server, ch), Act::EditChannel) => {
            super::dialogs::open_channel(ctx, *server, Some(ch), ch.category_id)
        }
        (Owner::Channel(server, ch), Act::DeleteChannel) => {
            super::dialogs::confirm_delete_channel(ctx, *server, ch)
        }
        (Owner::Category(server, cat), Act::AddChannel) => {
            super::dialogs::open_channel(ctx, *server, None, Some(cat.id))
        }
        (Owner::Category(server, cat), Act::RenameCategory) => {
            super::dialogs::open_category(ctx, *server, Some(cat))
        }
        (Owner::Category(server, cat), Act::DeleteCategory) => {
            super::dialogs::confirm_delete_category(ctx, *server, cat)
        }
        _ => {}
    }
}

/// How an entry looks.
struct RowLook<'a> {
    label: &'a str,
    icon: Option<Icon>,
    hint: Option<&'a str>,
    danger: bool,
    /// A radio entry: whether it is the one set.
    checked: Option<bool>,
    /// A submenu's entry: whether it is open.
    expanded: Option<bool>,
}

fn leaf_row(ui: &mut Ui, id: Id, leaf: &Leaf, width: f32) -> Option<egui::Response> {
    match leaf {
        Leaf::Separator => {
            // `hr`: 1px border line, margin 4px 6px.
            let (r, _) = ui.allocate_exact_size(Vec2::new(width, 9.0), Sense::hover());
            let theme = Theme::of(ui.ctx());
            hairline_h(
                ui.painter(),
                &theme,
                r.left() + 6.0..=r.right() - 6.0,
                r.top() + 4.0,
            );
            None
        }
        Leaf::Item {
            label,
            icon,
            hint,
            danger,
            ..
        } => Some(row(
            ui,
            id,
            RowLook {
                label,
                icon: *icon,
                hint: hint.as_deref(),
                danger: *danger,
                checked: None,
                expanded: None,
            },
            width,
        )),
        Leaf::Radio { label, checked, .. } => Some(row(
            ui,
            id,
            RowLook {
                label,
                icon: None,
                hint: None,
                danger: false,
                checked: Some(*checked),
                expanded: None,
            },
            width,
        )),
    }
}

/// `.shell-menu-item`: 36 high (more with a hint: padding 6 around two
/// lines), radius 5, 18px icon at 10, label at 38, hover fill; a radio's
/// check in the accent's text colour, a submenu's chevron at the end.
fn row(ui: &mut Ui, id: Id, look: RowLook<'_>, width: f32) -> egui::Response {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let color = if look.danger { c.danger } else { c.text };
    let label = Block::single(ui, look.label, text::UI, color);
    let hint = look.hint.map(|h| Block::single(ui, h, text::META, c.muted));
    let height = match &hint {
        Some(h) => (6.0 + label.height() + h.height() + 6.0).max(36.0),
        None => 36.0,
    };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    let role = match (look.checked, look.expanded) {
        (Some(_), _) => egui::Role::MenuItemRadio,
        _ => egui::Role::MenuItem,
    };
    response.widget_info(|| {
        let mut info = egui::WidgetInfo::labeled(role, true, look.label);
        info.selected = look.checked;
        info
    });
    popover::focus_first(&response);
    let painter = ui.painter();
    if response.hovered() || look.expanded == Some(true) {
        common::fill(painter, rect, theme.radius.sm, c.hover);
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    common::focus_visible(ui, &response, rect, theme.radius.sm);
    let icon_color = if look.danger {
        c.danger
    } else if look.checked == Some(true) {
        c.accent_text
    } else {
        c.muted
    };
    let icon = match look.checked {
        Some(true) => Some(Icon::Check),
        Some(false) => None,
        None => look.icon,
    };
    if let Some(icon) = icon {
        icons::paint(
            painter,
            icon,
            pos2(rect.left() + 10.0, rect.center().y - 9.0),
            18.0,
            icon_color,
        );
    }
    // Without an icon (a submenu's lengths) the label starts at the padding.
    let x = rect.left() + label_inset(icon.is_some() || look.checked.is_some());
    match hint {
        Some(h) => {
            let top = rect.top() + 6.0;
            label.paint(painter, pos2(x, top));
            h.paint(painter, pos2(x, top + label.height()));
        }
        None => paint_mid(painter, &label, x, rect.center().y),
    }
    if look.expanded.is_some() {
        icons::paint_turned(
            painter,
            Icon::Chevron,
            pos2(rect.right() - 10.0 - 16.0 + 4.0, rect.center().y - 8.0),
            16.0,
            c.muted,
            3,
        );
    }
    response
}

/// Wide enough for the longest entry (padding 10, icon 18, gap 10, label,
/// gap and chevron, padding 10), at least the popover's 180.
fn menu_width(ui: &Ui, entries: &[Entry]) -> f32 {
    let widest = entries
        .iter()
        .map(|e| match e {
            Entry::Leaf(leaf) => leaf_text_width(ui, leaf),
            Entry::Sub { label, .. } => {
                label_inset(true)
                    + Block::single(ui, label, text::UI, egui::Color32::WHITE).width()
                    + 10.0
                    + 16.0
                    - 4.0
            }
        })
        .fold(0.0, f32::max);
    (widest + 10.0).ceil().max(180.0 - 18.0)
}

/// Where an entry's label starts: after the 18px icon column, or at the
/// padding when the entry has none.
fn label_inset(icon: bool) -> f32 {
    if icon { 38.0 } else { 10.0 }
}

fn leaves_width(ui: &Ui, leaves: &[Leaf]) -> f32 {
    let widest = leaves
        .iter()
        .map(|leaf| leaf_text_width(ui, leaf))
        .fold(0.0, f32::max);
    (widest + 10.0).ceil().max(180.0 - 18.0)
}

/// The entry's label and hint with what comes before them.
fn leaf_text_width(ui: &Ui, leaf: &Leaf) -> f32 {
    let w = |s: &str, style| Block::single(ui, s, style, egui::Color32::WHITE).width();
    match leaf {
        Leaf::Item {
            label, hint, icon, ..
        } => {
            let label = w(label, text::UI);
            let text = hint
                .as_deref()
                .map_or(label, |h| label.max(w(h, text::META)));
            label_inset(icon.is_some()) + text
        }
        Leaf::Radio { label, .. } => label_inset(true) + w(label, text::UI),
        Leaf::Separator => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outline(entries: &[Entry]) -> Vec<String> {
        let leaf = |l: &Leaf| match l {
            Leaf::Item { label, .. } => label.clone(),
            Leaf::Radio { label, checked, .. } => {
                format!("{} {label}", if *checked { "(•)" } else { "( )" })
            }
            Leaf::Separator => "—".into(),
        };
        entries
            .iter()
            .map(|e| match e {
                Entry::Leaf(l) => leaf(l),
                Entry::Sub { label, entries, .. } => format!(
                    "{label} ▸ {}",
                    entries.iter().map(leaf).collect::<Vec<_>>().join(" | ")
                ),
            })
            .collect()
    }

    /// The same entries as the web's rowMenuEntries.test.ts.
    #[test]
    fn entries_match_the_web() {
        let now = chrono::Utc::now();
        let server = Uuid::from_u128(1);
        let channel = Target::channel(Uuid::from_u128(2), server, "#spiele".into());
        let inherited = notify::Setting {
            level: Some(NotificationLevel::Mentions),
            ..Default::default()
        };
        assert_eq!(
            outline(&notification_entries(
                &channel,
                notify::Setting::default(),
                Some(inherited),
                now
            )),
            [
                "Als gelesen markieren",
                "Stummschalten ▸ 15 Minuten | 1 Stunde | 8 Stunden | 24 Stunden | Bis ich es wieder einschalte",
                "Benachrichtigungen ▸ (•) Wie der Server (Nur @Erwähnungen) | ( ) Alle Nachrichten | ( ) Nur @Erwähnungen | ( ) Nichts",
                "Benachrichtigungseinstellungen…",
            ]
        );
        let home = Target::server(server, "Wohnzimmer");
        assert_eq!(
            outline(&notification_entries(
                &home,
                notify::Setting::default(),
                None,
                now
            ))[2],
            "Benachrichtigungen ▸ (•) Alle Nachrichten | ( ) Nur @Erwähnungen | ( ) Nichts"
        );
        let dm = Target::dm(Uuid::from_u128(3), "Jonas".into());
        assert_eq!(
            outline(&notification_entries(
                &dm,
                notify::Setting::default(),
                None,
                now
            )),
            [
                "Als gelesen markieren",
                "Stummschalten ▸ 15 Minuten | 1 Stunde | 8 Stunden | 24 Stunden | Bis ich es wieder einschalte",
                "Benachrichtigungseinstellungen…",
            ]
        );
        assert_eq!(
            outline(&notification_entries(
                &Target::all_dms(),
                notify::Setting::default(),
                None,
                now
            )),
            ["Alle als gelesen markieren"]
        );
    }

    #[test]
    fn a_running_mute_ends_in_one_step() {
        let now = chrono::Utc::now();
        let muted = notify::Setting {
            muted: true,
            ..Default::default()
        };
        let dm = Target::dm(Uuid::from_u128(3), "Jonas".into());
        let entries = notification_entries(&dm, muted, None, now);
        let Entry::Leaf(Leaf::Item {
            label, hint, act, ..
        }) = &entries[1]
        else {
            panic!("an entry expected");
        };
        assert_eq!(label, "Stummschaltung aufheben");
        assert_eq!(hint.as_deref(), Some("Stumm, bis du es aufhebst"));
        assert_eq!(*act, Act::Unmute);
    }

    /// Umschalt+F10 on a row opens its menu with focus on the first entry;
    /// the arrows walk it, → opens a submenu, Esc closes the submenu first,
    /// then the menu, and focus is back on the row.
    #[test]
    fn the_keyboard_opens_walks_and_closes_the_menu() {
        use crate::app::testkit::{
            ALLGEMEIN, FakeApi, HOME, focused_label, launcher, wait_for_label, workspace,
        };
        use crate::platform::paths::testing::TempConfig;
        use egui_kittest::kittest::Queryable;
        let _config = TempConfig::new("shell-row-menu");
        let api = FakeApi::start(true, |method, path, _| workspace(method, path));
        let mut harness = launcher(&api, &format!("/s/{HOME}/c/{ALLGEMEIN}"));
        wait_for_label(&mut harness, "allgemein");
        for _ in 0..8 {
            harness.key_press(egui::Key::Tab);
            harness.run_steps(2);
        }
        assert_eq!(focused_label(&harness).as_deref(), Some("allgemein"));
        let mut seen = Vec::new();
        let mut press = |harness: &mut egui_kittest::Harness<'_, crate::setup::Launcher>,
                         modifiers: Modifiers,
                         key: Key| {
            harness.key_press_modifiers(modifiers, key);
            harness.run_steps(3);
            seen.push(focused_label(harness).unwrap_or_default());
        };
        press(&mut harness, Modifiers::SHIFT, Key::F10);
        press(&mut harness, Modifiers::NONE, Key::ArrowDown);
        press(&mut harness, Modifiers::NONE, Key::ArrowRight);
        press(&mut harness, Modifiers::NONE, Key::ArrowDown);
        press(&mut harness, Modifiers::NONE, Key::ArrowLeft);
        press(&mut harness, Modifiers::NONE, Key::End);
        press(&mut harness, Modifiers::NONE, Key::ArrowDown);
        press(&mut harness, Modifiers::NONE, Key::ArrowDown);
        press(&mut harness, Modifiers::NONE, Key::ArrowRight);
        press(&mut harness, Modifiers::NONE, Key::Escape);
        assert!(harness.query_by_label("15 Minuten").is_none());
        press(&mut harness, Modifiers::NONE, Key::Escape);
        assert_eq!(
            seen,
            [
                "Als gelesen markieren",
                "Stummschalten",
                "15 Minuten",
                "1 Stunde",
                "Stummschalten",
                "Kanal löschen",
                "Als gelesen markieren",
                "Stummschalten",
                "15 Minuten",
                "Stummschalten",
                "allgemein",
            ]
        );
        assert!(harness.query_by_label("Kanal löschen").is_none());
    }

    /// A level keeps a timed mute's end; a server's "all" is no row.
    #[test]
    fn a_level_keeps_the_mute() {
        let now = chrono::Utc::now();
        let muted = notify::Setting {
            level: None,
            muted: true,
            until: Some(now + chrono::Duration::minutes(30)),
        };
        let body = muted.with_level(None, false, now);
        assert!(body.muted);
        assert_eq!(body.mute_minutes, Some(30));
        let body = notify::Setting::default().with_level(Some(NotificationLevel::All), true, now);
        assert_eq!(body.level, None);
    }
}

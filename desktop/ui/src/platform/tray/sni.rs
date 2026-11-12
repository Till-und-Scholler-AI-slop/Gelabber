//! Linux: a StatusNotifierItem through ksni (KDE, GNOME with the
//! AppIndicator extension, waybar, swaybar). A left click shows the window.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::StandardItem;

use super::{Event, Events, Picture, Unread, menu_labels};

struct Item {
    events: Events,
    wake: Arc<dyn Fn() + Send + Sync>,
    picture: Option<Picture>,
    unread: Unread,
    online: Arc<AtomicBool>,
    /// Open and quit, in the interface's language.
    labels: [&'static str; 2],
}

impl Item {
    fn send(&self, event: Event) {
        self.events.lock().push(event);
        (self.wake)();
    }
}

impl ksni::Tray for Item {
    fn id(&self) -> String {
        crate::platform::APP_ID.into()
    }

    fn title(&self) -> String {
        "Gelabber".into()
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::Communications
    }

    // No `icon_name`: hosts prefer a theme icon over the pixmap, so an
    // installed `gelabber` icon would hide the unread dot.
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let Some(picture) = &self.picture else {
            return Vec::new();
        };
        // RGBA to the specification's ARGB32 in network byte order.
        let data = picture
            .dotted(self.unread.dot())
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[3], p[0], p[1], p[2]])
            .collect();
        vec![ksni::Icon {
            width: picture.width as i32,
            height: picture.height as i32,
            data,
        }]
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: self.unread.tooltip(),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(Event::Show);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        let [open, quit] = self.labels;
        vec![
            StandardItem {
                label: open.into(),
                activate: Box::new(|item: &mut Self| item.send(Event::Show)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: quit.into(),
                activate: Box::new(|item: &mut Self| item.send(Event::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }

    fn watcher_online(&self) {
        self.online.store(true, Ordering::Relaxed);
    }

    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        self.online.store(false, Ordering::Relaxed);
        // Keep the item: a host that comes back shows it again.
        true
    }
}

pub struct Sni {
    handle: Handle<Item>,
    online: Arc<AtomicBool>,
}

impl Sni {
    pub fn start(events: Events, wake: Arc<dyn Fn() + Send + Sync>) -> Option<Self> {
        let online = Arc::new(AtomicBool::new(true));
        let item = Item {
            events,
            wake,
            picture: Picture::app(64),
            unread: Unread::default(),
            online: online.clone(),
            labels: menu_labels(),
        };
        match item.spawn() {
            Ok(handle) => Some(Self { handle, online }),
            Err(e) => {
                tracing::info!("no tray: {e}");
                None
            }
        }
    }

    pub fn visible(&self) -> bool {
        self.online.load(Ordering::Relaxed) && !self.handle.is_closed()
    }

    pub fn set_unread(&self, unread: Unread) {
        self.handle.update(|item| item.unread = unread);
    }

    /// The host asks for the menu and tooltip again after an update.
    pub fn relabel(&self, _unread: Unread) {
        let labels = menu_labels();
        self.handle.update(|item| item.labels = labels);
    }
}

impl Drop for Sni {
    fn drop(&mut self) {
        let _ = self.handle.shutdown();
    }
}

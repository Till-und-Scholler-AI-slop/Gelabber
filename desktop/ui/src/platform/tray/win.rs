//! Windows: the notification area icon through tray-icon (the crate the
//! Tauri app's tray uses). It lives on the UI thread, whose message loop
//! winit runs; clicks arrive through tray-icon's global event handlers.

use std::sync::Arc;

use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use super::{Event, Events, Picture, Unread};

pub struct Win {
    tray: TrayIcon,
    picture: Picture,
    // The menu items stay alive with the tray.
    _items: [MenuItem; 2],
}

impl Win {
    pub fn start(events: Events, wake: Arc<dyn Fn() + Send + Sync>) -> Option<Self> {
        let picture = Picture::app(32)?;
        let icon = Icon::from_rgba(picture.rgba.clone(), picture.width, picture.height).ok()?;
        let open = MenuItem::new("Gelabber öffnen", true, None);
        let quit = MenuItem::new("Beenden", true, None);
        let menu = Menu::with_items(&[&open, &quit]).ok()?;
        let tray = TrayIconBuilder::new()
            .with_icon(icon)
            .with_tooltip("Gelabber")
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build();
        let tray = match tray {
            Ok(tray) => tray,
            Err(e) => {
                tracing::warn!("tray icon: {e}");
                return None;
            }
        };
        let (clicks, click_wake) = (events.clone(), wake.clone());
        TrayIconEvent::set_event_handler(Some(move |event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                clicks.lock().push(Event::Show);
                click_wake();
            }
        }));
        let (open_id, quit_id) = (open.id().clone(), quit.id().clone());
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let picked = if event.id == open_id {
                Some(Event::Show)
            } else if event.id == quit_id {
                Some(Event::Quit)
            } else {
                None
            };
            if let Some(picked) = picked {
                events.lock().push(picked);
                wake();
            }
        }));
        Some(Self {
            tray,
            picture,
            _items: [open, quit],
        })
    }

    pub fn visible(&self) -> bool {
        true
    }

    pub fn set_unread(&self, unread: Unread) {
        if let Ok(icon) = Icon::from_rgba(
            self.picture.dotted(unread.dot()),
            self.picture.width,
            self.picture.height,
        ) {
            let _ = self.tray.set_icon(Some(icon));
        }
        let _ = self.tray.set_tooltip(Some(unread.tooltip()));
    }
}

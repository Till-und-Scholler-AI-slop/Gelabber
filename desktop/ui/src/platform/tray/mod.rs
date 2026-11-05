//! The tray icon with "Gelabber öffnen" and "Beenden" and a dot for unread
//! messages (desktop/app/src/shell.rs): a StatusNotifierItem on Linux
//! (`sni.rs`), the notification area on Windows (`win.rs`). Without a tray
//! the app runs as before and closing ends it.

#[cfg(target_os = "linux")]
mod sni;
#[cfg(windows)]
mod win;

use std::sync::Arc;

use parking_lot::Mutex;

/// Red of the unread dot, the web client's badge red.
pub const ALERT: [u8; 4] = [0xe5, 0x48, 0x4d, 0xff];
/// Ordinary unread messages: a light dot, like the web client's rail.
pub const UNREAD: [u8; 4] = [0xee, 0xf0, 0xee, 0xff];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Show,
    Quit,
}

/// `unreadMark` of web/src/messages/unreadIndicators.ts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Unread {
    /// The number for badges; 0 shows none.
    pub count: u32,
    /// Something is unread at all, counted or not.
    pub any: bool,
}

impl Unread {
    pub fn dot(self) -> Option<[u8; 4]> {
        if self.count > 0 {
            Some(ALERT)
        } else if self.any {
            Some(UNREAD)
        } else {
            None
        }
    }

    /// The tray's tooltip (shell.rs `shell_unread`).
    pub fn tooltip(self) -> String {
        match self.count {
            0 if self.any => "Gelabber: ungelesene Nachrichten".to_owned(),
            0 => "Gelabber".to_owned(),
            1 => "Gelabber: 1 neue Nachricht".to_owned(),
            n => format!("Gelabber: {n} neue Nachrichten"),
        }
    }
}

/// The app icon with a dot in its lower right corner and a transparent ring
/// that sets it off from the icon.
pub fn with_dot(rgba: &[u8], width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
    let mut out = rgba.to_vec();
    let size = width.min(height) as f32;
    let radius = size * 0.24;
    let ring = (size * 0.06).max(1.0);
    let cx = width as f32 - radius - ring;
    let cy = height as f32 - radius - ring;
    for y in 0..height {
        for x in 0..width {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let distance = (dx * dx + dy * dy).sqrt();
            let at = ((y * width + x) * 4) as usize;
            if distance <= radius {
                out[at..at + 4].copy_from_slice(&color);
            } else if distance <= radius + ring {
                out[at + 3] = 0;
            }
        }
    }
    out
}

/// The tray icon's picture: the app icon, small, RGBA.
pub(crate) struct Picture {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

impl Picture {
    pub fn app(size: u32) -> Option<Self> {
        let image = image::load_from_memory(include_bytes!("../../../assets/images/icon-192.png"))
            .ok()?
            .resize_exact(size, size, image::imageops::FilterType::Lanczos3)
            .to_rgba8();
        Some(Self {
            width: image.width(),
            height: image.height(),
            rgba: image.into_raw(),
        })
    }

    pub fn dotted(&self, dot: Option<[u8; 4]>) -> Vec<u8> {
        match dot {
            Some(color) => with_dot(&self.rgba, self.width, self.height, color),
            None => self.rgba.clone(),
        }
    }
}

pub type Events = Arc<Mutex<Vec<Event>>>;

/// A running tray icon.
pub struct Tray {
    #[cfg(target_os = "linux")]
    inner: sni::Sni,
    #[cfg(windows)]
    inner: win::Win,
    shown: Unread,
}

impl Tray {
    /// The tray, or `None` where the desktop has none. `wake` repaints the
    /// window so the app sees a click.
    pub fn start(events: Events, wake: Arc<dyn Fn() + Send + Sync>) -> Option<Self> {
        #[cfg(target_os = "linux")]
        let inner = sni::Sni::start(events, wake)?;
        #[cfg(windows)]
        let inner = win::Win::start(events, wake)?;
        #[cfg(not(any(windows, target_os = "linux")))]
        {
            let _ = (events, wake);
            return None;
        }
        #[cfg(any(windows, target_os = "linux"))]
        Some(Self {
            inner,
            shown: Unread::default(),
        })
    }

    /// A host shows it right now (Linux: a StatusNotifierWatcher runs).
    pub fn visible(&self) -> bool {
        #[cfg(any(windows, target_os = "linux"))]
        return self.inner.visible();
        #[cfg(not(any(windows, target_os = "linux")))]
        false
    }

    pub fn set_unread(&mut self, unread: Unread) {
        if unread != self.shown {
            self.shown = unread;
            #[cfg(any(windows, target_os = "linux"))]
            self.inner.set_unread(unread);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dot_sits_in_the_lower_right_corner_with_a_clear_ring() {
        let (w, h) = (20, 20);
        let base = vec![10; (w * h * 4) as usize];
        let out = with_dot(&base, w, h, ALERT);
        let at = |x: u32, y: u32| &out[((y * w + x) * 4) as usize..((y * w + x) * 4 + 4) as usize];
        assert_eq!(at(14, 14), ALERT);
        assert_eq!(at(1, 1), [10, 10, 10, 10]);
        assert!((0..w).any(|x| at(x, 14)[3] == 0));
    }

    #[test]
    fn counted_messages_are_red_others_light() {
        let two = Unread {
            count: 2,
            any: true,
        };
        assert_eq!(two.dot(), Some(ALERT));
        assert_eq!(two.tooltip(), "Gelabber: 2 neue Nachrichten");
        let quiet = Unread {
            count: 0,
            any: true,
        };
        assert_eq!(quiet.dot(), Some(UNREAD));
        assert_eq!(quiet.tooltip(), "Gelabber: ungelesene Nachrichten");
        assert_eq!(Unread::default().dot(), None);
        assert_eq!(
            Unread {
                count: 1,
                any: true
            }
            .tooltip(),
            "Gelabber: 1 neue Nachricht"
        );
        let picture = Picture::app(32).unwrap();
        assert_eq!(picture.rgba.len(), 32 * 32 * 4);
        assert_ne!(picture.dotted(Some(ALERT)), picture.rgba);
    }
}

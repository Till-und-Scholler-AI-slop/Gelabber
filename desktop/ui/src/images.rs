//! Images from the server (avatars, banners, attachments, the Living Room
//! artwork): fetched through the client's session, decoded on a worker
//! thread and uploaded as textures. The content behind a URL never
//! changes, so entries stay for the session. Animated GIFs and WebPs keep
//! every frame with its delay; text attachments are fetched here too.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, mpsc};
use std::time::Duration;

use egui::{ColorImage, TextureHandle, TextureOptions};
use gelabber_client::Client;
use parking_lot::Mutex;

/// What [`Images::get`] knows about a URL right now.
#[derive(Clone)]
pub enum ImageState {
    Loading,
    Ready(TextureHandle),
    /// Not found, not an image, or no connection: show the fallback
    /// (initials, a plain tile), never a broken-image icon.
    Failed,
}

impl ImageState {
    pub fn texture(&self) -> Option<&TextureHandle> {
        match self {
            Self::Ready(texture) => Some(texture),
            _ => None,
        }
    }
}

/// A text attachment's preview (`AttachmentText`).
#[derive(Clone, Debug, PartialEq)]
pub enum TextState {
    Loading,
    Ready(Arc<str>),
    Failed,
}

/// The frames of an animated picture and how long each stays, in seconds.
pub struct Animation {
    pub frames: Vec<(TextureHandle, f64)>,
}

impl Animation {
    /// The frame shown `time` seconds in, looping, and how long it still
    /// stays.
    pub fn at(&self, time: f64) -> (&TextureHandle, f64) {
        let total: f64 = self.frames.iter().map(|(_, d)| d).sum();
        let mut t = if total > 0.0 { time % total } else { 0.0 };
        for (texture, delay) in &self.frames {
            if t < *delay {
                return (texture, delay - t);
            }
            t -= delay;
        }
        let (texture, delay) = &self.frames[0];
        (texture, *delay)
    }
}

#[derive(Clone)]
pub struct Images {
    client: Client,
    entries: Arc<Mutex<HashMap<String, ImageState>>>,
    animations: Arc<Mutex<HashMap<String, Arc<Animation>>>>,
    texts: Arc<Mutex<HashMap<String, TextState>>>,
    loading: Arc<AtomicUsize>,
}

/// Linear filtering with mipmaps: photos are shown smaller than they are,
/// and the browser scales them smoothly too.
pub const PHOTO: TextureOptions = TextureOptions {
    magnification: egui::TextureFilter::Linear,
    minification: egui::TextureFilter::Linear,
    wrap_mode: egui::TextureWrapMode::ClampToEdge,
    mipmap_mode: Some(egui::TextureFilter::Linear),
};

impl Images {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            entries: Arc::default(),
            animations: Arc::default(),
            texts: Arc::default(),
            loading: Arc::default(),
        }
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    /// The image behind `url` (a server path like `/api/avatars/…` or an
    /// absolute URL); starts loading it the first time. An animation is
    /// its first frame here; [`Self::frame`] plays it.
    pub fn get(&self, ctx: &egui::Context, url: &str) -> ImageState {
        if let Some(state) = self.entries.lock().get(url) {
            return state.clone();
        }
        self.entries
            .lock()
            .insert(url.to_owned(), ImageState::Loading);
        self.loading.fetch_add(1, Ordering::AcqRel);
        let this = self.clone();
        let ctx = ctx.clone();
        let url = url.to_owned();
        let http = self.client.http().clone();
        self.client.spawn(async move {
            let bytes = http.bytes(&url).await;
            // Decoding a large PNG takes tens of milliseconds: not on the
            // UI thread and not on the two runtime workers.
            decode_off_thread(Box::new(move || {
                let frames = bytes.ok().map(|b| decode_frames(&b)).unwrap_or_default();
                let state = match frames.len() {
                    0 => ImageState::Failed,
                    1 => {
                        let (image, _) = frames.into_iter().next().expect("one frame");
                        ImageState::Ready(ctx.load_texture(&url, image, PHOTO))
                    }
                    _ => {
                        let frames: Vec<(TextureHandle, f64)> = frames
                            .into_iter()
                            .enumerate()
                            .map(|(i, (image, delay))| {
                                let name = format!("{url}#{i}");
                                (ctx.load_texture(name, image, PHOTO), delay)
                            })
                            .collect();
                        let first = frames[0].0.clone();
                        this.animations
                            .lock()
                            .insert(url.clone(), Arc::new(Animation { frames }));
                        ImageState::Ready(first)
                    }
                };
                this.entries.lock().insert(url, state);
                this.loading.fetch_sub(1, Ordering::AcqRel);
                ctx.request_repaint();
            }));
        });
        ImageState::Loading
    }

    /// The picture as it is now: an animation's current frame while
    /// `play` (the browser animates; reduced motion stops it), else
    /// [`Self::get`]. Repaints when the next frame is due.
    pub fn frame(&self, ctx: &egui::Context, url: &str, play: bool) -> ImageState {
        let state = self.get(ctx, url);
        if !play {
            return state;
        }
        let Some(animation) = self.animations.lock().get(url).cloned() else {
            return state;
        };
        let (texture, left) = animation.at(ctx.input(|i| i.time));
        ctx.request_repaint_after(Duration::from_secs_f64(left.max(0.01)));
        ImageState::Ready(texture.clone())
    }

    /// Asks for a picture that failed again ("Erneut laden").
    pub fn retry(&self, url: &str) {
        let mut entries = self.entries.lock();
        if matches!(entries.get(url), Some(ImageState::Failed)) {
            entries.remove(url);
        }
    }

    /// A text attachment as text (`decodeText`); starts fetching it the
    /// first time.
    pub fn text(&self, ctx: &egui::Context, url: &str) -> TextState {
        if let Some(state) = self.texts.lock().get(url) {
            return state.clone();
        }
        self.texts.lock().insert(url.to_owned(), TextState::Loading);
        self.loading.fetch_add(1, Ordering::AcqRel);
        let this = self.clone();
        let ctx = ctx.clone();
        let url = url.to_owned();
        let http = self.client.http().clone();
        self.client.spawn(async move {
            let state = match http.bytes(&url).await {
                Ok(bytes) => TextState::Ready(decode_text(&bytes).into()),
                Err(_) => TextState::Failed,
            };
            this.texts.lock().insert(url, state);
            this.loading.fetch_sub(1, Ordering::AcqRel);
            ctx.request_repaint();
        });
        TextState::Loading
    }

    /// True while any image is still being fetched or decoded.
    pub fn busy(&self) -> bool {
        self.loading.load(Ordering::Acquire) > 0
    }
}

/// `decodeText`: UTF-16 by its byte order mark, then UTF-8, anything else
/// as Windows-1252 (Excel CSV on German systems; Latin-1 logs read right).
pub fn decode_text(bytes: &[u8]) -> String {
    let utf16 = |be: bool| {
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| {
                if be {
                    u16::from_be_bytes(*c)
                } else {
                    u16::from_le_bytes(*c)
                }
            })
            .collect();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        [0xff, 0xfe, ..] => return utf16(false),
        [0xfe, 0xff, ..] => return utf16(true),
        _ => {}
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        // TextDecoder drops a UTF-8 byte order mark.
        return text.strip_prefix('\u{feff}').unwrap_or(text).to_owned();
    }
    bytes.iter().map(|b| windows_1252(*b)).collect()
}

fn windows_1252(b: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž',
        '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}',
        'ž', 'Ÿ',
    ];
    match b {
        0x80..=0x9f => HIGH[(b - 0x80) as usize],
        _ => b as char,
    }
}

type Job = Box<dyn FnOnce() + Send>;

/// Worker threads shared by all decodes: a member list or a chat full of
/// avatars must not start one OS thread per image.
const DECODERS: usize = 2;

fn decode_off_thread(job: Job) {
    static QUEUE: OnceLock<Option<mpsc::Sender<Job>>> = OnceLock::new();
    let queue = QUEUE.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        let started = (0..DECODERS)
            .filter(|_| {
                let rx = rx.clone();
                std::thread::Builder::new()
                    .name("gelabber-image".into())
                    .spawn(move || {
                        loop {
                            // The lock is held only to receive, not while decoding.
                            let next = rx.lock().recv();
                            match next {
                                Ok(job) => job(),
                                Err(_) => break,
                            }
                        }
                    })
                    .is_ok()
            })
            .count();
        (started > 0).then_some(tx)
    });
    // Without a worker the job runs here: slower, but the entry still settles.
    match queue {
        Some(tx) => {
            if let Err(mpsc::SendError(job)) = tx.send(job) {
                job();
            }
        }
        None => job(),
    }
}

/// PNG, JPEG, WebP or GIF (first frame) as an egui image.
pub fn decode(bytes: &[u8]) -> Option<ColorImage> {
    let image = image::load_from_memory(bytes).ok()?.to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    Some(ColorImage::from_rgba_unmultiplied(size, image.as_raw()))
}

/// Browsers show a frame of 0 or 10ms or less for 100ms.
fn frame_delay(delay: image::Delay) -> f64 {
    let (numer, denom) = delay.numer_denom_ms();
    let ms = f64::from(numer) / f64::from(denom.max(1));
    if ms <= 10.0 { 0.1 } else { ms / 1000.0 }
}

/// Every frame of an animated GIF or WebP with its delay in seconds; one
/// frame (delay 0) for still pictures, none when it is no picture.
pub fn decode_frames(bytes: &[u8]) -> Vec<(ColorImage, f64)> {
    use image::AnimationDecoder;
    let to_color = |buffer: &image::RgbaImage| {
        let size = [buffer.width() as usize, buffer.height() as usize];
        ColorImage::from_rgba_unmultiplied(size, buffer.as_raw())
    };
    let frames: Option<Vec<image::Frame>> = match image::guess_format(bytes) {
        Ok(image::ImageFormat::Gif) => {
            image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes))
                .ok()
                .and_then(|d| d.into_frames().collect_frames().ok())
        }
        Ok(image::ImageFormat::WebP) => {
            image::codecs::webp::WebPDecoder::new(std::io::Cursor::new(bytes))
                .ok()
                .filter(|d| d.has_animation())
                .and_then(|d| d.into_frames().collect_frames().ok())
        }
        _ => None,
    };
    match frames {
        Some(frames) if frames.len() > 1 => frames
            .iter()
            .map(|f| (to_color(f.buffer()), frame_delay(f.delay())))
            .collect(),
        _ => decode(bytes).map(|i| vec![(i, 0.0)]).unwrap_or_default(),
    }
}

/// A bundled image (assets/images), decoded once per context.
pub fn bundled(
    ctx: &egui::Context,
    name: &'static str,
    bytes: &'static [u8],
) -> Option<TextureHandle> {
    let id = egui::Id::unique(("gelabber-bundled", name));
    if let Some(texture) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return Some(texture);
    }
    let texture = ctx.load_texture(name, decode(bytes)?, PHOTO);
    ctx.data_mut(|d| d.insert_temp(id, texture.clone()));
    Some(texture)
}

/// The UV rect that makes an image of `image` size cover `target` like
/// CSS `background-size: cover; background-position: center`.
pub fn cover_uv(image: egui::Vec2, target: egui::Vec2) -> egui::Rect {
    let scale = (target.x / image.x).max(target.y / image.y);
    let shown = target / scale / image;
    let min = (egui::Vec2::splat(1.0) - shown) / 2.0;
    egui::Rect::from_min_size(min.to_pos2(), shown)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_frame_gif() -> Vec<u8> {
        use image::{Delay, Frame, Rgba, RgbaImage};
        let mut out = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut out);
            for (color, ms) in [([255, 0, 0, 255], 120), ([0, 0, 255, 255], 300)] {
                let buffer = RgbaImage::from_pixel(4, 3, Rgba(color));
                let frame = Frame::from_parts(buffer, 0, 0, Delay::from_numer_denom_ms(ms, 1));
                encoder.encode_frame(frame).expect("frame");
            }
        }
        out
    }

    #[test]
    fn a_two_frame_gif_keeps_both_frames_and_delays() {
        let frames = decode_frames(&two_frame_gif());
        assert_eq!(frames.len(), 2);
        assert!((frames[0].1 - 0.12).abs() < 1e-9);
        assert!((frames[1].1 - 0.3).abs() < 1e-9);
        assert_eq!(frames[0].0.size, [4, 3]);
        assert_eq!(frames[0].0.pixels[0], egui::Color32::from_rgb(255, 0, 0));
        assert_eq!(frames[1].0.pixels[0], egui::Color32::from_rgb(0, 0, 255));
        // Two textures, played by their delays and looping.
        let ctx = egui::Context::default();
        let animation = Animation {
            frames: frames
                .into_iter()
                .enumerate()
                .map(|(i, (image, d))| (ctx.load_texture(format!("f{i}"), image, PHOTO), d))
                .collect(),
        };
        assert_eq!(animation.at(0.05).0.name(), "f0");
        assert_eq!(animation.at(0.2).0.name(), "f1");
        let (texture, left) = animation.at(0.42 + 0.05);
        assert_eq!(texture.name(), "f0");
        assert!((left - 0.07).abs() < 1e-6);
    }

    #[test]
    fn a_still_picture_is_one_frame() {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .expect("png");
        assert_eq!(decode_frames(&png).len(), 1);
        assert!(decode_frames(b"kein Bild").is_empty());
    }

    #[test]
    fn text_decodes_like_the_web() {
        assert_eq!(decode_text("Grüße".as_bytes()), "Grüße");
        assert_eq!(decode_text(&[0xef, 0xbb, 0xbf, b'a']), "a");
        assert_eq!(decode_text(&[0xff, 0xfe, b'h', 0, b'i', 0]), "hi");
        // Windows-1252: "Größe €".
        assert_eq!(
            decode_text(&[0x47, 0x72, 0xf6, 0xdf, 0x65, 0x20, 0x80]),
            "Größe €"
        );
    }
}

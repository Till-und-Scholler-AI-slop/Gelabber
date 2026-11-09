//! Images from the server (avatars, banners, attachments, the Living Room
//! artwork): fetched through the client's session, decoded on a worker
//! thread, scaled down there to what the device takes and the place shows,
//! and uploaded as textures. Decoded pictures share a byte budget: what was
//! not drawn lately makes room, least recently drawn first, and is fetched
//! again when it is drawn again. A picture that failed is asked for again
//! after a while. Animated GIFs and WebPs keep every frame with its delay,
//! up to a cap; text attachments are fetched here too.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, mpsc};
use std::time::{Duration, Instant};

use egui::{ColorImage, TextureHandle, TextureOptions, Vec2, ViewportId};
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

/// How large a picture is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Fit {
    /// Up to what covers the monitor (the lightbox, artwork, avatars).
    Full,
    /// Inside the chat: up to what covers [`INLINE`] at the screen's scale.
    Inline,
}

/// The largest box the chat shows a picture in: a link preview's image is
/// up to 432 wide, a picture up to 240 high.
pub const INLINE: [f32; 2] = [432.0, 240.0];

/// Decoded pictures and frames kept at once, in bytes of RGBA.
const BUDGET: usize = 256 * 1024 * 1024;
/// What one animation may hold; a larger one shows its first frame.
const ANIMATION_MAX: usize = 64 * 1024 * 1024;
/// Full-size frames one animation may make the decoder produce before it
/// shows its first frame instead: a tiny file with a huge canvas and many
/// frames must not keep a worker busy for minutes.
const ANIMATION_WORK: usize = 1024 * 1024 * 1024;
/// Larger pictures are not decoded at all (a 48 MP photo still is).
const MAX_PIXELS: u64 = 50_000_000;
/// The first wait before a failed picture is asked for again; it doubles
/// with each failure up to [`RETRY_MAX`].
const RETRY_FIRST: Duration = Duration::from_secs(30);
const RETRY_MAX: Duration = Duration::from_secs(15 * 60);

enum Kind {
    Loading,
    Ready {
        texture: TextureHandle,
        animation: Option<Arc<Animation>>,
        bytes: usize,
    },
    /// Dropped for the budget; fetched again when it is drawn.
    Evicted,
    Failed {
        at: Instant,
        failures: u32,
    },
}

struct Entry {
    kind: Kind,
    /// Where and in which pass it was last drawn, and when.
    used: (ViewportId, u64, Instant),
    /// The texture's size, kept after eviction so a box does not jump.
    size: Option<Vec2>,
}

struct Cache {
    entries: HashMap<(String, Fit), Entry>,
    bytes: usize,
    budget: usize,
    /// The pass that last evicted.
    evicted: (ViewportId, u64),
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            bytes: 0,
            budget: BUDGET,
            evicted: (ViewportId::ROOT, u64::MAX),
        }
    }
}

#[derive(Clone)]
pub struct Images {
    client: Client,
    cache: Arc<Mutex<Cache>>,
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

fn used_now(ctx: &egui::Context) -> (ViewportId, u64, Instant) {
    let viewport = ctx.viewport_id();
    (
        viewport,
        ctx.cumulative_pass_nr_for(viewport),
        Instant::now(),
    )
}

fn retry_after(failures: u32) -> Duration {
    RETRY_FIRST
        .saturating_mul(1 << failures.saturating_sub(1).min(10))
        .min(RETRY_MAX)
}

impl Images {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            cache: Arc::default(),
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
        self.shown(ctx, url, Fit::Full)
    }

    /// [`Self::get`] for a picture drawn at `fit`.
    pub fn shown(&self, ctx: &egui::Context, url: &str, fit: Fit) -> ImageState {
        self.look(ctx, url, fit, true).0
    }

    /// The size of the picture at `fit` for laying out its box, without
    /// counting as drawn; starts loading it the first time.
    pub fn natural(&self, ctx: &egui::Context, url: &str, fit: Fit) -> Option<Vec2> {
        self.look(ctx, url, fit, false).1
    }

    /// Whether the picture at `fit` failed, without counting as drawn.
    pub fn failed(&self, ctx: &egui::Context, url: &str, fit: Fit) -> bool {
        matches!(self.look(ctx, url, fit, false).0, ImageState::Failed)
    }

    /// The state and size of an entry; `drawn` marks it used and brings
    /// an evicted one back.
    fn look(
        &self,
        ctx: &egui::Context,
        url: &str,
        fit: Fit,
        drawn: bool,
    ) -> (ImageState, Option<Vec2>) {
        let key = (url.to_owned(), fit);
        // Evicted textures are freed after the lock is released.
        let mut _evicted = Vec::new();
        let failures = {
            let mut cache = self.cache.lock();
            // Over budget, what left the screen goes, once per pass.
            let pass = used_now(ctx);
            if cache.bytes > cache.budget && cache.evicted != (pass.0, pass.1) {
                cache.evicted = (pass.0, pass.1);
                _evicted = evict(ctx, &mut cache, &key);
            }
            let entry = cache.entries.entry(key).or_insert_with(|| Entry {
                kind: Kind::Evicted,
                used: used_now(ctx),
                size: None,
            });
            if drawn {
                entry.used = used_now(ctx);
            }
            let failures = match &entry.kind {
                Kind::Ready { texture, .. } => {
                    return (
                        ImageState::Ready(texture.clone()),
                        Some(texture.size_vec2()),
                    );
                }
                Kind::Loading => return (ImageState::Loading, entry.size),
                Kind::Failed { at, failures } => {
                    if at.elapsed() < retry_after(*failures) {
                        return (ImageState::Failed, entry.size);
                    }
                    *failures
                }
                // Never loaded (size unknown) or evicted: only a draw
                // brings an evicted picture back, a layout keeps its size.
                Kind::Evicted if entry.size.is_some() && !drawn => {
                    return (ImageState::Loading, entry.size);
                }
                Kind::Evicted => 0,
            };
            entry.kind = Kind::Loading;
            failures
        };
        self.load(ctx, url, fit, failures);
        (
            ImageState::Loading,
            self.cache
                .lock()
                .entries
                .get(&(url.to_owned(), fit))
                .and_then(|e| e.size),
        )
    }

    /// Fetches and decodes an entry [`Self::look`] set to loading.
    fn load(&self, ctx: &egui::Context, url: &str, fit: Fit, failures: u32) {
        self.loading.fetch_add(1, Ordering::AcqRel);
        let scale = ctx.pixels_per_point().max(1.0);
        let cover = match fit {
            Fit::Full => ctx.input(|i| i.viewport().monitor_size).map(|m| m * scale),
            Fit::Inline => Some(Vec2::from(INLINE) * scale),
        };
        let limit = Limit {
            // What the GPU device takes, as eframe reports it.
            max_side: ctx.input(|i| i.max_texture_side),
            cover: cover.map(|c| [c.x.ceil() as u32, c.y.ceil() as u32]),
        };
        let this = self.clone();
        let ctx = ctx.clone();
        let url = url.to_owned();
        let http = self.client.http().clone();
        self.client.spawn(async move {
            let bytes = http.bytes(&url).await;
            // Decoding a large PNG takes tens of milliseconds: not on the
            // UI thread and not on the two runtime workers.
            decode_off_thread(Box::new(move || {
                let frames = bytes
                    .ok()
                    // A decoder that panics on a crafted file fails the picture, not the worker.
                    .and_then(|b| std::panic::catch_unwind(|| decode_fitted(&b, &limit)).ok())
                    .unwrap_or_default();
                let bytes: usize = frames.iter().map(|(i, _)| i.pixels.len() * 4).sum();
                let name = match fit {
                    Fit::Full => url.clone(),
                    Fit::Inline => format!("{url}@inline"),
                };
                let kind = match frames.len() {
                    0 => Kind::Failed {
                        at: Instant::now(),
                        failures: failures + 1,
                    },
                    1 => {
                        let (image, _) = frames.into_iter().next().expect("one frame");
                        Kind::Ready {
                            texture: ctx.load_texture(name, image, PHOTO),
                            animation: None,
                            bytes,
                        }
                    }
                    _ => {
                        let frames: Vec<(TextureHandle, f64)> = frames
                            .into_iter()
                            .enumerate()
                            .map(|(i, (image, delay))| {
                                (ctx.load_texture(format!("{name}#{i}"), image, PHOTO), delay)
                            })
                            .collect();
                        Kind::Ready {
                            texture: frames[0].0.clone(),
                            animation: Some(Arc::new(Animation { frames })),
                            bytes,
                        }
                    }
                };
                let dropped = this.settle(&ctx, (url, fit), kind);
                drop(dropped);
                this.loading.fetch_sub(1, Ordering::AcqRel);
                ctx.request_repaint();
            }));
        });
    }

    /// Stores a finished load and makes room for it; returns what was
    /// evicted, to be dropped outside the lock.
    fn settle(&self, ctx: &egui::Context, key: (String, Fit), kind: Kind) -> Vec<Kind> {
        let mut cache = self.cache.lock();
        let added = match &kind {
            Kind::Ready { bytes, .. } => *bytes,
            _ => 0,
        };
        let size = match &kind {
            Kind::Ready { texture, .. } => Some(texture.size_vec2()),
            _ => None,
        };
        let entry = cache.entries.entry(key.clone()).or_insert_with(|| Entry {
            kind: Kind::Evicted,
            used: used_now(ctx),
            size: None,
        });
        let old = std::mem::replace(&mut entry.kind, kind);
        entry.size = size.or(entry.size);
        let freed = match &old {
            Kind::Ready { bytes, .. } => *bytes,
            _ => 0,
        };
        cache.bytes = cache.bytes - freed + added;
        let mut dropped = vec![old];
        dropped.extend(evict(ctx, &mut cache, &key));
        dropped
    }

    /// The picture as it is now: an animation's current frame while
    /// `play` (the browser animates; reduced motion stops it), else
    /// [`Self::shown`]. Repaints when the next frame is due.
    pub fn frame(&self, ctx: &egui::Context, url: &str, play: bool, fit: Fit) -> ImageState {
        let state = self.shown(ctx, url, fit);
        if !play {
            return state;
        }
        let animation = match self.cache.lock().entries.get(&(url.to_owned(), fit)) {
            Some(Entry {
                kind:
                    Kind::Ready {
                        animation: Some(animation),
                        ..
                    },
                ..
            }) => animation.clone(),
            _ => return state,
        };
        let (texture, left) = animation.at(ctx.input(|i| i.time));
        ctx.request_repaint_after(Duration::from_secs_f64(left.max(0.01)));
        ImageState::Ready(texture.clone())
    }

    /// Asks for a picture that failed again ("Erneut laden").
    pub fn retry(&self, url: &str) {
        let mut cache = self.cache.lock();
        for fit in [Fit::Full, Fit::Inline] {
            let key = (url.to_owned(), fit);
            if matches!(
                cache.entries.get(&key),
                Some(Entry {
                    kind: Kind::Failed { .. },
                    ..
                })
            ) {
                cache.entries.remove(&key);
            }
        }
    }

    /// Bytes of decoded pictures held now.
    #[cfg(test)]
    fn held(&self) -> usize {
        self.cache.lock().bytes
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

/// Drops pictures not drawn in their viewport's last pass, least recently
/// drawn first, until the cache fits [`BUDGET`]. What is on screen stays,
/// even over budget, so nothing flickers; `keep` just arrived.
fn evict(ctx: &egui::Context, cache: &mut Cache, keep: &(String, Fit)) -> Vec<Kind> {
    if cache.bytes <= cache.budget {
        return Vec::new();
    }
    let mut idle: Vec<((String, Fit), Instant)> = cache
        .entries
        .iter()
        .filter(|(key, entry)| {
            let (viewport, pass, at) = entry.used;
            *key != keep
                && matches!(entry.kind, Kind::Ready { .. })
                && (ctx.cumulative_pass_nr_for(viewport) > pass + 1
                    || at.elapsed() > Duration::from_secs(60))
        })
        .map(|(key, entry)| (key.clone(), entry.used.2))
        .collect();
    idle.sort_by_key(|(_, at)| *at);
    let mut dropped = Vec::new();
    for (key, _) in idle {
        if cache.bytes <= cache.budget {
            break;
        }
        if let Some(entry) = cache.entries.get_mut(&key) {
            let old = std::mem::replace(&mut entry.kind, Kind::Evicted);
            if let Kind::Ready { bytes, .. } = &old {
                cache.bytes -= bytes;
            }
            dropped.push(old);
        }
    }
    dropped
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

/// PNG, JPEG, WebP or GIF (first frame) as an egui image, at full size:
/// for the app's own bundled pictures.
pub fn decode(bytes: &[u8]) -> Option<ColorImage> {
    let image = image::load_from_memory(bytes).ok()?.to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    Some(ColorImage::from_rgba_unmultiplied(size, image.as_raw()))
}

/// How large a decoded picture may be.
#[derive(Clone, Copy, Debug)]
pub struct Limit {
    /// The device's largest texture side: no side gets longer.
    pub max_side: usize,
    /// A box in pixels: the picture shrinks until it just covers it.
    pub cover: Option<[u32; 2]>,
}

/// The size a `width`×`height` picture is kept at: never larger than it
/// is, covering `limit.cover`, both sides within `limit.max_side`.
pub fn fitted(width: u32, height: u32, limit: &Limit) -> [u32; 2] {
    let (w, h) = (f64::from(width.max(1)), f64::from(height.max(1)));
    let side = limit.max_side.clamp(1, u32::MAX as usize) as f64;
    let mut scale = (side / w).min(side / h).min(1.0);
    if let Some([bw, bh]) = limit.cover {
        scale = scale.min((f64::from(bw) / w).max(f64::from(bh) / h));
    }
    if scale >= 1.0 {
        return [width, height];
    }
    let fit = |n: f64| ((n * scale).round() as u32).clamp(1, side as u32);
    [fit(w), fit(h)]
}

/// The decoders' own limits: dimensions above [`MAX_PIXELS`] and buffers
/// beyond it fail instead of allocating.
fn limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(1 << 16);
    limits.max_image_height = Some(1 << 16);
    limits.max_alloc = Some(MAX_PIXELS * 4 + 16 * 1024 * 1024);
    limits
}

fn to_color(image: &image::RgbaImage) -> ColorImage {
    let size = [image.width() as usize, image.height() as usize];
    ColorImage::from_rgba_unmultiplied(size, image.as_raw())
}

/// One picture of `bytes`, scaled to `limit`.
fn decode_still(bytes: &[u8], limit: &Limit) -> Option<ColorImage> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    reader.limits(limits());
    let image = reader.decode().ok()?;
    let [w, h] = fitted(image.width(), image.height(), limit);
    let image = if [w, h] == [image.width(), image.height()] {
        image
    } else {
        image.resize_exact(w, h, image::imageops::FilterType::Triangle)
    };
    Some(to_color(&image.to_rgba8()))
}

/// Browsers show a frame of 0 or 10ms or less for 100ms.
fn frame_delay(delay: image::Delay) -> f64 {
    let (numer, denom) = delay.numer_denom_ms();
    let ms = f64::from(numer) / f64::from(denom.max(1));
    if ms <= 10.0 { 0.1 } else { ms / 1000.0 }
}

/// The frames of an animation, each scaled as it is decoded; `None` when
/// it is no animation, broken, holds more than [`ANIMATION_MAX`], or has
/// the decoder produce more than `work` bytes of full-size frames.
fn decode_animation(
    frames: image::Frames<'_>,
    width: u32,
    height: u32,
    limit: &Limit,
    work_max: usize,
) -> Option<Vec<(ColorImage, f64)>> {
    let [w, h] = fitted(width, height, limit);
    let mut out = Vec::new();
    let mut held = 0;
    let mut work = 0;
    for frame in frames {
        let frame = frame.ok()?;
        let delay = frame_delay(frame.delay());
        let buffer = frame.into_buffer();
        work += buffer.as_raw().len();
        if work > work_max {
            return None;
        }
        let buffer = if [w, h] == [buffer.width(), buffer.height()] {
            buffer
        } else {
            image::imageops::resize(&buffer, w, h, image::imageops::FilterType::Triangle)
        };
        held += buffer.as_raw().len();
        if held > ANIMATION_MAX {
            return None;
        }
        out.push((to_color(&buffer), delay));
    }
    (out.len() > 1).then_some(out)
}

/// Every frame of an animated GIF or WebP with its delay in seconds; one
/// frame (delay 0) for still pictures, none when it is no picture or too
/// large to decode. Every frame is scaled to `limit`, and the dimensions
/// are read before anything is decoded.
pub fn decode_fitted(bytes: &[u8], limit: &Limit) -> Vec<(ColorImage, f64)> {
    use image::{AnimationDecoder, ImageDecoder};
    let dimensions = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()
        .and_then(|r| r.into_dimensions().ok());
    let Some((width, height)) = dimensions else {
        return Vec::new();
    };
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Vec::new();
    }
    let cursor = || std::io::Cursor::new(bytes);
    let frames = match image::guess_format(bytes) {
        Ok(image::ImageFormat::Gif) => image::codecs::gif::GifDecoder::new(cursor())
            .ok()
            .and_then(|mut d| d.set_limits(limits()).ok().map(|()| d))
            .and_then(|d| decode_animation(d.into_frames(), width, height, limit, ANIMATION_WORK)),
        Ok(image::ImageFormat::WebP) => image::codecs::webp::WebPDecoder::new(cursor())
            .ok()
            .filter(|d| d.has_animation())
            .and_then(|mut d| d.set_limits(limits()).ok().map(|()| d))
            .and_then(|d| decode_animation(d.into_frames(), width, height, limit, ANIMATION_WORK)),
        _ => None,
    };
    match frames {
        Some(frames) => frames,
        None => decode_still(bytes, limit)
            .map(|i| vec![(i, 0.0)])
            .unwrap_or_default(),
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
pub(crate) mod tests {
    use super::*;

    const ANY: Limit = Limit {
        max_side: 16384,
        cover: None,
    };

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
        let frames = decode_fitted(&two_frame_gif(), &ANY);
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
        assert_eq!(decode_fitted(&png, &ANY).len(), 1);
        assert!(decode_fitted(b"kein Bild", &ANY).is_empty());
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(width, height, image::Rgba([9, 8, 7, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .expect("png");
        png
    }

    /// A server that answers `GET <path>` with the bytes for it, else 404.
    pub(crate) fn serve(files: Vec<(&'static str, Vec<u8>)>) -> String {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let origin = format!("http://{}", listener.local_addr().expect("addr"));
        let files = Arc::new(files);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let files = files.clone();
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                    let mut out = stream;
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 {
                            return;
                        }
                        let path = line.split_whitespace().nth(1).unwrap_or("").to_owned();
                        loop {
                            let mut header = String::new();
                            if reader.read_line(&mut header).unwrap_or(0) == 0 {
                                return;
                            }
                            if header.trim().is_empty() {
                                break;
                            }
                        }
                        let body = files.iter().find(|(p, _)| *p == path).map(|(_, b)| b);
                        let (status, body) = match body {
                            Some(b) => (200, b.as_slice()),
                            None => (404, &b"{}"[..]),
                        };
                        let head = format!(
                            "HTTP/1.1 {status} X\r\ncontent-length: {}\r\n\r\n",
                            body.len()
                        );
                        if out.write_all(head.as_bytes()).is_err() || out.write_all(body).is_err() {
                            return;
                        }
                    }
                });
            }
        });
        origin
    }

    /// Runs passes of `ctx` as a device with `max_side` textures until
    /// `done` holds for what `draw` returns.
    fn run_until<T>(
        ctx: &egui::Context,
        max_side: usize,
        mut draw: impl FnMut(&egui::Context) -> T,
        done: impl Fn(&T) -> bool,
    ) -> T {
        for _ in 0..500 {
            let mut out = None;
            let input = egui::RawInput {
                max_texture_side: Some(max_side),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| out = Some(draw(ui.ctx())));
            // No painter takes the uploads here.
            output.textures_delta.clear();
            let out = out.expect("drawn");
            if done(&out) {
                return out;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("did not settle");
    }

    #[test]
    fn a_picture_wider_than_the_device_takes_is_scaled_down() {
        let origin = serve(vec![("/wide.png", png(9000, 10))]);
        let images = Images::new(Client::new(&origin).expect("client"));
        let ctx = egui::Context::default();
        let full = run_until(
            &ctx,
            8192,
            |ctx| images.get(ctx, "/wide.png"),
            |s| !matches!(s, ImageState::Loading),
        );
        let texture = full.texture().expect("decoded");
        assert_eq!(texture.size(), [8192, 9]);
        // Inline it covers the chat's box at most, never more than it is.
        let inline = run_until(
            &ctx,
            8192,
            |ctx| images.shown(ctx, "/wide.png", Fit::Inline),
            |s| !matches!(s, ImageState::Loading),
        );
        assert_eq!(inline.texture().expect("decoded").size(), [8192, 9]);
        assert_eq!(fitted(9000, 10, &ANY), [9000, 10]);
        assert_eq!(
            fitted(
                20,
                10,
                &Limit {
                    max_side: 8192,
                    cover: Some([432, 240])
                }
            ),
            [20, 10]
        );
        assert_eq!(
            fitted(
                4000,
                3000,
                &Limit {
                    max_side: 8192,
                    cover: Some([432, 240])
                }
            ),
            [432, 324]
        );
    }

    #[test]
    fn pictures_not_drawn_make_room_for_new_ones() {
        // Three 1000×1000 pictures, 4 MB each, in a budget of two.
        let origin = serve(vec![
            ("/a.png", png(1000, 1000)),
            ("/b.png", png(1000, 1000)),
            ("/c.png", png(1000, 1000)),
        ]);
        let images = Images::new(Client::new(&origin).expect("client"));
        images.cache.lock().budget = 9_000_000;
        let ctx = egui::Context::default();
        let ready = |s: &Vec<ImageState>| s.iter().all(|s| matches!(s, ImageState::Ready(_)));
        run_until(&ctx, 2048, |ctx| vec![images.get(ctx, "/a.png")], ready);
        run_until(&ctx, 2048, |ctx| vec![images.get(ctx, "/b.png")], ready);
        // Only c is drawn now: a, the least recently drawn, goes.
        run_until(&ctx, 2048, |ctx| vec![images.get(ctx, "/c.png")], ready);
        for _ in 0..3 {
            run_until(&ctx, 2048, |ctx| images.get(ctx, "/c.png"), |_| true);
        }
        assert_eq!(images.held(), 8_000_000);
        assert!(ctx.tex_manager().read().num_allocated() <= 3);
        // A layout still knows a's size; drawing it brings it back.
        assert_eq!(
            images.natural(&ctx, "/a.png", Fit::Full),
            Some(egui::vec2(1000.0, 1000.0))
        );
        run_until(&ctx, 2048, |ctx| vec![images.get(ctx, "/a.png")], ready);
        assert_eq!(images.held(), 8_000_000);
    }

    #[test]
    fn a_failed_picture_is_asked_for_again_later() {
        let origin = serve(vec![]);
        let images = Images::new(Client::new(&origin).expect("client"));
        let ctx = egui::Context::default();
        let failed = |s: &ImageState| matches!(s, ImageState::Failed);
        run_until(&ctx, 2048, |ctx| images.get(ctx, "/gone.png"), failed);
        assert!(matches!(images.get(&ctx, "/gone.png"), ImageState::Failed));
        // Once its wait is over, the next draw asks again.
        if let Some(Entry {
            kind: Kind::Failed { at, .. },
            ..
        }) = images
            .cache
            .lock()
            .entries
            .get_mut(&("/gone.png".into(), Fit::Full))
        {
            *at -= RETRY_FIRST;
        }
        assert!(matches!(images.get(&ctx, "/gone.png"), ImageState::Loading));
        run_until(&ctx, 2048, |ctx| images.get(ctx, "/gone.png"), failed);
        assert_eq!(retry_after(2), RETRY_FIRST * 2);
        assert_eq!(retry_after(30), RETRY_MAX);
    }

    #[test]
    fn an_animation_over_its_cap_keeps_no_frames() {
        // Five 2000×2000 frames hold 80 MB.
        let frames = (0..5).map(|_| {
            let buffer = image::RgbaImage::new(2000, 2000);
            Ok(image::Frame::new(buffer))
        });
        let frames = image::Frames::new(Box::new(frames));
        assert!(decode_animation(frames, 2000, 2000, &ANY, ANIMATION_WORK).is_none());
        // Scaled to fit, the same frames stay.
        let frames = image::Frames::new(Box::new(
            (0..5).map(|_| Ok(image::Frame::new(image::RgbaImage::new(2000, 2000)))),
        ));
        let small = Limit {
            max_side: 8192,
            cover: Some([432, 240]),
        };
        let kept = decode_animation(frames, 2000, 2000, &small, ANIMATION_WORK).expect("frames");
        assert_eq!(kept.len(), 5);
        assert_eq!(kept[0].0.size, [432, 432]);
        // Small once scaled, but more full-size frames than the decoder
        // may produce (here 5 × 16 MB over 64 MB).
        let frames = image::Frames::new(Box::new(
            (0..5).map(|_| Ok(image::Frame::new(image::RgbaImage::new(2000, 2000)))),
        ));
        assert!(decode_animation(frames, 2000, 2000, &small, 64 << 20).is_none());
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

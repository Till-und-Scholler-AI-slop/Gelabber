//! Images from the server (avatars, banners, attachments, the Living Room
//! artwork): fetched through the client's session, decoded on a worker
//! thread and uploaded as textures. The content behind a URL never
//! changes, so entries stay for the session.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

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

#[derive(Clone)]
pub struct Images {
    client: Client,
    entries: Arc<Mutex<HashMap<String, ImageState>>>,
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
            loading: Arc::default(),
        }
    }

    /// The image behind `url` (a server path like `/api/avatars/…` or an
    /// absolute URL); starts loading it the first time.
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
            std::thread::Builder::new()
                .name("gelabber-image".into())
                .spawn(move || {
                    let state = match bytes.ok().and_then(|b| decode(&b)) {
                        Some(image) => ImageState::Ready(ctx.load_texture(&url, image, PHOTO)),
                        None => ImageState::Failed,
                    };
                    this.entries.lock().insert(url, state);
                    this.loading.fetch_sub(1, Ordering::AcqRel);
                    ctx.request_repaint();
                })
                .ok();
        });
        ImageState::Loading
    }

    /// True while any image is still being fetched or decoded.
    pub fn busy(&self) -> bool {
        self.loading.load(Ordering::Acquire) > 0
    }
}

/// PNG, JPEG, WebP or GIF (first frame) as an egui image.
pub fn decode(bytes: &[u8]) -> Option<ColorImage> {
    let image = image::load_from_memory(bytes).ok()?.to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    Some(ColorImage::from_rgba_unmultiplied(size, image.as_raw()))
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

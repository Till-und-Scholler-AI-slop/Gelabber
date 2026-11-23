//! Pictures (#574): metadata stripped before binding, thumbnails and the
//! placeholder made after it.
//!
//! [`prepare`] runs in `check_uploads`, before a picture joins its message:
//! it checks the pixel bounds on the whole file, removes privacy metadata
//! from the original (`metadata::strip`) and notes the size as shown, so
//! the message JSON carries `width`/`height` from the start. The worker
//! started by [`start`] then decodes each bound picture once, newest
//! first, and writes two thumbnails next to the original (`<key>/t400`,
//! `<key>/t800`, long edge in pixels) plus the average colour as
//! placeholder. Older attachments start at
//! `media_version` 0, so the same worker is the backfill; it is idempotent
//! and resumes after a restart.
//!
//! Decoding uses the `image` crate's pure-Rust decoders only, off the async
//! runtime, within the upload pixel bounds (`sniff`), an allocation cap and
//! a time cap, one picture at a time. A picture that fails stays a plain
//! attachment without thumbnails. Thumbnails are JPEG (quality 80), or
//! lossless WebP where the picture is transparent (Rust has no lossy WebP
//! encoder), with the original's ICC profile and no other metadata.

use std::io::Cursor;
use std::sync::atomic::Ordering;
use std::time::Duration;

use futures_util::StreamExt;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::webp::WebPEncoder;
use image::{DynamicImage, ImageDecoder, ImageEncoder, ImageFormat, ImageReader, Limits};
use sqlx::{FromRow, PgPool};
use tokio::sync::Semaphore;
use uuid::Uuid;

use super::{metadata, store_internal};
use crate::error::ApiError;
use crate::state::AppState;
use crate::storage::{ObjectStore, StoreError};

/// The version of what the worker makes. Raising it (and the index
/// predicate in a migration) makes the worker redo every picture.
pub const MEDIA_VERSION: i16 = 1;
/// `placeholder` form: 1 = average colour `#rrggbb`.
pub const PLACEHOLDER_VERSION: i16 = 1;
pub const EDGES: [u32; 2] = [400, 800];
const JPEG_QUALITY: u8 = 80;
const DECODE_TIMEOUT: Duration = Duration::from_secs(60);
/// Bytes a decoder may allocate: an 8192² RGBA canvas and some room.
const DECODE_ALLOC_MAX: u64 = 320 * 1024 * 1024;
const ATTEMPTS_MAX: i16 = 5;

/// Whole originals in memory at once while binding: each is at most
/// 25 MiB (`validate::FILE_SIZE_MAX`).
static PREPARING: Semaphore = Semaphore::const_new(2);
/// One decode at a time. The permit moves into the blocking task, so a
/// decode past [`DECODE_TIMEOUT`] still holds it until it really ends.
static DECODING: Semaphore = Semaphore::const_new(1);

pub fn is_picture(content_type: &str) -> bool {
    matches!(
        content_type,
        "image/jpeg" | "image/png" | "image/gif" | "image/webp"
    )
}

pub fn thumb_key(object_key: &str, edge: u32) -> String {
    format!("{object_key}/t{edge}")
}

/// The whole object, refused when it is larger than `size` says.
async fn read_all(store: &ObjectStore, key: &str, size: i64) -> Result<Vec<u8>, StoreError> {
    let limit = usize::try_from(size).unwrap_or(0);
    let mut stream = store.read(key).await?;
    let mut bytes = Vec::with_capacity(limit);
    while let Some(chunk) = stream.next().await {
        bytes.extend_from_slice(&chunk?);
        if bytes.len() > limit {
            return Err(StoreError::SizeMismatch);
        }
    }
    Ok(bytes)
}

/// What [`prepare`] made of a picture.
#[derive(Debug, PartialEq, Eq)]
#[must_use]
pub enum Prepared {
    /// Stripped and measured: it may join the message.
    Ready,
    /// Its whole file declares a canvas past `sniff::IMAGE_BOUNDS`.
    /// Nothing was written.
    TooManyPixels,
}

/// Strip the original's privacy metadata and note its size as shown. Runs
/// once per pending picture; the row records the stored size, which
/// becomes `size_bytes` at binding.
///
/// The content check reads only a prefix, and comments or extensions can
/// push a JPEG's frame header or a GIF's first frame past it (GEL-45).
/// Here the whole file is at hand, so the pixel bounds are checked again
/// on it: clients show a picture without thumbnails as the original.
/// A size even the whole file does not prove (a truncated file, or a
/// layout `image_size` refuses, such as a stray byte between JPEG
/// segments that libjpeg skips) is still let through.
pub async fn prepare(
    db: &PgPool,
    store: &ObjectStore,
    id: Uuid,
    object_key: &str,
    content_type: &str,
    size: i64,
) -> Result<Prepared, ApiError> {
    let _permit = PREPARING.acquire().await.expect("never closed");
    let bytes = match read_all(store, object_key, size).await {
        Ok(bytes) => bytes,
        Err(StoreError::NotFound | StoreError::SizeMismatch) => return Err(super::invalid()),
        Err(err) => return Err(store_internal(err)),
    };
    let kind = content_type.to_owned();
    // What is stored now, which a concurrent send may already have
    // stripped: recorded as is when there is nothing (more) to strip.
    let read_len = bytes.len() as i64;
    let measured = tokio::task::spawn_blocking(move || {
        if super::sniff::too_many_pixels(&kind, &bytes) {
            return None;
        }
        let shown = metadata::display_size(&kind, &bytes);
        let animated = metadata::animated(&kind, &bytes);
        let stripped = metadata::strip(&kind, &bytes);
        Some((shown, animated, stripped))
    })
    .await
    .map_err(|err| ApiError::Internal(format!("picture metadata: {err}")))?;
    let Some((shown, animated, stripped)) = measured else {
        return Ok(Prepared::TooManyPixels);
    };
    let stored_size = match stripped {
        Some(clean) => {
            let len = clean.len() as i64;
            // Recorded first: if this request ends between the two, the
            // object is either still the original or the stripped one, and
            // `check_uploads` accepts both and finishes the job.
            sqlx::query(
                "UPDATE attachments SET stripped_size = $2 WHERE id = $1 AND message_id IS NULL",
            )
            .bind(id)
            .bind(len)
            .execute(db)
            .await?;
            store
                .put(object_key, content_type, clean)
                .await
                .map_err(store_internal)?;
            len
        }
        None => read_len,
    };
    let dimension = |value: u32| i32::try_from(value).ok().filter(|v| *v > 0);
    sqlx::query(
        "UPDATE attachments SET width = $2, height = $3, animated = $4, stripped_size = $5 \
         WHERE id = $1 AND message_id IS NULL",
    )
    .bind(id)
    .bind(shown.and_then(|(w, _)| dimension(w)))
    .bind(shown.and_then(|(_, h)| dimension(h)))
    .bind(animated)
    .bind(stored_size)
    .execute(db)
    .await?;
    Ok(Prepared::Ready)
}

pub fn start(state: AppState) {
    if state.media_worker_started.swap(true, Ordering::AcqRel) {
        return;
    }
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(2));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            if state.db.is_closed() {
                break;
            }
            // Drain what is due, then wait for the next tick.
            loop {
                match run_once(&state).await {
                    Ok(true) => continue,
                    Ok(false) => break,
                    Err(err) => {
                        tracing::warn!(error = ?err, "picture worker failed");
                        break;
                    }
                }
            }
        }
    });
}

#[derive(FromRow)]
struct Job {
    id: Uuid,
    object_key: String,
    content_type: String,
    size_bytes: i64,
    media_attempts: i16,
    message_id: Option<Uuid>,
    /// Sent in the last hour: clients may show it with `thumbnail: false`
    /// right now. The backfill of old pictures sends no events.
    fresh: bool,
}

/// Process the newest due picture. `Ok(false)` when none is due. Public
/// for tests, which call it instead of waiting for the worker.
pub async fn run_once(state: &AppState) -> Result<bool, ApiError> {
    let job = sqlx::query_as::<_, Job>(
        "WITH due AS (SELECT id FROM attachments \
           WHERE media_version < 1 AND message_id IS NOT NULL \
             AND content_type IN ('image/jpeg', 'image/png', 'image/gif', 'image/webp') \
             AND media_retry_at <= now() \
           ORDER BY created_at DESC LIMIT 1 FOR UPDATE SKIP LOCKED) \
         UPDATE attachments a SET media_retry_at = now() + interval '5 minutes', \
             media_attempts = media_attempts + 1 \
         FROM due WHERE a.id = due.id \
         RETURNING a.id, a.object_key, a.content_type, a.size_bytes, a.media_attempts, \
             a.message_id, a.created_at > now() - interval '1 hour' AS fresh",
    )
    .fetch_optional(&state.db)
    .await?;
    let Some(job) = job else {
        return Ok(false);
    };
    // Claimed again and again without ever finishing: the process died
    // while on it (out of memory, an abort). Not one more try.
    if job.media_attempts > ATTEMPTS_MAX {
        tracing::warn!(attachment = %job.id, "picture thumbnails given up after crashes");
        finish_without_thumbnails(&state.db, job.id).await?;
        return Ok(true);
    }
    match process(state, &job).await {
        Ok(()) => {}
        Err(Failure::Picture(reason)) => {
            tracing::info!(attachment = %job.id, reason, "picture kept without thumbnails");
            finish_without_thumbnails(&state.db, job.id).await?;
        }
        Err(Failure::Store(err)) if job.media_attempts >= ATTEMPTS_MAX => {
            tracing::warn!(attachment = %job.id, error = %err, "picture thumbnails given up");
            finish_without_thumbnails(&state.db, job.id).await?;
        }
        Err(Failure::Store(err)) => {
            tracing::warn!(attachment = %job.id, error = %err, "picture thumbnails retried later");
            sqlx::query(
                "UPDATE attachments SET media_retry_at = now() + $2 * interval '1 minute' \
                 WHERE id = $1",
            )
            .bind(job.id)
            .bind(f64::from(1u32 << job.media_attempts.clamp(0, 6)))
            .execute(&state.db)
            .await?;
        }
        Err(Failure::Db(err)) => return Err(err),
    }
    Ok(true)
}

enum Failure {
    /// The picture itself: will not get better on a retry.
    Picture(String),
    Store(StoreError),
    Db(ApiError),
}

impl From<sqlx::Error> for Failure {
    fn from(err: sqlx::Error) -> Self {
        Self::Db(err.into())
    }
}

async fn finish_without_thumbnails(db: &PgPool, id: Uuid) -> Result<(), ApiError> {
    sqlx::query("UPDATE attachments SET media_version = $2, thumb_type = NULL WHERE id = $1")
        .bind(id)
        .bind(MEDIA_VERSION)
        .execute(db)
        .await?;
    Ok(())
}

async fn process(state: &AppState, job: &Job) -> Result<(), Failure> {
    let bytes = match read_all(&state.store, &job.object_key, job.size_bytes).await {
        Ok(bytes) => bytes,
        Err(StoreError::NotFound) => return Err(Failure::Picture("object missing".into())),
        Err(StoreError::SizeMismatch) => return Err(Failure::Picture("size differs".into())),
        Err(err) => return Err(Failure::Store(err)),
    };
    let kind = job.content_type.clone();
    let permit = DECODING.acquire().await.expect("never closed");
    let rendered = tokio::time::timeout(
        DECODE_TIMEOUT,
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            render(&kind, &bytes)
        }),
    )
    .await
    .map_err(|_| Failure::Picture("decoding took too long".into()))?
    .map_err(|err| Failure::Picture(format!("decoder panicked: {err}")))?
    .map_err(Failure::Picture)?;

    for (edge, bytes) in rendered.thumbs {
        state
            .store
            .put(
                &thumb_key(&job.object_key, edge),
                rendered.thumb_type,
                bytes,
            )
            .await
            .map_err(Failure::Store)?;
    }
    let updated = sqlx::query(
        "UPDATE attachments SET width = $2, height = $3, placeholder = $4, \
             placeholder_version = $5, animated = $6, thumb_type = $7, media_version = $8 \
         WHERE id = $1",
    )
    .bind(job.id)
    .bind(i32::try_from(rendered.width).ok())
    .bind(i32::try_from(rendered.height).ok())
    .bind(&rendered.placeholder)
    .bind(PLACEHOLDER_VERSION)
    .bind(rendered.animated)
    .bind(rendered.thumb_type)
    .bind(MEDIA_VERSION)
    .execute(&state.db)
    .await?;
    if updated.rows_affected() > 0
        && job.fresh
        && let Some(message_id) = job.message_id
    {
        crate::messages::attachments_ready(state, message_id)
            .await
            .map_err(Failure::Db)?;
    }
    if updated.rows_affected() == 0 {
        // Deleted meanwhile: its cleanup may have run before these were
        // written, so they are queued again.
        for edge in EDGES {
            sqlx::query(
                "INSERT INTO storage_cleanup (object_key, retain_until) VALUES ($1, now()) \
                 ON CONFLICT (object_key) DO UPDATE SET next_attempt = now()",
            )
            .bind(thumb_key(&job.object_key, edge))
            .execute(&state.db)
            .await?;
        }
    }
    Ok(())
}

pub struct Rendered {
    pub width: u32,
    pub height: u32,
    pub placeholder: String,
    pub animated: bool,
    pub thumb_type: &'static str,
    pub thumbs: Vec<(u32, Vec<u8>)>,
}

/// Decode, orient, and encode the thumbnails. Blocking; call it off the
/// async runtime.
pub fn render(content_type: &str, bytes: &[u8]) -> Result<Rendered, String> {
    let format = match content_type {
        "image/jpeg" => ImageFormat::Jpeg,
        "image/png" => ImageFormat::Png,
        "image/gif" => ImageFormat::Gif,
        "image/webp" => ImageFormat::WebP,
        other => return Err(format!("not a picture: {other}")),
    };
    // The bounds uploads are checked against (`sniff`), for the decoder.
    let (edge_max, pixels_max) = super::sniff::IMAGE_BOUNDS;
    let mut limits = Limits::default();
    limits.max_image_width = Some(edge_max);
    limits.max_image_height = Some(edge_max);
    limits.max_alloc = Some(DECODE_ALLOC_MAX);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|err| err.to_string())?;
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > pixels_max {
        return Err("too many pixels".into());
    }
    // `into_decoder` checks the edges only; `decode` would check this,
    // `from_decoder` does not (a 16-bit RGBA 8192² PNG is 512 MiB).
    if decoder.total_bytes() > DECODE_ALLOC_MAX {
        return Err("too large to decode".into());
    }
    let orientation = decoder.orientation().map_err(|err| err.to_string())?;
    let icc = decoder
        .icc_profile()
        .ok()
        .flatten()
        .filter(|icc| rgb_profile(icc));
    let picture = DynamicImage::from_decoder(decoder).map_err(|err| err.to_string())?;
    // Turned upright on the small copies, not the full-size picture: the
    // long edge is the same either way, and no second full-size copy.
    let mut large = fit(&picture, 800);
    drop(picture);
    let mut small = fit(&large, 400);
    large.apply_orientation(orientation);
    small.apply_orientation(orientation);
    let (width, height) = if turns_sideways(orientation) {
        (height, width)
    } else {
        (width, height)
    };
    let transparent = large.color().has_alpha() && large.to_rgba8().pixels().any(|p| p[3] < 255);
    let thumb_type = if transparent {
        "image/webp"
    } else {
        "image/jpeg"
    };
    let mut thumbs = Vec::with_capacity(2);
    for (edge, thumb) in [(400, &small), (800, &large)] {
        thumbs.push((edge, encode(thumb, transparent, icc.clone())?));
    }
    Ok(Rendered {
        width,
        height,
        placeholder: average_colour(&small),
        animated: metadata::animated(content_type, bytes),
        thumb_type,
        thumbs,
    })
}

fn turns_sideways(orientation: image::metadata::Orientation) -> bool {
    use image::metadata::Orientation::{Rotate90, Rotate90FlipH, Rotate270, Rotate270FlipH};
    matches!(
        orientation,
        Rotate90 | Rotate270 | Rotate90FlipH | Rotate270FlipH
    )
}

/// Whether an ICC profile describes RGB data (its data colour space,
/// bytes 16..20). Thumbnails are always RGB(A), so the profile of a
/// grayscale or CMYK picture would not match them.
fn rgb_profile(icc: &[u8]) -> bool {
    icc.get(16..20) == Some(b"RGB ")
}

/// At most `edge` pixels on the long side; never enlarged.
fn fit(picture: &DynamicImage, edge: u32) -> DynamicImage {
    if picture.width().max(picture.height()) > edge {
        picture.thumbnail(edge, edge)
    } else {
        picture.clone()
    }
}

fn encode(
    thumb: &DynamicImage,
    transparent: bool,
    icc: Option<Vec<u8>>,
) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let (width, height) = (thumb.width(), thumb.height());
    if transparent {
        let mut encoder = WebPEncoder::new_lossless(&mut out);
        if let Some(icc) = icc {
            let _ = encoder.set_icc_profile(icc);
        }
        let rgba = thumb.to_rgba8();
        encoder
            .write_image(&rgba, width, height, image::ExtendedColorType::Rgba8)
            .map_err(|err| err.to_string())?;
    } else {
        let mut encoder = JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY);
        if let Some(icc) = icc {
            let _ = encoder.set_icc_profile(icc);
        }
        let rgb = thumb.to_rgb8();
        encoder
            .write_image(&rgb, width, height, image::ExtendedColorType::Rgb8)
            .map_err(|err| err.to_string())?;
    }
    Ok(out)
}

/// The average colour, weighted by opacity, as `#rrggbb`.
fn average_colour(picture: &DynamicImage) -> String {
    let rgba = picture.to_rgba8();
    let (mut r, mut g, mut b, mut weight) = (0u64, 0u64, 0u64, 0u64);
    for pixel in rgba.pixels() {
        let alpha = u64::from(pixel[3]);
        r += u64::from(pixel[0]) * alpha;
        g += u64::from(pixel[1]) * alpha;
        b += u64::from(pixel[2]) * alpha;
        weight += alpha;
    }
    if weight == 0 {
        return "#808080".into();
    }
    format!("#{:02x}{:02x}{:02x}", r / weight, g / weight, b / weight)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage, Rgba, RgbaImage};

    fn jpeg(width: u32, height: u32) -> Vec<u8> {
        let picture = RgbImage::from_fn(width, height, |x, _| {
            if x < width / 2 {
                Rgb([200, 0, 0])
            } else {
                Rgb([0, 0, 200])
            }
        });
        let mut out = Vec::new();
        JpegEncoder::new_with_quality(&mut out, 90)
            .write_image(&picture, width, height, image::ExtendedColorType::Rgb8)
            .unwrap();
        out
    }

    #[test]
    fn photo_gets_jpeg_thumbnails_and_an_average_colour() {
        let rendered = render("image/jpeg", &jpeg(1600, 1200)).unwrap();
        assert_eq!((rendered.width, rendered.height), (1600, 1200));
        assert_eq!(rendered.thumb_type, "image/jpeg");
        let sizes: Vec<_> = rendered
            .thumbs
            .iter()
            .map(|(edge, bytes)| (*edge, image::load_from_memory(bytes).unwrap().width()))
            .collect();
        assert_eq!(sizes, [(400, 400), (800, 800)]);
        // Half red, half blue.
        let colour = u32::from_str_radix(&rendered.placeholder[1..], 16).unwrap();
        let (r, b) = (colour >> 16, colour & 0xff);
        assert!(
            (80..120).contains(&r) && (80..120).contains(&b),
            "{}",
            rendered.placeholder
        );
        assert!(!rendered.animated);
    }

    #[test]
    fn exif_orientation_is_applied() {
        let mut photo = jpeg(64, 32);
        // APP1 with orientation 6 (rotate 90°) right after SOI.
        let mut exif =
            b"Exif\0\0II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0".to_vec();
        let mut segment = vec![0xFF, 0xE1];
        segment.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        segment.append(&mut exif);
        photo.splice(2..2, segment);
        assert_eq!(metadata::display_size("image/jpeg", &photo), Some((32, 64)));
        let rendered = render("image/jpeg", &photo).unwrap();
        assert_eq!((rendered.width, rendered.height), (32, 64));
    }

    #[test]
    fn transparent_png_gets_lossless_webp_and_small_pictures_are_not_enlarged() {
        let picture = RgbaImage::from_fn(100, 50, |x, _| {
            Rgba([0, 255, 0, if x < 50 { 0 } else { 255 }])
        });
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&picture, 100, 50, image::ExtendedColorType::Rgba8)
            .unwrap();
        let rendered = render("image/png", &png).unwrap();
        assert_eq!(rendered.thumb_type, "image/webp");
        assert_eq!(rendered.placeholder, "#00ff00");
        for (_, bytes) in &rendered.thumbs {
            let thumb = image::load_from_memory(bytes).unwrap();
            assert_eq!((thumb.width(), thumb.height()), (100, 50));
            assert_eq!(thumb.to_rgba8().get_pixel(0, 0)[3], 0);
        }
    }

    #[test]
    fn broken_and_bomb_pictures_fail_without_panicking() {
        let whole = jpeg(200, 200);
        // Cut short: an error or a partial picture, never a panic.
        let _ = render("image/jpeg", &whole[..whole.len() / 2]);
        assert!(render("image/jpeg", &whole[..20]).is_err());
        assert!(render("image/png", b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR").is_err());
        assert!(render("image/gif", b"GIF89a").is_err());
        // A PNG that declares 20000×20000 (decompression bomb).
        let mut bomb = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        bomb.extend_from_slice(&20000u32.to_be_bytes());
        bomb.extend_from_slice(&20000u32.to_be_bytes());
        bomb.extend_from_slice(&[8, 2, 0, 0, 0, 0, 0, 0, 0]);
        assert!(render("image/png", &bomb).is_err());
        // 8192×8192 fits the edge bound but not with 9000.
        let mut wide = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        wide.extend_from_slice(&16000u32.to_be_bytes());
        wide.extend_from_slice(&9000u32.to_be_bytes());
        wide.extend_from_slice(&[8, 2, 0, 0, 0, 0, 0, 0, 0]);
        assert!(render("image/png", &wide).is_err());
    }
}

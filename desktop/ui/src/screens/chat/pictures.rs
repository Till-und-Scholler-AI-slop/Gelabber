//! The boxes pictures are drawn in (#574; web/src/components/imageBox.ts):
//! the server sends each picture's size as shown, so a box is reserved
//! before any byte arrives and loading never moves the list. One picture
//! takes its own shape, never enlarged, at most [`PICTURE_BOX`]² and no
//! narrower or taller than 1:3 (beyond that it crops); several make a
//! mosaic by count, at most [`PICTURE_BOX`] wide.

use egui::{Color32, Rect, Vec2, pos2, vec2};
use gelabber_client::Attachment;

/// The largest box of a single picture, and the mosaic's width.
pub const PICTURE_BOX: f32 = 400.0;
/// Even a tiny picture stays a target to click.
const BOX_MIN: f32 = 24.0;
/// The most extreme shape a single box takes; beyond it, it crops.
const SHAPE_MAX: f32 = 3.0;
/// Box of a picture whose size nobody knows yet.
pub const UNKNOWN_BOX_HEIGHT: f32 = 300.0;
/// Tiles a mosaic shows; the last says "+N" for the rest.
pub const MOSAIC_TILES: usize = 6;
/// The gap between tiles.
pub const GAP: f32 = 4.0;
/// Animated pictures up to this size play inline; larger ones show their
/// first frame (the thumbnail) and play in the lightbox.
pub const ANIMATED_INLINE_MAX: i64 = 5 * 1024 * 1024;

/// The size as shown, as the server sent it.
pub fn known_size(att: &Attachment) -> Option<Vec2> {
    match (att.width, att.height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => Some(vec2(w as f32, h as f32)),
        _ => None,
    }
}

/// `imageFit`: the box of a single picture of `size`, also no wider than
/// `width`; whether the picture is cropped in it.
pub fn fit(size: Vec2, width: f32) -> (Vec2, bool) {
    let ratio = size.x / size.y;
    let shape = ratio.clamp(1.0 / SHAPE_MAX, SHAPE_MAX);
    let cropped = shape != ratio;
    // The natural size in the clamped shape, cropped along the longer side,
    // so no side is enlarged.
    let natural = if !cropped {
        size
    } else if shape > 1.0 {
        vec2(size.y * shape, size.y)
    } else {
        vec2(size.x, size.x / shape)
    };
    let limit = PICTURE_BOX.min(width.max(1.0));
    let scale = 1f32.min(limit / natural.x).min(PICTURE_BOX / natural.y);
    (
        vec2(
            (natural.x * scale).max(BOX_MIN),
            (natural.y * scale).max(BOX_MIN),
        ),
        cropped,
    )
}

/// `mosaic`: the height of the mosaic for `count` pictures as a share of
/// its width, and how many tiles it shows.
pub fn mosaic(count: usize) -> (f32, usize) {
    match count {
        0..=2 => (1.0 / 2.0, 2),
        3 => (2.0 / 3.0, 3),
        4 => (1.0, 4),
        5 => (5.0 / 6.0, 5),
        _ => (2.0 / 3.0, MOSAIC_TILES),
    }
}

/// The mosaic's tiles, relative to its top left corner, for `count`
/// pictures `width` wide (chat.css `.lr-attachment-grid.is-*`): 2 halves,
/// one big and 2, 2×2, 2 over 3, 3×2.
pub fn tiles(count: usize, width: f32) -> (Vec2, Vec<Rect>) {
    let w = width.min(PICTURE_BOX);
    let (ratio, shown) = mosaic(count);
    let h = w * ratio;
    let cell = |x: f32, y: f32, cw: f32, ch: f32| Rect::from_min_size(pos2(x, y), vec2(cw, ch));
    // Equal columns (`repeat(n, 1fr)`) with the gaps between them.
    let col = |n: f32| (w - GAP * (n - 1.0)) / n;
    let rects = match shown {
        2 => {
            let c = col(2.0);
            vec![cell(0.0, 0.0, c, h), cell(c + GAP, 0.0, c, h)]
        }
        3 => {
            // 2fr 1fr, the first spanning both rows.
            let free = w - GAP;
            let (big, small) = (free * 2.0 / 3.0, free / 3.0);
            let row = (h - GAP) / 2.0;
            vec![
                cell(0.0, 0.0, big, h),
                cell(big + GAP, 0.0, small, row),
                cell(big + GAP, row + GAP, small, row),
            ]
        }
        4 => {
            let c = col(2.0);
            let row = (h - GAP) / 2.0;
            vec![
                cell(0.0, 0.0, c, row),
                cell(c + GAP, 0.0, c, row),
                cell(0.0, row + GAP, c, row),
                cell(c + GAP, row + GAP, c, row),
            ]
        }
        5 => {
            // Six columns, rows 3fr 2fr: two tiles over three.
            let six = col(6.0);
            let free = h - GAP;
            let (top, bottom) = (free * 3.0 / 5.0, free * 2.0 / 5.0);
            let span = |n: f32| six * n + GAP * (n - 1.0);
            let mut out = Vec::new();
            for i in 0..2 {
                out.push(cell(i as f32 * (span(3.0) + GAP), 0.0, span(3.0), top));
            }
            for i in 0..3 {
                out.push(cell(
                    i as f32 * (span(2.0) + GAP),
                    top + GAP,
                    span(2.0),
                    bottom,
                ));
            }
            out
        }
        _ => {
            let c = col(3.0);
            let row = (h - GAP) / 2.0;
            (0..6)
                .map(|i| {
                    cell(
                        (i % 3) as f32 * (c + GAP),
                        (i / 3) as f32 * (row + GAP),
                        c,
                        row,
                    )
                })
                .collect()
        }
    };
    (vec2(w, h), rects)
}

pub fn original_url(att: &Attachment) -> String {
    format!("/api/attachments/{}", att.id)
}

/// The URL a picture is drawn from in a box `width` points wide at
/// `pixels_per_point`: the server's thumbnail of 400 or 800 once it has
/// one, the original before, and the original of an animation small
/// enough to play.
pub fn inline_url(att: &Attachment, width: f32, pixels_per_point: f32) -> String {
    let plays = att.animated && att.size <= ANIMATED_INLINE_MAX;
    if !att.thumbnail || plays {
        return original_url(att);
    }
    let edge = if width * pixels_per_point > 400.0 {
        800
    } else {
        400
    };
    format!("{}?size={edge}", original_url(att))
}

/// The lightbox's first picture: the 800 thumbnail while the original is
/// on its way, when the server has one.
pub fn preview_url(att: &Attachment) -> Option<String> {
    att.thumbnail
        .then(|| format!("{}?size=800", original_url(att)))
}

/// The colour drawn while the picture loads ("#rrggbb", version 1).
pub fn placeholder(att: &Attachment) -> Option<Color32> {
    if att.placeholder_version != Some(1) {
        return None;
    }
    let hex = att.placeholder.as_deref()?.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(hex, 16).ok()?;
    Some(Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(w: u32, h: u32) -> Attachment {
        Attachment {
            content_type: "image/jpeg".into(),
            width: Some(w),
            height: Some(h),
            size: 1000,
            ..Default::default()
        }
    }

    fn near(a: Vec2, b: Vec2) -> bool {
        (a - b).length() < 0.01
    }

    #[test]
    fn a_single_picture_keeps_its_shape_and_is_never_enlarged() {
        assert_eq!(
            fit(vec2(4000.0, 3000.0), 1000.0),
            (vec2(400.0, 300.0), false)
        );
        assert_eq!(
            fit(vec2(3000.0, 4000.0), 1000.0),
            (vec2(300.0, 400.0), false)
        );
        assert_eq!(fit(vec2(120.0, 80.0), 1000.0), (vec2(120.0, 80.0), false));
        // Narrower than the column: the column wins.
        assert_eq!(
            fit(vec2(4000.0, 3000.0), 200.0),
            (vec2(200.0, 150.0), false)
        );
    }

    #[test]
    fn beyond_one_to_three_it_crops_without_enlarging() {
        // A panorama 10:1 becomes 3:1.
        let (b, cropped) = fit(vec2(5000.0, 500.0), 1000.0);
        assert!(near(b, vec2(400.0, 400.0 / 3.0)) && cropped, "{b:?}");
        // A small strip 300×10 is cropped to 30×10, then the minimum.
        assert_eq!(fit(vec2(300.0, 10.0), 1000.0), (vec2(30.0, 24.0), true));
        // A tall screenshot 1:6 becomes 1:3.
        let (b, cropped) = fit(vec2(600.0, 3600.0), 1000.0);
        assert!(near(b, vec2(400.0 / 3.0, 400.0)) && cropped, "{b:?}");
    }

    #[test]
    fn mosaics_have_a_fixed_shape_by_count() {
        for (count, ratio, shown) in [
            (2, 0.5, 2),
            (3, 2.0 / 3.0, 3),
            (4, 1.0, 4),
            (5, 5.0 / 6.0, 5),
            (6, 2.0 / 3.0, 6),
            (11, 2.0 / 3.0, 6),
        ] {
            let (size, rects) = tiles(count, 1000.0);
            assert!(near(size, vec2(400.0, 400.0 * ratio)), "{count}: {size:?}");
            assert_eq!(rects.len(), shown, "{count}");
            let frame = Rect::from_min_size(egui::Pos2::ZERO, size).expand(0.01);
            for (i, r) in rects.iter().enumerate() {
                assert!(frame.contains_rect(*r), "{count}: tile {i} {r:?}");
                for other in &rects[i + 1..] {
                    assert!(!r.shrink(0.01).intersects(other.shrink(0.01)), "{count}");
                }
            }
        }
        // Three: the big one spans both rows.
        let (_, rects) = tiles(3, 400.0);
        assert!((rects[0].height() - 400.0 * 2.0 / 3.0).abs() < 0.01);
        assert!(rects[0].width() > rects[1].width());
    }

    #[test]
    fn thumbnails_follow_the_box_and_animations_play_the_original() {
        let mut a = picture(4000, 3000);
        let id = a.id;
        assert_eq!(inline_url(&a, 400.0, 1.0), format!("/api/attachments/{id}"));
        a.thumbnail = true;
        assert_eq!(
            inline_url(&a, 400.0, 1.0),
            format!("/api/attachments/{id}?size=400")
        );
        assert_eq!(
            inline_url(&a, 400.0, 2.0),
            format!("/api/attachments/{id}?size=800")
        );
        a.animated = true;
        assert_eq!(inline_url(&a, 400.0, 1.0), format!("/api/attachments/{id}"));
        a.size = ANIMATED_INLINE_MAX + 1;
        assert_eq!(
            inline_url(&a, 400.0, 1.0),
            format!("/api/attachments/{id}?size=400")
        );
    }

    #[test]
    fn the_placeholder_is_a_known_version() {
        let mut a = picture(10, 10);
        a.placeholder = Some("#336699".into());
        assert_eq!(placeholder(&a), None);
        a.placeholder_version = Some(1);
        assert_eq!(placeholder(&a), Some(Color32::from_rgb(0x33, 0x66, 0x99)));
        a.placeholder_version = Some(2);
        assert_eq!(placeholder(&a), None);
    }

    #[test]
    fn the_server_s_fields_are_read_and_old_servers_still_work() {
        let id = uuid::Uuid::nil();
        let json = format!(
            r##"{{"id":"{id}","filename":"a.jpg","content_type":"image/jpeg","size":5,
               "width":40,"height":30,"placeholder":"#000000","placeholder_version":1,
               "animated":false,"thumbnail":true}}"##
        );
        let a: Attachment = serde_json::from_str(&json).expect("new");
        assert_eq!(known_size(&a), Some(vec2(40.0, 30.0)));
        assert!(a.thumbnail);
        let old =
            format!(r#"{{"id":"{id}","filename":"a.jpg","content_type":"image/jpeg","size":5}}"#);
        let a: Attachment = serde_json::from_str(&old).expect("old");
        assert_eq!(known_size(&a), None);
        assert!(!a.thumbnail && !a.animated);
    }
}

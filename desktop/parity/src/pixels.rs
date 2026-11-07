//! Two stricter measures next to SSIM, which forgives large flat dark areas:
//!
//! * pixel match: the share of pixels whose colour is within `TOLERANCE`
//!   (largest channel difference) of a pixel at most one pixel away in the
//!   other image, both ways; tolerates antialiasing and 1 px shifts;
//! * edge match: F1 of the edge pixels (text, lines, borders) of both
//!   images, an edge counting as matched when the other image has one within
//!   two pixels. Missing or misplaced text drops it even on a dark page.

use image::{GrayImage, Luma, RgbaImage};

const TOLERANCE: i16 = 20;
const EDGE: f64 = 40.0;
const EDGE_REACH: i64 = 2;

fn close(a: [u8; 4], b: [u8; 4]) -> bool {
    (0..3).all(|i| (i16::from(a[i]) - i16::from(b[i])).abs() <= TOLERANCE)
}

fn matched_somewhere(from: &RgbaImage, to: &RgbaImage, x: u32, y: u32) -> bool {
    let p = from.get_pixel(x, y).0;
    let (w, h) = to.dimensions();
    for dy in -1i64..=1 {
        for dx in -1i64..=1 {
            let (xx, yy) = (x as i64 + dx, y as i64 + dy);
            if xx >= 0
                && yy >= 0
                && (xx as u32) < w
                && (yy as u32) < h
                && close(p, to.get_pixel(xx as u32, yy as u32).0)
            {
                return true;
            }
        }
    }
    false
}

/// Share of pixels that match, and a map of the ones that do not (white).
pub fn pixel_match(a: &RgbaImage, b: &RgbaImage) -> (f64, GrayImage) {
    let (w, h) = a.dimensions();
    let mut good = 0u64;
    let mut map = GrayImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let ok = matched_somewhere(a, b, x, y) && matched_somewhere(b, a, x, y);
            if ok {
                good += 1;
            } else {
                map.put_pixel(x, y, Luma([255]));
            }
        }
    }
    (good as f64 / f64::from(w * h), map)
}

fn edges(image: &RgbaImage) -> Vec<bool> {
    let (w, h) = image.dimensions();
    let lum: Vec<f64> = image
        .pixels()
        .map(|p| 0.299 * f64::from(p.0[0]) + 0.587 * f64::from(p.0[1]) + 0.114 * f64::from(p.0[2]))
        .collect();
    let at = |x: u32, y: u32| lum[(y * w + x) as usize];
    let mut out = vec![false; (w * h) as usize];
    for y in 1..h.saturating_sub(1) {
        for x in 1..w.saturating_sub(1) {
            let gx = at(x + 1, y) - at(x - 1, y);
            let gy = at(x, y + 1) - at(x, y - 1);
            out[(y * w + x) as usize] = (gx * gx + gy * gy).sqrt() > EDGE;
        }
    }
    out
}

fn recall(from: &[bool], to: &[bool], w: u32, h: u32) -> (u64, u64) {
    let (mut total, mut hit) = (0u64, 0u64);
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            if !from[(y * w as i64 + x) as usize] {
                continue;
            }
            total += 1;
            'search: for dy in -EDGE_REACH..=EDGE_REACH {
                for dx in -EDGE_REACH..=EDGE_REACH {
                    let (xx, yy) = (x + dx, y + dy);
                    if xx >= 0
                        && yy >= 0
                        && xx < w as i64
                        && yy < h as i64
                        && to[(yy * w as i64 + xx) as usize]
                    {
                        hit += 1;
                        break 'search;
                    }
                }
            }
        }
    }
    (hit, total)
}

/// F1 of the edges of `reference` found in `candidate` and back.
pub fn edge_match(reference: &RgbaImage, candidate: &RgbaImage) -> f64 {
    let (w, h) = reference.dimensions();
    let (er, ec) = (edges(reference), edges(candidate));
    let (hit_r, total_r) = recall(&er, &ec, w, h);
    let (hit_c, total_c) = recall(&ec, &er, w, h);
    if total_r == 0 && total_c == 0 {
        return 1.0;
    }
    let r = if total_r == 0 {
        0.0
    } else {
        hit_r as f64 / total_r as f64
    };
    let p = if total_c == 0 {
        0.0
    } else {
        hit_c as f64 / total_c as f64
    };
    if r + p == 0.0 {
        0.0
    } else {
        2.0 * r * p / (r + p)
    }
}

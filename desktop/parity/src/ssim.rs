//! Structural similarity (Wang et al. 2004) on luminance, Gaussian window
//! σ = 1.5 (11×11), the usual constants. 1.0 means identical.

use image::{GrayImage, Luma, RgbaImage};

const K1: f64 = 0.01;
const K2: f64 = 0.03;
const L: f64 = 255.0;

fn luminance(image: &RgbaImage) -> Vec<f64> {
    image
        .pixels()
        .map(|p| {
            let [r, g, b, _] = p.0;
            0.299 * f64::from(r) + 0.587 * f64::from(g) + 0.114 * f64::from(b)
        })
        .collect()
}

fn kernel() -> [f64; 11] {
    let mut k = [0.0; 11];
    let mut sum = 0.0;
    for (i, v) in k.iter_mut().enumerate() {
        let x = i as f64 - 5.0;
        *v = (-(x * x) / (2.0 * 1.5 * 1.5)).exp();
        sum += *v;
    }
    k.map(|v| v / sum)
}

/// Separable Gaussian blur with edge clamping.
fn blur(data: &[f64], w: usize, h: usize) -> Vec<f64> {
    let k = kernel();
    let mut tmp = vec![0.0; data.len()];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (i, kv) in k.iter().enumerate() {
                let xx = (x as isize + i as isize - 5).clamp(0, w as isize - 1) as usize;
                acc += kv * data[y * w + xx];
            }
            tmp[y * w + x] = acc;
        }
    }
    let mut out = vec![0.0; data.len()];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (i, kv) in k.iter().enumerate() {
                let yy = (y as isize + i as isize - 5).clamp(0, h as isize - 1) as usize;
                acc += kv * tmp[yy * w + x];
            }
            out[y * w + x] = acc;
        }
    }
    out
}

pub struct Score {
    pub ssim: f64,
    /// Per-pixel dissimilarity map (white = different), for review.
    pub diff: GrayImage,
}

/// Both images must have the same size.
pub fn compare(a: &RgbaImage, b: &RgbaImage) -> Score {
    assert_eq!(a.dimensions(), b.dimensions(), "images differ in size");
    let (w, h) = (a.width() as usize, a.height() as usize);
    let x = luminance(a);
    let y = luminance(b);
    let xx: Vec<f64> = x.iter().map(|v| v * v).collect();
    let yy: Vec<f64> = y.iter().map(|v| v * v).collect();
    let xy: Vec<f64> = x.iter().zip(&y).map(|(a, b)| a * b).collect();
    let (mx, my) = (blur(&x, w, h), blur(&y, w, h));
    let (sxx, syy, sxy) = (blur(&xx, w, h), blur(&yy, w, h), blur(&xy, w, h));
    let c1 = (K1 * L).powi(2);
    let c2 = (K2 * L).powi(2);
    let mut total = 0.0;
    let mut diff = GrayImage::new(w as u32, h as u32);
    for i in 0..w * h {
        let (ux, uy) = (mx[i], my[i]);
        let vx = (sxx[i] - ux * ux).max(0.0);
        let vy = (syy[i] - uy * uy).max(0.0);
        let cov = sxy[i] - ux * uy;
        let s =
            ((2.0 * ux * uy + c1) * (2.0 * cov + c2)) / ((ux * ux + uy * uy + c1) * (vx + vy + c2));
        total += s;
        let shade = ((1.0 - s).clamp(0.0, 1.0) * 255.0) as u8;
        diff.put_pixel((i % w) as u32, (i / w) as u32, Luma([shade]));
    }
    Score {
        ssim: total / (w * h) as f64,
        diff,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_images_score_one() {
        let img = RgbaImage::from_fn(40, 30, |x, y| {
            image::Rgba([(x * 6) as u8, (y * 8) as u8, 90, 255])
        });
        assert!((compare(&img, &img).ssim - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_moved_block_scores_lower() {
        let a = RgbaImage::from_fn(60, 60, |x, _| {
            image::Rgba(if x < 30 { [0, 0, 0, 255] } else { [255; 4] })
        });
        let b = RgbaImage::from_fn(60, 60, |x, _| {
            image::Rgba(if x < 36 { [0, 0, 0, 255] } else { [255; 4] })
        });
        let s = compare(&a, &b).ssim;
        assert!(s < 0.95 && s > 0.5, "{s}");
    }
}

//! Batch layer for marks stamped at the same position on every photo of a set.
//!
//! The per-pixel median of image gradients over the set keeps a fixed watermark and averages
//! the varying content out (Dekel et al., CVPR 2017). The method and thresholds follow the
//! research prototype (numpy/scipy/skimage), so the numbers keep their meaning.

use crate::mask::{self, Mask};
use crate::{Error, Result};
use image::{ImageBuffer, Luma, RgbImage};

/// Low hysteresis threshold on the median gradient magnitude.
pub const BATCH_LOW: f32 = 40.0;
/// High hysteresis threshold on the median gradient magnitude.
pub const BATCH_HIGH: f32 = 80.0;

/// Rows of output per band.
const BAND: usize = 512;
/// Border (in pixels) added around the strong pixels to form the region of interest.
const ROI_PAD: u32 = 60;
/// Result is discarded when it covers more than this fraction of the image.
const MAX_COVERAGE: f32 = 0.10;

type MagImage = ImageBuffer<Luma<f32>, Vec<f32>>;

/// Per-pixel magnitude of the median (over images) Sobel gradient.
///
/// Each image gets its own 2-D Sobel (as `scipy.ndimage.sobel`, 'reflect' boundary) on its
/// ITU-R 601 luma; gx and gy are then medianed separately across images and combined with
/// `hypot`. Works in 512-row bands with a 1-row overlap to bound memory. Errors with
/// [`Error::SizeMismatch`] when sizes differ and [`Error::Image`] for an empty slice.
pub fn median_gradient(images: &[&RgbImage]) -> Result<MagImage> {
    let first = images
        .first()
        .ok_or_else(|| Error::Image("no images for the batch layer".into()))?;
    let (w, h) = first.dimensions();
    if images.iter().any(|i| i.dimensions() != (w, h)) {
        return Err(Error::SizeMismatch);
    }
    let (wu, hu, n) = (w as usize, h as usize, images.len());
    let mut out = vec![0f32; wu * hu];
    if wu == 0 || hu == 0 {
        return Ok(MagImage::from_raw(w, h, out).expect("sized buffer"));
    }

    let mut gx = vec![0f32; n * BAND * wu];
    let mut gy = vec![0f32; n * BAND * wu];
    let mut luma = vec![0f32; (BAND + 2) * wu];
    let mut vals = vec![0f32; n];

    for y0 in (0..hu).step_by(BAND) {
        let rows = BAND.min(hu - y0);
        for (k, img) in images.iter().enumerate() {
            // Luma rows y0-1 ..= y0+rows, with the edge row repeated ('reflect').
            for r in 0..rows + 2 {
                let sy = (y0 + r).saturating_sub(1).min(hu - 1);
                let src = &img.as_raw()[sy * wu * 3..(sy + 1) * wu * 3];
                luma_row(src, &mut luma[r * wu..(r + 1) * wu]);
            }
            for r in 0..rows {
                let (up, mid, dn) = (
                    &luma[r * wu..(r + 1) * wu],
                    &luma[(r + 1) * wu..(r + 2) * wu],
                    &luma[(r + 2) * wu..(r + 3) * wu],
                );
                let base = (k * BAND + r) * wu;
                sobel_row(
                    up,
                    mid,
                    dn,
                    &mut gx[base..base + wu],
                    &mut gy[base..base + wu],
                );
            }
        }
        for r in 0..rows {
            for x in 0..wu {
                for (k, v) in vals.iter_mut().enumerate() {
                    *v = gx[(k * BAND + r) * wu + x];
                }
                let mx = median(&mut vals);
                for (k, v) in vals.iter_mut().enumerate() {
                    *v = gy[(k * BAND + r) * wu + x];
                }
                let my = median(&mut vals);
                out[(y0 + r) * wu + x] = mx.hypot(my);
            }
        }
    }
    Ok(MagImage::from_raw(w, h, out).expect("sized buffer"))
}

/// ITU-R 601 luma of packed RGB pixels `src` into `dst` (one value per pixel).
pub(crate) fn luma_row(src: &[u8], dst: &mut [f32]) {
    for (d, p) in dst.iter_mut().zip(src.as_chunks::<3>().0) {
        *d = (p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114) as f32 / 1000.0;
    }
}

/// 3x3 Sobel derivatives of row `mid`, with `up`/`dn` the rows above and below and the edge
/// columns repeated (scipy 'reflect').
pub(crate) fn sobel_row(up: &[f32], mid: &[f32], dn: &[f32], gx: &mut [f32], gy: &mut [f32]) {
    let w = mid.len();
    for x in 0..w {
        let (xl, xr) = (x.saturating_sub(1), (x + 1).min(w - 1));
        // Smooth along y [1,2,1], derivative along x [-1,0,1].
        let sm = |c: usize| up[c] + 2.0 * mid[c] + dn[c];
        gx[x] = sm(xr) - sm(xl);
        // Smooth along x [1,2,1], derivative along y [-1,0,1].
        let d = |c: usize| dn[c] - up[c];
        gy[x] = d(xl) + 2.0 * d(x) + d(xr);
    }
}

/// Sobel gradient magnitude of a row-major `w` x `h` luma image, edge pixels repeated.
pub(crate) fn sobel_magnitude(luma: &[f32], w: usize, h: usize) -> Vec<f32> {
    let mut out = vec![0f32; w * h];
    let (mut gx, mut gy) = (vec![0f32; w], vec![0f32; w]);
    for y in 0..h {
        let row = |r: usize| &luma[r * w..(r + 1) * w];
        let (up, dn) = (row(y.saturating_sub(1)), row((y + 1).min(h - 1)));
        sobel_row(up, row(y), dn, &mut gx, &mut gy);
        for ((o, a), b) in out[y * w..(y + 1) * w].iter_mut().zip(&gx).zip(&gy) {
            *o = a.hypot(*b);
        }
    }
    out
}

/// numpy-style median (mean of the two middle values for an even count). Reorders `v`.
fn median(v: &mut [f32]) -> f32 {
    v.sort_unstable_by(f32::total_cmp);
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// Turn a median-gradient magnitude into a mask of the fixed mark, or `None` when there is no
/// pixel above `high` or the result covers more than 10 % of the image.
pub fn batch_lines(mag: &MagImage, low: f32, high: f32) -> Option<Mask> {
    let (w, h) = mag.dimensions();
    let data = mag.as_raw();
    let wu = w as usize;

    // Region of interest: bbox of the strong pixels, padded and clamped.
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for (i, &v) in data.iter().enumerate() {
        if v > high {
            let (x, y) = ((i % wu) as u32, (i / wu) as u32);
            x0 = x0.min(x);
            x1 = x1.max(x);
            y0 = y0.min(y);
            y1 = y1.max(y);
        }
    }
    if x0 == u32::MAX {
        return None;
    }
    let (x0, y0) = (x0.saturating_sub(ROI_PAD), y0.saturating_sub(ROI_PAD));
    let (x1, y1) = ((x1 + ROI_PAD).min(w - 1), (y1 + ROI_PAD).min(h - 1));
    let (rw, rh) = ((x1 - x0 + 1) as usize, (y1 - y0 + 1) as usize);

    // Magnitude inside the ROI (everything outside is 0 and cannot contribute).
    let mut m = vec![0f32; rw * rh];
    for r in 0..rh {
        let s = (y0 as usize + r) * wu + x0 as usize;
        m[r * rw..(r + 1) * rw].copy_from_slice(&data[s..s + rw]);
    }

    // Blobs: opening of (m > low) with a 7x7 square, then 2 dilations with the 3x3 cross.
    let above: Vec<bool> = m.iter().map(|&v| v > low).collect();
    let mut blobs = open_square(&above, rw, rh, 3);
    for _ in 0..2 {
        blobs = dilate_cross(&blobs, rw, rh);
    }

    // Hysteresis on the magnitude with the blobs zeroed, plus all strong pixels.
    let masked: Vec<f32> = m
        .iter()
        .zip(&blobs)
        .map(|(&v, &b)| if b { 0.0 } else { v })
        .collect();
    let mut lines = hysteresis(&masked, rw, rh, low, high);
    for (l, &v) in lines.iter_mut().zip(&m) {
        *l |= v > high;
    }

    let mut full = Mask::new(w, h);
    for r in 0..rh {
        let d = (y0 as usize + r) * wu + x0 as usize;
        for c in 0..rw {
            if lines[r * rw + c] {
                full.as_mut()[d + c] = 255;
            }
        }
    }
    let out = mask::dilate(&full, 3);
    (mask::coverage(&out) <= MAX_COVERAGE).then_some(out)
}

/// Pixels above `low` in a 4-connected component that holds a pixel above `high`
/// (skimage `apply_hysteresis_threshold`).
fn hysteresis(m: &[f32], w: usize, h: usize, low: f32, high: f32) -> Vec<bool> {
    let mut keep = vec![false; w * h];
    let mut stack: Vec<usize> = Vec::new();
    for (i, &v) in m.iter().enumerate() {
        if v > high {
            keep[i] = true;
            stack.push(i);
        }
    }
    while let Some(i) = stack.pop() {
        let (x, y) = (i % w, i / w);
        let mut visit = |j: usize| {
            if !keep[j] && m[j] > low {
                keep[j] = true;
                stack.push(j);
            }
        };
        if x > 0 {
            visit(i - 1);
        }
        if x + 1 < w {
            visit(i + 1);
        }
        if y > 0 {
            visit(i - w);
        }
        if y + 1 < h {
            visit(i + w);
        }
    }
    keep
}

/// Binary opening with a square of side `2r+1`; pixels outside the image count as unset
/// (scipy `binary_opening` default), so the erosion eats into the borders.
fn open_square(a: &[bool], w: usize, h: usize, r: usize) -> Vec<bool> {
    let eroded = box_all(a, w, h, r);
    box_any(&eroded, w, h, r)
}

/// True where every pixel of the (2r+1) square is set; outside the image counts as unset.
fn box_all(a: &[bool], w: usize, h: usize, r: usize) -> Vec<bool> {
    let unset: Vec<bool> = a.iter().map(|&v| !v).collect();
    // Padding with "unset" keeps the border eroding: it shows up as set in the inverse.
    let (pw, ph) = (w + 2 * r, h + 2 * r);
    let mut padded = vec![true; pw * ph];
    for y in 0..h {
        padded[(y + r) * pw + r..(y + r) * pw + r + w].copy_from_slice(&unset[y * w..(y + 1) * w]);
    }
    let any = box_any(&padded, pw, ph, r);
    let mut out = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            out[y * w + x] = !any[(y + r) * pw + x + r];
        }
    }
    out
}

/// True where any pixel of the (2r+1) square is set (separable sliding window).
fn box_any(a: &[bool], w: usize, h: usize, r: usize) -> Vec<bool> {
    let mut tmp = vec![false; w * h];
    for y in 0..h {
        let row = &a[y * w..(y + 1) * w];
        let mut count = row[..(r + 1).min(w)].iter().filter(|&&v| v).count();
        for x in 0..w {
            tmp[y * w + x] = count > 0;
            if x + r + 1 < w && row[x + r + 1] {
                count += 1;
            }
            if x >= r && row[x - r] {
                count -= 1;
            }
        }
    }
    let mut out = vec![false; w * h];
    let mut counts = vec![0u32; w];
    for y in 0..(r + 1).min(h) {
        for x in 0..w {
            counts[x] += tmp[y * w + x] as u32;
        }
    }
    for y in 0..h {
        for x in 0..w {
            out[y * w + x] = counts[x] > 0;
        }
        if y + r + 1 < h {
            for x in 0..w {
                counts[x] += tmp[(y + r + 1) * w + x] as u32;
            }
        }
        if y >= r {
            for x in 0..w {
                counts[x] -= tmp[(y - r) * w + x] as u32;
            }
        }
    }
    out
}

/// One dilation with the 3x3 cross (scipy default structure).
fn dilate_cross(a: &[bool], w: usize, h: usize) -> Vec<bool> {
    let mut out = a.to_vec();
    for y in 0..h {
        for x in 0..w {
            if a[y * w + x] {
                if x > 0 {
                    out[y * w + x - 1] = true;
                }
                if x + 1 < w {
                    out[y * w + x + 1] = true;
                }
                if y > 0 {
                    out[(y - 1) * w + x] = true;
                }
                if y + 1 < h {
                    out[(y + 1) * w + x] = true;
                }
            }
        }
    }
    out
}

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

/// Border (in pixels) added around the strong pixels to form the region of interest.
const ROI_PAD: u32 = 60;
/// Result is discarded when it covers more than this fraction of the image.
const MAX_COVERAGE: f32 = 0.10;

#[cfg(test)]
thread_local! {
    /// Largest number of luma values [`PackedLumas::reserve_photos`] reserved on this thread.
    pub(crate) static MAX_RESERVED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

type MagImage = ImageBuffer<Luma<f32>, Vec<f32>>;

/// Per-pixel magnitude of the median (over images) Sobel gradient.
///
/// Each image gets its own 2-D Sobel (as `scipy.ndimage.sobel`, 'reflect' boundary) on its
/// ITU-R 601 luma; gx and gy are then medianed separately across images and combined with
/// `hypot`. Works row by row, so beyond the images it needs only the output and a few rows per
/// image. Errors with [`Error::SizeMismatch`] when sizes differ and [`Error::Image`] for an
/// empty slice.
pub fn median_gradient(images: &[&RgbImage]) -> Result<MagImage> {
    let first = images
        .first()
        .ok_or_else(|| Error::Image("no images for the batch layer".into()))?;
    let (w, h) = first.dimensions();
    if images.iter().any(|i| i.dimensions() != (w, h)) {
        return Err(Error::SizeMismatch);
    }
    let wu = w as usize;
    Ok(median_core(images.len(), w, h, |k, y, dst| {
        luma_row(&images[k].as_raw()[y * wu * 3..(y + 1) * wu * 3], dst);
    }))
}

/// Lumas of the photos of one batch layer, kept at 2 bytes per pixel in one block (a single
/// allocation, so its memory goes back to the system as a whole when dropped): ITU-R 601 luma
/// x 256, rounded. That is at most 1/512 off per pixel and at most 1/64 in a Sobel derivative,
/// far below the thresholds ([`BATCH_LOW`] is 40).
pub(crate) struct PackedLumas {
    w: u32,
    h: u32,
    data: Vec<u16>,
}

impl PackedLumas {
    /// An empty set of `w` x `h` photos. Nothing is reserved here (the size may come from a
    /// forged header); see [`Self::reserve_photos`].
    pub(crate) fn new(w: u32, h: u32) -> Self {
        PackedLumas {
            w,
            h,
            data: Vec::new(),
        }
    }

    /// Reserves room for `n` photos, once, with checked arithmetic; the caller has already
    /// checked that `n` photos fit the memory budget. [`Error::Image`] on overflow or when the
    /// allocation fails.
    pub(crate) fn reserve_photos(&mut self, n: usize) -> Result<()> {
        let total = (self.w as usize)
            .checked_mul(self.h as usize)
            .and_then(|px| px.checked_mul(n))
            .ok_or_else(|| Error::Image("batch layer too large".into()))?;
        self.data
            .try_reserve_exact(total)
            .map_err(|_| Error::Image("batch layer: out of memory".into()))?;
        #[cfg(test)]
        MAX_RESERVED.with(|m| m.set(m.get().max(total)));
        Ok(())
    }

    /// Adds the luma of `img`; [`Error::SizeMismatch`] unless it is `w` x `h`.
    pub(crate) fn push(&mut self, img: &RgbImage) -> Result<()> {
        if img.dimensions() != (self.w, self.h) {
            return Err(Error::SizeMismatch);
        }
        self.data
            .extend(img.as_raw().as_chunks::<3>().0.iter().map(|p| {
                let l1000 = p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114;
                // l1000 / 1000 * 256, rounded; at most 65280.
                ((l1000 * 32 + 62) / 125) as u16
            }));
        Ok(())
    }

    /// Number of photos held.
    pub(crate) fn len(&self) -> usize {
        let px = (self.w as usize).saturating_mul(self.h as usize);
        self.data.len().checked_div(px).unwrap_or(0)
    }
}

/// [`median_gradient`] over packed lumas.
pub(crate) fn median_gradient_packed(lumas: &PackedLumas) -> Result<MagImage> {
    let n = lumas.len();
    if n == 0 {
        return Err(Error::Image("no images for the batch layer".into()));
    }
    let wu = lumas.w as usize;
    let px = wu * lumas.h as usize;
    Ok(median_core(n, lumas.w, lumas.h, |k, y, dst| {
        let row = &lumas.data[k * px + y * wu..k * px + (y + 1) * wu];
        for (d, &v) in dst.iter_mut().zip(row) {
            *d = f32::from(v) / 256.0;
        }
    }))
}

/// The median-gradient computation for `n` images of `w` x `h`; `luma(k, y, dst)` writes row
/// `y` of image `k`'s luma into `dst`.
fn median_core(
    n: usize,
    w: u32,
    h: u32,
    mut luma: impl FnMut(usize, usize, &mut [f32]),
) -> MagImage {
    let (wu, hu) = (w as usize, h as usize);
    let mut out = vec![0f32; wu * hu];
    if wu == 0 || hu == 0 || n == 0 {
        return MagImage::from_raw(w, h, out).expect("sized buffer");
    }
    // The last three luma rows of each image, row r in slot r % 3.
    let mut rows = vec![0f32; n * 3 * wu];
    let slot = |k: usize, r: usize| (k * 3 + r % 3) * wu;
    let mut gx = vec![0f32; n * wu];
    let mut gy = vec![0f32; n * wu];
    let mut vals = vec![0f32; n];
    let mut loaded = 0;
    for k in 0..n {
        let s = slot(k, 0);
        luma(k, 0, &mut rows[s..s + wu]);
    }
    for y in 0..hu {
        // Rows y-1 ..= y+1 with the edge row repeated ('reflect').
        let (up, dn) = (y.saturating_sub(1), (y + 1).min(hu - 1));
        while loaded < dn {
            loaded += 1;
            for k in 0..n {
                let s = slot(k, loaded);
                luma(k, loaded, &mut rows[s..s + wu]);
            }
        }
        for k in 0..n {
            let (u, m, d) = (slot(k, up), slot(k, y), slot(k, dn));
            sobel_row(
                &rows[u..u + wu],
                &rows[m..m + wu],
                &rows[d..d + wu],
                &mut gx[k * wu..(k + 1) * wu],
                &mut gy[k * wu..(k + 1) * wu],
            );
        }
        for (x, o) in out[y * wu..(y + 1) * wu].iter_mut().enumerate() {
            for (k, v) in vals.iter_mut().enumerate() {
                *v = gx[k * wu + x];
            }
            let mx = median(&mut vals);
            for (k, v) in vals.iter_mut().enumerate() {
                *v = gy[k * wu + x];
            }
            let my = median(&mut vals);
            *o = mx.hypot(my);
        }
    }
    MagImage::from_raw(w, h, out).expect("sized buffer")
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
    let n = v.len();
    let (lower, mid, _) = v.select_nth_unstable_by(n / 2, f32::total_cmp);
    let mid = *mid;
    if n % 2 == 1 {
        mid
    } else {
        let below = lower.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        (below + mid) / 2.0
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

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(w: u32, h: u32, seed: u64) -> RgbImage {
        let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        RgbImage::from_fn(w, h, |_, _| {
            let mut px = [0u8; 3];
            for c in &mut px {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                *c = (s >> 56) as u8;
            }
            image::Rgb(px)
        })
    }

    #[test]
    fn packed_luma_matches_full_precision() {
        let imgs: Vec<RgbImage> = (0..5).map(|k| noise(37, 29, k + 1)).collect();
        let refs: Vec<&RgbImage> = imgs.iter().collect();
        let exact = median_gradient(&refs).expect("median");
        let mut packed = PackedLumas::new(37, 29);
        for img in &imgs {
            packed.push(img).expect("same size");
        }
        assert_eq!(packed.len(), 5);
        let got = median_gradient_packed(&packed).expect("median");
        let max = exact
            .as_raw()
            .iter()
            .zip(got.as_raw())
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        // Each Sobel derivative is off by at most 8 x 1/512, the magnitude by at most sqrt(2)x.
        assert!(max <= 0.023, "max abs diff {max}");
        let mut white = PackedLumas::new(1, 1);
        white
            .push(&RgbImage::from_pixel(1, 1, image::Rgb([255, 255, 255])))
            .expect("push");
        assert_eq!(white.data, [65280]);
        assert!(matches!(
            packed.push(&noise(3, 3, 9)),
            Err(Error::SizeMismatch)
        ));
        assert!(median_gradient_packed(&PackedLumas::new(4, 4)).is_err());
    }

    #[test]
    fn packed_lumas_reserves_nothing_for_a_claimed_size() {
        let p = PackedLumas::new(65535, 65535);
        assert_eq!(p.data.capacity(), 0);
        assert_eq!(p.len(), 0);
    }

    #[test]
    fn median_matches_sorting() {
        for n in 1..9 {
            let v: Vec<f32> = (0..n).map(|i| ((i * 37 + 11) % 17) as f32).collect();
            let mut sorted = v.clone();
            sorted.sort_by(f32::total_cmp);
            let want = if n % 2 == 1 {
                sorted[n / 2]
            } else {
                (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
            };
            assert_eq!(median(&mut v.clone()), want, "n {n}");
        }
    }
}

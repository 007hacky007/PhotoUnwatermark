//! Tiled LaMa inpainting: fills the masked area of a photo and pastes the
//! result back over the untouched original with a soft edge.

use std::sync::atomic::{AtomicBool, Ordering};

use image::{RgbImage, imageops};

use crate::engine::Engine;
use crate::mask::{self, Mask};
use crate::tiles::{STRIDE, TILE, tile_starts};
use crate::{Error, Result};

/// Default working long side in pixels; larger photos are inpainted at this size.
pub const WORK_LONG_SIDE: u32 = 4096;

/// Pixels of each tile's border that are only context (the tile fills its central part).
const MARGIN: u32 = (TILE - STRIDE) / 2;
/// Gaussian sigma of the paste edge in pixels.
const EDGE_SIGMA: f32 = 1.5;
/// Truncation radius of the paste-edge Gaussian: the blurred alpha is exactly
/// zero further than this many pixels from any mask pixel.
pub const EDGE_RADIUS: u32 = 2;

/// Progress and cancellation hooks for [`inpaint`].
pub struct Progress<'a> {
    /// Called with the finished fraction (0..1) before the first tile and after each tile; the
    /// last call is always 1.0 (a single 1.0 when there is nothing to fill).
    pub on_fraction: &'a (dyn Fn(f32) + Sync),
    /// Checked before each tile; when set, [`inpaint`] returns [`Error::Cancelled`].
    pub cancel: &'a AtomicBool,
}

fn ignore_fraction(_: f32) {}
static NEVER_CANCELLED: AtomicBool = AtomicBool::new(false);

impl Progress<'static> {
    /// No progress reporting and no cancellation.
    pub fn none() -> Self {
        Progress {
            on_fraction: &ignore_fraction,
            cancel: &NEVER_CANCELLED,
        }
    }
}

/// One tile with holes to fill: crop origin and the region it is responsible for.
struct Step {
    x: u32,
    y: u32,
    /// Region `[x0, x1) x [y0, y1)` in image coordinates.
    region: (u32, u32, u32, u32),
}

fn plan(todo: &[bool], w: u32, h: u32) -> Vec<Step> {
    let mut todo = todo.to_vec();
    let mut steps = Vec::new();
    for y in tile_starts(h, TILE, STRIDE) {
        for x in tile_starts(w, TILE, STRIDE) {
            let y0 = if y == 0 { 0 } else { y + MARGIN };
            let y1 = if y + TILE >= h { h } else { y + TILE - MARGIN };
            let x0 = if x == 0 { 0 } else { x + MARGIN };
            let x1 = if x + TILE >= w { w } else { x + TILE - MARGIN };
            let mut any = false;
            for yy in y0..y1 {
                let row = &mut todo[(yy * w + x0) as usize..(yy * w + x1) as usize];
                for t in row {
                    any |= *t;
                    *t = false;
                }
            }
            if any {
                steps.push(Step {
                    x,
                    y,
                    region: (x0, y0, x1, y1),
                });
            }
        }
    }
    steps
}

/// Tiled LaMa at the image's own size. `img` and `todo` must be at least 512 on each side.
fn fill_tiles(
    engine: &Engine,
    img: &mut RgbImage,
    todo: &mut [bool],
    progress: &Progress,
) -> Result<()> {
    let (w, h) = img.dimensions();
    let steps = plan(todo, w, h);
    if steps.is_empty() {
        (progress.on_fraction)(1.0);
        return Ok(());
    }
    let total = steps.len().max(1) as f32;
    (progress.on_fraction)(0.0);
    let t = TILE as usize;
    for (i, s) in steps.iter().enumerate() {
        if progress.cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let mut in_img = vec![0f32; 3 * t * t];
        let mut in_mask = vec![0f32; t * t];
        for ty in 0..t {
            for tx in 0..t {
                let (px, py) = (s.x + tx as u32, s.y + ty as u32);
                let p = img.get_pixel(px, py).0;
                for c in 0..3 {
                    in_img[c * t * t + ty * t + tx] = p[c] as f32 / 255.0;
                }
                if todo[(py * w + px) as usize] {
                    in_mask[ty * t + tx] = 1.0;
                }
            }
        }
        let out = engine.run_lama(&in_img, &in_mask)?;
        let (x0, y0, x1, y1) = s.region;
        for py in y0..y1 {
            for px in x0..x1 {
                let idx = (py * w + px) as usize;
                if !todo[idx] {
                    continue;
                }
                let o = (py - s.y) as usize * t + (px - s.x) as usize;
                let mut rgb = [0u8; 3];
                for (c, v) in rgb.iter_mut().enumerate() {
                    *v = out[c * t * t + o].clamp(0.0, 255.0).round() as u8;
                }
                img.put_pixel(px, py, image::Rgb(rgb));
                todo[idx] = false;
            }
        }
        (progress.on_fraction)((i + 1) as f32 / total);
    }
    Ok(())
}

/// Inpaint at the given size, padding sides shorter than a tile by edge replication.
fn inpaint_at_size(
    engine: &Engine,
    img: &RgbImage,
    mask: &Mask,
    progress: &Progress,
) -> Result<RgbImage> {
    let (w, h) = img.dimensions();
    let (pw, ph) = (w.max(TILE), h.max(TILE));
    let mut work = if (pw, ph) == (w, h) {
        img.clone()
    } else {
        RgbImage::from_fn(pw, ph, |x, y| *img.get_pixel(x.min(w - 1), y.min(h - 1)))
    };
    let mut todo = vec![false; (pw * ph) as usize];
    for (x, y, p) in mask.enumerate_pixels() {
        todo[(y * pw + x) as usize] = p.0[0] != 0;
    }
    fill_tiles(engine, &mut work, &mut todo, progress)?;
    if (pw, ph) == (w, h) {
        Ok(work)
    } else {
        Ok(imageops::crop_imm(&work, 0, 0, w, h).to_image())
    }
}

/// Alpha in 0..1 over the whole image: 1 on every mask pixel, and the Gaussian-blurred mask
/// (truncated at [`EDGE_RADIUS`]) around it. Only the mask's bounding box grown by the radius
/// is computed; everything else is exactly zero.
fn soft_alpha(mask: &Mask) -> Option<(u32, u32, u32, u32, Vec<f32>)> {
    let bb = mask::bbox(mask)?;
    let r = EDGE_RADIUS as i64;
    let (w, h) = (mask.width() as i64, mask.height() as i64);
    let x0 = (bb.left() as i64 - r).max(0);
    let y0 = (bb.top() as i64 - r).max(0);
    let x1 = (bb.right() as i64 + 1 + r).min(w);
    let y1 = (bb.bottom() as i64 + 1 + r).min(h);
    let (bw, bh) = ((x1 - x0) as usize, (y1 - y0) as usize);

    let mut kernel: Vec<f32> = (-r..=r)
        .map(|d| (-((d * d) as f32) / (2.0 * EDGE_SIGMA * EDGE_SIGMA)).exp())
        .collect();
    let sum: f32 = kernel.iter().sum();
    kernel.iter_mut().for_each(|k| *k /= sum);

    // Horizontal pass; samples outside the image count as zero.
    let mut tmp = vec![0f32; bw * bh];
    for y in 0..bh {
        for x in 0..bw {
            let mut acc = 0.0;
            for (i, k) in kernel.iter().enumerate() {
                let sx = x0 + x as i64 + i as i64 - r;
                if (0..w).contains(&sx)
                    && mask.get_pixel(sx as u32, (y0 + y as i64) as u32).0[0] != 0
                {
                    acc += k;
                }
            }
            tmp[y * bw + x] = acc;
        }
    }
    // Vertical pass.
    let mut out = vec![0f32; bw * bh];
    for y in 0..bh {
        for x in 0..bw {
            let mut acc = 0.0;
            for (i, k) in kernel.iter().enumerate() {
                let sy = y as i64 + i as i64 - r;
                if (0..bh as i64).contains(&sy) {
                    acc += k * tmp[sy as usize * bw + x];
                }
            }
            let (mx, my) = ((x0 + x as i64) as u32, (y0 + y as i64) as u32);
            out[y * bw + x] = if mask.get_pixel(mx, my).0[0] != 0 {
                1.0
            } else {
                acc.min(1.0)
            };
        }
    }
    Some((x0 as u32, y0 as u32, bw as u32, bh as u32, out))
}

/// Remove the masked area of `img` with LaMa.
///
/// `mask` marks the hole with any non-zero value and must match the size of
/// `img`. Photos whose long side exceeds `work_long_side` are inpainted at
/// that size and the result is scaled back up; the mask is shrunk with
/// [`mask::resize_any`] and grown by one working pixel, so a hole thinner
/// than a working pixel is still filled. The fill replaces every mask pixel and is blended into the
/// original around it with the mask blurred by a Gaussian (sigma 1.5,
/// truncated at [`EDGE_RADIUS`]), so pixels further than that from the mask
/// are returned unchanged. An empty mask returns the input without running
/// the model. Progress always ends with 1.0.
pub fn inpaint(
    engine: &Engine,
    img: &RgbImage,
    mask: &Mask,
    work_long_side: Option<u32>,
    progress: Progress,
) -> Result<RgbImage> {
    if img.dimensions() != mask.dimensions() {
        return Err(Error::SizeMismatch);
    }
    let (w, h) = img.dimensions();
    let Some((ax, ay, aw, ah, alpha)) = soft_alpha(mask) else {
        (progress.on_fraction)(1.0);
        return Ok(img.clone());
    };

    let long = w.max(h);
    let filled = match work_long_side {
        Some(l) if l > 0 && long > l => {
            let scale = l as f64 / long as f64;
            let sw = ((w as f64 * scale).round() as u32).max(1);
            let sh = ((h as f64 * scale).round() as u32).max(1);
            let small = imageops::resize(img, sw, sh, imageops::FilterType::Lanczos3);
            // Any mask pixel under a working pixel makes it a hole, plus one working pixel
            // around: resampling spreads the mark into the neighbours of the hole.
            let small_mask = mask::dilate(&mask::resize_any(mask, sw, sh), 1);
            let done = inpaint_at_size(engine, &small, &small_mask, &progress)?;
            imageops::resize(&done, w, h, imageops::FilterType::Lanczos3)
        }
        _ => inpaint_at_size(engine, img, mask, &progress)?,
    };

    let mut out = img.clone();
    for y in 0..ah {
        for x in 0..aw {
            let a = alpha[(y * aw + x) as usize];
            if a <= 0.0 {
                continue;
            }
            let (px, py) = (ax + x, ay + y);
            let o = img.get_pixel(px, py).0;
            let f = filled.get_pixel(px, py).0;
            let mut rgb = [0u8; 3];
            for c in 0..3 {
                let v = o[c] as f32 * (1.0 - a) + f[c] as f32 * a;
                rgb[c] = v.round().clamp(0.0, 255.0) as u8;
            }
            out.put_pixel(px, py, image::Rgb(rgb));
        }
    }
    Ok(out)
}

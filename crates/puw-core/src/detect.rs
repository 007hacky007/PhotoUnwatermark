//! Automatic first guess of the watermark area.
//!
//! The photo is resized to several long sides, cut into 512 px tiles, run through the
//! segmenter, and the per-scale probability maps vote on which pixels are watermark.

use image::imageops::{self, FilterType};
use image::{ImageBuffer, Luma, RgbImage};

use crate::Result;
use crate::engine::Engine;
use crate::mask::Mask;
use crate::tiles::{STRIDE, TILE, tile_starts};

const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];

/// Scales whose long side exceeds the photo's long side by more than this factor are skipped.
const MAX_UPSCALE: f32 = 1.5;

/// Probability map, one `f32` per pixel.
pub type ProbMap = ImageBuffer<Luma<f32>, Vec<f32>>;

/// Settings of [`guess_single`].
#[derive(Debug, Clone)]
pub struct GuessOptions {
    /// Long sides the photo is analysed at.
    pub long_sides: [u32; 3],
    /// A pixel votes for the watermark when its probability exceeds this.
    pub threshold: f32,
    /// Number of scales that must agree.
    pub votes: usize,
    /// Growth radius as a fraction of the long side, applied by the caller.
    pub grow_frac: f32,
}

impl Default for GuessOptions {
    fn default() -> Self {
        Self {
            long_sides: [768, 1280, 2048],
            threshold: 0.5,
            votes: 2,
            grow_frac: 0.004,
        }
    }
}

/// Result of [`guess_single`].
#[derive(Debug, Clone)]
pub struct Guess {
    /// Voted mask, 255 = watermark.
    pub mask: Mask,
    /// Mean over mask pixels of the per-pixel max probability across scales; 0 when empty.
    pub confidence: f32,
}

/// Watermark probability for every pixel of `img`, analysed at `long_side`.
///
/// The photo is resized so its long side is `long_side`, cut into 512 px tiles with stride
/// 384, and the per-pixel max over overlapping tiles is resized back to the photo size.
/// Tiles narrower or shorter than 512 are padded by edge replication.
pub fn predict_prob(engine: &Engine, img: &RgbImage, long_side: u32) -> Result<ProbMap> {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Ok(ProbMap::new(w, h));
    }
    let scale = long_side as f32 / w.max(h) as f32;
    let sw = ((w as f32 * scale).round() as u32).max(1);
    let sh = ((h as f32 * scale).round() as u32).max(1);
    let scaled = imageops::resize(img, sw, sh, FilterType::Lanczos3);

    let mut acc = ProbMap::new(sw, sh);
    let xs = tile_starts(sw, TILE, STRIDE);
    let ys = tile_starts(sh, TILE, STRIDE);
    let plane = (TILE * TILE) as usize;
    let mut input = vec![0.0f32; 3 * plane];
    for &y0 in &ys {
        for &x0 in &xs {
            let tw = TILE.min(sw - x0);
            let th = TILE.min(sh - y0);
            for ty in 0..TILE {
                let sy = y0 + ty.min(th - 1);
                for tx in 0..TILE {
                    let px = scaled.get_pixel(x0 + tx.min(tw - 1), sy);
                    let i = (ty * TILE + tx) as usize;
                    for c in 0..3 {
                        input[c * plane + i] = (f32::from(px[c]) / 255.0 - MEAN[c]) / STD[c];
                    }
                }
            }
            let out = engine.run_segmenter(&input)?;
            for ty in 0..th {
                for tx in 0..tw {
                    let p = out[(ty * TILE + tx) as usize];
                    let dst = acc.get_pixel_mut(x0 + tx, y0 + ty);
                    if p > dst.0[0] {
                        dst.0[0] = p;
                    }
                }
            }
        }
    }
    if (sw, sh) == (w, h) {
        return Ok(acc);
    }
    Ok(imageops::resize(&acc, w, h, FilterType::Triangle))
}

/// Guess the watermark area of one photo by voting across scales.
///
/// No face guard and no growth are applied. Scales above 1.5x the photo's long side are
/// skipped (when all would be, the smallest is used), and the number of required votes is
/// `min(votes, scales used)`.
pub fn guess_single(engine: &Engine, img: &RgbImage, opts: &GuessOptions) -> Result<Guess> {
    let (w, h) = img.dimensions();
    let long = w.max(h);
    let mut sides: Vec<u32> = opts
        .long_sides
        .iter()
        .copied()
        .filter(|&l| l as f32 <= long as f32 * MAX_UPSCALE)
        .collect();
    if sides.is_empty()
        && let Some(&smallest) = opts.long_sides.iter().min()
    {
        sides.push(smallest);
    }
    let needed = opts.votes.min(sides.len());

    let n = w as usize * h as usize;
    let mut votes = vec![0usize; n];
    let mut best = vec![0.0f32; n];
    for &l in &sides {
        let prob = predict_prob(engine, img, l)?;
        for (i, p) in prob.as_raw().iter().enumerate() {
            if *p > opts.threshold {
                votes[i] += 1;
            }
            if *p > best[i] {
                best[i] = *p;
            }
        }
    }

    let mut mask = Mask::new(w, h);
    let mut sum = 0.0f64;
    let mut count = 0u64;
    for (i, m) in mask.as_mut().iter_mut().enumerate() {
        if needed > 0 && votes[i] >= needed {
            *m = 255;
            sum += f64::from(best[i]);
            count += 1;
        }
    }
    let confidence = if count == 0 {
        0.0
    } else {
        (sum / count as f64) as f32
    };
    Ok(Guess { mask, confidence })
}

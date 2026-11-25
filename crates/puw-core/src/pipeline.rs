//! The engine's workflow as the app uses it: the batch layers of a set, the automatic first
//! guess per photo, the photos to pick the editor's first photo from, and removing a confirmed
//! area from one photo.
//!
//! A set is never decoded at once. [`build_set_layers`] reads the photos one at a time (a
//! bounded sample of each size group), [`guess_photo`] works on one loaded photo, and
//! [`representative_sample`] bounds how many photos are guessed before the editor opens:
//!
//! ```no_run
//! # use std::path::PathBuf;
//! # use puw_core::{Engine, GuessOptions, Progress, load};
//! # use puw_core::pipeline::{build_set_layers, guess_photo, pick_representative,
//! #     representative_sample};
//! # fn demo(engine: &Engine, paths: &[PathBuf]) -> puw_core::Result<()> {
//! let layers = build_set_layers(paths, &Progress::none())?;
//! let sample = representative_sample(paths.len());
//! let mut guesses = Vec::new();
//! for &i in &sample {
//!     let photo = load(&paths[i])?;
//!     guesses.push(guess_photo(engine, &photo, &layers, &GuessOptions::default())?);
//! }
//! let first = sample[pick_representative(&guesses)];
//! # let _ = first;
//! # Ok(())
//! # }
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use crate::batch::{
    BATCH_HIGH, BATCH_LOW, PackedLumas, batch_lines, filter_layer, median_gradient_packed,
};
use crate::detect::{Guess, GuessOptions, guess_single};
use crate::engine::Engine;
use crate::faces::{Face, detect_faces_near, drop_eye_components, guard};
use crate::image_io::{LoadedPhoto, load, save_result, upright_size};
use crate::inpaint::{Progress, inpaint};
use crate::mask::{self, Mask, is_empty};
use crate::template::{
    MatchBasis, MatchResult, MatchStatus, WatermarkTemplate, find, saved_placement, saved_score,
};
use crate::{Error, Result};
use image::RgbImage;

/// Photos of one size needed before the batch layer is computed for them.
pub const MIN_BATCH_PHOTOS: usize = 3;
/// Most photos of one size group that go into its batch layer.
pub const BATCH_SAMPLE_CAP: usize = 24;
/// Memory for the photos of one batch layer, which are held as 2-byte luma while the median
/// is taken: the sample is cut to fit (16 photos at 36 MP). A size whose
/// [`MIN_BATCH_PHOTOS`] photos would not fit gets no batch layer. [`build_set_layers`] peaks at
/// about this plus 0.4 GB at 36 MP (measured 1.53 GB for 40 and for 200 photos of 7360x4912).
pub const BATCH_LUMA_BUDGET: u64 = 1_200_000_000;
/// Most photos [`representative_sample`] picks.
pub const REPRESENTATIVE_CAP: usize = 3;

/// The batch layers of a set: for each upright photo size whose group got one, the mask of
/// the mark stamped at the same place on every photo.
#[derive(Debug, Clone, Default)]
pub struct SetLayers {
    layers: BTreeMap<(u32, u32), Mask>,
}

impl SetLayers {
    /// No layers (a set too small or too mixed for one).
    pub fn none() -> Self {
        Self::default()
    }

    /// The batch layer for photos of upright size `w` x `h`, if their group has one.
    pub fn get(&self, w: u32, h: u32) -> Option<&Mask> {
        self.layers.get(&(w, h))
    }

    /// Sizes that have a layer, with the layer, smallest size first.
    pub fn iter(&self) -> impl Iterator<Item = ((u32, u32), &Mask)> {
        self.layers.iter().map(|(&size, m)| (size, m))
    }

    /// True when no size group has a layer.
    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }
}

/// `cap` evenly spaced indexes into `0..n` (the middle of each of `cap` equal parts), or all
/// of them when `n <= cap`. Ascending and distinct.
fn evenly_spaced(n: usize, cap: usize) -> Vec<usize> {
    if n <= cap {
        return (0..n).collect();
    }
    (0..cap).map(|k| (2 * k + 1) * n / (2 * cap)).collect()
}

/// Indexes of the photos (of a set of `n`) to run [`guess_photo`] on before the editor opens:
/// at most [`REPRESENTATIVE_CAP`], evenly spaced, ascending. [`pick_representative`] over their
/// guesses gives a position in this list.
pub fn representative_sample(n: usize) -> Vec<usize> {
    evenly_spaced(n, REPRESENTATIVE_CAP)
}

/// Photos of one `w` x `h` group that go into its batch layer: as many as
/// [`BATCH_LUMA_BUDGET`] holds at 2 bytes per pixel, at most [`BATCH_SAMPLE_CAP`]. 0 when not
/// even [`MIN_BATCH_PHOTOS`] fit: such a group is skipped, never clamped up.
fn batch_sample_len(w: u32, h: u32) -> usize {
    let Some(per_photo) = 2u64
        .checked_mul(u64::from(w))
        .and_then(|v| v.checked_mul(u64::from(h)))
    else {
        return 0;
    };
    let per_photo = per_photo.max(1);
    let n = (BATCH_LUMA_BUDGET / per_photo).min(BATCH_SAMPLE_CAP as u64) as usize;
    if n < MIN_BATCH_PHOTOS { 0 } else { n }
}

/// Progress in work units, reported as a fraction.
struct Units<'a> {
    progress: &'a Progress<'a>,
    done: usize,
    total: usize,
}

impl Units<'_> {
    fn check(&self) -> Result<()> {
        if self.progress.cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        Ok(())
    }

    fn add(&mut self, n: usize) {
        self.done += n;
        (self.progress.on_fraction)((self.done as f32 / self.total.max(1) as f32).min(1.0));
    }
}

/// Work units of reading one photo's header, of decoding one sampled photo, and of its share of
/// the median.
const HEADER_UNITS: usize = 1;
const DECODE_UNITS: usize = 8;
const MEDIAN_UNITS: usize = 4;

/// Batch layers of the photos at `paths`, one per upright size group of at least
/// [`MIN_BATCH_PHOTOS`] photos.
///
/// Photos are read one at a time: first every photo's header for its upright size, then, per
/// group, an evenly spaced sample (see [`BATCH_SAMPLE_CAP`] and [`BATCH_LUMA_BUDGET`]) is
/// decoded and kept as luma for [`crate::batch::median_gradient`]'s median, followed by
/// [`batch_lines`] with [`BATCH_LOW`] / [`BATCH_HIGH`] and [`filter_layer`]. Peak memory is
/// bounded by the sample (see [`BATCH_LUMA_BUDGET`]), whatever the size of the set.
///
/// A group whose size is so large that [`MIN_BATCH_PHOTOS`] photos exceed [`BATCH_LUMA_BUDGET`]
/// (sizes come from file headers, which may lie) has no layer either (logged). Memory is
/// claimed only as photos decode.
///
/// A photo that cannot be read is left out (logged); a group with fewer than
/// [`MIN_BATCH_PHOTOS`] readable photos in its sample, or whose layer finds nothing (or nothing
/// that survives [`filter_layer`]), has no layer, so a mixed or damaged set never fails here.
/// `progress` gets fractions from 0.0 to 1.0, and its cancel flag is checked before each photo
/// and each median; when set, this returns [`Error::Cancelled`].
pub fn build_set_layers(paths: &[PathBuf], progress: &Progress) -> Result<SetLayers> {
    let mut units = Units {
        progress,
        done: 0,
        total: paths.len() * HEADER_UNITS,
    };
    (progress.on_fraction)(0.0);
    let mut groups: BTreeMap<(u32, u32), Vec<usize>> = BTreeMap::new();
    for (i, path) in paths.iter().enumerate() {
        units.check()?;
        match upright_size(path) {
            Ok(size) => groups.entry(size).or_default().push(i),
            Err(e) => tracing::warn!("batch layer: photo {} skipped: {e}", i + 1),
        }
    }
    let samples: Vec<((u32, u32), Vec<usize>)> = groups
        .into_iter()
        .filter(|(_, members)| members.len() >= MIN_BATCH_PHOTOS)
        .filter_map(|((w, h), members)| {
            let n = batch_sample_len(w, h);
            if n == 0 {
                tracing::warn!("batch layer skipped for the {w}x{h} photos: too large");
                return None;
            }
            let pick = evenly_spaced(members.len(), n);
            Some(((w, h), pick.into_iter().map(|k| members[k]).collect()))
        })
        .collect();
    units.total += samples
        .iter()
        .map(|(_, s)| s.len() * (DECODE_UNITS + MEDIAN_UNITS))
        .sum::<usize>();
    units.add(paths.len() * HEADER_UNITS);

    let mut layers = BTreeMap::new();
    for ((w, h), sample) in samples {
        let mut lumas = PackedLumas::new(w, h);
        // The sample was cut to fit BATCH_LUMA_BUDGET, so this is bounded by it.
        if let Err(e) = lumas.reserve_photos(sample.len()) {
            tracing::warn!("batch layer skipped for the {w}x{h} photos: {e}");
            units.add(sample.len() * (DECODE_UNITS + MEDIAN_UNITS));
            continue;
        }
        for &i in &sample {
            units.check()?;
            match load(&paths[i]) {
                Ok(photo) => {
                    if lumas.push(&photo.rgb).is_err() {
                        tracing::warn!("batch layer: photo {} changed size, skipped", i + 1);
                    }
                }
                Err(e) => tracing::warn!("batch layer: photo {} skipped: {e}", i + 1),
            }
            units.add(DECODE_UNITS);
        }
        units.check()?;
        if lumas.len() >= MIN_BATCH_PHOTOS {
            let mag = median_gradient_packed(&lumas);
            drop(lumas);
            match mag {
                Ok(mag) => {
                    let layer = batch_lines(&mag, BATCH_LOW, BATCH_HIGH)
                        .map(|l| filter_layer(&l))
                        .filter(|l| !is_empty(l));
                    if let Some(layer) = layer {
                        layers.insert((w, h), layer);
                    }
                }
                Err(e) => tracing::warn!("batch layer skipped for the {w}x{h} photos: {e}"),
            }
        }
        units.add(sample.len() * MEDIAN_UNITS);
    }
    (progress.on_fraction)(1.0);
    Ok(SetLayers { layers })
}

/// Share of the saved placement's pixels that must lie in the grown batch layer.
const PRIOR_SUPPORT: f64 = 0.5;
/// Growth of the batch layer for the support check, as a fraction of the long side.
const PRIOR_GROW: f32 = 0.004;
/// Score at the saved position (masked NCC, +-2 px, scale 1) for `Found` on set support.
/// Calibrated on the synthetic fixed set with its own layer and the template of photo 0:
/// photos without the mark (the clean/ originals other than the reference's scene, and the
/// negative set resized to 1536x1024) score 0.00-0.155, the 7 other faint (alpha 0.12) marked
/// photos 0.175-0.331. See `docs/notes/thresholds.md`.
pub const PRIOR_FOUND: f32 = 0.28;
/// Score at the saved position for `Unsure` on set support (the user opens the photo).
pub const PRIOR_UNSURE: f32 = 0.16;

/// [`find`] with a position prior for marks stamped at the same place on every photo.
///
/// When `find` does not return `Found`, `img` has exactly the template's reference size and
/// `layers` has a layer for that size, the template's pieces are placed at their saved
/// position and scale 1. If at least half of those pixels lie inside the layer grown by
/// [`mask::grow_for`] (0.004), the photo itself must also show the mark there: the template's
/// score at the saved position (the fast path of [`find`], +-2 px) decides. At least
/// [`PRIOR_FOUND`] gives `Found`, at least [`PRIOR_UNSURE`] gives `Unsure`, both with
/// [`MatchBasis::SetPosition`], that score as the confidence and the saved placement as the
/// mask. In every other case the `find` result is returned as is.
pub fn match_photo(t: &WatermarkTemplate, img: &RgbImage, layers: &SetLayers) -> MatchResult {
    let found = find(t, img);
    let (w, h) = img.dimensions();
    if found.status == MatchStatus::Found || (w, h) != (t.ref_w, t.ref_h) {
        return found;
    }
    let Some(layer) = layers.get(w, h) else {
        return found;
    };
    let Some(placed) = saved_placement(t) else {
        return found;
    };
    let grown = mask::dilate(layer, mask::grow_for(w, h, PRIOR_GROW));
    let (mut total, mut inside) = (0u64, 0u64);
    for (p, g) in placed.pixels().zip(grown.pixels()) {
        if p.0[0] != 0 {
            total += 1;
            inside += (g.0[0] != 0) as u64;
        }
    }
    if total == 0 || (inside as f64) < PRIOR_SUPPORT * total as f64 {
        return found;
    }
    let Some(score) = saved_score(t, img) else {
        return found;
    };
    let status = if score >= PRIOR_FOUND {
        MatchStatus::Found
    } else if score >= PRIOR_UNSURE {
        MatchStatus::Unsure
    } else {
        return found;
    };
    MatchResult {
        status,
        confidence: score,
        mask: mask::binarize(&placed),
        basis: MatchBasis::SetPosition,
    }
}

/// How near, as a share of the photo's long side, the detector must come to a pixel of the
/// batch layer to count as seeing it, and a detector component to the layer to count as part
/// of the mark it holds: the distance at which components join one piece of a mark in
/// [`filter_layer`] and in the template (2 %, 30 px at 1536). The layer is the median edge of
/// the set, the detector's area is the mark's fill grown a little, and the two can lie that
/// far apart around a thick stroke.
const LAYER_NEAR_FRAC: f32 = 0.02;
/// Share of the batch layer the detector must see for the layer to be taken as the mark
/// itself, so that only detector components near it are kept. Synthetic sets: 1.0 on every
/// photo of the fixed set (its layer is the mark); 0.0-1.0, mostly below 0.6, where the layer
/// holds static scene edges and the marks move (random, styles).
const LAYER_SEEN: f64 = 0.9;
/// Otherwise detector components are grouped like the pieces of a template (boxes closer than
/// [`LAYER_NEAR_FRAC`] of the long side) and a group with less than this share of the largest
/// group's pixels is dropped as a speck. Tiled text is many groups of about the same size, so
/// it stays. Components near the batch layer are kept whatever their size.
const SPECK_FRAC: f64 = 0.1;

/// The parts of the detector's `guess` that are the mark, without the specks it also finds on
/// fur, foliage and stars. See [`LAYER_NEAR_FRAC`], [`LAYER_SEEN`] and [`SPECK_FRAC`]:
/// - with a non-empty batch `layer` (same size) that the detector sees almost all of, only the
///   components near the layer are kept;
/// - otherwise the components near the layer (if any) and every group of at least
///   [`SPECK_FRAC`] of the largest group are kept.
pub fn keep_mark_parts(guess: &Mask, layer: Option<&Mask>) -> Mask {
    let (w, h) = guess.dimensions();
    let comps = mask::components(guess);
    if comps.len() <= 1 {
        return guess.clone();
    }
    let long = w.max(h) as f32;
    let near = (LAYER_NEAR_FRAC * long).round() as u32;
    let layer = layer.filter(|l| l.dimensions() == (w, h) && !is_empty(l));
    let mut keep = vec![false; comps.len()];
    let mut seen = false;
    if let Some(l) = layer {
        let around = mask::dilate(l, near);
        for (k, c) in keep.iter_mut().zip(&comps) {
            let (ox, oy) = (c.bbox.left() as u32, c.bbox.top() as u32);
            *k = c
                .mask
                .enumerate_pixels()
                .any(|(x, y, p)| p.0[0] != 0 && around.get_pixel(ox + x, oy + y).0[0] != 0);
        }
        drop(around);
        let reach = mask::dilate(guess, near);
        let (mut all, mut hit) = (0u64, 0u64);
        for (p, r) in l.as_raw().iter().zip(reach.as_raw()) {
            if *p != 0 {
                all += 1;
                hit += (*r != 0) as u64;
            }
        }
        seen = hit as f64 >= LAYER_SEEN * all as f64;
    }
    if !seen {
        let groups = mask::group_near(&comps, LAYER_NEAR_FRAC * long);
        let sizes: Vec<u64> = groups
            .iter()
            .map(|g| g.iter().map(|&i| u64::from(comps[i].pixels)).sum())
            .collect();
        let most = sizes.iter().copied().max().unwrap_or(0);
        for (g, &size) in groups.iter().zip(&sizes) {
            if size as f64 >= SPECK_FRAC * most as f64 {
                g.iter().for_each(|&i| keep[i] = true);
            }
        }
    }
    let mut out = Mask::new(w, h);
    for c in comps.iter().zip(&keep).filter(|(_, k)| **k).map(|(c, _)| c) {
        let (ox, oy) = (c.bbox.left() as u32, c.bbox.top() as u32);
        for (x, y, p) in c.mask.enumerate_pixels() {
            if p.0[0] != 0 {
                out.put_pixel(ox + x, oy + y, *p);
            }
        }
    }
    out
}

/// Automatic first guess for one photo.
///
/// [`guess_single`] with its specks dropped ([`keep_mark_parts`], which keeps only what is near
/// the batch layer when the layer is the mark), then the face guard ([`detect_faces_near`]),
/// the union with the batch layer of the photo's size from `layers` (if any) and growth by
/// [`mask::grow_for`] with `opts.grow_frac`. The guard runs before growth ([`drop_eye_components`]) and after it
/// ([`guard`]), so growth never fills an eye, and the batch layer is the only thing it keeps
/// in an eye.
///
/// `confidence` is the one from [`guess_single`] when its guess survives the face guard, 1.0
/// when only the batch layer contributed, and 0.0 when the mask is empty.
pub fn guess_photo(
    engine: &Engine,
    photo: &LoadedPhoto,
    layers: &SetLayers,
    opts: &GuessOptions,
) -> Result<Guess> {
    let img = &photo.rgb;
    let (w, h) = img.dimensions();
    let layer = layers.get(w, h);
    let single = guess_single(engine, img, opts)?;
    let single = Guess {
        mask: keep_mark_parts(&single.mask, layer),
        ..single
    };
    let near = match layer {
        Some(l) => mask::union(&single.mask, l),
        None => single.mask.clone(),
    };
    let grow = mask::grow_for(w, h, opts.grow_frac);
    // The guard only clears pixels, so an empty mask needs no face detection.
    let faces = if is_empty(&near) {
        Vec::new()
    } else {
        detect_faces_near(engine, img, &near, grow)?
    };
    drop(near);
    Ok(finish(single, &faces, layer, grow))
}

/// Face guard, batch layer and growth applied to the detector's guess, in this order:
/// 1. guessed components reaching an eye are dropped unless the batch layer supports them
///    ([`drop_eye_components`]);
/// 2. the batch layer is added;
/// 3. the mask grows by `grow`;
/// 4. the eye circles are cleared again except the batch layer's own pixels ([`guard`]), so
///    growth never fills an eye.
///
/// `confidence` is the detector's when its guess survives step 1, 1.0 when only the batch
/// layer contributed, and 0.0 when the mask is empty.
fn finish(single: Guess, faces: &[Face], layer: Option<&Mask>, grow: u32) -> Guess {
    let kept = drop_eye_components(&single.mask, faces, layer);
    let from_single = !is_empty(&kept);
    let combined = match layer {
        Some(l) => mask::union(&kept, l),
        None => kept,
    };
    let mask = guard(&mask::dilate(&combined, grow), faces, layer);
    let confidence = if is_empty(&mask) {
        0.0
    } else if from_single {
        single.confidence
    } else {
        1.0
    };
    Guess { mask, confidence }
}

/// Index of the photo to show first in the editor: the highest confidence among guesses with a
/// non-empty mask (for guesses of a [`representative_sample`], a position in that sample). Equal confidences go to the earlier photo; with no non-empty guess (or no
/// guesses at all) the answer is 0.
pub fn pick_representative(guesses: &[Guess]) -> usize {
    let mut best: Option<(usize, f32)> = None;
    for (i, g) in guesses.iter().enumerate() {
        if is_empty(&g.mask) {
            continue;
        }
        if best.is_none_or(|(_, c)| g.confidence > c) {
            best = Some((i, g.confidence));
        }
    }
    best.map_or(0, |(i, _)| i)
}

/// Removes the area `mask` marks (painted on the upright `photo.rgb`, same size) and saves the
/// result into `out_dir` with [`save_result`]: same name and format, a new name instead of
/// replacing any existing file, orientation normalised to 1. Returns the path written.
///
/// Nothing is read from disk, and nothing is written anywhere but `out_dir`; the original file
/// is never opened for writing. `work_long_side` and `progress` are passed to [`inpaint`].
pub fn remove_one(
    engine: &Engine,
    photo: &LoadedPhoto,
    mask: &Mask,
    out_dir: &Path,
    work_long_side: Option<u32>,
    progress: Progress,
) -> Result<PathBuf> {
    let filled = inpaint(engine, &photo.rgb, mask, work_long_side, progress)?;
    save_result(photo, &filled, out_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Luma;
    use imageproc::rect::Rect;

    /// A 100 px wide face with its right eye at `eye` (radius 12) and the left one 40 px right.
    fn face_at(eye: (f32, f32)) -> Face {
        Face {
            bbox: Rect::at(eye.0 as i32 - 30, eye.1 as i32 - 40).of_size(100, 120),
            width: 100.0,
            eyes: [eye, (eye.0 + 40.0, eye.1)],
            score: 0.9,
        }
    }

    fn dist(x: u32, y: u32, c: (f32, f32)) -> f32 {
        (x as f32 - c.0).hypot(y as f32 - c.1)
    }

    /// Sets the pixels whose distance from `c` is in `r0..=r1`.
    fn ring(m: &mut Mask, c: (f32, f32), r0: f32, r1: f32) {
        let (w, h) = m.dimensions();
        for y in 0..h {
            for x in 0..w {
                if (r0..=r1).contains(&dist(x, y, c)) {
                    m.put_pixel(x, y, Luma([255]));
                }
            }
        }
    }

    /// Set pixels within the eye circles of `faces`.
    fn in_eyes(m: &Mask, faces: &[Face]) -> Vec<(u32, u32)> {
        m.enumerate_pixels()
            .filter(|(x, y, p)| {
                p.0[0] != 0
                    && faces
                        .iter()
                        .any(|f| f.eyes.iter().any(|&e| dist(*x, *y, e) <= 0.12 * f.width))
            })
            .map(|(x, y, _)| (x, y))
            .collect()
    }

    /// Sets the pixels of the rectangle `x0..x1` x `y0..y1`.
    fn rect(m: &mut Mask, x0: u32, y0: u32, x1: u32, y1: u32) {
        for y in y0..y1 {
            for x in x0..x1 {
                m.put_pixel(x, y, Luma([255]));
            }
        }
    }

    fn set_at(m: &Mask, x: u32, y: u32) -> bool {
        m.get_pixel(x, y).0[0] != 0
    }

    /// 1000 x 600 (near = 20 px): a mark of 6 letters at y 400..430 and specks all over.
    fn mark_and_specks() -> (Mask, Mask) {
        let mut guess = Mask::new(1000, 600);
        for k in 0..6 {
            rect(&mut guess, 300 + 40 * k, 400, 330 + 40 * k, 430);
        }
        // Specks: far from the mark, each a few pixels, and one big blob of scene.
        for (x, y) in [(50, 50), (900, 80), (120, 300), (700, 200), (950, 550)] {
            rect(&mut guess, x, y, x + 4, y + 4);
        }
        let mut layer = Mask::new(1000, 600);
        for k in 0..6 {
            // The layer holds the letters' edges, a little off the detector's fill.
            rect(&mut layer, 302 + 40 * k, 398, 328 + 40 * k, 400);
        }
        (guess, layer)
    }

    #[test]
    fn a_layer_the_detector_sees_keeps_only_what_is_near_it() {
        let (mut guess, layer) = mark_and_specks();
        // A large blob far away goes too: the layer says where the mark is.
        rect(&mut guess, 600, 20, 700, 60);
        // A part 14 px above the layer, touching neither it nor the mark, is within reach.
        rect(&mut guess, 300, 380, 320, 384);
        let kept = keep_mark_parts(&guess, Some(&layer));
        assert!(
            set_at(&kept, 310, 410) && set_at(&kept, 510, 410),
            "mark lost"
        );
        assert!(set_at(&kept, 305, 382), "near part lost");
        for (x, y) in [(51, 51), (901, 81), (650, 40), (951, 551)] {
            assert!(!set_at(&kept, x, y), "speck at {x},{y} kept");
        }
    }

    #[test]
    fn a_layer_of_scene_keeps_large_parts_and_drops_specks() {
        let (guess, _) = mark_and_specks();
        // A layer far from anything the detector found (a static scene edge).
        let mut layer = Mask::new(1000, 600);
        rect(&mut layer, 10, 580, 400, 584);
        let kept = keep_mark_parts(&guess, Some(&layer));
        assert!(
            set_at(&kept, 310, 410) && set_at(&kept, 510, 410),
            "mark lost"
        );
        for (x, y) in [(51, 51), (901, 81), (121, 301)] {
            assert!(!set_at(&kept, x, y), "speck at {x},{y} kept");
        }
        // Without any layer it is the same.
        assert_eq!(keep_mark_parts(&guess, None), kept);
        // An empty layer counts as none.
        assert_eq!(keep_mark_parts(&guess, Some(&Mask::new(1000, 600))), kept);
    }

    #[test]
    fn tiled_marks_survive_without_a_layer() {
        let mut guess = Mask::new(1200, 900);
        // 12 tiles of text of slightly different sizes, far apart.
        for (i, (tx, ty)) in (0..4)
            .flat_map(|c| (0..3).map(move |r| (40 + 290 * c, 60 + 290 * r)))
            .enumerate()
        {
            for k in 0..4 {
                let x = tx + 30 * k;
                rect(&mut guess, x, ty, x + 20, ty + 25 + (i as u32 % 5));
            }
        }
        rect(&mut guess, 1180, 880, 1183, 883);
        let kept = keep_mark_parts(&guess, None);
        let tiles = (0..4)
            .flat_map(|c| (0..3).map(move |r| (45 + 290 * c, 65 + 290 * r)))
            .filter(|&(x, y)| set_at(&kept, x, y))
            .count();
        assert_eq!(tiles, 12);
        assert!(!set_at(&kept, 1181, 881));
    }

    #[test]
    fn evenly_spaced_is_bounded_and_spread() {
        assert_eq!(evenly_spaced(0, 6), Vec::<usize>::new());
        assert_eq!(evenly_spaced(4, 6), [0, 1, 2, 3]);
        assert_eq!(evenly_spaced(12, 6), [1, 3, 5, 7, 9, 11]);
        for n in 7..300 {
            let v = evenly_spaced(n, 6);
            assert_eq!(v.len(), 6, "n {n}");
            assert!(v.windows(2).all(|p| p[0] < p[1]), "n {n}: {v:?}");
            assert!(*v.last().unwrap() < n);
            // Spread over the whole set: within one step of either end.
            assert!(v[0] <= n / 6 && n - 1 - v[5] <= n / 6, "n {n}: {v:?}");
        }
        assert_eq!(representative_sample(200).len(), REPRESENTATIVE_CAP);
    }

    #[test]
    fn batch_sample_fits_the_budget() {
        // 36 MP: 16 photos of 2 bytes per pixel, 1.16 GB.
        assert_eq!(batch_sample_len(7360, 4912), 16);
        const { assert!(16 * 2 * 7360 * 4912 <= BATCH_LUMA_BUDGET) };
        assert_eq!(batch_sample_len(6000, 4000), BATCH_SAMPLE_CAP);
        // Three photos would not fit the budget: skipped, not clamped up.
        assert_eq!(batch_sample_len(30000, 20000), 0);
        assert_eq!(batch_sample_len(65535, 65535), 0);
        // 2 * w * h wraps in u64 here; must skip, not "fit".
        assert_eq!(batch_sample_len(3_037_000_500, 3_037_000_500), 0);
        assert_eq!(batch_sample_len(u32::MAX, u32::MAX), 0);
        assert_eq!(batch_sample_len(0, 0), BATCH_SAMPLE_CAP);
    }

    /// A PNG whose header claims `w` x `h` and that holds no pixel data.
    fn forged_png(w: u32, h: u32) -> Vec<u8> {
        fn crc(data: &[u8]) -> u32 {
            let mut c = !0u32;
            for &b in data {
                c ^= u32::from(b);
                for _ in 0..8 {
                    c = if c & 1 != 0 {
                        0xEDB8_8320 ^ (c >> 1)
                    } else {
                        c >> 1
                    };
                }
            }
            !c
        }
        let mut ihdr = b"IHDR".to_vec();
        ihdr.extend(w.to_be_bytes());
        ihdr.extend(h.to_be_bytes());
        ihdr.extend([8, 2, 0, 0, 0]);
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        out.extend(13u32.to_be_bytes());
        out.extend(&ihdr);
        out.extend(crc(&ihdr).to_be_bytes());
        // One IDAT with junk instead of pixel data, so the header reads but nothing decodes.
        let idat = b"IDAT\0\0\0\0";
        out.extend(4u32.to_be_bytes());
        out.extend(idat);
        out.extend(crc(idat).to_be_bytes());
        out.extend(0u32.to_be_bytes());
        out.extend(b"IEND");
        out.extend(crc(b"IEND").to_be_bytes());
        out
    }

    #[test]
    fn forged_headers_get_no_layer_and_no_allocation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths: Vec<PathBuf> = (0..3)
            .map(|i| {
                let p = dir.path().join(format!("f{i}.png"));
                std::fs::write(&p, forged_png(60000, 60000)).expect("write");
                p
            })
            .collect();
        assert_eq!(upright_size(&paths[0]).expect("header"), (60000, 60000));
        let progress = Progress {
            on_fraction: &|_| {},
            cancel: &std::sync::atomic::AtomicBool::new(false),
        };
        crate::batch::MAX_RESERVED.with(|m| m.set(0));
        let layers = build_set_layers(&paths, &progress).expect("ok");
        assert!(layers.get(60000, 60000).is_none());
        // A skipped group reserves nothing.
        assert_eq!(crate::batch::MAX_RESERVED.with(|m| m.get()), 0);

        // A real group reserves its sample once, within the budget.
        let real: Vec<PathBuf> = (0..3)
            .map(|i| {
                let p = dir.path().join(format!("r{i}.png"));
                image::RgbImage::new(16, 8).save(&p).expect("save");
                p
            })
            .collect();
        build_set_layers(&real, &progress).expect("ok");
        let reserved = crate::batch::MAX_RESERVED.with(|m| m.get());
        assert_eq!(reserved, 3 * 16 * 8);
        assert!(2 * reserved as u64 <= BATCH_LUMA_BUDGET);
    }

    #[test]
    fn growth_never_refills_eye_circles() {
        let (a, b) = ((60.0, 60.0), (300.0, 300.0));
        let faces = [face_at(a), face_at(b)];
        let mut m = Mask::new(400, 400);
        // Just outside eye a's circle: kept, then grown towards the eye.
        ring(&mut m, a, 13.0, 18.0);
        // Over eye b: touches the circle, so the whole blob goes.
        ring(&mut m, b, 0.0, 20.0);
        // Far from any face.
        ring(&mut m, (200.0, 380.0), 0.0, 5.0);
        let g = finish(
            Guess {
                mask: m,
                confidence: 0.8,
            },
            &faces,
            None,
            24,
        );
        assert_eq!(in_eyes(&g.mask, &faces), [], "eye circles filled");
        assert_eq!(g.mask.get_pixel(60 + 15, 60).0[0], 255, "ring kept");
        assert_eq!(
            g.mask.get_pixel(300 + 16, 300).0[0],
            0,
            "blob at eye b kept"
        );
        assert_eq!(g.mask.get_pixel(200, 380).0[0], 255, "far component lost");
        assert_eq!(g.confidence, 0.8);
    }

    #[test]
    fn batch_layer_keeps_components_at_the_eye() {
        let b = (300.0, 300.0);
        let faces = [face_at(b)];
        let mut m = Mask::new(400, 400);
        ring(&mut m, b, 0.0, 20.0);
        // The batch layer: a horizontal line through eye b.
        let mut layer = Mask::new(400, 400);
        for x in 250..350 {
            layer.put_pixel(x, 300, Luma([255]));
        }
        let g = finish(
            Guess {
                mask: m,
                confidence: 0.8,
            },
            &faces,
            Some(&layer),
            24,
        );
        assert_eq!(
            g.mask.get_pixel(300 + 16, 300 + 5).0[0],
            255,
            "supported blob dropped"
        );
        // Inside the circles only the batch layer's own pixels remain.
        let inside = in_eyes(&g.mask, &faces);
        assert!(!inside.is_empty());
        assert!(inside.iter().all(|&(_, y)| y == 300), "{inside:?}");
        assert_eq!(g.confidence, 0.8);
    }

    #[test]
    fn only_the_batch_layer_gives_confidence_one() {
        let b = (300.0, 300.0);
        let mut m = Mask::new(400, 400);
        ring(&mut m, b, 0.0, 20.0);
        let mut layer = Mask::new(400, 400);
        layer.put_pixel(100, 100, Luma([255]));
        let g = finish(
            Guess {
                mask: m.clone(),
                confidence: 0.8,
            },
            &[face_at(b)],
            Some(&layer),
            7,
        );
        assert_eq!(g.confidence, 1.0);
        assert_eq!(g.mask.get_pixel(300, 300 + 16).0[0], 0);
        let empty = finish(
            Guess {
                mask: m,
                confidence: 0.8,
            },
            &[face_at(b)],
            None,
            7,
        );
        assert_eq!(empty.confidence, 0.0);
        assert!(is_empty(&empty.mask));
    }
}

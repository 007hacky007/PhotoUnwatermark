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
use std::sync::atomic::{AtomicBool, Ordering};

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
    self, MatchBasis, MatchResult, MatchStatus, Transform, WatermarkTemplate, find_until,
    saved_placement,
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
/// the mark stamped at the same place on every photo, and the set's own edges of that mark.
#[derive(Debug, Clone, Default)]
pub struct SetLayers {
    layers: BTreeMap<(u32, u32), Mask>,
    /// Per size with a layer: the same median (see [`crate::batch::agree_count`]) of the photos'
    /// Sobel gradients taken where the photos are matched, at long side 1536 (see
    /// [`template::with_edges`]): the mark's edges without any one photo's scene.
    edges: BTreeMap<(u32, u32), Vec<f32>>,
    /// Per size with a layer, at the same long side 1536: how far apart the photos' lumas lie
    /// at each pixel. A mark blended over scenes that differ shows the scenes through it; a
    /// piece of scene that stays put does not change (see [`PRIOR_VARIES`]).
    spread: BTreeMap<(u32, u32), Vec<f32>>,
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

    /// The set's edges of the mark for photos of upright size `w` x `h` (see [`SetLayers`]), if
    /// their group has a layer.
    pub fn edges(&self, w: u32, h: u32) -> Option<&[f32]> {
        self.edges.get(&(w, h)).map(Vec::as_slice)
    }

    /// How far apart the photos' lumas lie under `m` (native size `w` x `h`), against the photo
    /// as a whole (the mean over `m` divided by the mean over every pixel). `None` without the
    /// numbers for that size or with an empty `m`.
    pub fn spread_ratio(&self, w: u32, h: u32, m: &Mask) -> Option<f64> {
        let spread = self.spread.get(&(w, h))?;
        let mean = mean_edges(spread, template::ref_dims(w, h), m)?;
        let all = spread.iter().map(|v| f64::from(*v)).sum::<f64>() / spread.len().max(1) as f64;
        (all > 0.0).then(|| mean / all)
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
/// [`batch_lines`] with [`BATCH_LOW`] / [`BATCH_HIGH`] and [`filter_layer`]. Each sampled photo
/// is also kept at long side 1536 (at most 3 MB), and a group that gets a layer gets the same
/// median of those as its edges ([`SetLayers::edges`]). Peak memory is bounded by the sample
/// (see [`BATCH_LUMA_BUDGET`]), whatever the size of the set.
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

    let (mut layers, mut edges, mut spread) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
    for ((w, h), sample) in samples {
        let mut lumas = PackedLumas::new(w, h);
        let (rw, rh) = template::ref_dims(w, h);
        let mut small = PackedLumas::new(rw as u32, rh as u32);
        // The sample was cut to fit BATCH_LUMA_BUDGET, so this is bounded by it (the copies at
        // long side 1536 add at most 3 MB per photo).
        if let Err(e) = lumas
            .reserve_photos(sample.len())
            .and_then(|()| small.reserve_photos(sample.len()))
        {
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
                    } else {
                        // Never fails: the size is the group's.
                        let _ = small.push(&template::working_rgb(&photo.rgb, rw, rh));
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
                        if let Ok(e) = median_gradient_packed(&small) {
                            edges.insert((w, h), e.into_raw());
                            spread.insert((w, h), small.range());
                        }
                    }
                }
                Err(e) => tracing::warn!("batch layer skipped for the {w}x{h} photos: {e}"),
            }
        }
        units.add(sample.len() * MEDIAN_UNITS);
    }
    (progress.on_fraction)(1.0);
    Ok(SetLayers {
        layers,
        edges,
        spread,
    })
}

/// Share of the saved placement's pixels that must lie in the grown batch layer.
const PRIOR_SUPPORT: f64 = 0.5;
/// Growth of the batch layer (and of the saved placement) for the support checks, as a
/// fraction of the long side.
const PRIOR_GROW: f32 = 0.004;
/// Share of the batch layer's pixels that must lie in the grown saved placement for the layer
/// to be taken as the mark under the drawing (the second support rule). Synthetic fixed set,
/// the true area grown by 0-48 px: 1.00 every time. Random and styles sets (the layer holds
/// static scene edges, the marks move): mostly 0.0-0.8, up to 1.0 for a few photos whose layer
/// is tiny or whose area covers most of the photo.
const PRIOR_LAYER_INSIDE: f64 = 0.9;
/// With that rule, the layer inside the drawing must also make up at least this share of the
/// drawing's pixels (a drawing up to about twenty times the mark's own lines). Keeps a few
/// static scene pixels under a large drawing from becoming the template the position is scored
/// with. Calibrated at 0.1 on median layers: synthetic fixed set 0.36-1.00, the real set's
/// grown first guess 0.19, random and styles photos that passed [`PRIOR_LAYER_INSIDE`]
/// 0.01-0.15. Layers that 6 photos must agree on (see [`crate::batch::agree_count`]) hold the
/// mark's lines only, about half as many pixels: the real first guess then holds 0.12-0.14 of
/// its layer, and a drawing 1.6 times as large fell under 0.1 (the set position was lost, and
/// the search found the scene). No drawing of the random and styles sets passes
/// [`PRIOR_LAYER_INSIDE`] on such layers.
const PRIOR_LAYER_SHARE: f64 = 0.05;
/// Least share of the drawing that must lie on the layer for an unsupported set position to
/// still give `Unsure` at the saved place when the search finds nothing better. A drawing grown
/// by 3 % of the long side around the real fixed mark keeps more; the styles set's drawings (a
/// tiled or full-frame mark around a layer of 210 px of scene) and a moving logo that touches a
/// speck of such a layer keep far less, and the search alone decides there (held back).
const PRIOR_SPEAKS: f64 = 0.01;
/// Largest share of the drawing that a piece not reaching the layer may hold for the saved
/// position to still stand (`Unsure`) when the set does not support it and the search finds
/// nothing better. In a drawing of a moving mark that touches a speck of a layer of scene that
/// piece is the mark itself, most of the drawing; the specks of scene in a first guess grown
/// by 3 % of the long side become separate pieces of a few tenths of it at most.
const PRIOR_LOOSE_MAIN: f64 = 0.5;
/// How far, as a share of the long side, the drawing may reach beyond the batch layer: the
/// distance at which the first guess keeps detector parts near the layer
/// ([`LAYER_NEAR_FRAC`], 2 %) and as much again for a person painting with a big brush. At 2.4 %
/// (the first fix) a drawing grown by 3.4 % of the long side on the fixed set lost the set
/// position; static scene beside a moving mark is told apart by [`PRIOR_VARIES`] instead.
const PRIOR_REACH: f32 = 0.04;
/// Largest share of the drawing that one connected stretch of it may hold farther than
/// [`PRIOR_REACH`] from the layer before the set's edges there are asked to show it (see
/// `explained_by_set`). The first guess always holds the layer (it is added before the guess
/// grows), so without this a layer of static scene beside a mark that moves supported the
/// guess, and the mark was `Found` where it was not on every other photo. Synthetic sets: the
/// fixed set's own drawing grown by 0-30 px 0.00, by 40 px 0.05; a static scene patch with the
/// mark moving, or with the mark missing from the layer, 0.24-0.44 in all. Real set-a, the
/// first guess: 0.09-0.25 in all on triples (the layer of 3 photos misses stretches of the
/// faint mark), and on all 8 photos 0.107 in three blobs of scene of 0.04 each.
const PRIOR_UNEXPLAINED: f64 = 0.1;
/// How much farther from the layer than the drawing's edge typically lies (the median over the
/// edge of the pieces that reach the layer) a stretch of the drawing may reach, as a share of
/// the long side (see `explained_by_set`): a brush or a box keeps about the same margin all
/// round the mark, and its edge wanders by less than this.
const PRIOR_EVEN: f32 = 0.01;
/// Largest size of one connected stretch reaching out farther than [`PRIOR_EVEN`], as a share
/// of the mark's own area in the drawing (its pixels on the layer or within the 0.4 % growth of
/// it), for a photo to be `Found` at the saved place. A second mark drawn beside the fixed one
/// sticks out of an even margin by its own height and gap; the engine cannot tell it from a
/// stretch of scene there, as the moving mark is elsewhere on the other photos, so beyond this
/// the result is `Unsure` at most. Synthetic sets (3, 5 and 8 photos; grown or boxed by 0-61
/// px, 0-4 % of the long side): the fixed set 0.000; a fixed mark and a second, thin mark that
/// moves, 10-45 px above it, 0.200-0.525. Real set-a, all 56 triples: the first guess as it
/// is, cleaned, with brush dots, with its middle erased or shrunk by 8 px up to 0.064 (a blob
/// of scene the guess caught beside the mark); grown by 8 px up to 0.138 (2 triples over
/// 0.12), by 1-3 % of the long side up to 0.21-0.65 (38-45 triples over 0.12, those are
/// `Unsure`: the guess's specks merge with the mark when it grows and stick out like a second
/// mark).
const PRIOR_UNEVEN: f64 = 0.12;
/// How much the set's edges over the far part of the drawing must stand out from the edges of
/// the whole photo, as a share of how much they stand out over the layer. Real set-a triples
/// whose far part is above [`PRIOR_UNEXPLAINED`]: 0.13-0.47 (the mark is there, too faint for
/// the layer); the synthetic static-patch sets: -0.09 to -0.03 (nothing is stamped there).
const PRIOR_FAR_EDGES: f64 = 0.05;
/// Parts of a rule-1 layer are its components grouped closer than this share of the long side.
/// 1 % keeps a text mark in 2-5 parts on the real set, and splits off a static object 8 px
/// from the mark (the synthetic adjacent case).
const PRIOR_PART_NEAR: f32 = 0.01;
/// A part needs at least this share of the largest part's pixels; smaller ones are fragments.
const PRIOR_PART_MIN: f64 = 0.3;
/// Least ratio of the luma spread across the set's photos under the drawing's layer to the
/// spread over the whole photo (`SetLayers::spread_ratio`) for the set to support a position. A
/// mark blended over scenes that differ lets them show through; a piece of scene that stays put
/// on every photo does not change at all. Synthetic: a static patch of scene 0.06-0.11; the
/// fixed set's mark 0.74-1.11; the mark and a static patch together 0.42-0.84. Real set-a
/// subsets of 3-8 photos: 1.41-1.80. Below it the set is no evidence of a mark (an opaque mark
/// that never changes looks the same), and the result is at most `Unsure`.
const PRIOR_VARIES: f64 = 0.25;
/// Overlap (intersection over union) at which a search result and the saved placement are the
/// same place. With a set position, a search `Found` elsewhere is at most `Unsure`.
const PRIOR_AGREE: f64 = 0.8;
/// Score at the saved position (masked NCC, +-2 px, scale 1) for `Found` on set support.
/// Calibrated on the synthetic fixed set with its own layer and the template of photo 0:
/// photos without the mark (the clean/ originals other than the reference's scene, and the
/// negative set resized to 1536x1024) scored 0.00-0.155, the 7 other faint (alpha 0.12) marked
/// photos 0.175-0.331. Since rule 1 scores on the set's own edges of the mark
/// ([`SetLayers::edges`]) the same photos score 0.00-0.07 and 0.51-0.95, a clean photo inside
/// the set 0.01-0.07, and the real set's photos 0.45-0.57. See `docs/notes/thresholds.md`.
pub const PRIOR_FOUND: f32 = 0.28;
/// Score at the saved position for `Unsure` on set support (the user opens the photo).
pub const PRIOR_UNSURE: f32 = 0.16;

/// The position prior of one template over one set, worked out once and used for every photo of
/// the template's size ([`match_with`]).
///
/// It exists when `layers` has a layer for the template's reference size and the placement
/// holds at least `PRIOR_LAYER_SHARE` of layer pixels. With `g` = [`mask::grow_for`] (0.004)
/// and "placement" = the template's pieces at their saved position:
/// 1. of the layer's components that reach into the placement grown by `g` (a component the
///    person erased whole is a piece of scene, not the mark), at least 90 % lies in it (see
///    `PRIOR_LAYER_INSIDE`): the position is scored with the template cut down to the raw
///    layer ([`template::within`]), its edges taken from the set's own edges of the mark
///    ([`SetLayers::edges`], [`template::with_edges`]) rather than from the reference photo,
///    whose scene the other photos do not share; and with each part of the layer on its own
///    (see `PRIOR_PART_NEAR`). A drawing grown well beyond the mark (the first guess is
///    grown, and people draw generously) is then judged on the mark's own edges, not on the
///    scene around it;
/// 2. else the position is scored with the template itself.
///
/// The set backs the position when rule 1 holds or at least half of the placement lies in the
/// layer grown by `g` (the drawing follows the layer); the photos' lumas under the layer change
/// with the scene behind it (`PRIOR_VARIES`); and the set explains the drawing's reach (no
/// stretch of `PRIOR_UNEXPLAINED` of it farther than `PRIOR_REACH` from the layer, and no
/// piece of it of that size that does not reach the layer at all, unless the set's edges show
/// something stamped there). Else the prior is cautious: no photo is `Found` through it, and
/// the search decides (see [`match_with`]).
///
/// A backed position is `Found` only when the set also accounts for the whole drawing: the
/// pieces that reach the layer keep an even margin around it (see `PRIOR_EVEN` and
/// `PRIOR_UNEVEN`). A drawing that reaches out much farther on one side than around the rest
/// of the mark may hold something the set does not show at that place, such as a second mark
/// that moves; the engine cannot tell it from more scene, so such a drawing is `Unsure` at the
/// saved position at most ([`PositionPrior::supports`] is false).
///
/// Either way a set-position result places the whole template, so removal covers all of the
/// drawing.
#[derive(Debug, Clone)]
pub struct PositionPrior {
    /// The template's pieces at their saved position, binary: the area of a set-position result.
    placed: Mask,
    /// What is scored at the saved position: the template within the layer (rule 1) or the
    /// template itself (rule 2).
    scored: WatermarkTemplate,
    /// Rule 1 with a layer of several parts: `scored` cut to each part (see
    /// [`PRIOR_PART_NEAR`]). A photo that does not show one of them is not `Found`.
    parts: Vec<WatermarkTemplate>,
    /// False when the drawing holds part of the layer but the set does not back its position:
    /// the result is at most `Unsure`, and the search decides.
    backed: bool,
    /// The set accounts for the whole drawing: its pieces that reach the layer keep an even
    /// margin around it (see [`PRIOR_UNEVEN`]).
    whole: bool,
    /// Without `backed`: whether the score at the saved position may still place the photo
    /// there as `Unsure` when the search finds no better place (see [`PRIOR_SPEAKS`]); else
    /// the search alone decides, held back.
    speaks: bool,
}

/// Mean of `edges` (the set's edges, reference space `rw` x `rh`) over the pixels of `m`
/// (native size), or `None` for an empty mask.
fn mean_edges(edges: &[f32], (rw, rh): (usize, usize), m: &Mask) -> Option<f64> {
    let small = mask::resize_any(m, rw as u32, rh as u32);
    let (mut sum, mut n) = (0f64, 0u64);
    for (e, p) in edges.iter().zip(small.as_raw()) {
        if *p != 0 {
            sum += f64::from(*e);
            n += 1;
        }
    }
    (n > 0).then(|| sum / n as f64)
}

/// Mean of `values` over the set pixels of `m` (both reference space, row-major).
fn mean_over(values: &[f32], m: &Mask) -> Option<f64> {
    let (mut sum, mut n) = (0f64, 0u64);
    for (v, p) in values.iter().zip(m.as_raw()) {
        if *p != 0 {
            sum += f64::from(*v);
            n += 1;
        }
    }
    (n > 0).then(|| sum / n as f64)
}

/// Whether the set's own `edges` (reference space) over `part` stand out from the edges of the
/// whole photo by at least [`PRIOR_FAR_EDGES`] of how much they stand out over the layer: a
/// faint stretch of the mark the layer missed, not something that is not there on the other
/// photos.
fn stamped(edges: &[f32], part: &Mask, layer: &Mask) -> bool {
    let e_all = edges.iter().map(|v| f64::from(*v)).sum::<f64>() / edges.len().max(1) as f64;
    match (mean_over(edges, part), mean_over(edges, layer)) {
        (Some(e_part), Some(e_layer)) => {
            e_layer > e_all && e_part - e_all >= PRIOR_FAR_EDGES * (e_layer - e_all)
        }
        _ => false,
    }
}

/// Chebyshev distance, in pixels, from every pixel of `m` to its nearest set pixel (the radius
/// of the square [`mask::dilate`] that first reaches it), row-major; `u32::MAX / 2` everywhere
/// when `m` is empty. Two chamfer passes over the 8 neighbours, exact for this metric.
fn distance_to(m: &Mask) -> Vec<u32> {
    let (w, h) = (m.width() as usize, m.height() as usize);
    let far = u32::MAX / 2;
    let mut d: Vec<u32> = m
        .as_raw()
        .iter()
        .map(|v| if *v != 0 { 0 } else { far })
        .collect();
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let mut v = d[i];
            if x > 0 {
                v = v.min(d[i - 1] + 1);
            }
            if y > 0 {
                v = v.min(d[i - w] + 1);
                if x > 0 {
                    v = v.min(d[i - w - 1] + 1);
                }
                if x + 1 < w {
                    v = v.min(d[i - w + 1] + 1);
                }
            }
            d[i] = v;
        }
    }
    for y in (0..h).rev() {
        for x in (0..w).rev() {
            let i = y * w + x;
            let mut v = d[i];
            if x + 1 < w {
                v = v.min(d[i + 1] + 1);
            }
            if y + 1 < h {
                v = v.min(d[i + w] + 1);
                if x > 0 {
                    v = v.min(d[i + w - 1] + 1);
                }
                if x + 1 < w {
                    v = v.min(d[i + w + 1] + 1);
                }
            }
            d[i] = v;
        }
    }
    d
}

/// How the set explains a drawing (see `explained_by_set`).
#[derive(Debug, Clone)]
struct Explained {
    /// No connected stretch of the drawing farther than [`PRIOR_REACH`] from the layer holds
    /// more than [`PRIOR_UNEXPLAINED`] of it, or the set's edges show something stamped over
    /// that far part (see `stamped`). A drawing grown well past the layer fails it.
    near: bool,
    /// No piece of the drawing (a connected part of it) that does not reach the layer holds
    /// more than [`PRIOR_UNEXPLAINED`] of it, or the set's edges show something stamped over
    /// those pieces. A second mark drawn beside the fixed one fails it, however close.
    joined: bool,
    /// The pieces that reach the layer keep an even margin around it: no connected stretch of
    /// them reaches more than [`PRIOR_EVEN`] farther from the layer than the drawing's edge
    /// typically lies and holds more than [`PRIOR_UNEVEN`] of the mark's area in the drawing.
    /// Unlike `near` and `joined`, the set's edges cannot excuse such a stretch: on photos of
    /// one shoot they also show the scene the photos share.
    even: bool,
    /// The share of the drawing that its largest piece not reaching the layer holds.
    loose: f64,
}

/// How the set explains the drawing `placed`, worked out at long side 1536 (see [`Explained`]).
fn explained_by_set(placed: &Mask, layer: &Mask, g: u32, edges: Option<&[f32]>) -> Explained {
    let (w, h) = placed.dimensions();
    let (rw, rh) = template::ref_dims(w, h);
    let (rw, rh) = (rw as u32, rh as u32);
    let scale = rw.max(rh) as f32 / w.max(h) as f32;
    let placed = mask::resize_any(placed, rw, rh);
    let layer = mask::resize_any(layer, rw, rh);
    let (all, _) = count_in(&placed, &placed);
    if all == 0 {
        return Explained {
            near: false,
            joined: false,
            even: false,
            loose: 1.0,
        };
    }
    let largest = |m: &Mask| {
        mask::components(m)
            .iter()
            .map(|c| u64::from(c.pixels))
            .max()
            .unwrap_or(0) as f64
            / all as f64
    };
    let big = |m: &Mask| largest(m) > PRIOR_UNEXPLAINED;
    let explains = |part: &Mask| !big(part) || edges.is_some_and(|e| stamped(e, part, &layer));
    let reach = (PRIOR_REACH * rw.max(rh) as f32).round() as u32;
    let near_layer = mask::dilate(&layer, reach);
    let mut far = Mask::new(rw, rh);
    for ((f, p), n) in far
        .as_mut()
        .iter_mut()
        .zip(placed.as_raw())
        .zip(near_layer.as_raw())
    {
        *f = if *p != 0 && *n == 0 { 255 } else { 0 };
    }
    let touching = mask::dilate(&layer, ((g as f32 * scale).round() as u32).max(1));
    let mut loose = Mask::new(rw, rh);
    for c in mask::components(&placed) {
        let (ox, oy) = (c.bbox.left() as u32, c.bbox.top() as u32);
        let px = || c.mask.enumerate_pixels().filter(|(_, _, p)| p.0[0] != 0);
        if !px().any(|(x, y, _)| touching.get_pixel(ox + x, oy + y).0[0] != 0) {
            for (x, y, p) in px() {
                loose.put_pixel(ox + x, oy + y, *p);
            }
        }
    }
    // The drawing's edge where it reaches the layer, and how far it typically lies from it.
    let dist = distance_to(&layer);
    let (wu, hu) = (rw as usize, rh as usize);
    let attached: Vec<bool> = placed
        .as_raw()
        .iter()
        .zip(loose.as_raw())
        .map(|(p, l)| *p != 0 && *l == 0)
        .collect();
    let set = |x: usize, y: usize| placed.as_raw()[y * wu + x] != 0;
    let mut edge: Vec<u32> = Vec::new();
    for y in 0..hu {
        for x in 0..wu {
            let i = y * wu + x;
            if attached[i]
                && (x == 0
                    || y == 0
                    || x + 1 == wu
                    || y + 1 == hu
                    || !set(x - 1, y)
                    || !set(x + 1, y)
                    || !set(x, y - 1)
                    || !set(x, y + 1))
            {
                edge.push(dist[i]);
            }
        }
    }
    let typical = if edge.is_empty() {
        0
    } else {
        let mid = edge.len() / 2;
        *edge.select_nth_unstable(mid).1
    };
    let beyond = typical + (PRIOR_EVEN * rw.max(rh) as f32).round() as u32;
    let mut uneven = Mask::new(rw, rh);
    for ((u, a), d) in uneven.as_mut().iter_mut().zip(&attached).zip(&dist) {
        *u = if *a && *d > beyond { 255 } else { 0 };
    }
    // Measured against the mark's own area in the drawing (what lies on the layer or next to
    // it), so that a big brush or box does not make a stretch look small.
    let (_, at_mark) = count_in(&placed, &touching);
    let uneven_share = largest(&uneven) * all as f64 / at_mark.max(1) as f64;
    let even = uneven_share <= PRIOR_UNEVEN;
    Explained {
        near: explains(&far),
        joined: explains(&loose),
        even,
        loose: largest(&loose),
    }
}

/// The components of `layer` with at least one pixel in `area`.
fn touched(layer: &Mask, area: &Mask) -> Mask {
    let (w, h) = layer.dimensions();
    let mut out = Mask::new(w, h);
    for c in mask::components(layer) {
        let (ox, oy) = (c.bbox.left() as u32, c.bbox.top() as u32);
        let px = || c.mask.enumerate_pixels().filter(|(_, _, p)| p.0[0] != 0);
        if px().any(|(x, y, _)| area.get_pixel(ox + x, oy + y).0[0] != 0) {
            for (x, y, p) in px() {
                out.put_pixel(ox + x, oy + y, *p);
            }
        }
    }
    out
}

/// The layer pixels in `mask` grouped as parts of the mark: components whose boxes are closer
/// than [`PRIOR_PART_NEAR`] of the long side, each with at least [`PRIOR_PART_MIN`] of the
/// largest part's pixels. Empty when there is only one such part.
fn layer_parts(layer: &Mask, mask: &Mask) -> Vec<Mask> {
    let (w, h) = layer.dimensions();
    let mut inside = Mask::new(w, h);
    for ((o, l), m) in inside
        .as_mut()
        .iter_mut()
        .zip(layer.as_raw())
        .zip(mask.as_raw())
    {
        *o = if *l != 0 && *m != 0 { 255 } else { 0 };
    }
    let comps = mask::components(&inside);
    let groups = mask::group_near(&comps, PRIOR_PART_NEAR * w.max(h) as f32);
    let size = |g: &Vec<usize>| g.iter().map(|&i| u64::from(comps[i].pixels)).sum::<u64>();
    let most = groups.iter().map(size).max().unwrap_or(0);
    let big: Vec<&Vec<usize>> = groups
        .iter()
        .filter(|g| size(g) as f64 >= PRIOR_PART_MIN * most as f64)
        .collect();
    if big.len() < 2 {
        return Vec::new();
    }
    big.into_iter()
        .map(|g| {
            let mut m = Mask::new(w, h);
            for c in g.iter().map(|&i| &comps[i]) {
                let (ox, oy) = (c.bbox.left() as u32, c.bbox.top() as u32);
                for (x, y, p) in c.mask.enumerate_pixels() {
                    if p.0[0] != 0 {
                        m.put_pixel(ox + x, oy + y, *p);
                    }
                }
            }
            m
        })
        .collect()
}

/// Pixels set in `a`, and how many of them are also set in `b`.
fn count_in(a: &Mask, b: &Mask) -> (u64, u64) {
    let (mut total, mut inside) = (0u64, 0u64);
    for (p, q) in a.as_raw().iter().zip(b.as_raw()) {
        if *p != 0 {
            total += 1;
            inside += (*q != 0) as u64;
        }
    }
    (total, inside)
}

impl PositionPrior {
    /// See [`PositionPrior`]; `None` when the set does not support the template's position.
    pub fn new(t: &WatermarkTemplate, layers: &SetLayers) -> Option<PositionPrior> {
        let (w, h) = (t.ref_w, t.ref_h);
        let layer = layers.get(w, h)?;
        let placed = mask::binarize(&saved_placement(t)?);
        let g = mask::grow_for(w, h, PRIOR_GROW);
        // Rule 1 looks at the parts of the layer the drawing holds: a part the person erased
        // whole (a piece of scene the set happened to share) says nothing about the mark.
        let grown = mask::dilate(&placed, g);
        let held = touched(layer, &grown);
        let (layer_px, layer_in) = count_in(&held, &grown);
        // The drawing follows the layer: at least half of it lies in the layer grown by `g`.
        let (total, inside) = count_in(&placed, &mask::dilate(layer, g));
        let follows = total > 0 && inside as f64 >= PRIOR_SUPPORT * total as f64;
        // What the set shows under the drawing must change with the scene behind it: a piece
        // of scene that stays put on every photo is no evidence of a mark.
        let mut under = placed.clone();
        for (u, l) in under.as_mut().iter_mut().zip(layer.as_raw()) {
            if *l == 0 {
                *u = 0;
            }
        }
        let varies = layers
            .spread_ratio(w, h, &under)
            .is_some_and(|r| r >= PRIOR_VARIES);
        // A drawing that reaches into no part of the layer: the set says nothing about it, and
        // the search decides alone. Any part it holds makes the set's word count: without
        // support no photo is `Found` through it, not even by the search (a moving mark that
        // touches a speck of scene the set shares is then `Unsure` where it is found; a
        // drawing shrunk to scraps of the guess no longer finds the scene elsewhere).
        if layer_px == 0 {
            return None;
        }
        let (placed_px, on_layer) = count_in(&placed, layer);
        let on_it = on_layer as f64 >= PRIOR_LAYER_SHARE * placed_px as f64;
        let rule_1 = layer_in as f64 >= PRIOR_LAYER_INSIDE * layer_px as f64 && on_it;
        // Scored on the layer under the drawing whatever the rule: a set that does not
        // support the position still says how well the photo shows the mark there.
        let cut = template::within(t, layer);
        // Whatever the rule, the drawing may not hold much that the set does not show fixed.
        let set = explained_by_set(&placed, layer, g, layers.edges(w, h));
        let backed = (rule_1 || follows) && varies && set.near && set.joined;
        let (whole, loose) = (set.even, set.loose);
        // Without support, the saved position may still stand against the search: what the
        // set shows there is a mark (it varies), no piece of the drawing that the set does not
        // reach is its main part (as the moving mark is in a drawing of it that only touches a
        // speck of scene; the grown specks of a generous first guess are not), and enough of
        // it lies on the layer.
        let speaks = varies
            && loose <= PRIOR_LOOSE_MAIN
            && on_layer as f64 >= PRIOR_SPEAKS * placed_px as f64;
        let Some(cut) = cut else {
            return Some(PositionPrior {
                placed,
                scored: t.clone(),
                parts: Vec::new(),
                backed,
                whole,
                speaks,
            });
        };
        // Scored on the set's own edges of the mark where the set has them.
        let edges = layers.edges(w, h);
        let on_edges = |c: WatermarkTemplate| match edges {
            Some(e) => template::with_edges(&c, e).unwrap_or(c),
            None => c,
        };
        let parts = layer_parts(layer, &placed)
            .iter()
            .filter_map(|m| template::within(t, m))
            .map(on_edges)
            .collect();
        Some(PositionPrior {
            placed,
            scored: on_edges(cut),
            parts,
            backed,
            whole,
            speaks,
        })
    }

    /// Whether a photo may be `Found` through this prior: the set backs the saved position and
    /// accounts for the whole drawing (see [`PositionPrior`]). Otherwise no photo is `Found`
    /// through it, at the saved position or by the search (see [`match_with`]).
    pub fn supports(&self) -> bool {
        self.backed && self.whole
    }

    /// The template the saved position is scored with.
    pub fn scored(&self) -> &WatermarkTemplate {
        &self.scored
    }

    fn result(&self, status: MatchStatus, score: f32) -> MatchResult {
        MatchResult {
            status,
            confidence: score,
            mask: self.placed.clone(),
            basis: MatchBasis::SetPosition,
            transform: Some(Transform::IDENTITY),
        }
    }
}

/// [`template::find`] with a position prior for marks stamped at the same place on every photo:
/// [`match_with`] with the [`PositionPrior`] of `t` over `layers`, worked out for this call.
/// Matching many photos, work the prior out once and call [`match_with`].
pub fn match_photo(t: &WatermarkTemplate, img: &RgbImage, layers: &SetLayers) -> MatchResult {
    let prior = PositionPrior::new(t, layers);
    match_with(t, prior.as_ref(), img, &AtomicBool::new(false))
}

/// [`find_until`] with the set's position `prior` (see [`PositionPrior`]).
///
/// Without a prior, or for a photo of another size than the template's reference, this is
/// [`find_until`]. Otherwise the score at the saved position ([`template::saved_score`] of the
/// cut template, +-2 px, scale 1) and the set's support decide.
///
/// When the set backs the position (see [`PositionPrior`]), no search unless the score is low:
/// - At least [`PRIOR_FOUND`], every part of the layer at least [`PRIOR_UNSURE`], and the set
///   accounts for the whole drawing: `Found` at the saved position. A photo that shows only
///   some parts (a static object beside the mark, on a photo without the mark) stays `Unsure`,
///   and so does every photo of a drawing that reaches out unevenly.
/// - At least [`PRIOR_UNSURE`]: `Unsure` at the saved position; the person opens the photo. A
///   search `Found` somewhere else is not trusted on a set that shows the mark here: on the real
///   set the drawing is mostly scene, and the search finds the scene where it moved between
///   shots (0.52-0.68, against 0.13 for the drawing at the saved position).
/// - Below: the search's result, but a `Found` that does not overlap the saved placement by at
///   least `PRIOR_AGREE`, or of an uneven drawing, is `Unsure`.
///
/// When the set does not back the position the search always runs, and its result is held
/// back: never `Found`, but `Unsure` where it found the mark. The saved position stands instead
/// (`Unsure`, from [`PRIOR_UNSURE`] up) only where the set speaks for it (its layer varies like
/// a mark, no piece of the drawing that does not reach the layer holds more than
/// `PRIOR_LOOSE_MAIN` of it, and at least `PRIOR_SPEAKS` of the drawing is on the layer)
/// and the search finds no better place: a search result (`Found` or `Unsure`) elsewhere wins
/// only when the scored template scores higher there than at the saved position
/// ([`template::scores_at`]), like for like.
///
/// A set-position result has basis [`MatchBasis::SetPosition`], the score as its confidence,
/// the saved placement of the whole template as its mask and [`Transform::IDENTITY`].
///
/// Once `stop` is set the result means nothing (see [`find_until`]).
pub fn match_with(
    t: &WatermarkTemplate,
    prior: Option<&PositionPrior>,
    img: &RgbImage,
    stop: &AtomicBool,
) -> MatchResult {
    let prior = prior.filter(|p| p.placed.dimensions() == img.dimensions());
    let scored = prior.and_then(|p| {
        let all: Vec<&WatermarkTemplate> = std::iter::once(&p.scored).chain(&p.parts).collect();
        template::saved_scores(&all, img).map(|s| (p, s))
    });
    let Some((p, scores)) = scored else {
        return find_until(t, img, stop);
    };
    let score = scores[0];
    let weakest = scores.iter().copied().fold(score, f32::min);
    if p.backed {
        if p.whole && score >= PRIOR_FOUND && weakest >= PRIOR_UNSURE {
            return p.result(MatchStatus::Found, score);
        }
        if score >= PRIOR_UNSURE {
            return p.result(MatchStatus::Unsure, score);
        }
        return held_back(find_until(t, img, stop), p);
    }
    // The set does not back the saved position: the search decides, held back to `Unsure`.
    // The saved position stands only where the set speaks for it and the search finds no
    // better place: like for like, what the set shows of the mark (the scored template) is
    // scored where the search put the drawing and compared with its score here. The search's
    // own score is not compared: it scores the whole drawing, mostly scene, and on photos of
    // one shoot it follows the scene where it moved between shots.
    let found = find_until(t, img, stop);
    let elsewhere = found.status != MatchStatus::NotFound
        && overlap(&found.mask, &p.placed) < PRIOR_AGREE
        && found
            .transform
            .and_then(|at| template::scores_at(&[&p.scored], img, at))
            .is_some_and(|s| s[0] > score);
    if score >= PRIOR_UNSURE && p.speaks && !elsewhere {
        return p.result(MatchStatus::Unsure, score);
    }
    held_back(found, p)
}

/// `found` (a search result) under the set's `prior`: a `Found` is `Unsure` when no photo may be
/// `Found` through the prior ([`PositionPrior::supports`]), or when it does not overlap the
/// saved placement by at least [`PRIOR_AGREE`].
fn held_back(mut found: MatchResult, prior: &PositionPrior) -> MatchResult {
    if found.status == MatchStatus::Found
        && (!prior.supports() || overlap(&found.mask, &prior.placed) < PRIOR_AGREE)
    {
        found.status = MatchStatus::Unsure;
    }
    found
}

/// Share of the pixels set in both masks among those set in either.
fn overlap(a: &Mask, b: &Mask) -> f64 {
    let (mut inter, mut uni) = (0u64, 0u64);
    for (p, q) in a.as_raw().iter().zip(b.as_raw()) {
        let (x, y) = (*p != 0, *q != 0);
        inter += (x && y) as u64;
        uni += (x || y) as u64;
    }
    inter as f64 / uni.max(1) as f64
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
/// holds static scene edges and the marks move (random, styles). The layer must also hold at
/// least [`SPECK_FRAC`] of the pixels of the detector's largest group: on layers from
/// [`crate::batch::agree_count`] the styles set keeps a 210 px scene layer that the detector
/// covers, 0.004-0.011 of its marks; the fixed set's layer is about as large as its mark's
/// group, the real set's 3-10 times (the detector sees only part of a faint mark).
const LAYER_SEEN: f64 = 0.9;
/// Otherwise detector components are grouped like the pieces of a template (boxes closer than
/// [`LAYER_NEAR_FRAC`] of the long side) and a group with less than this share of the largest
/// group's pixels is dropped as a speck. Tiled text is many groups of about the same size, so
/// it stays. Components near the batch layer are kept whatever their size.
const SPECK_FRAC: f64 = 0.1;

/// The parts of the detector's `guess` that are the mark, without the specks it also finds on
/// fur, foliage and stars. See `LAYER_NEAR_FRAC`, `LAYER_SEEN` and `SPECK_FRAC`:
/// - with a non-empty batch `layer` (same size) that the detector sees almost all of, and that
///   is no speck beside the detector's largest group, only the components near the layer are
///   kept;
/// - otherwise the components near the layer (if any) and every group of at least
///   `SPECK_FRAC` of the largest group are kept.
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
    let groups = mask::group_near(&comps, LAYER_NEAR_FRAC * long);
    let sizes: Vec<u64> = groups
        .iter()
        .map(|g| g.iter().map(|&i| u64::from(comps[i].pixels)).sum())
        .collect();
    let most = sizes.iter().copied().max().unwrap_or(0);
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
        // A layer no bigger than a speck beside the detector's largest group is not the mark,
        // however much of it the detector covers.
        seen = hit as f64 >= LAYER_SEEN * all as f64 && all as f64 >= SPECK_FRAC * most as f64;
    }
    if !seen {
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
            // The layer holds the letters' edges, a little off the detector's fill, grown by
            // 3 px as `batch_lines` grows them.
            rect(&mut layer, 302 + 40 * k, 394, 328 + 40 * k, 400);
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
    fn a_speck_of_layer_does_not_decide() {
        // A tiny layer of scene the detector happens to cover, under one speck: it is no mark,
        // so the mark far from it stays (as without a layer).
        let (guess, _) = mark_and_specks();
        let mut layer = Mask::new(1000, 600);
        rect(&mut layer, 901, 81, 903, 83);
        let kept = keep_mark_parts(&guess, Some(&layer));
        assert!(
            set_at(&kept, 310, 410) && set_at(&kept, 510, 410),
            "mark lost"
        );
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
        // The larger of the two reservations: the photos as they are, and as matching sees
        // them (long side 1536, here larger than the photos).
        let (rw, rh) = template::ref_dims(16, 8);
        assert_eq!(reserved, 3 * (16 * 8).max(rw * rh));
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

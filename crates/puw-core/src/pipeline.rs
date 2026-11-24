//! The engine's workflow as the app uses it: the automatic first guess for a whole set, the
//! photo shown first in the editor, and removing a confirmed area from one photo.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use image::RgbImage;

use crate::Result;
use crate::batch::{BATCH_HIGH, BATCH_LOW, batch_lines, median_gradient};
use crate::detect::{Guess, GuessOptions, guess_single};
use crate::engine::Engine;
use crate::faces::{detect_faces, guard};
use crate::image_io::{LoadedPhoto, save_result};
use crate::inpaint::{Progress, inpaint};
use crate::mask::{self, Mask};

/// Photos of one size needed before the batch layer is computed for them.
pub const MIN_BATCH_PHOTOS: usize = 3;

/// Automatic first guess for every photo of a set, in input order.
///
/// Per photo: [`guess_single`], then the face guard ([`guard`]) with the batch layer of the
/// photo's group as `keep`, then the union with that batch layer, then growth by
/// [`mask::grow_for`] with `opts.grow_frac`. Photos are grouped by their upright size, and only
/// a group of at least [`MIN_BATCH_PHOTOS`] gets a batch layer ([`median_gradient`] and
/// [`batch_lines`] with [`BATCH_LOW`] / [`BATCH_HIGH`]); a group whose batch layer fails or
/// finds nothing simply has none, so mixed sizes never fail the set.
///
/// `confidence` is the one from [`guess_single`] when its guess survives the face guard, 1.0
/// when only the batch layer contributed, and 0.0 when the mask is empty.
pub fn auto_guess_set(
    engine: &Engine,
    photos: &[LoadedPhoto],
    opts: &GuessOptions,
) -> Result<Vec<Guess>> {
    let (layers, layer_of) = batch_layers(photos);
    photos
        .iter()
        .zip(layer_of)
        .map(|(p, l)| guess_one(engine, &p.rgb, l.map(|i| &layers[i]), opts))
        .collect()
}

/// Batch layers of the same-size groups, and for each photo the index of its group's layer.
fn batch_layers(photos: &[LoadedPhoto]) -> (Vec<Mask>, Vec<Option<usize>>) {
    let mut groups: BTreeMap<(u32, u32), Vec<usize>> = BTreeMap::new();
    for (i, p) in photos.iter().enumerate() {
        groups.entry(p.rgb.dimensions()).or_default().push(i);
    }
    let mut layers = Vec::new();
    let mut layer_of = vec![None; photos.len()];
    for ((w, h), members) in groups {
        if members.len() < MIN_BATCH_PHOTOS {
            continue;
        }
        let images: Vec<&RgbImage> = members.iter().map(|&i| &photos[i].rgb).collect();
        let layer = match median_gradient(&images) {
            Ok(mag) => batch_lines(&mag, BATCH_LOW, BATCH_HIGH),
            Err(e) => {
                tracing::warn!("batch layer skipped for the {w}x{h} photos: {e}");
                None
            }
        };
        if let Some(layer) = layer {
            for &i in &members {
                layer_of[i] = Some(layers.len());
            }
            layers.push(layer);
        }
    }
    (layers, layer_of)
}

fn is_empty(m: &Mask) -> bool {
    m.as_raw().iter().all(|&v| v == 0)
}

fn guess_one(
    engine: &Engine,
    img: &RgbImage,
    layer: Option<&Mask>,
    opts: &GuessOptions,
) -> Result<Guess> {
    let single = guess_single(engine, img, opts)?;
    // The guard only clears pixels, so an empty guess needs no face detection.
    let guarded = if is_empty(&single.mask) {
        single.mask
    } else {
        let faces = detect_faces(engine, img)?;
        guard(&single.mask, &faces, layer)
    };
    let from_single = !is_empty(&guarded);
    let combined = match layer {
        Some(l) => mask::union(&guarded, l),
        None => guarded,
    };
    let confidence = if from_single {
        single.confidence
    } else if !is_empty(&combined) {
        1.0
    } else {
        0.0
    };
    let (w, h) = img.dimensions();
    let mask = mask::dilate(&combined, mask::grow_for(w, h, opts.grow_frac));
    Ok(Guess { mask, confidence })
}

/// Index of the photo to show first in the editor: the highest confidence among guesses with a
/// non-empty mask. Equal confidences go to the earlier photo; with no non-empty guess (or no
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

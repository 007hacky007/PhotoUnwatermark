//! The engine's workflow as the app uses it: the automatic first guess for a whole set, the
//! photo shown first in the editor, and removing a confirmed area from one photo.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use image::RgbImage;

use crate::Result;
use crate::batch::{BATCH_HIGH, BATCH_LOW, batch_lines, median_gradient};
use crate::detect::{Guess, GuessOptions, guess_single};
use crate::engine::Engine;
use crate::faces::{Face, detect_faces_near, drop_eye_components, guard};
use crate::image_io::{LoadedPhoto, save_result};
use crate::inpaint::{Progress, inpaint};
use crate::mask::{self, Mask};

/// Photos of one size needed before the batch layer is computed for them.
pub const MIN_BATCH_PHOTOS: usize = 3;

/// Automatic first guess for every photo of a set, in input order.
///
/// Per photo: [`guess_single`], then the face guard, the union with the batch layer of the
/// photo's group and growth by [`mask::grow_for`] with `opts.grow_frac`. The guard runs before
/// growth ([`drop_eye_components`]) and after it ([`guard`]), so growth never fills an eye. Photos are grouped by their upright size, and only
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
    let near = match layer {
        Some(l) => mask::union(&single.mask, l),
        None => single.mask.clone(),
    };
    let (w, h) = img.dimensions();
    let grow = mask::grow_for(w, h, opts.grow_frac);
    // The guard only clears pixels, so an empty mask needs no face detection.
    let faces = if is_empty(&near) {
        Vec::new()
    } else {
        detect_faces_near(engine, img, &near, grow)?
    };
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

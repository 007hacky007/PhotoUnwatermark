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

use crate::batch::{BATCH_HIGH, BATCH_LOW, PackedLumas, batch_lines, median_gradient_packed};
use crate::detect::{Guess, GuessOptions, guess_single};
use crate::engine::Engine;
use crate::faces::{Face, detect_faces_near, drop_eye_components, guard};
use crate::image_io::{LoadedPhoto, load, save_result, upright_size};
use crate::inpaint::{Progress, inpaint};
use crate::mask::{self, Mask, is_empty};
use crate::{Error, Result};

/// Photos of one size needed before the batch layer is computed for them.
pub const MIN_BATCH_PHOTOS: usize = 3;
/// Most photos of one size group that go into its batch layer.
pub const BATCH_SAMPLE_CAP: usize = 24;
/// Memory for the photos of one batch layer, which are held as 2-byte luma while the median
/// is taken: the sample is cut to fit (16 photos at 36 MP). A size whose
/// [`MIN_BATCH_PHOTOS`] photos would not fit gets no batch layer. [`build_set_layers`] peaks at about this plus 0.4 GB at 36 MP
/// (measured 1.53 GB for 40 and for 200 photos of 7360x4912).
pub const BATCH_LUMA_BUDGET: u64 = 1_200_000_000;
/// Most photos [`representative_sample`] picks.
pub const REPRESENTATIVE_CAP: usize = 6;

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
    let per_photo = (2 * u64::from(w) * u64::from(h)).max(1);
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
/// [`batch_lines`] with [`BATCH_LOW`] / [`BATCH_HIGH`]. Peak memory is bounded by the sample
/// (see [`BATCH_LUMA_BUDGET`]), whatever the size of the set.
///
/// A group whose size is so large that [`MIN_BATCH_PHOTOS`] photos exceed [`BATCH_LUMA_BUDGET`]
/// (sizes come from file headers, which may lie) has no layer either (logged). Memory is
/// claimed only as photos decode.
///
/// A photo that cannot be read is left out (logged); a group with fewer than
/// [`MIN_BATCH_PHOTOS`] readable photos in its sample, or whose layer finds nothing, has no
/// layer, so a mixed or damaged set never fails here. `progress` gets fractions from 0.0 to
/// 1.0, and its cancel flag is checked before each photo and each median; when set, this
/// returns [`Error::Cancelled`].
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
                    if let Some(layer) = batch_lines(&mag, BATCH_LOW, BATCH_HIGH) {
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

/// Automatic first guess for one photo.
///
/// [`guess_single`], then the face guard ([`detect_faces_near`]), the union with the batch
/// layer of the photo's size from `layers` (if any) and growth by [`mask::grow_for`] with
/// `opts.grow_frac`. The guard runs before growth ([`drop_eye_components`]) and after it
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
        let layers = build_set_layers(&paths, &progress).expect("ok");
        assert!(layers.get(60000, 60000).is_none());
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

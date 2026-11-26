mod common;

use std::collections::BTreeSet;
use std::fs;
use std::hash::{DefaultHasher, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use image::{GrayImage, RgbImage, imageops};
use puw_core::batch::{BATCH_HIGH, BATCH_LOW, batch_lines, filter_layer, median_gradient};
use puw_core::detect::{Guess, GuessOptions, guess_single};
use puw_core::faces::{detect_faces_near, drop_eye_components, guard};
use puw_core::mask::{self, Mask, is_empty};
use puw_core::pipeline::{PRIOR_FOUND, PRIOR_UNSURE, keep_mark_parts};
use puw_core::template::MatchBasis;
use puw_core::{
    Engine, Error, LoadedPhoto, MatchStatus, ModelPaths, PositionPrior, Progress, SetLayers,
    Transform, build_set_layers, build_template, find, guess_photo, load, match_photo, match_with,
    output_dir_for, pick_representative, remove_one, representative_sample,
};

fn engine() -> Option<&'static Engine> {
    static ENGINE: OnceLock<Option<Engine>> = OnceLock::new();
    ENGINE
        .get_or_init(|| {
            let Some(dir) = common::models_dir() else {
                eprintln!("skipped: PUW_MODELS_DIR not set");
                return None;
            };
            let paths = ModelPaths::from_dir(&dir).expect("model files present");
            Some(Engine::new(&paths, common::shipping_options()).expect("engine loads"))
        })
        .as_ref()
}

fn open_rgb(p: &Path) -> RgbImage {
    image::open(p).expect("open").to_rgb8()
}

fn open_mask(p: &Path) -> Mask {
    image::open(p).expect("open mask").to_luma8()
}

/// Minimal little-endian EXIF block holding only an Orientation tag.
fn exif_orientation_blob(orientation: u16) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(b"II*\0");
    b.extend_from_slice(&8u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&0x0112u16.to_le_bytes());
    b.extend_from_slice(&3u16.to_le_bytes());
    b.extend_from_slice(&1u32.to_le_bytes());
    b.extend_from_slice(&orientation.to_le_bytes());
    b.extend_from_slice(&[0, 0]);
    b.extend_from_slice(&0u32.to_le_bytes());
    b
}

/// JPEG (quality 98) of `img`, with an EXIF Orientation tag when given.
fn jpeg_bytes(img: &RgbImage, orientation: Option<u16>) -> Vec<u8> {
    use img_parts::{Bytes, ImageEXIF};
    let mut out = Vec::new();
    let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 98);
    image::ImageEncoder::write_image(
        enc,
        img.as_raw(),
        img.width(),
        img.height(),
        image::ExtendedColorType::Rgb8,
    )
    .expect("encode jpeg");
    let Some(o) = orientation else { return out };
    let mut j = img_parts::jpeg::Jpeg::from_bytes(Bytes::from(out)).expect("parse jpeg");
    j.set_exif(Some(Bytes::from(exif_orientation_blob(o))));
    let mut with_exif = Vec::new();
    j.encoder().write_to(&mut with_exif).expect("write jpeg");
    with_exif
}

/// EXIF Orientation of a whole image file, if it has one.
fn file_orientation(bytes: &[u8]) -> Option<u32> {
    let mut cur = std::io::Cursor::new(bytes);
    let exif = exif::Reader::new().read_from_container(&mut cur).ok()?;
    exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)?
        .value
        .get_uint(0)
}

/// PSNR of `a` against `b` over the non-zero pixels of `region`.
fn psnr_over(a: &RgbImage, b: &RgbImage, region: &Mask) -> f64 {
    let (mut se, mut n) = (0f64, 0u64);
    for (x, y, p) in region.enumerate_pixels() {
        if p.0[0] != 0 {
            let (pa, pb) = (a.get_pixel(x, y).0, b.get_pixel(x, y).0);
            for c in 0..3 {
                let d = pa[c] as f64 - pb[c] as f64;
                se += d * d;
            }
            n += 3;
        }
    }
    let mse = se / n.max(1) as f64;
    10.0 * (255.0f64 * 255.0 / mse.max(1e-9)).log10()
}

/// Mean absolute difference over the pixels where `region` is zero.
fn mean_diff_outside(a: &RgbImage, b: &RgbImage, region: &Mask) -> f64 {
    let (mut sum, mut n) = (0f64, 0u64);
    for (x, y, p) in region.enumerate_pixels() {
        if p.0[0] == 0 {
            let (pa, pb) = (a.get_pixel(x, y).0, b.get_pixel(x, y).0);
            for c in 0..3 {
                sum += pa[c].abs_diff(pb[c]) as f64;
            }
            n += 3;
        }
    }
    sum / n.max(1) as f64
}

fn file_hash(p: &Path) -> u64 {
    let mut h = DefaultHasher::new();
    h.write(&fs::read(p).expect("read"));
    h.finish()
}

fn names_in(dir: &Path) -> BTreeSet<String> {
    fs::read_dir(dir)
        .expect("read dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect()
}

/// Fractions reported through a [`Progress`], and the cancel flag it checks.
struct Recorder {
    seen: Mutex<Vec<f32>>,
    cancel: AtomicBool,
}

impl Recorder {
    fn new(cancelled: bool) -> Self {
        Recorder {
            seen: Mutex::new(Vec::new()),
            cancel: AtomicBool::new(cancelled),
        }
    }

    fn fractions(&self) -> Vec<f32> {
        self.seen.lock().unwrap().clone()
    }
}

fn assert_progress(v: &[f32]) {
    assert_eq!(v.first(), Some(&0.0), "{v:?}");
    assert_eq!(v.last(), Some(&1.0), "{v:?}");
    assert!(v.windows(2).all(|p| p[0] <= p[1]), "{v:?}");
}

/// Intersection over union of the set pixels.
fn iou(a: &Mask, b: &Mask) -> f64 {
    let (mut inter, mut uni) = (0u64, 0u64);
    for (p, q) in a.pixels().zip(b.pixels()) {
        let (x, y) = (p.0[0] != 0, q.0[0] != 0);
        inter += (x && y) as u64;
        uni += (x || y) as u64;
    }
    inter as f64 / uni.max(1) as f64
}

/// Six photos in three upright sizes: three of the fixed set (one batch layer), the same mark
/// in two smaller photos (too few for a layer) and one of a third size.
fn mixed_set(dir: &Path) -> Vec<PathBuf> {
    let synth = common::synth_dir();
    for i in 0..3 {
        let src = synth.join("fixed/marked").join(format!("0{i}.png"));
        fs::copy(&src, dir.join(format!("a{i}.png"))).expect("copy");
    }
    for i in 3..5 {
        let src = open_rgb(&synth.join("fixed/marked").join(format!("0{i}.png")));
        let small = imageops::resize(&src, 1200, 800, imageops::FilterType::Triangle);
        small.save(dir.join(format!("b{i}.png"))).expect("save");
    }
    fs::copy(synth.join("random/marked/02.png"), dir.join("c.png")).expect("copy");
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .expect("read dir")
        .map(|e| e.expect("entry").path())
        .collect();
    paths.sort();
    paths
}

#[test]
fn set_layers_and_guesses_group_by_size() {
    let Some(engine) = engine() else { return };
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = mixed_set(dir.path());
    let rec = Recorder::new(false);
    let report = |f: f32| rec.seen.lock().unwrap().push(f);
    let progress = Progress {
        on_fraction: &report,
        cancel: &rec.cancel,
    };
    let layers = build_set_layers(&paths, &progress).expect("mixed sizes are fine");
    assert_progress(&rec.fractions());

    let photos: Vec<LoadedPhoto> = paths.iter().map(|p| load(p).expect("load")).collect();
    let sizes: Vec<(u32, u32)> = photos.iter().map(|p| p.rgb.dimensions()).collect();
    assert_eq!(
        sizes,
        [
            (1536, 1024),
            (1536, 1024),
            (1536, 1024),
            (1200, 800),
            (1200, 800),
            (1024, 682)
        ]
    );
    // One layer, for the group of three; it matches the full-precision batch layer.
    assert_eq!(layers.iter().count(), 1);
    assert!(layers.get(1200, 800).is_none() && layers.get(1024, 682).is_none());
    let layer = layers
        .get(1536, 1024)
        .expect("fixed mark in the batch layer");
    let group: Vec<&RgbImage> = photos[..3].iter().map(|p| &p.rgb).collect();
    let mag = median_gradient(&group).expect("median");
    let exact = filter_layer(&batch_lines(&mag, BATCH_LOW, BATCH_HIGH).expect("exact layer"));
    let agree = iou(layer, &exact);
    eprintln!("batch layer vs full precision: IoU {agree:.5}");
    assert!(agree > 0.995, "IoU {agree}");

    let opts = GuessOptions::default();
    let mut guesses = Vec::new();
    for (i, photo) in photos.iter().enumerate() {
        let (w, h) = photo.rgb.dimensions();
        let got = guess_photo(engine, photo, &layers, &opts).expect("guess");
        let keep = (i < 3).then_some(layer);
        // The documented recipe, step by step.
        let single = guess_single(engine, &photo.rgb, &opts).expect("guess");
        let single = Guess {
            mask: keep_mark_parts(&single.mask, keep),
            ..single
        };
        let grow = mask::grow_for(w, h, opts.grow_frac);
        let near = match keep {
            Some(l) => mask::union(&single.mask, l),
            None => single.mask.clone(),
        };
        let faces = detect_faces_near(engine, &photo.rgb, &near, grow).expect("faces");
        let kept = drop_eye_components(&single.mask, &faces, keep);
        let combined = match keep {
            Some(l) => mask::union(&kept, l),
            None => kept.clone(),
        };
        let grown = mask::dilate(&combined, grow);
        let want = guard(&grown, &faces, keep);
        assert!(got.mask == want, "photo {i}: mask differs from the recipe");
        let want_conf = if is_empty(&want) {
            0.0
        } else if !is_empty(&kept) {
            single.confidence
        } else {
            1.0
        };
        assert_eq!(got.confidence, want_conf, "photo {i}");
        if let Some(l) = keep {
            let missing = l
                .pixels()
                .zip(got.mask.pixels())
                .filter(|(a, b)| a.0[0] != 0 && b.0[0] == 0)
                .count();
            assert_eq!(missing, 0, "photo {i}: batch layer not in the guess");
        }
        eprintln!(
            "photo {i} {w}x{h}: coverage {:.4} conf {:.3}, {} faces",
            mask::coverage(&got.mask),
            got.confidence,
            faces.len()
        );
        guesses.push(got);
    }

    // Three of the six photos, evenly spaced.
    let sample = representative_sample(photos.len());
    assert_eq!(sample, [1, 3, 5]);
    let rep = pick_representative(&guesses);
    assert!(!is_empty(&guesses[rep].mask));
    let best = guesses
        .iter()
        .filter(|g| !is_empty(&g.mask))
        .map(|g| g.confidence)
        .fold(f32::MIN, f32::max);
    assert_eq!(guesses[rep].confidence, best);
}

#[test]
fn representative_sample_caps_at_three() {
    let v = representative_sample(120);
    assert_eq!(v.len(), 3, "{v:?}");
    assert!(v.windows(2).all(|p| p[0] < p[1]), "{v:?}");
    assert!(v.iter().all(|&i| i < 120), "{v:?}");
}

#[test]
fn set_layers_skip_unreadable_photos_and_cancel() {
    let synth = common::synth_dir();
    let dir = tempfile::tempdir().expect("tempdir");
    let mut paths = Vec::new();
    for i in 0..4 {
        let p = dir.path().join(format!("f{i}.png"));
        fs::copy(synth.join("fixed/marked").join(format!("0{i}.png")), &p).expect("copy");
        paths.push(p);
    }
    let garbage = dir.path().join("broken.jpg");
    fs::write(&garbage, b"not a photo").expect("write");
    paths.insert(2, garbage);
    paths.push(dir.path().join("missing.png"));

    let rec = Recorder::new(false);
    let report = |f: f32| rec.seen.lock().unwrap().push(f);
    let progress = Progress {
        on_fraction: &report,
        cancel: &rec.cancel,
    };
    let layers = build_set_layers(&paths, &progress).expect("bad photos are skipped");
    assert_progress(&rec.fractions());
    assert!(layers.get(1536, 1024).is_some());
    assert!(
        build_set_layers(&[], &Progress::none())
            .expect("empty")
            .is_empty()
    );

    // Cancelled before it starts.
    let rec = Recorder::new(true);
    let report = |f: f32| rec.seen.lock().unwrap().push(f);
    let progress = Progress {
        on_fraction: &report,
        cancel: &rec.cancel,
    };
    let r = build_set_layers(&paths, &progress);
    assert!(matches!(r, Err(Error::Cancelled)), "{r:?}");
    assert_eq!(rec.fractions(), [0.0]);

    // Cancelled while decoding.
    let rec = Recorder::new(false);
    let report = |f: f32| {
        rec.seen.lock().unwrap().push(f);
        if f > 0.3 {
            rec.cancel.store(true, Ordering::Relaxed);
        }
    };
    let progress = Progress {
        on_fraction: &report,
        cancel: &rec.cancel,
    };
    let r = build_set_layers(&paths, &progress);
    assert!(matches!(r, Err(Error::Cancelled)), "{r:?}");
    let seen = rec.fractions();
    assert!(seen.iter().all(|&f| f < 1.0), "{seen:?}");
}

#[test]
fn remove_one_respects_orientation() {
    let Some(engine) = engine() else { return };
    let set = common::synth_dir().join("styles");
    let name = "00_centered_text.png";
    let upright = open_rgb(&set.join("marked").join(name));
    let clean = open_rgb(&set.join("clean").join(name));
    let gt = open_mask(&set.join("mask").join(name));

    // Orientation 6: the camera stored the upright picture rotated 90 degrees anticlockwise.
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("rotated.jpg");
    fs::write(&src, jpeg_bytes(&imageops::rotate270(&upright), Some(6))).expect("write");
    let photo = load(&src).expect("load");
    assert_eq!(photo.rgb.dimensions(), upright.dimensions());

    // The mask is painted on the upright photo.
    let painted = mask::dilate(&gt, 3);
    let out_dir = dir.path().join("out");
    let out =
        remove_one(engine, &photo, &painted, &out_dir, None, Progress::none()).expect("remove");
    assert_eq!(out.parent(), Some(out_dir.as_path()));
    assert_eq!(out.file_name().unwrap(), "rotated.jpg");

    let bytes = fs::read(&out).expect("read result");
    assert_eq!(file_orientation(&bytes), Some(1));
    // Stored pixels are upright: decoded without applying any orientation.
    let stored = image::load_from_memory(&bytes).expect("decode").to_rgb8();
    assert_eq!(stored.dimensions(), upright.dimensions());

    let before = psnr_over(&photo.rgb, &clean, &gt);
    let after = psnr_over(&stored, &clean, &gt);
    eprintln!("mark psnr: before {before:.2} dB, after {after:.2} dB");
    assert!(
        after >= before + 6.0,
        "before {before:.2}, after {after:.2}"
    );
    let away = mask::dilate(&painted, 4);
    let diff = mean_diff_outside(&stored, &photo.rgb, &away);
    eprintln!("mean diff away from the mask {diff:.3}");
    assert!(diff < 2.0, "mean diff outside the mask {diff}");
}

#[test]
fn remove_one_never_touches_original() {
    let Some(engine) = engine() else { return };
    let set = common::synth_dir().join("styles");
    let name = "01_corner_logo.png";
    let img = open_rgb(&set.join("marked").join(name));
    let gt = open_mask(&set.join("mask").join(name));

    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("photo.jpg");
    fs::write(&src, jpeg_bytes(&img, None)).expect("write");
    let hash = file_hash(&src);
    let modified = fs::metadata(&src)
        .and_then(|m| m.modified())
        .expect("mtime");
    let photo = load(&src).expect("load");
    let painted: GrayImage = mask::dilate(&gt, 3);

    // Into the default output folder next to the original.
    let first = remove_one(
        engine,
        &photo,
        &painted,
        &output_dir_for(&src),
        None,
        Progress::none(),
    )
    .expect("remove");
    assert_eq!(first.parent(), Some(output_dir_for(&src).as_path()));
    // Into the original's own folder: the result takes a new name.
    let second =
        remove_one(engine, &photo, &painted, dir.path(), None, Progress::none()).expect("remove");
    assert_eq!(second, dir.path().join("photo (2).jpg"));

    assert_eq!(file_hash(&src), hash, "original changed");
    assert_eq!(
        fs::metadata(&src)
            .and_then(|m| m.modified())
            .expect("mtime"),
        modified
    );
    let want: BTreeSet<String> = ["Without watermark", "photo (2).jpg", "photo.jpg"]
        .map(String::from)
        .into();
    assert_eq!(names_in(dir.path()), want);
    assert_ne!(file_hash(&second), hash);
}

const FIXED_ALPHA: f32 = 0.25;

/// The fixed set re-blended at `alpha` (the stored photos carry the mark at 0.25): the faint
/// photos, their ground-truth masks and the photos written as PNGs into `dir`.
fn faint_fixed(alpha: f32, dir: &Path) -> (Vec<RgbImage>, Vec<Mask>, Vec<PathBuf>) {
    let set = common::synth_dir().join("fixed");
    let (mut photos, mut gts, mut paths) = (Vec::new(), Vec::new(), Vec::new());
    for i in 0..8 {
        let clean = open_rgb(&set.join("clean").join(format!("0{i}.png")));
        let mut marked = open_rgb(&set.join("marked").join(format!("0{i}.png")));
        for (m, c) in marked.pixels_mut().zip(clean.pixels()) {
            for ch in 0..3 {
                let d = m.0[ch] as f32 - c.0[ch] as f32;
                m.0[ch] = (c.0[ch] as f32 + d * alpha / FIXED_ALPHA).round() as u8;
            }
        }
        let path = dir.join(format!("f{i}.png"));
        marked.save(&path).expect("save");
        photos.push(marked);
        gts.push(open_mask(&set.join("mask").join(format!("0{i}.png"))));
        paths.push(path);
    }
    (photos, gts, paths)
}

fn no_progress_layers(paths: &[PathBuf]) -> SetLayers {
    build_set_layers(paths, &Progress::none()).expect("layers")
}

#[test]
fn prior_finds_faint_fixed_mark() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (photos, gts, paths) = faint_fixed(0.12, dir.path());
    let layers = no_progress_layers(&paths);
    assert!(layers.get(1536, 1024).is_some(), "the set has a layer");
    let tmpl = build_template(&photos[0], &gts[0]);
    let free: Vec<MatchStatus> = photos.iter().map(|p| find(&tmpl, p).status).collect();
    assert!(
        free.iter().any(|&s| s != MatchStatus::Found),
        "premise: free search misses at least one photo, got {free:?}"
    );
    let mut found = 0;
    for (i, p) in photos.iter().enumerate() {
        let m = match_photo(&tmpl, p, &layers);
        assert_ne!(m.status, MatchStatus::NotFound, "photo {i}");
        found += (m.status == MatchStatus::Found) as usize;
        if m.basis == MatchBasis::SetPosition {
            assert!(m.confidence >= PRIOR_UNSURE, "photo {i}");
            assert_eq!(m.transform, Some(Transform::IDENTITY), "photo {i}");
        }
        let q = iou(&m.mask, &gts[i]);
        assert!(q >= 0.8, "photo {i}: iou {q:.3}");
    }
    // Scored on the set's own edges of the mark, every faint photo clears PRIOR_FOUND (0.51 at
    // least); on photo 0's edges, mark and scene, 3 of the 7 others stayed Unsure.
    assert_eq!(found, 8, "only {found} of 8 Found");
}

#[test]
fn prior_unsure_on_weak_evidence() {
    // Photo 5 of the faint fixed set faded further, to alpha 0.06, scores 0.23 at the saved
    // position on the set's edges (0.49 at the set's own 0.12): between the thresholds, so it
    // is Unsure on the set position.
    let dir = tempfile::tempdir().expect("tempdir");
    let (photos, gts, paths) = faint_fixed(0.12, dir.path());
    let layers = no_progress_layers(&paths);
    let fainter = tempfile::tempdir().expect("tempdir");
    let (weak, _, _) = faint_fixed(0.06, fainter.path());
    let tmpl = build_template(&photos[0], &gts[0]);
    let m = match_photo(&tmpl, &weak[5], &layers);
    assert_eq!(m.status, MatchStatus::Unsure);
    assert_eq!(m.basis, MatchBasis::SetPosition);
    assert!(
        (PRIOR_UNSURE..PRIOR_FOUND).contains(&m.confidence),
        "{}",
        m.confidence
    );
    assert!(iou(&m.mask, &gts[5]) >= 0.8);
}

/// Photos `which` of the fixed set with their (stored, alpha 0.25) mark moved 400 px up, on
/// their clean scene, written as PNGs into `dir`; with the moved ground truth.
fn moved_up(which: std::ops::Range<u32>, dir: &Path) -> (Vec<PathBuf>, Vec<Mask>) {
    let set = common::synth_dir().join("fixed");
    let (mut paths, mut truths) = (Vec::new(), Vec::new());
    for i in which {
        let clean = open_rgb(&set.join("clean").join(format!("0{i}.png")));
        let marked = open_rgb(&set.join("marked").join(format!("0{i}.png")));
        let gt = open_mask(&set.join("mask").join(format!("0{i}.png")));
        let mut out = clean.clone();
        let (w, h) = clean.dimensions();
        let mut truth = Mask::new(w, h);
        for y in 400..h {
            for x in 0..w {
                let (m, c) = (marked.get_pixel(x, y), clean.get_pixel(x, y));
                if m != c {
                    out.put_pixel(x, y - 400, *m);
                }
                truth.put_pixel(x, y - 400, *gt.get_pixel(x, y));
            }
        }
        let path = dir.join(format!("m{i}.png"));
        out.save(&path).expect("save");
        paths.push(path);
        truths.push(truth);
    }
    (paths, truths)
}

#[test]
fn prior_ignored_without_support() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (photos, gts, _) = faint_fixed(0.12, dir.path());
    // Another set whose strong mark sits 400 px higher than the template's.
    let moved_dir = tempfile::tempdir().expect("tempdir");
    let (moved, _) = moved_up(0..4, moved_dir.path());
    let elsewhere = no_progress_layers(&moved);
    assert!(
        elsewhere.get(1536, 1024).is_some(),
        "the other layer exists"
    );
    let tmpl = build_template(&photos[0], &gts[0]);
    for p in &photos {
        let (a, b) = (find(&tmpl, p), match_photo(&tmpl, p, &elsewhere));
        assert_eq!((a.status, a.confidence), (b.status, b.confidence));
        assert_eq!(a.mask, b.mask);
        assert_eq!(b.basis, MatchBasis::Search);
    }
}

#[test]
fn prior_ignored_for_other_size() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (photos, gts, paths) = faint_fixed(0.12, dir.path());
    let layers = no_progress_layers(&paths);
    let tmpl = build_template(&photos[0], &gts[0]);
    let small = imageops::resize(&photos[1], 1200, 800, imageops::FilterType::Triangle);
    let m = match_photo(&tmpl, &small, &layers);
    assert_eq!(m.basis, MatchBasis::Search);
}

#[test]
fn negatives_stay_not_found() {
    // The fixed set's real layer and the template of its photo 0 against photos without the
    // mark: the other clean scenes of the set (photo 0's own scene is the template's, so it is
    // left out) and the negative set resized to the template size.
    let dir = tempfile::tempdir().expect("tempdir");
    let (photos, gts, paths) = faint_fixed(0.12, dir.path());
    let layers = no_progress_layers(&paths);
    assert!(layers.get(1536, 1024).is_some(), "the set has a layer");
    let tmpl = build_template(&photos[0], &gts[0]);
    let synth = common::synth_dir();
    let mut negatives: Vec<RgbImage> = (1..8)
        .map(|i| open_rgb(&synth.join(format!("fixed/clean/0{i}.png"))))
        .collect();
    for e in fs::read_dir(synth.join("negative/marked")).expect("read dir") {
        let img = open_rgb(&e.expect("entry").path());
        negatives.push(imageops::resize(
            &img,
            1536,
            1024,
            imageops::FilterType::Triangle,
        ));
    }
    assert_eq!(negatives.len(), 13);
    let prior = PositionPrior::new(&tmpl, &layers)
        .filter(PositionPrior::supports)
        .expect("the set supports the position");
    for (i, img) in negatives.iter().enumerate() {
        let m = match_photo(&tmpl, img, &layers);
        assert_ne!(m.status, MatchStatus::Found, "negative {i}");
        // At the saved position a photo without the mark scores at most 0.07 on the set's own
        // edges (0.155 on photo 0's), far under the Unsure bar.
        let score = puw_core::template::saved_score(prior.scored(), img).expect("same size");
        assert!(score < PRIOR_UNSURE / 2.0, "negative {i}: {score:.3}");
        // With no layer the prior cannot apply at all.
        let m = match_photo(&tmpl, img, &SetLayers::none());
        assert_ne!(m.status, MatchStatus::Found, "negative {i} without a layer");
        assert_eq!(m.basis, MatchBasis::Search);
    }
}

/// A mark of thin lines (a 1 px rectangle outline, 600 x 400, white at alpha 0.5) on the
/// fixed set's 8 clean scenes enlarged to 3072 x 2048, and a drawing 70 px around it (as far as
/// the drawing may reach beyond the layer at this size): the layer holds the lines only,
/// a small share of such a generous drawing, yet the set supports its position. The share sits
/// between `PRIOR_LAYER_SHARE` (0.05) and the old bar of 0.1, which refused it.
#[test]
fn thin_mark_in_a_generous_drawing_keeps_the_set_position() {
    let set = common::synth_dir().join("fixed");
    let dir = tempfile::tempdir().expect("tempdir");
    let (x0, y0, x1, y1) = (1200u32, 600u32, 1800u32, 1000u32);
    let mut truth = Mask::new(3072, 2048);
    let (mut photos, mut paths) = (Vec::new(), Vec::new());
    for i in 0..8 {
        let small = open_rgb(&set.join(format!("clean/0{i}.png")));
        let mut img = imageops::resize(&small, 3072, 2048, imageops::FilterType::Triangle);
        for y in y0..=y1 {
            for x in x0..=x1 {
                if x == x0 || x == x1 || y == y0 || y == y1 {
                    for v in img.get_pixel_mut(x, y).0.iter_mut() {
                        *v = (*v as f32 * 0.5 + 255.0 * 0.5).round() as u8;
                    }
                    truth.put_pixel(x, y, image::Luma([255]));
                }
            }
        }
        let path = dir.path().join(format!("t{i}.png"));
        img.save(&path).expect("save");
        photos.push(img);
        paths.push(path);
    }
    let layers = no_progress_layers(&paths);
    let layer = layers.get(3072, 2048).expect("the set has a layer");
    let drawing = mask::dilate(&truth, 70);
    let (all, on) = drawing
        .as_raw()
        .iter()
        .zip(layer.as_raw())
        .filter(|(d, _)| **d != 0)
        .fold((0u64, 0u64), |(a, o), (_, l)| (a + 1, o + (*l != 0) as u64));
    let share = on as f64 / all as f64;
    assert!(
        (0.05..0.1).contains(&share),
        "premise: layer share {share:.3}"
    );
    let tmpl = build_template(&photos[0], &drawing);
    let prior = PositionPrior::new(&tmpl, &layers)
        .filter(PositionPrior::supports)
        .expect("the set supports the position");
    for (i, p) in photos.iter().enumerate().skip(1) {
        let m = match_with(&tmpl, Some(&prior), p, &AtomicBool::new(false));
        assert_eq!(m.basis, MatchBasis::SetPosition, "photo {i}");
        assert_eq!(
            m.status,
            MatchStatus::Found,
            "photo {i}: {:.3}",
            m.confidence
        );
    }
}

/// Where the mark sits on photo `i` of [`patch_set`] when it moves.
const MOVES: [(i64, i64); 8] = [
    (0, 0),
    (-500, -600),
    (300, -650),
    (-550, 100),
    (350, -250),
    (-200, -400),
    (150, -350),
    (-600, -150),
];

/// How the mark and a static patch of scene sit in [`patch_set`].
#[derive(Clone, Copy, Debug)]
enum Patch {
    /// The mark moves ([`MOVES`]); the patch is far from it.
    MovingFar,
    /// The mark moves; on photo 0 the patch ends 8 px above it.
    MovingNext,
    /// As `MovingNext`, the mark cut to its top 30 rows (a thin line of text), so that all of
    /// it lies close to the patch.
    MovingNextThin,
    /// The mark stays, on every photo but the last (which is clean); the patch is far from it.
    CleanLastFar,
    /// As `CleanLastFar`, the patch ending 8 px above the mark.
    CleanLastNext,
    /// The mark stays, on every photo; a smaller patch (300 x 120) sits 25 px to its right,
    /// close enough to be grouped with it.
    FixedNear,
}

/// `n` photos of different scenes (the fixed set's clean photos) sharing one static patch of
/// scene (340 x 250 unless said otherwise, from a negative photo) at the same place, with the
/// fixed set's mark blended at `alpha` as `kind` says. The photos, their true marks (empty
/// where there is none) and their paths in `dir`.
fn patch_set(
    kind: Patch,
    n: usize,
    alpha: f32,
    dir: &Path,
) -> (Vec<RgbImage>, Vec<Mask>, Vec<PathBuf>) {
    let set = common::synth_dir().join("fixed");
    let clean0 = open_rgb(&set.join("clean/00.png"));
    let marked0 = open_rgb(&set.join("marked/00.png"));
    let gt = open_mask(&set.join("mask/00.png"));
    let src = open_rgb(&common::synth_dir().join("negative/clean/00.png"));
    let (px, py, pw, ph) = match kind {
        Patch::MovingNext | Patch::MovingNextThin | Patch::CleanLastNext => (680, 510, 340, 250),
        Patch::FixedNear => (1098, 760, 300, 120),
        _ => (1150, 700, 340, 250),
    };
    let (mut photos, mut truths, mut paths) = (Vec::new(), Vec::new(), Vec::new());
    for (i, &shift) in MOVES.iter().enumerate().take(n) {
        let mut img = open_rgb(&set.join(format!("clean/0{}.png", i % 8)));
        let (w, h) = img.dimensions();
        for y in 0..ph {
            for x in 0..pw {
                img.put_pixel(px + x, py + y, *src.get_pixel(300 + x, 200 + y));
            }
        }
        let moving = matches!(
            kind,
            Patch::MovingFar | Patch::MovingNext | Patch::MovingNextThin
        );
        let (dx, dy) = if moving { shift } else { (0, 0) };
        let mut truth = Mask::new(w, h);
        if moving || i + 1 < n || matches!(kind, Patch::FixedNear) {
            for (x, y, v) in gt.enumerate_pixels() {
                if v.0[0] == 0 || (matches!(kind, Patch::MovingNextThin) && y >= 768 + 30) {
                    continue;
                }
                let (tx, ty) = ((x as i64 + dx) as u32, (y as i64 + dy) as u32);
                let (c, m) = (clean0.get_pixel(x, y), marked0.get_pixel(x, y));
                let p = img.get_pixel_mut(tx, ty);
                for ch in 0..3 {
                    // The mark's own colour, from photo 0 where it is blended at 0.25.
                    let mark = c.0[ch] as f32 + (m.0[ch] as f32 - c.0[ch] as f32) / FIXED_ALPHA;
                    let b = p.0[ch] as f32;
                    p.0[ch] = (b + alpha * (mark - b)).round().clamp(0.0, 255.0) as u8;
                }
                truth.put_pixel(tx, ty, image::Luma([255]));
            }
        }
        let path = dir.join(format!("{kind:?}-{i}.png"));
        img.save(&path).expect("save");
        photos.push(img);
        truths.push(truth);
        paths.push(path);
    }
    (photos, truths, paths)
}

/// The first guess as `guess_photo` makes it when the detector finds the mark `truth`: the mark
/// and the set's layer, grown by `grow_for(0.004)`.
fn first_guess(truth: &Mask, layers: &SetLayers) -> Mask {
    let (w, h) = truth.dimensions();
    let both = match layers.get(w, h) {
        Some(l) => mask::union(truth, l),
        None => truth.clone(),
    };
    mask::dilate(&both, mask::grow_for(w, h, 0.004))
}

/// Matches the first guess of photo 0 of `photos` on every other photo, and checks that no
/// photo is `Found` where its own mark is not (at least half of the mark covered; a photo
/// without the mark is never `Found`). Returns how many photos were placed (`Found` or
/// `Unsure`) at their mark.
fn no_false_found(label: &str, photos: &[RgbImage], truths: &[Mask], layers: &SetLayers) -> usize {
    let tmpl = build_template(&photos[0], &first_guess(&truths[0], layers));
    let prior = PositionPrior::new(&tmpl, layers);
    let mut right = 0;
    for (j, (p, truth)) in photos.iter().zip(truths).enumerate().skip(1) {
        let m = match_with(&tmpl, prior.as_ref(), p, &AtomicBool::new(false));
        let c = covers(&m.mask, truth);
        if m.status == MatchStatus::Found {
            assert!(
                c >= 0.5,
                "{label} photo {j}: Found ({:?}, {:.2}) covering {c:.2} of its mark",
                m.basis,
                m.confidence
            );
        }
        right += (m.status != MatchStatus::NotFound && c >= 0.5) as usize;
    }
    right
}

/// A photo without the mark is not `Found` at the set position, whatever else the set holds:
/// - the faint fixed set with one photo replaced by a clean scene, the true area as drawing;
/// - a static patch of scene far from the mark, or ending 8 px above it (so that mark and patch
///   can share one piece of the drawing), with the last photo clean, in sets of 3, 4, 5 and 8,
///   the first guess as drawing.
///
/// Before the parts of the layer were scored one by one and the drawing had to stay near the
/// layer, the clean photo next to the patch was `Found` at 0.45-0.80.
#[test]
fn clean_photo_of_the_set_is_not_found() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (photos, gts, mut paths) = faint_fixed(0.12, dir.path());
    let clean = common::synth_dir().join("fixed/clean/07.png");
    paths[7] = clean.clone();
    let layers = no_progress_layers(&paths);
    let tmpl = build_template(&photos[0], &gts[0]);
    let img = open_rgb(&clean);
    assert_ne!(match_photo(&tmpl, &img, &layers).status, MatchStatus::Found);
    if let Some(prior) = PositionPrior::new(&tmpl, &layers).filter(PositionPrior::supports) {
        let score = puw_core::template::saved_score(prior.scored(), &img).expect("same size");
        assert!(score < PRIOR_UNSURE, "{score:.3}");
    }

    for kind in [Patch::CleanLastFar, Patch::CleanLastNext] {
        for n in [3, 4, 5, 8] {
            let dir = tempfile::tempdir().expect("tempdir");
            let (photos, truths, paths) = patch_set(kind, n, FIXED_ALPHA, dir.path());
            let layers = no_progress_layers(&paths);
            assert!(
                layers.get(1536, 1024).is_some(),
                "premise: {kind:?} {n} has a layer"
            );
            no_false_found(&format!("{kind:?} {n}"), &photos, &truths, &layers);
        }
    }
}

/// A faint fixed mark (alpha 0.12) and a small static patch of scene just beside it, so that
/// the layer holds both in one group: the first guess holds both too, and a person who erases
/// the patch from it (it is not the watermark) keeps the set position; every other photo is
/// `Found` there, in sets of 3, 5 and 8 photos. Before, rule 1 asked 90 % of the whole layer
/// to lie in the drawing, so erasing the patch left only rule 2, which scores the reference's
/// own edges, and the faint mark was not `Found` (the owner's failing sets: the search then
/// found the scene).
#[test]
fn erased_scene_part_keeps_the_set_position() {
    for n in [3, 5, 8] {
        let dir = tempfile::tempdir().expect("tempdir");
        let (photos, truths, paths) = patch_set(Patch::FixedNear, n, 0.12, dir.path());
        let layers = no_progress_layers(&paths);
        let guess = first_guess(&truths[0], &layers);
        let erased = mask::dilate(&truths[0], mask::grow_for(1536, 1024, 0.004));
        for (name, drawing) in [("as guessed", guess), ("patch erased", erased)] {
            let tmpl = build_template(&photos[0], &drawing);
            let prior = PositionPrior::new(&tmpl, &layers);
            assert!(
                prior.as_ref().is_some_and(PositionPrior::supports),
                "{n} photos, {name}: no set position"
            );
            for (j, p) in photos.iter().enumerate().skip(1) {
                let m = match_with(&tmpl, prior.as_ref(), p, &AtomicBool::new(false));
                assert_eq!(
                    m.basis,
                    MatchBasis::SetPosition,
                    "{n} photos, {name}, photo {j}"
                );
                assert_eq!(
                    m.status,
                    MatchStatus::Found,
                    "{n} photos, {name}, photo {j}"
                );
            }
        }
    }
}

/// A static patch of scene on every photo and a mark that moves: the first guess holds the
/// patch's layer (it is added to every guess), yet no photo is ever `Found` where its own mark
/// is not. The set position is held back (the patch never changes from photo to photo, so it
/// is no evidence of a mark): at most `Unsure`. In sets of 3, 5 and 8 photos, with the patch
/// far from the mark, ending 8 px above it, and ending 8 px above a thin mark (its top 30 rows)
/// that lies wholly within the drawing's reach of the patch. Before, every other photo was
/// `Found` at the saved place (0.36-0.94; the thin mark 0.68-0.87 after the first fix).
#[test]
fn static_scene_never_places_a_moving_mark() {
    for kind in [Patch::MovingFar, Patch::MovingNext, Patch::MovingNextThin] {
        for n in [3, 5, 8] {
            let dir = tempfile::tempdir().expect("tempdir");
            let (photos, truths, paths) = patch_set(kind, n, FIXED_ALPHA, dir.path());
            let layers = no_progress_layers(&paths);
            assert!(
                layers.get(1536, 1024).is_some(),
                "premise: {kind:?} {n} has a layer"
            );
            let label = format!("{kind:?} {n}");
            let tmpl = build_template(&photos[0], &first_guess(&truths[0], &layers));
            let prior = PositionPrior::new(&tmpl, &layers);
            assert!(
                prior.as_ref().is_some_and(|p| !p.supports()),
                "{label}: the set position is not held back"
            );
            no_false_found(&label, &photos, &truths, &layers);
        }
    }
}

/// Drawings grown by 1, 2, 3 and 3.9 % of the long side (16, 31, 46 and 60 px) around the
/// mark, on sets of 3 and of 8 photos of the fixed set: every other photo stays at the saved
/// place, `Found`, never not found and never elsewhere (a person painting with a big brush
/// overshoots).
#[test]
fn generous_drawings_stay_at_the_set_position() {
    let set = common::synth_dir().join("fixed");
    let truth = open_mask(&set.join("mask/00.png"));
    for n in [3, 8] {
        let paths: Vec<PathBuf> = (0..n)
            .map(|i| set.join("marked").join(format!("0{i}.png")))
            .collect();
        let photos: Vec<RgbImage> = paths.iter().map(|p| open_rgb(p)).collect();
        let layers = no_progress_layers(&paths);
        for grow in [16, 31, 46, 60] {
            let drawing = mask::dilate(&truth, grow);
            let tmpl = build_template(&photos[0], &drawing);
            for (i, p) in photos.iter().enumerate().skip(1) {
                let m = match_photo(&tmpl, p, &layers);
                eprintln!(
                    "{n} photos, grown {grow} px, photo {i}: {:?} {:?} {:.3}",
                    m.status, m.basis, m.confidence
                );
                assert_eq!(
                    m.basis,
                    MatchBasis::SetPosition,
                    "{n} photos, {grow} px, photo {i}"
                );
                assert_eq!(
                    m.status,
                    MatchStatus::Found,
                    "{n} photos, {grow} px, photo {i}"
                );
            }
        }
    }
}

/// A drawing grown far beyond the mark (60 px around it, about five times the mark's area,
/// 3.9 % of the long side, as a person painting with a big brush draws it) on the fixed set:
/// the search alone finds it on none of the other photos, and at least half of the drawing lies
/// off the layer, yet the layer lies inside the drawing, so the position is scored on the layer
/// and every photo is `Found` at the saved position, with the whole drawing as its area.
#[test]
fn prior_matches_a_drawing_grown_beyond_the_mark() {
    let set = common::synth_dir().join("fixed");
    let paths: Vec<PathBuf> = (0..8)
        .map(|i| set.join("marked").join(format!("0{i}.png")))
        .collect();
    let photos: Vec<RgbImage> = paths.iter().map(|p| open_rgb(p)).collect();
    let layers = no_progress_layers(&paths);
    let drawing = mask::dilate(&open_mask(&set.join("mask/00.png")), 60);
    let tmpl = build_template(&photos[0], &drawing);
    let prior = PositionPrior::new(&tmpl, &layers)
        .filter(PositionPrior::supports)
        .expect("the set supports the position");
    assert!(
        prior.scored() != &tmpl,
        "scored on the layer, not on the drawing"
    );
    for (i, p) in photos.iter().enumerate().skip(1) {
        assert_ne!(
            find(&tmpl, p).status,
            MatchStatus::Found,
            "premise, photo {i}"
        );
        let m = match_photo(&tmpl, p, &layers);
        assert_eq!(m.status, MatchStatus::Found, "photo {i}");
        assert_eq!(m.basis, MatchBasis::SetPosition, "photo {i}");
        assert_eq!(m.transform, Some(Transform::IDENTITY), "photo {i}");
        assert!(iou(&m.mask, &drawing) >= 0.99, "photo {i}");
        let again = match_with(&tmpl, Some(&prior), p, &AtomicBool::new(false));
        assert_eq!((again.status, again.confidence), (m.status, m.confidence));
    }
}

/// Share of the pixels set in `truth` that are also set in `m`.
fn covers(m: &Mask, truth: &Mask) -> f64 {
    let (all, hit) = truth
        .as_raw()
        .iter()
        .zip(m.as_raw())
        .filter(|(t, _)| **t != 0)
        .fold((0u64, 0u64), |(a, h), (_, v)| (a + 1, h + (*v != 0) as u64));
    hit as f64 / all.max(1) as f64
}

/// Marks that move are never placed at the set position, and a mark the search finds where it
/// moved to keeps its place.
///
/// 1. The random and styles sets: the batch layer, where there is one, holds static scene
///    edges and the marks move. With the first guess as drawing (the mark and the layer, grown),
///    no photo is `Found` where its own mark is not.
/// 2. A set position the layer does support (the fixed set's eight photos, the drawing the true
///    area of photo 0, as it is and grown by 24 px) on photos whose mark moved 400 px up: no
///    set-position result, and every photo the search alone finds where its mark is (IoU at
///    least 0.5 with the moved mark) keeps that place and score, as `Unsure`: a set that shows
///    the mark at one place does not trust a `Found` somewhere else.
#[test]
fn prior_never_places_a_moving_mark() {
    let synth = common::synth_dir();
    for set in ["random", "styles"] {
        let mut names: Vec<String> = fs::read_dir(synth.join(set).join("marked"))
            .expect("read dir")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        let paths: Vec<PathBuf> = names
            .iter()
            .map(|n| synth.join(set).join("marked").join(n))
            .collect();
        let layers = no_progress_layers(&paths);
        let photos: Vec<RgbImage> = paths.iter().map(|p| open_rgb(p)).collect();
        let truths: Vec<Mask> = names
            .iter()
            .map(|n| open_mask(&synth.join(set).join("mask").join(n)))
            .collect();
        let mut with_layer = 0;
        for (i, img) in photos.iter().enumerate() {
            let (w, h) = img.dimensions();
            with_layer += layers.get(w, h).is_some() as usize;
            let tmpl = build_template(img, &first_guess(&truths[i], &layers));
            let prior = PositionPrior::new(&tmpl, &layers);
            for (j, p) in photos.iter().enumerate() {
                if j == i || p.dimensions() != img.dimensions() {
                    continue;
                }
                let m = match_with(&tmpl, prior.as_ref(), p, &AtomicBool::new(false));
                if m.status == MatchStatus::Found {
                    let c = covers(&m.mask, &truths[j]);
                    assert!(c >= 0.5, "{set} {i} on {j}: {:?} covers {c:.2}", m.basis);
                }
            }
        }
        assert!(with_layer > 0, "premise: {set} has a layer for some size");
    }

    let set = synth.join("fixed");
    let fixed: Vec<PathBuf> = (0..8)
        .map(|i| set.join("marked").join(format!("0{i}.png")))
        .collect();
    let layers = no_progress_layers(&fixed);
    let reference = open_rgb(&fixed[0]);
    let dir = tempfile::tempdir().expect("tempdir");
    // Photo 0's own scene is the template's, so its moved copy is left out (see
    // `negatives_stay_not_found`).
    let (moved, truths) = moved_up(1..8, dir.path());
    let mut kept = 0;
    for grow in [0, 24] {
        let drawing = mask::dilate(&open_mask(&set.join("mask/00.png")), grow);
        let tmpl = build_template(&reference, &drawing);
        let prior = PositionPrior::new(&tmpl, &layers)
            .filter(PositionPrior::supports)
            .expect("premise: the layer supports it");
        for (path, truth) in moved.iter().zip(&truths) {
            let p = open_rgb(path);
            let m = match_with(&tmpl, Some(&prior), &p, &AtomicBool::new(false));
            assert_ne!(
                m.basis,
                MatchBasis::SetPosition,
                "{path:?} grow {grow}: {:?}",
                m.status
            );
            let free = find(&tmpl, &p);
            let moved_by =
                |r: &puw_core::MatchResult| r.transform.map(|t| ((t.dx).round(), (t.dy).round()));
            if free.status == MatchStatus::Found && covers(&free.mask, truth) >= 0.5 {
                kept += 1;
                assert_eq!(m.status, MatchStatus::Unsure, "{path:?} grow {grow}");
                assert_eq!(m.confidence, free.confidence, "{path:?} grow {grow}");
                assert_eq!(moved_by(&m), moved_by(&free), "{path:?} grow {grow}");
            }
        }
    }
    assert!(kept > 0, "premise: the search finds some moved marks");
}

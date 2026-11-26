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
    assert!(found >= 4, "only {found} of 8 Found");
}

#[test]
fn prior_unsure_on_weak_evidence() {
    // Photo 5 of the faint fixed set scores 0.175 at the saved position: between the
    // thresholds, so it is Unsure on the set position.
    let dir = tempfile::tempdir().expect("tempdir");
    let (photos, gts, paths) = faint_fixed(0.12, dir.path());
    let layers = no_progress_layers(&paths);
    let tmpl = build_template(&photos[0], &gts[0]);
    let m = match_photo(&tmpl, &photos[5], &layers);
    assert_eq!(m.status, MatchStatus::Unsure);
    assert_eq!(m.basis, MatchBasis::SetPosition);
    assert!(
        (PRIOR_UNSURE..PRIOR_FOUND).contains(&m.confidence),
        "{}",
        m.confidence
    );
    assert!(iou(&m.mask, &gts[5]) >= 0.8);
}

#[test]
fn prior_ignored_without_support() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (photos, gts, _) = faint_fixed(0.12, dir.path());
    // Another set whose strong mark sits 400 px higher than the template's.
    let set = common::synth_dir().join("fixed");
    let moved_dir = tempfile::tempdir().expect("tempdir");
    let mut moved = Vec::new();
    for i in 0..4 {
        let clean = open_rgb(&set.join("clean").join(format!("0{i}.png")));
        let marked = open_rgb(&set.join("marked").join(format!("0{i}.png")));
        let mut out = clean.clone();
        let (w, h) = clean.dimensions();
        for y in 400..h {
            for x in 0..w {
                let (m, c) = (marked.get_pixel(x, y), clean.get_pixel(x, y));
                if m != c {
                    out.put_pixel(x, y - 400, *m);
                }
            }
        }
        let path = moved_dir.path().join(format!("m{i}.png"));
        out.save(&path).expect("save");
        moved.push(path);
    }
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
    for (i, img) in negatives.iter().enumerate() {
        let m = match_photo(&tmpl, img, &layers);
        assert_ne!(m.status, MatchStatus::Found, "negative {i}");
        // With no layer the prior cannot apply at all.
        let m = match_photo(&tmpl, img, &SetLayers::none());
        assert_ne!(m.status, MatchStatus::Found, "negative {i} without a layer");
        assert_eq!(m.basis, MatchBasis::Search);
    }
}

/// A drawing grown far beyond the mark (60 px around it, about five times the mark's area,
/// like the grown first guess on a real outline mark) on the fixed set: the search alone finds
/// it on none of the other photos, and at least half of the drawing lies off the layer, yet the
/// layer lies inside the drawing, so the position is scored on the layer and every photo is
/// `Found` at the saved position, with the whole drawing as its area.
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
    let prior = PositionPrior::new(&tmpl, &layers).expect("the set supports the position");
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

/// On the random and styles sets the batch layer holds static scene edges and the marks move.
/// Whatever the drawing (the true area, or grown by 48 px), the set position never places the
/// template on a photo whose own mark is elsewhere.
#[test]
fn prior_never_places_a_moving_mark() {
    let synth = common::synth_dir();
    let mut priors = 0;
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
        for (i, img) in photos.iter().enumerate() {
            for grow in [0, 48] {
                let drawing = if grow == 0 {
                    truths[i].clone()
                } else {
                    mask::dilate(&truths[i], grow)
                };
                let tmpl = build_template(img, &drawing);
                let Some(prior) = PositionPrior::new(&tmpl, &layers) else {
                    continue;
                };
                priors += 1;
                for (j, p) in photos.iter().enumerate() {
                    if j == i || p.dimensions() != img.dimensions() {
                        continue;
                    }
                    let m = match_with(&tmpl, Some(&prior), p, &AtomicBool::new(false));
                    if m.basis == MatchBasis::SetPosition {
                        let q = iou(&m.mask, &truths[j]);
                        assert!(q > 0.0, "{set} {i} grow {grow} on {j}: {:?}", m.status);
                    }
                }
            }
        }
    }
    // Some drawings do pass a support rule (a tiny layer near the mark): the scores decide.
    assert!(priors > 0, "premise: no prior was ever made");
}

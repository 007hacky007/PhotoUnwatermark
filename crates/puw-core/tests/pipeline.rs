mod common;

use std::collections::BTreeSet;
use std::fs;
use std::hash::{DefaultHasher, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use image::{GrayImage, RgbImage, imageops};
use puw_core::batch::{BATCH_HIGH, BATCH_LOW, batch_lines, filter_layer, median_gradient};
use puw_core::detect::{GuessOptions, guess_single};
use puw_core::faces::{detect_faces_near, drop_eye_components, guard};
use puw_core::mask::{self, Mask, is_empty};
use puw_core::{
    Engine, Error, LoadedPhoto, ModelPaths, Progress, build_set_layers, guess_photo, load,
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

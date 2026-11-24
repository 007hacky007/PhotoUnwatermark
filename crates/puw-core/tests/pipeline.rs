mod common;

use std::collections::BTreeSet;
use std::fs;
use std::hash::{DefaultHasher, Hasher};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use image::{GrayImage, RgbImage, imageops};
use puw_core::batch::{BATCH_HIGH, BATCH_LOW, batch_lines, median_gradient};
use puw_core::detect::{GuessOptions, guess_single};
use puw_core::faces::{detect_faces, guard};
use puw_core::mask::{self, Mask};
use puw_core::{
    Engine, LoadedPhoto, ModelPaths, Progress, auto_guess_set, load, output_dir_for,
    pick_representative, remove_one,
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
            Some(Engine::new(&paths, false, None).expect("engine loads"))
        })
        .as_ref()
}

fn open_rgb(p: &Path) -> RgbImage {
    image::open(p).expect("open").to_rgb8()
}

fn open_mask(p: &Path) -> Mask {
    image::open(p).expect("open mask").to_luma8()
}

fn is_empty(m: &Mask) -> bool {
    m.as_raw().iter().all(|&v| v == 0)
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

#[test]
fn auto_guess_set_groups_by_size() {
    let Some(engine) = engine() else { return };
    let synth = common::synth_dir();
    let dir = tempfile::tempdir().expect("tempdir");
    // Three photos of one size: the fixed mark is in the batch layer.
    for i in 0..3 {
        let src = synth.join("fixed/marked").join(format!("0{i}.png"));
        fs::copy(&src, dir.path().join(format!("a{i}.png"))).expect("copy");
    }
    // The same mark in two photos of another size: too few for the batch layer.
    for i in 3..5 {
        let src = open_rgb(&synth.join("fixed/marked").join(format!("0{i}.png")));
        let small = imageops::resize(&src, 1200, 800, imageops::FilterType::Triangle);
        small
            .save(dir.path().join(format!("b{i}.png")))
            .expect("save");
    }
    // One photo of a third size.
    fs::copy(synth.join("random/marked/02.png"), dir.path().join("c.png")).expect("copy");

    let mut paths: Vec<PathBuf> = fs::read_dir(dir.path())
        .expect("read dir")
        .map(|e| e.expect("entry").path())
        .collect();
    paths.sort();
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

    let opts = GuessOptions::default();
    let guesses = auto_guess_set(engine, &photos, &opts).expect("mixed sizes are fine");
    assert_eq!(guesses.len(), photos.len());

    let group: Vec<&RgbImage> = photos[..3].iter().map(|p| &p.rgb).collect();
    let mag = median_gradient(&group).expect("median");
    let layer = batch_lines(&mag, BATCH_LOW, BATCH_HIGH).expect("fixed mark in the batch layer");

    for (i, (photo, got)) in photos.iter().zip(&guesses).enumerate() {
        let (w, h) = photo.rgb.dimensions();
        let keep = (i < 3).then_some(&layer);
        // The documented recipe, step by step.
        let single = guess_single(engine, &photo.rgb, &opts).expect("guess");
        let faces = detect_faces(engine, &photo.rgb).expect("faces");
        let guarded = guard(&single.mask, &faces, keep);
        let combined = match keep {
            Some(l) => mask::union(&guarded, l),
            None => guarded.clone(),
        };
        let want = mask::dilate(&combined, mask::grow_for(w, h, opts.grow_frac));
        assert!(got.mask == want, "photo {i}: mask differs from the recipe");
        let want_conf = if !is_empty(&guarded) {
            single.confidence
        } else if !is_empty(&combined) {
            1.0
        } else {
            0.0
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
            "photo {i} {w}x{h}: coverage {:.4} conf {:.3}",
            mask::coverage(&got.mask),
            got.confidence
        );
    }

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

mod common;

use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

use image::{GrayImage, Luma, RgbImage};
use puw_core::Error;
use puw_core::engine::{Engine, EngineOptions, ModelPaths};
use puw_core::inpaint::{EDGE_RADIUS, Progress, inpaint};
use puw_core::mask::{self, Mask};

fn engine() -> Option<&'static Engine> {
    static ENGINE: OnceLock<Option<Engine>> = OnceLock::new();
    ENGINE
        .get_or_init(|| {
            let Some(dir) = common::models_dir() else {
                eprintln!("skipped: PUW_MODELS_DIR not set");
                return None;
            };
            let paths = ModelPaths::from_dir(&dir).expect("model files present");
            Some(Engine::new(&paths, EngineOptions::default()).expect("engine loads"))
        })
        .as_ref()
}

fn pngs(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .expect("read dir")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "png"))
        .collect();
    v.sort();
    v
}

/// Smooth gradient with some texture, so a fill differs from the input.
fn photo(w: u32, h: u32) -> RgbImage {
    RgbImage::from_fn(w, h, |x, y| {
        let t = ((x * 7 + y * 13) % 17) as u8;
        image::Rgb([
            (x * 255 / w) as u8 / 2 + t,
            (y * 255 / h) as u8 / 2 + t,
            90 + t,
        ])
    })
}

fn rect_mask(w: u32, h: u32, x0: u32, y0: u32, x1: u32, y1: u32) -> Mask {
    GrayImage::from_fn(w, h, |x, y| {
        Luma([if (x0..x1).contains(&x) && (y0..y1).contains(&y) {
            255
        } else {
            0
        }])
    })
}

#[test]
fn pixels_outside_mask_unchanged() {
    let Some(engine) = engine() else { return };
    let img = photo(700, 600);
    let m = rect_mask(700, 600, 300, 250, 420, 330);
    let out = inpaint(engine, &img, &m, None, Progress::none()).expect("inpaint");
    let near = mask::dilate(&m, EDGE_RADIUS);
    for (x, y, p) in out.enumerate_pixels() {
        if near.get_pixel(x, y).0[0] == 0 {
            assert_eq!(p, img.get_pixel(x, y), "({x},{y})");
        }
    }
}

#[test]
fn every_hole_filled() {
    let Some(engine) = engine() else { return };
    let img = photo(700, 600);
    let m = rect_mask(700, 600, 300, 250, 420, 330);
    let out = inpaint(engine, &img, &m, None, Progress::none()).expect("inpaint");
    let (mut total, mut changed) = (0, 0);
    for (x, y, p) in m.enumerate_pixels() {
        if p.0[0] != 0 {
            total += 1;
            changed += (out.get_pixel(x, y) != img.get_pixel(x, y)) as u32;
        }
    }
    assert!(changed as f32 > 0.95 * total as f32, "{changed}/{total}");
}

#[test]
fn inpaint_small_image() {
    let Some(engine) = engine() else { return };
    let img = photo(300, 200);
    let m = rect_mask(300, 200, 100, 60, 180, 120);
    let out = inpaint(engine, &img, &m, None, Progress::none()).expect("inpaint");
    assert_eq!(out.dimensions(), (300, 200));
    assert_ne!(out.get_pixel(140, 90), img.get_pixel(140, 90));
}

#[test]
fn inpaint_cancel() {
    let Some(engine) = engine() else { return };
    let img = photo(300, 200);
    let m = rect_mask(300, 200, 100, 60, 180, 120);
    let cancel = AtomicBool::new(true);
    let r = inpaint(
        engine,
        &img,
        &m,
        None,
        Progress {
            on_fraction: &|_| {},
            cancel: &cancel,
        },
    );
    assert!(matches!(r, Err(Error::Cancelled)));
}

#[test]
fn empty_mask_returns_input() {
    let Some(engine) = engine() else { return };
    let img = photo(300, 200);
    let m = GrayImage::new(300, 200);
    let out = inpaint(engine, &img, &m, None, Progress::none()).expect("inpaint");
    assert_eq!(out, img);
}

#[test]
fn work_long_side_reduces_and_keeps_outside() {
    let Some(engine) = engine() else { return };
    let img = photo(900, 700);
    let m = rect_mask(900, 700, 400, 300, 500, 380);
    let out = inpaint(engine, &img, &m, Some(600), Progress::none()).expect("inpaint");
    assert_eq!(out.dimensions(), (900, 700));
    assert_ne!(out.get_pixel(450, 340), img.get_pixel(450, 340));
    let near = mask::dilate(&m, EDGE_RADIUS);
    for (x, y, p) in out.enumerate_pixels() {
        if near.get_pixel(x, y).0[0] == 0 {
            assert_eq!(p, img.get_pixel(x, y), "({x},{y})");
        }
    }
}

#[test]
fn progress_reaches_one() {
    let Some(engine) = engine() else { return };
    let img = photo(1000, 800);
    // Holes in two distant places so several tiles have work.
    let mut m = rect_mask(1000, 800, 50, 50, 150, 120);
    for y in 600..700 {
        for x in 800..950 {
            m.put_pixel(x, y, Luma([255]));
        }
    }
    let seen = Mutex::new(Vec::new());
    let cancel = AtomicBool::new(false);
    let report = |f: f32| seen.lock().unwrap().push(f);
    inpaint(
        engine,
        &img,
        &m,
        None,
        Progress {
            on_fraction: &report,
            cancel: &cancel,
        },
    )
    .expect("inpaint");
    assert!(!cancel.load(Ordering::Relaxed));
    let v = seen.into_inner().unwrap();
    assert!(v.len() >= 3, "{v:?}");
    assert_eq!(v[0], 0.0);
    assert!(v.windows(2).all(|p| p[0] <= p[1]), "{v:?}");
    assert_eq!(*v.last().unwrap(), 1.0);
}

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

#[test]
fn styles_quality() {
    let Some(engine) = engine() else { return };
    let set = common::synth_dir().join("styles");
    let mut psnrs = Vec::new();
    for marked in pngs(&set.join("marked")) {
        let name = marked.file_name().unwrap();
        let img = image::open(&marked).expect("open").to_rgb8();
        let clean = image::open(set.join("clean").join(name))
            .expect("clean")
            .to_rgb8();
        let gt: Mask = image::open(set.join("mask").join(name))
            .expect("gt")
            .to_luma8();
        let m = mask::dilate(&gt, 2);
        let calls = AtomicUsize::new(0);
        let count = |_: f32| {
            calls.fetch_add(1, Ordering::Relaxed);
        };
        let cancel = AtomicBool::new(false);
        let t = Instant::now();
        let out = inpaint(
            engine,
            &img,
            &m,
            None,
            Progress {
                on_fraction: &count,
                cancel: &cancel,
            },
        )
        .expect("inpaint");
        // One report before the first tile, then one per tile.
        let tiles = calls.load(Ordering::Relaxed).saturating_sub(1);
        let secs = t.elapsed().as_secs_f32();
        let p = psnr_over(&out, &clean, &gt);
        eprintln!(
            "{name:?}: {}x{} psnr {p:.2} dB, {secs:.1}s, {tiles} tiles, {:.1}s/tile",
            img.width(),
            img.height(),
            secs / tiles.max(1) as f32
        );
        psnrs.push(p);
    }
    assert!(!psnrs.is_empty());
    let mean = psnrs.iter().sum::<f64>() / psnrs.len() as f64;
    eprintln!("mean psnr {mean:.2} dB");
    assert!(mean >= 22.0, "mean psnr {mean}");
}

/// Fractions reported by one [`inpaint`] run.
fn inpaint_fractions(
    engine: &Engine,
    img: &RgbImage,
    m: &Mask,
    work_long_side: Option<u32>,
) -> (RgbImage, Vec<f32>) {
    let seen = Mutex::new(Vec::new());
    let cancel = AtomicBool::new(false);
    let report = |f: f32| seen.lock().unwrap().push(f);
    let out = inpaint(
        engine,
        img,
        m,
        work_long_side,
        Progress {
            on_fraction: &report,
            cancel: &cancel,
        },
    )
    .expect("inpaint");
    (out, seen.into_inner().unwrap())
}

/// Mean absolute difference of `a` and `b` over the non-zero pixels of `region`.
fn mean_diff_over(a: &RgbImage, b: &RgbImage, region: &Mask) -> f64 {
    let (mut sum, mut n) = (0f64, 0u64);
    for (x, y, p) in region.enumerate_pixels() {
        if p.0[0] != 0 {
            let (pa, pb) = (a.get_pixel(x, y).0, b.get_pixel(x, y).0);
            for c in 0..3 {
                sum += pa[c].abs_diff(pb[c]) as f64;
            }
            n += 3;
        }
    }
    sum / n.max(1) as f64
}

#[test]
fn thin_stroke_above_work_side_is_filled() {
    let Some(engine) = engine() else { return };
    let clean = photo(2000, 1000);
    // A 1 px white stroke; the photo is inpainted at a quarter of its size.
    let mut marked = clean.clone();
    let mut m = GrayImage::new(2000, 1000);
    for x in 600..1400 {
        marked.put_pixel(x, 500, image::Rgb([255, 255, 255]));
        m.put_pixel(x, 500, Luma([255]));
    }
    let (out, fractions) = inpaint_fractions(engine, &marked, &m, Some(500));
    let before = mean_diff_over(&marked, &clean, &m);
    let after = mean_diff_over(&out, &clean, &m);
    eprintln!("1 px stroke: mean diff before {before:.1}, after {after:.1}, {fractions:?}");
    assert!(fractions.len() >= 2, "no tile ran: {fractions:?}");
    assert_eq!(fractions.last(), Some(&1.0));
    assert!(after < 15.0, "stroke not filled: mean diff {after:.1}");
}

#[test]
fn antialiased_mask_value_is_hole() {
    let Some(engine) = engine() else { return };
    let clean = photo(700, 600);
    let mut marked = clean.clone();
    let mut m = GrayImage::new(700, 600);
    for y in 250..330 {
        for x in 300..420 {
            marked.put_pixel(x, y, image::Rgb([255, 255, 255]));
            m.put_pixel(x, y, Luma([64]));
        }
    }
    let (out, fractions) = inpaint_fractions(engine, &marked, &m, None);
    let after = mean_diff_over(&out, &clean, &m);
    eprintln!("mask value 64: mean diff after {after:.1}, {fractions:?}");
    assert!(fractions.len() >= 2, "no tile ran: {fractions:?}");
    assert!(after < 30.0, "hole not filled: mean diff {after:.1}");
}

#[test]
fn progress_ends_at_one_for_empty_mask() {
    let Some(engine) = engine() else { return };
    let img = photo(300, 200);
    let (out, fractions) = inpaint_fractions(engine, &img, &GrayImage::new(300, 200), None);
    assert_eq!(out, img);
    assert_eq!(fractions, [1.0]);
}

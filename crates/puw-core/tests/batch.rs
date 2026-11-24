mod common;

use image::{RgbImage, imageops};
use puw_core::Error;
use puw_core::batch::{BATCH_HIGH, BATCH_LOW, batch_lines, median_gradient};
use puw_core::mask::{self, Mask};
use std::path::{Path, PathBuf};
use std::time::Instant;

fn pngs(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("read dir")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "png"))
        .collect();
    v.sort();
    v
}

fn random_image(w: u32, h: u32, seed: u64) -> RgbImage {
    let mut s = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    RgbImage::from_fn(w, h, |_, _| {
        let mut px = [0u8; 3];
        for c in &mut px {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            *c = (s >> 56) as u8;
        }
        image::Rgb(px)
    })
}

/// Reference: per-image Sobel (reflect boundary) on 601 luma, then median, then hypot.
fn naive(images: &[&RgbImage]) -> Vec<f32> {
    let (w, h) = images[0].dimensions();
    let (w, h) = (w as i64, h as i64);
    let mut gxs = Vec::new();
    let mut gys = Vec::new();
    for img in images {
        let g = |x: i64, y: i64| -> f64 {
            let p = img
                .get_pixel(x.clamp(0, w - 1) as u32, y.clamp(0, h - 1) as u32)
                .0;
            (p[0] as f64 * 299.0 + p[1] as f64 * 587.0 + p[2] as f64 * 114.0) / 1000.0
        };
        let (sm, df) = ([1.0, 2.0, 1.0], [-1.0, 0.0, 1.0]);
        let mut gx = vec![0f64; (w * h) as usize];
        let mut gy = vec![0f64; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                for j in 0..3 {
                    for i in 0..3 {
                        let v = g(x + i - 1, y + j - 1);
                        gx[(y * w + x) as usize] += v * sm[j as usize] * df[i as usize];
                        gy[(y * w + x) as usize] += v * df[j as usize] * sm[i as usize];
                    }
                }
            }
        }
        gxs.push(gx);
        gys.push(gy);
    }
    let med = |mut v: Vec<f64>| {
        v.sort_by(f64::total_cmp);
        let n = v.len();
        if n % 2 == 1 {
            v[n / 2]
        } else {
            (v[n / 2 - 1] + v[n / 2]) / 2.0
        }
    };
    (0..(w * h) as usize)
        .map(|i| {
            let mx = med(gxs.iter().map(|g| g[i]).collect());
            let my = med(gys.iter().map(|g| g[i]).collect());
            mx.hypot(my) as f32
        })
        .collect()
}

#[test]
fn median_gradient_rejects_mixed_sizes() {
    let a = RgbImage::new(10, 10);
    let b = RgbImage::new(10, 11);
    assert!(matches!(
        median_gradient(&[&a, &b]),
        Err(Error::SizeMismatch)
    ));
    assert!(median_gradient(&[]).is_err());
}

#[test]
fn median_is_per_image() {
    let imgs = [
        random_image(64, 48, 1),
        random_image(64, 48, 2),
        random_image(64, 48, 3),
    ];
    let refs: Vec<&RgbImage> = imgs.iter().collect();
    let got = median_gradient(&refs).expect("median");
    let want = naive(&refs);
    let max = got
        .as_raw()
        .iter()
        .zip(&want)
        .map(|(a, b)| (a - b).abs())
        .fold(0f32, f32::max);
    assert!(max < 1e-3, "max abs diff {max}");
}

#[test]
fn median_spans_band_boundary() {
    // 600 rows crosses the 512-row band edge; even count exercises the two-middle mean.
    let imgs = [random_image(20, 600, 7), random_image(20, 600, 8)];
    let refs: Vec<&RgbImage> = imgs.iter().collect();
    let got = median_gradient(&refs).expect("median");
    let want = naive(&refs);
    let max = got
        .as_raw()
        .iter()
        .zip(&want)
        .map(|(a, b)| (a - b).abs())
        .fold(0f32, f32::max);
    assert!(max < 1e-3, "max abs diff {max}");
}

#[test]
fn fixed_set_found() {
    let set = common::synth_dir().join("fixed");
    let imgs: Vec<RgbImage> = pngs(&set.join("marked"))
        .iter()
        .map(|p| image::open(p).expect("open").to_rgb8())
        .collect();
    assert!(imgs.len() >= 3);
    let refs: Vec<&RgbImage> = imgs.iter().collect();
    let t = Instant::now();
    let mag = median_gradient(&refs).expect("median");
    let m = batch_lines(&mag, BATCH_LOW, BATCH_HIGH).expect("mark found");
    let secs = t.elapsed().as_secs_f32();

    let gt: Mask = image::open(set.join("mask").join("00.png"))
        .expect("gt")
        .to_luma8();
    let gt = mask::dilate(&gt, 1);
    let total = gt.pixels().filter(|p| p.0[0] != 0).count();
    let hit = gt
        .pixels()
        .zip(m.pixels())
        .filter(|(a, b)| a.0[0] != 0 && b.0[0] != 0)
        .count();
    let recall = hit as f32 / total.max(1) as f32;
    let cov = mask::coverage(&m);
    eprintln!(
        "fixed: {} images {:?}, recall {recall:.3}, coverage {cov:.4}, time {secs:.2}s",
        imgs.len(),
        imgs[0].dimensions()
    );
    assert!(recall >= 0.8, "recall {recall}");
    assert!(cov < 0.05, "coverage {cov}");
}

#[test]
fn negative_set_none() {
    let set = common::synth_dir().join("negative");
    let imgs: Vec<RgbImage> = pngs(&set.join("marked"))
        .iter()
        .map(|p| {
            let i = image::open(p).expect("open").to_rgb8();
            imageops::resize(&i, 640, 480, imageops::FilterType::Triangle)
        })
        .collect();
    assert_eq!(imgs.len(), 6);
    let refs: Vec<&RgbImage> = imgs.iter().collect();
    let mag = median_gradient(&refs).expect("median");
    match batch_lines(&mag, BATCH_LOW, BATCH_HIGH) {
        None => eprintln!("negative: None"),
        Some(m) => {
            let c = mask::coverage(&m);
            eprintln!("negative: coverage {c:.4}");
            assert!(c < 0.01, "coverage {c}");
        }
    }
}

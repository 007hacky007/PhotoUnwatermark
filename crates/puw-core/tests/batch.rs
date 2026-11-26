mod common;

use image::{Luma, RgbImage, imageops};
use puw_core::batch::{AGREE, BATCH_HIGH, BATCH_LOW, batch_lines, filter_layer, median_gradient};
use puw_core::mask::{self, Mask};
use puw_core::{Error, Progress, build_set_layers};
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

/// Reference: per-image Sobel (reflect boundary) on 601 luma, then median (for 3 or 4 images
/// the value 3 of them agree on), then hypot.
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
        if (AGREE..2 * AGREE - 1).contains(&n) {
            // 3 or 4 photos: the value AGREE of them reach on one side of zero.
            let (hi, lo) = (v[n - AGREE], v[AGREE - 1]);
            if hi > 0.0 {
                hi
            } else if lo < 0.0 {
                lo
            } else {
                0.0
            }
        } else if n % 2 == 1 {
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
    // 3 and 4 images take the value 3 of them agree on, 5 and 6 the median.
    for n in 3..=6 {
        let imgs: Vec<RgbImage> = (1..=n).map(|s| random_image(64, 48, s)).collect();
        let refs: Vec<&RgbImage> = imgs.iter().collect();
        let got = median_gradient(&refs).expect("median");
        let want = naive(&refs);
        let max = got
            .as_raw()
            .iter()
            .zip(&want)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        assert!(max < 1e-3, "{n} images: max abs diff {max}");
    }
}

/// With 3 photos an edge that only 2 of them show is not in the layer's gradient; one all 3
/// show is, at the weakest of the three.
#[test]
fn three_photos_keep_only_edges_all_show() {
    let (w, h) = (40u32, 30u32);
    // A vertical step at x = 20 of height `a` (0 = flat) on a grey photo.
    let step = |a: u8| {
        RgbImage::from_fn(w, h, |x, _| {
            image::Rgb([if x >= 20 { 100 + a } else { 100 }; 3])
        })
    };
    let at = |m: &image::ImageBuffer<Luma<f32>, Vec<f32>>| m.get_pixel(20, 15).0[0];
    let (a, b, flat) = (step(40), step(30), step(0));
    let two = median_gradient(&[&a, &b, &flat]).expect("median");
    assert_eq!(at(&two), 0.0);
    let all = median_gradient(&[&a, &b, &step(20)]).expect("median");
    let weakest = median_gradient(&[&step(20)]).expect("one photo");
    assert!(
        (at(&all) - at(&weakest)).abs() < 1e-3,
        "{} vs {}",
        at(&all),
        at(&weakest)
    );
}

/// Three photos of one shoot (one scene, two shots lined up as in a burst, the third moved a
/// little) with the fixed set's mark blended at 0.14 at one place: the layer is the mark.
#[test]
fn three_photos_of_one_shoot_give_the_mark() {
    let set = common::synth_dir().join("fixed");
    let open = |p: &str| image::open(set.join(p)).expect("open").to_rgb8();
    let (clean0, marked0, scene) = (
        open("clean/00.png"),
        open("marked/00.png"),
        open("clean/03.png"),
    );
    let gt: Mask = image::open(set.join("mask/00.png")).expect("gt").to_luma8();
    let (w, h) = scene.dimensions();
    let imgs: Vec<RgbImage> = [(0i64, 0i64), (1, 1), (24, -16)]
        .iter()
        .map(|&(sx, sy)| {
            RgbImage::from_fn(w, h, |x, y| {
                let px = (x as i64 + sx).clamp(0, w as i64 - 1) as u32;
                let py = (y as i64 + sy).clamp(0, h as i64 - 1) as u32;
                let mut p = *scene.get_pixel(px, py);
                if gt.get_pixel(x, y).0[0] != 0 {
                    let (c, m) = (clean0.get_pixel(x, y), marked0.get_pixel(x, y));
                    for ch in 0..3 {
                        let mark = c.0[ch] as f32 + (m.0[ch] as f32 - c.0[ch] as f32) / 0.25;
                        let b = p.0[ch] as f32;
                        p.0[ch] = (b + 0.14 * (mark - b)).round().clamp(0.0, 255.0) as u8;
                    }
                }
                p
            })
        })
        .collect();
    let refs: Vec<&RgbImage> = imgs.iter().collect();
    let mag = median_gradient(&refs).expect("median");
    let layer = filter_layer(&batch_lines(&mag, BATCH_LOW, BATCH_HIGH).expect("a layer"));
    let near = mask::dilate(&gt, mask::grow_for(w, h, 0.004));
    let (recall, inside) = (covered(&layer, &gt), covered(&near, &layer));
    eprintln!("one shoot of 3: recall {recall:.3}, layer near the mark {inside:.3}");
    assert!(recall >= 0.8, "recall {recall}");
    // The share the position prior asks of a layer that is the mark (`PRIOR_LAYER_INSIDE`). The
    // median of the three kept the scene: more than 10 % of the photo, so no layer at all.
    assert!(inside >= 0.9, "layer near the mark {inside}");
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

/// Share (0..1) of the set pixels of `truth` that are also set in `found`.
fn covered(found: &Mask, truth: &Mask) -> f32 {
    let total = truth.pixels().filter(|p| p.0[0] != 0).count();
    let hit = truth
        .pixels()
        .zip(found.pixels())
        .filter(|(a, b)| a.0[0] != 0 && b.0[0] != 0)
        .count();
    hit as f32 / total.max(1) as f32
}

#[test]
fn filter_keeps_watermark() {
    let set = common::synth_dir().join("fixed");
    let paths = pngs(&set.join("marked"));
    let layers = build_set_layers(&paths, &Progress::none()).expect("layers");
    let gt: Mask = image::open(set.join("mask").join("00.png"))
        .expect("gt")
        .to_luma8();
    let (w, h) = gt.dimensions();
    let layer = layers.get(w, h).expect("fixed mark in the stored layer");
    let share = covered(layer, &gt);
    eprintln!(
        "fixed: stored layer covers {share:.3} of the mark, coverage {:.4}",
        mask::coverage(layer)
    );
    assert!(share >= 0.9, "mark covered {share}");
}

#[test]
fn filter_drops_scene_line() {
    let set = common::synth_dir().join("fixed");
    let gt: Mask = image::open(set.join("mask").join("00.png"))
        .expect("gt")
        .to_luma8();
    let (w, h) = gt.dimensions();
    let bbox = mask::bbox(&gt).expect("mark in the ground truth");
    // The photo is 1024 px tall and the mark sits near the bottom, so the full-width line runs
    // 300 px above the mark; the full-height line runs 300 px left of it.
    let row = (bbox.top() - 300) as u32;
    let col = (bbox.left() - 300) as u32;
    let mut lines = Mask::new(w, h);
    for x in 0..w {
        lines.put_pixel(x, row, Luma([255]));
    }
    for y in 0..h {
        lines.put_pixel(col, y, Luma([255]));
    }
    let layer = mask::union(&gt, &lines);
    let out = filter_layer(&layer);
    let (mark, line) = (covered(&out, &gt), covered(&out, &lines));
    eprintln!("scene lines: mark kept {mark:.3}, lines kept {line:.3}");
    assert!(mark >= 0.9, "mark kept {mark}");
    assert!(line < 0.05, "lines kept {line}");
}

#[test]
fn filter_keeps_empty() {
    let out = filter_layer(&Mask::new(64, 48));
    assert_eq!(out.dimensions(), (64, 48));
    assert!(mask::is_empty(&out));
}

/// Sets the `w` x `h` block at (`x`, `y`).
fn block(m: &mut Mask, x: u32, y: u32, w: u32, h: u32) {
    for yy in y..y + h {
        for xx in x..x + w {
            m.put_pixel(xx, yy, Luma([255]));
        }
    }
}

#[test]
fn filter_keeps_tall_mark_and_drops_specks() {
    // 1000 x 1000, so components closer than 20 px group. The mark is a column of 100 x 40
    // blocks 5 px apart from y 25 to 965: each block is far under 60 % of the image, the
    // column spans 94 % of the height (the shape of a mark running the photo's full height).
    let mut mark = Mask::new(1000, 1000);
    let mut y = 25;
    while y + 40 <= 975 {
        block(&mut mark, 450, y, 100, 40);
        y += 45;
    }
    // 7x7 specks (49 px, under 0.1 % of the layer): some 10 px from the column, where they
    // would join its group, and some scattered far from it.
    let mut specks = Mask::new(1000, 1000);
    for k in 0..10 {
        block(&mut specks, 560, 30 + 90 * k, 7, 7);
        block(&mut specks, 60 + 25 * (k % 4), 100 + 80 * k, 7, 7);
        block(&mut specks, 800 + 30 * (k % 3), 70 + 85 * k, 7, 7);
    }
    let layer = mask::union(&mark, &specks);
    let out = filter_layer(&layer);
    let (kept, speck) = (covered(&out, &mark), covered(&out, &specks));
    eprintln!("tall mark: kept {kept:.3}, specks kept {speck:.3}");
    assert_eq!(kept, 1.0, "mark kept {kept}");
    assert_eq!(speck, 0.0, "specks kept {speck}");
}

#[test]
fn filter_keeps_groups_of_half_the_largest() {
    // 1000 x 1000: groups are blocks more than 20 px apart. The largest holds 10000 px.
    let mut largest = Mask::new(1000, 1000);
    block(&mut largest, 100, 100, 100, 100);
    let mut half = Mask::new(1000, 1000);
    block(&mut half, 500, 100, 50, 100); // 5000 px: exactly half, kept
    let mut under = Mask::new(1000, 1000);
    block(&mut under, 100, 500, 49, 100); // 4900 px: under half, dropped
    let layer = mask::union(&mask::union(&largest, &half), &under);
    let out = filter_layer(&layer);
    assert_eq!(covered(&out, &largest), 1.0);
    assert_eq!(covered(&out, &half), 1.0);
    assert_eq!(covered(&out, &under), 0.0);
}

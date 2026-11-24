mod common;

use image::{GrayImage, Luma, RgbImage};
use puw_core::mask::Mask;
use puw_core::ncc::masked_ncc;
use puw_core::template::{MatchResult, MatchStatus, WatermarkTemplate, build_template, find};
use std::path::Path;
use std::time::Instant;

fn names(set: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(set.join("marked"))
        .expect("read dir")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "png"))
        .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

fn rgb(set: &Path, sub: &str, name: &str) -> RgbImage {
    image::open(set.join(sub).join(format!("{name}.png")))
        .expect("open image")
        .to_rgb8()
}

fn gt(set: &Path, name: &str) -> Mask {
    image::open(set.join("mask").join(format!("{name}.png")))
        .expect("open mask")
        .to_luma8()
}

fn iou(a: &Mask, b: &Mask) -> f32 {
    assert_eq!(a.dimensions(), b.dimensions());
    let (mut inter, mut uni) = (0u64, 0u64);
    for (p, q) in a.pixels().zip(b.pixels()) {
        let (p, q) = (p.0[0] != 0, q.0[0] != 0);
        inter += (p && q) as u64;
        uni += (p || q) as u64;
    }
    if uni == 0 {
        0.0
    } else {
        inter as f32 / uni as f32
    }
}

fn timed_find(t: &WatermarkTemplate, img: &RgbImage) -> (MatchResult, f32) {
    let start = Instant::now();
    let r = find(t, img);
    (r, start.elapsed().as_secs_f32())
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
}

#[test]
fn masked_ncc_finds_shift() {
    let (iw, ih, tw, th, ox, oy) = (120, 90, 30, 20, 37, 51);
    let mut rng = Lcg(42);
    let image: Vec<f32> = (0..iw * ih).map(|_| rng.next() * 255.0).collect();
    let mut tmpl = vec![0f32; tw * th];
    for y in 0..th {
        for x in 0..tw {
            tmpl[y * tw + x] = image[(oy + y) * iw + ox + x];
        }
    }
    let ones = vec![1f32; tw * th];
    let map = masked_ncc(&image, iw, ih, &tmpl, &ones, tw, th);
    let ow = iw - tw + 1;
    assert_eq!(map.len(), ow * (ih - th + 1));
    let best = (0..map.len())
        .max_by(|&a, &b| map[a].total_cmp(&map[b]))
        .unwrap();
    assert_eq!((best % ow, best / ow), (ox, oy));
    assert!(map[best] > 0.99, "score {}", map[best]);
}

#[test]
fn fixed_set_all_found() {
    let set = common::synth_dir().join("fixed");
    let names = names(&set);
    assert_eq!(names.len(), 8);
    let t = build_template(&rgb(&set, "marked", &names[0]), &gt(&set, &names[0]));
    assert_eq!(t.pieces.len(), 1);
    for name in &names[1..] {
        let (r, secs) = timed_find(&t, &rgb(&set, "marked", name));
        let v = iou(&r.mask, &gt(&set, name));
        eprintln!(
            "fixed {name}: {:?} conf {:.3} iou {v:.3} {secs:.2}s",
            r.status, r.confidence
        );
        assert_eq!(
            r.status,
            MatchStatus::Found,
            "{name}: conf {}",
            r.confidence
        );
        assert!(v >= 0.8, "{name}: iou {v}");
    }
}

/// Marks that change the luma under their GT mask by less than this on average are treated as
/// invisible: `random/02` (white at alpha ~0.2 over a bright, textured 209 +- 28 background)
/// changes it by 6.9, every other synthetic mark by 16 or more.
const MIN_VISIBLE: f32 = 10.0;

/// Mean luma change (ITU-R 601) the mark makes under its GT mask, marked minus clean.
fn visibility(set: &Path, name: &str) -> f32 {
    let (m, c, g) = (
        rgb(set, "marked", name),
        rgb(set, "clean", name),
        gt(set, name),
    );
    let luma =
        |p: &image::Rgb<u8>| 0.299 * p.0[0] as f32 + 0.587 * p.0[1] as f32 + 0.114 * p.0[2] as f32;
    let (mut sum, mut n) = (0f32, 0f32);
    for ((a, b), q) in m.pixels().zip(c.pixels()).zip(g.pixels()) {
        if q.0[0] != 0 {
            sum += luma(a) - luma(b);
            n += 1.0;
        }
    }
    sum / n.max(1.0)
}

/// Template from `random/00` (landscape 1536x1024) with its GT mask; the other 11 photos are
/// portrait and landscape at long side 1536 or 1024 with the mark elsewhere. A wrong `Found`
/// (IoU < 0.6) fails on any photo; >= 95 % of the visible marks must be `Found`, the rest
/// `Unsure`. An invisible mark may also end `NotFound`.
#[test]
fn random_set() {
    let set = common::synth_dir().join("random");
    let names = names(&set);
    assert_eq!(names.len(), 12);
    let t = build_template(&rgb(&set, "marked", &names[0]), &gt(&set, &names[0]));
    let (mut visible, mut found) = (0, 0);
    for name in &names[1..] {
        let img = rgb(&set, "marked", name);
        let (r, secs) = timed_find(&t, &img);
        let v = iou(&r.mask, &gt(&set, name));
        let vis = visibility(&set, name);
        eprintln!(
            "random {name} {:?} visibility {vis:.1}: {:?} conf {:.3} iou {v:.3} {secs:.2}s",
            img.dimensions(),
            r.status,
            r.confidence
        );
        if r.status == MatchStatus::Found {
            assert!(v >= 0.6, "{name}: wrong Found, iou {v}");
        }
        if vis < MIN_VISIBLE {
            continue;
        }
        visible += 1;
        match r.status {
            MatchStatus::Found => found += 1,
            MatchStatus::Unsure => {}
            MatchStatus::NotFound => panic!("{name}: NotFound, conf {}", r.confidence),
        }
    }
    assert!(visible >= 10, "only {visible} visible marks");
    assert!(found * 100 >= visible * 95, "Found {found} of {visible}");
}

#[test]
fn negatives_never_found() {
    let fixed = common::synth_dir().join("fixed");
    let random = common::synth_dir().join("random");
    let templates = [
        build_template(&rgb(&fixed, "marked", "00"), &gt(&fixed, "00")),
        build_template(&rgb(&random, "marked", "00"), &gt(&random, "00")),
    ];
    let set = common::synth_dir().join("negative");
    let names = names(&set);
    assert_eq!(names.len(), 6);
    for (ti, t) in templates.iter().enumerate() {
        for name in &names {
            let r = find(t, &rgb(&set, "marked", name));
            eprintln!(
                "negative {name} (template {ti}): {:?} conf {:.3}",
                r.status, r.confidence
            );
            assert_ne!(
                r.status,
                MatchStatus::Found,
                "{name}: conf {}",
                r.confidence
            );
            if r.status == MatchStatus::NotFound {
                assert!(r.mask.pixels().all(|p| p.0[0] == 0));
            }
        }
    }
}

#[test]
fn roundtrip_serde() {
    let set = common::synth_dir().join("fixed");
    let t = build_template(&rgb(&set, "marked", "00"), &gt(&set, "00"));
    let json = serde_json::to_string(&t).expect("serialize");
    let back: WatermarkTemplate = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, t);
    let img = rgb(&set, "marked", "03");
    let (a, b) = (find(&t, &img), find(&back, &img));
    assert_eq!(a.status, b.status);
    assert_eq!(a.confidence, b.confidence);
    assert_eq!(a.mask, b.mask);
}

#[test]
fn empty_mask_gives_not_found() {
    let set = common::synth_dir().join("fixed");
    let img = rgb(&set, "marked", "00");
    let t = build_template(&img, &GrayImage::new(img.width(), img.height()));
    assert!(t.pieces.is_empty());
    let r = find(&t, &img);
    assert_eq!(r.status, MatchStatus::NotFound);
    assert_eq!(r.mask.dimensions(), img.dimensions());
    assert!(r.mask.pixels().all(|p| p.0[0] == 0));
}

/// Two-piece mark: a ring logo and, 60 px to its right, the outline text of the fixed set.
fn two_piece_layer(text: &Mask) -> Mask {
    let (r_out, r_in) = (40.0f32, 27.0f32);
    let gap = 60;
    let side = (2.0 * r_out) as u32 + 1;
    let w = side + gap + text.width();
    let h = side.max(text.height());
    let mut layer = GrayImage::new(w, h);
    let c = r_out;
    for y in 0..side {
        for x in 0..side {
            let d = ((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt();
            // Ring plus a horizontal bar through the middle.
            let bar = (y as f32 - c).abs() <= 5.0 && d <= r_out;
            if (d <= r_out && d >= r_in) || bar {
                layer.put_pixel(x, y, Luma([255]));
            }
        }
    }
    let ty = (h - text.height()) / 2;
    for (x, y, p) in text.enumerate_pixels() {
        if p.0[0] != 0 {
            layer.put_pixel(side + gap + x, ty + y, Luma([255]));
        }
    }
    layer
}

/// Blend white through `layer` at `alpha` with `layer`'s top-left at `(x, y)`; returns the GT.
fn stamp(img: &mut RgbImage, layer: &Mask, x: u32, y: u32, alpha: f32) -> Mask {
    let mut gt = GrayImage::new(img.width(), img.height());
    for (lx, ly, p) in layer.enumerate_pixels() {
        if p.0[0] == 0 {
            continue;
        }
        let (ix, iy) = (x + lx, y + ly);
        let px = img.get_pixel_mut(ix, iy);
        for c in &mut px.0 {
            *c = (*c as f32 * (1.0 - alpha) + 255.0 * alpha).round() as u8;
        }
        gt.put_pixel(ix, iy, Luma([255]));
    }
    gt
}

fn crop_text(fixed: &Path) -> Mask {
    let m = gt(fixed, "00");
    let b = puw_core::mask::bbox(&m).expect("mark");
    image::imageops::crop_imm(&m, b.left() as u32, b.top() as u32, b.width(), b.height()).to_image()
}

/// Template from `fixed/clean/00` with the two-piece mark stamped at (200, 150).
fn two_piece_template(fixed: &Path) -> (WatermarkTemplate, Mask) {
    let layer = two_piece_layer(&crop_text(fixed));
    let mut ref_img = rgb(fixed, "clean", "00");
    let ref_gt = stamp(&mut ref_img, &layer, 200, 150, 0.3);
    (build_template(&ref_img, &ref_gt), layer)
}

const TOGETHER: [(u32, u32); 4] = [(900, 700), (60, 820), (520, 40), (1000, 380)];

/// Logo and text positions, moved independently of each other.
const APART: [((u32, u32), (u32, u32)); 4] = [
    ((100, 100), (900, 800)),
    ((1300, 200), (300, 600)),
    ((700, 850), (200, 300)),
    ((50, 900), (1000, 100)),
];

/// `fixed/clean/<i+1>` with the two-piece layer stamped as a whole at `TOGETHER[i]`.
fn together_case(fixed: &Path, layer: &Mask, i: usize) -> (RgbImage, Mask) {
    let mut img = rgb(fixed, "clean", &format!("{:02}", i + 1));
    let (x, y) = TOGETHER[i];
    let truth = stamp(&mut img, layer, x, y, 0.3);
    (img, truth)
}

/// `fixed/clean/<i+1>` with the logo and the text stamped separately at `APART[i]`.
fn apart_case(fixed: &Path, layer: &Mask, i: usize) -> (RgbImage, Mask) {
    let mut img = rgb(fixed, "clean", &format!("{:02}", i + 1));
    let logo = image::imageops::crop_imm(layer, 0, 0, 81, 81).to_image();
    let text = crop_text(fixed);
    let ((lx, ly), (tx, ty)) = APART[i];
    let g1 = stamp(&mut img, &logo, lx, ly, 0.3);
    let g2 = stamp(&mut img, &text, tx, ty, 0.3);
    (img, puw_core::mask::union(&g1, &g2))
}

#[test]
fn multi_piece_moved_together() {
    let fixed = common::synth_dir().join("fixed");
    let (t, layer) = two_piece_template(&fixed);
    assert_eq!(t.pieces.len(), 2);
    for i in 0..TOGETHER.len() {
        let (img, truth) = together_case(&fixed, &layer, i);
        let (r, secs) = timed_find(&t, &img);
        let v = iou(&r.mask, &truth);
        eprintln!(
            "two-piece together {i}: {:?} conf {:.3} iou {v:.3} {secs:.2}s",
            r.status, r.confidence
        );
        assert_eq!(r.status, MatchStatus::Found, "{i}: conf {}", r.confidence);
        assert!(v >= 0.6, "{i}: iou {v}");
    }
}

/// The pieces moved independently: the group match is carried by the text alone, so the
/// per-piece check rejects it and each piece is searched on its own.
#[test]
fn multi_piece_moved_apart() {
    let fixed = common::synth_dir().join("fixed");
    let (t, layer) = two_piece_template(&fixed);
    for i in 0..APART.len() {
        let (img, truth) = apart_case(&fixed, &layer, i);
        let (r, secs) = timed_find(&t, &img);
        let v = iou(&r.mask, &truth);
        eprintln!(
            "two-piece apart {i}: {:?} conf {:.3} iou {v:.3} {secs:.2}s",
            r.status, r.confidence
        );
        assert_eq!(r.status, MatchStatus::Found, "{i}: conf {}", r.confidence);
        assert!(v >= 0.6, "{i}: iou {v}");
    }
}

/// Scale and position of the mark for `scaled_case`: the midpoints of the coarse scale grid
/// (the worst case for the coarse search).
const SCALED: [(f32, u32, u32); 9] = [
    (0.725, 900, 700),
    (0.775, 100, 850),
    (0.825, 500, 60),
    (0.925, 950, 400),
    (1.025, 300, 500),
    (1.1375, 800, 100),
    (1.2125, 100, 300),
    (1.2875, 600, 800),
    (1.3625, 900, 500),
];

/// Template from `fixed/clean/00` with the fixed set's text stamped at (300, 200).
fn text_template(fixed: &Path) -> WatermarkTemplate {
    let mut ref_img = rgb(fixed, "clean", "00");
    let ref_gt = stamp(&mut ref_img, &crop_text(fixed), 300, 200, 0.3);
    build_template(&ref_img, &ref_gt)
}

/// A 1536x1024 clean photo with the text stamped at `SCALED[i]`: the mark did not scale with
/// the photo.
fn scaled_case(fixed: &Path, i: usize) -> (RgbImage, Mask) {
    let text = crop_text(fixed);
    let (scale, x, y) = SCALED[i];
    let (w, h) = (text.width() as f32 * scale, text.height() as f32 * scale);
    let scaled = puw_core::mask::resize(&text, w.round() as u32, h.round() as u32);
    let mut img = rgb(fixed, "clean", &format!("{:02}", i % 7 + 1));
    let truth = stamp(&mut img, &scaled, x, y, 0.3);
    (img, truth)
}

#[test]
fn scaled_mark_found() {
    let fixed = common::synth_dir().join("fixed");
    let t = text_template(&fixed);
    let mut bad = Vec::new();
    for (i, &(scale, _, _)) in SCALED.iter().enumerate() {
        let (img, truth) = scaled_case(&fixed, i);
        let (r, secs) = timed_find(&t, &img);
        let v = iou(&r.mask, &truth);
        eprintln!(
            "scaled x{scale}: {:?} conf {:.3} iou {v:.3} {secs:.2}s",
            r.status, r.confidence
        );
        if r.status != MatchStatus::Found || v < 0.6 {
            bad.push(scale);
        }
    }
    assert!(
        bad.is_empty(),
        "not Found with iou >= 0.6 at scales {bad:?}"
    );
}

/// Prints score distributions and timings used to choose `FOUND`/`UNSURE`:
/// `cargo test -p puw-core --release --test template score_sweep -- --ignored --nocapture`.
#[test]
#[ignore = "prints score distributions, run with --release --nocapture"]
fn score_sweep() {
    let synth = common::synth_dir();
    let (fixed, random, negative) = (
        synth.join("fixed"),
        synth.join("random"),
        synth.join("negative"),
    );
    let (two, layer) = two_piece_template(&fixed);
    let t_fixed = build_template(&rgb(&fixed, "marked", "00"), &gt(&fixed, "00"));
    let t_random = build_template(&rgb(&random, "marked", "00"), &gt(&random, "00"));
    let mut times = Vec::new();
    let mut run = |t: &WatermarkTemplate, img: &RgbImage| {
        let (r, secs) = timed_find(t, img);
        times.push(secs);
        r
    };

    let mut pos: Vec<(String, f32)> = Vec::new();
    for n in names(&fixed).iter().skip(1) {
        pos.push((
            format!("fixed/{n}"),
            run(&t_fixed, &rgb(&fixed, "marked", n)).confidence,
        ));
    }
    for n in names(&random).iter().skip(1) {
        let tag = if visibility(&random, n) < MIN_VISIBLE {
            " (invisible)"
        } else {
            ""
        };
        let c = run(&t_random, &rgb(&random, "marked", n)).confidence;
        pos.push((format!("random/{n}{tag}"), c));
    }
    let text = text_template(&fixed);
    for (i, (scale, _, _)) in SCALED.iter().enumerate() {
        let c = run(&text, &scaled_case(&fixed, i).0).confidence;
        pos.push((format!("scaled x{scale}"), c));
    }
    for i in 0..TOGETHER.len() {
        let c = run(&two, &together_case(&fixed, &layer, i).0).confidence;
        pos.push((format!("two-piece together {i}"), c));
        let c = run(&two, &apart_case(&fixed, &layer, i).0).confidence;
        pos.push((format!("two-piece apart {i}"), c));
    }

    // Negatives: photos without the mark, for each template (its own source photo excluded).
    let mut neg: Vec<(String, f32)> = Vec::new();
    for (tn, t) in [
        ("fixed", &t_fixed),
        ("random", &t_random),
        ("two-piece", &two),
        ("text", &text),
    ] {
        let mut photos: Vec<(String, RgbImage)> = Vec::new();
        for n in names(&negative) {
            photos.push((format!("negative/{n}"), rgb(&negative, "marked", &n)));
        }
        for (set, dir) in [("fixed", &fixed), ("random", &random)] {
            for n in names(dir).iter().skip(1) {
                photos.push((format!("{set}/clean/{n}"), rgb(dir, "clean", n)));
            }
        }
        for (name, img) in photos {
            neg.push((format!("{tn} template on {name}"), run(t, &img).confidence));
        }
    }

    for (label, v) in [("positives", &mut pos), ("negatives", &mut neg)] {
        v.sort_by(|a, b| a.1.total_cmp(&b.1));
        eprintln!("{label} ({}):", v.len());
        for (name, c) in v.iter() {
            eprintln!("  {c:.3} {name}");
        }
    }
    times.sort_by(f32::total_cmp);
    eprintln!(
        "find on 1024-1536 px photos: {} runs, median {:.2}s, max {:.2}s",
        times.len(),
        times[times.len() / 2],
        times[times.len() - 1]
    );

    // Large photos (mark scaled with the photo): fast path and full search.
    let big = |set: &Path, name: &str, w: u32, h: u32| {
        let img = image::imageops::resize(
            &rgb(set, "marked", name),
            w,
            h,
            image::imageops::FilterType::Triangle,
        );
        let gt =
            image::imageops::resize(&gt(set, name), w, h, image::imageops::FilterType::Nearest);
        (img, gt)
    };
    for (t, set, name, w, h) in [
        (&t_fixed, &fixed, "01", 6000, 4000),
        (&t_random, &random, "01", 4000, 6000),
    ] {
        let (img, truth) = big(set, name, w, h);
        let (r, secs) = timed_find(t, &img);
        let v = iou(&r.mask, &truth);
        eprintln!(
            "{w}x{h}: {:?} conf {:.3} iou {v:.3} in {secs:.2}s",
            r.status, r.confidence
        );
    }
}

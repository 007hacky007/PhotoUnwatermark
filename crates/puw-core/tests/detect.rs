mod common;

use std::path::Path;
use std::sync::OnceLock;
use std::time::Instant;

use puw_core::detect::{GuessOptions, guess_single, predict_prob};
use puw_core::engine::{Engine, ModelPaths};
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
            Some(Engine::new(&paths, false, None).expect("engine loads"))
        })
        .as_ref()
}

fn pngs(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .expect("read dir")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "png"))
        .collect();
    v.sort();
    v
}

#[test]
fn predict_prob_small_image() {
    let Some(engine) = engine() else { return };
    let img = image::RgbImage::from_pixel(300, 200, image::Rgb([120, 130, 140]));
    let prob = predict_prob(engine, &img, 768).expect("predict");
    assert_eq!(prob.dimensions(), (300, 200));
    let opts = GuessOptions::default();
    let g = guess_single(engine, &img, &opts).expect("guess");
    assert_eq!(g.mask.dimensions(), (300, 200));
}

#[test]
fn styles_recall() {
    let Some(engine) = engine() else { return };
    let set = common::synth_dir().join("styles");
    let opts = GuessOptions::default();
    let mut recalls = Vec::new();
    for marked in pngs(&set.join("marked")) {
        let name = marked.file_name().unwrap();
        let img = image::open(&marked).expect("open").to_rgb8();
        let gt: Mask = image::open(set.join("mask").join(name))
            .expect("gt")
            .to_luma8();
        let t = Instant::now();
        let g = guess_single(engine, &img, &opts).expect("guess");
        let secs = t.elapsed().as_secs_f32();
        let guess = mask::dilate(&g.mask, 3);
        let gt = mask::dilate(&gt, 1);
        let total = gt.pixels().filter(|p| p.0[0] != 0).count();
        let hit = gt
            .pixels()
            .zip(guess.pixels())
            .filter(|(a, b)| a.0[0] != 0 && b.0[0] != 0)
            .count();
        let recall = hit as f32 / total.max(1) as f32;
        eprintln!(
            "{:?}: recall {recall:.3} coverage {:.4} conf {:.3} time {secs:.1}s",
            name,
            mask::coverage(&g.mask),
            g.confidence
        );
        recalls.push(recall);
    }
    assert!(!recalls.is_empty());
    let mean = recalls.iter().sum::<f32>() / recalls.len() as f32;
    eprintln!("mean recall {mean:.3}");
    assert!(mean >= 0.9, "mean recall {mean}");
}

#[test]
fn negative_set_quiet() {
    let Some(engine) = engine() else { return };
    let set = common::synth_dir().join("negative");
    let opts = GuessOptions::default();
    let mut covs = Vec::new();
    for marked in pngs(&set.join("marked")) {
        let img = image::open(&marked).expect("open").to_rgb8();
        let g = guess_single(engine, &img, &opts).expect("guess");
        let c = mask::coverage(&g.mask);
        eprintln!("{:?}: coverage {c:.4}", marked.file_name().unwrap());
        covs.push(c);
    }
    assert!(!covs.is_empty());
    let mean = covs.iter().sum::<f32>() / covs.len() as f32;
    eprintln!("mean coverage {mean:.4}");
    assert!(mean < 0.02, "mean coverage {mean}");
}

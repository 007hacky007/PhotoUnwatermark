//! Helpers shared by the integration tests.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

use image::{GrayImage, Luma};
use puw_app::{AppCore, DataDirs, PhotoState, ProgressEvent, Settings};

/// Folder with the model files, from `PUW_MODELS_DIR`. Tests that need models return early
/// with `eprintln!("skipped: PUW_MODELS_DIR not set")` when this is `None`.
pub fn models_dir() -> Option<PathBuf> {
    std::env::var_os("PUW_MODELS_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Compiled-model cache shared by the model tests of this crate, in `<workspace>/target`.
pub fn model_cache_dir() -> PathBuf {
    workspace_root()
        .join("target")
        .join("test-model-cache")
        .join("puw-app")
}

fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root above crates/puw-app")
}

/// Synthetic test sets in `<workspace>/target/synth`, generated with tools/synth/make_sets.py
/// when the folder is missing.
///
/// Tests in one binary share a single generation through a `OnceLock`. Separate test binaries
/// may race, so each generates into its own `target/synth.tmp-<pid>` and renames it into place;
/// the loser of a race deletes its copy and uses the folder that won.
pub fn synth_dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let root = workspace_root();
        let dir = root.join("target").join("synth");
        if !dir.exists() {
            generate_into(root, &dir);
        }
        dir
    })
    .clone()
}

fn generate_into(root: &Path, dir: &Path) {
    let tmp = dir.with_file_name(format!("synth.tmp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let status = Command::new("uv")
        .args([
            "run",
            "--project",
            "tools",
            "python",
            "tools/synth/make_sets.py",
            "--out",
        ])
        .arg(&tmp)
        .current_dir(root)
        .status()
        .expect("run uv to generate the synthetic sets");
    if !status.success() {
        // Do not leave a half-written folder behind.
        let _ = std::fs::remove_dir_all(&tmp);
        panic!("make_sets.py failed: {status}");
    }
    if let Err(err) = std::fs::rename(&tmp, dir) {
        // Another test binary finished first; its folder is complete because it was renamed
        // into place only after generation succeeded.
        let _ = std::fs::remove_dir_all(&tmp);
        assert!(dir.is_dir(), "rename {} failed: {err}", tmp.display());
    }
}

// ---- Cores, events and areas shared by the app tests ----

/// Model tests run one at a time, so their timings and memory do not add up and only one of
/// them writes to the shared compiled-model cache.
pub static MODEL_TESTS: Mutex<()> = Mutex::new(());

pub struct Fixture {
    pub core: AppCore,
    pub dirs: DataDirs,
    pub _tmp: tempfile::TempDir,
    pub _serial: Option<MutexGuard<'static, ()>>,
}

/// A core with its own data folder. `models` replaces the data folder's (empty) models folder.
pub fn fixture(models: Option<PathBuf>) -> Fixture {
    let serial = models
        .as_ref()
        .map(|_| MODEL_TESTS.lock().unwrap_or_else(PoisonError::into_inner));
    let tmp = tempfile::tempdir().unwrap();
    let mut dirs = DataDirs::at(tmp.path());
    if let Some(m) = models {
        dirs.models = m;
        dirs.cache = model_cache_dir();
    }
    Settings {
        prefer_gpu: cfg!(target_os = "macos"),
        ..Settings::default()
    }
    .save(&dirs.settings)
    .unwrap();
    Fixture {
        core: AppCore::new(dirs.clone()).unwrap(),
        dirs,
        _tmp: tmp,
        _serial: serial,
    }
}

/// A core that can load models, or `None` (with a message) when `PUW_MODELS_DIR` is not set.
pub fn model_fixture() -> Option<Fixture> {
    match models_dir() {
        Some(m) => Some(fixture(Some(m))),
        None => {
            eprintln!("skipped: PUW_MODELS_DIR not set");
            None
        }
    }
}

pub fn noop(_: ProgressEvent) {}

#[derive(Default)]
pub struct Recorder(Mutex<Vec<ProgressEvent>>);

impl Recorder {
    pub fn push(&self, e: ProgressEvent) {
        self.0.lock().unwrap().push(e);
    }

    pub fn events(&self) -> Vec<ProgressEvent> {
        self.0.lock().unwrap().clone()
    }
}

/// The `(done, total)` of every `Matching` event, in order.
pub fn matching(events: &[ProgressEvent]) -> Vec<(u32, u32)> {
    events
        .iter()
        .filter_map(|e| match e {
            ProgressEvent::Matching { done, total } => Some((*done, *total)),
            _ => None,
        })
        .collect()
}

/// An edit-size area (row-major, 0/255) with the rectangle `x0..x1` x `y0..y1` painted.
pub fn rect_area(w: u32, h: u32, x0: u32, y0: u32, x1: u32, y1: u32) -> Vec<u8> {
    let mut v = vec![0u8; (w * h) as usize];
    for y in y0..y1 {
        v[(y * w + x0) as usize..(y * w + x1) as usize].fill(255);
    }
    v
}

pub fn iou(truth: &GrayImage, area: &[u8]) -> f64 {
    assert_eq!(truth.as_raw().len(), area.len());
    let (mut inter, mut uni) = (0u64, 0u64);
    for (t, a) in truth.as_raw().iter().zip(area) {
        let (t, a) = (*t != 0, *a != 0);
        inter += (t && a) as u64;
        uni += (t || a) as u64;
    }
    if uni == 0 {
        0.0
    } else {
        inter as f64 / uni as f64
    }
}

pub fn state(core: &AppCore, id: u32) -> PhotoState {
    core.photo_states()
        .into_iter()
        .find(|s| s.id == id)
        .expect("photo in the states")
}

// ---- Synthetic sets ----

pub fn synth(set: &str) -> PathBuf {
    synth_dir().join(set)
}

/// Adds the marked photos `names` of synthetic set `set`; returns their ids in the same order.
pub fn add_synth(core: &AppCore, set: &str, names: &[&str]) -> Vec<u32> {
    let paths: Vec<PathBuf> = names
        .iter()
        .map(|n| synth(set).join("marked").join(format!("{n}.png")))
        .collect();
    let added = core.add_paths(&paths).unwrap();
    assert_eq!(added.added.len(), names.len(), "{:?}", added.skipped);
    added.added.iter().map(|p| p.id).collect()
}

pub fn synth_truth(set: &str, name: &str) -> GrayImage {
    image::open(synth(set).join("mask").join(format!("{name}.png")))
        .unwrap()
        .to_luma8()
}

/// Sends `truth` as the draft of photo `id`. Synthetic photos are below the edit size, so the
/// edit size is the photo size.
pub fn put_truth(core: &AppCore, id: u32, truth: &GrayImage) {
    core.put_draft(id, truth.width(), truth.height(), truth.as_raw().clone())
        .unwrap();
}

/// Confirms the ground truth of `name` (photo `id`) as the current group's area (group 1 when
/// there is none).
pub fn confirm_truth(core: &AppCore, set: &str, name: &str, id: u32) {
    put_truth(core, id, &synth_truth(set, name));
    core.confirm_set_area(id).unwrap();
}

/// Unticks every ticked unsure photo nobody opened.
pub fn settle(core: &AppCore) {
    for st in core.photo_states() {
        if st.status == puw_app::AreaStatus::Unsure && st.included && !st.reviewed {
            core.set_included(st.id, false).unwrap();
        }
    }
}

/// A second watermark: a ring (outer radius 40, inner radius 27) with a 5 px bar through the
/// middle, 81 x 81, as in puw-core's `tests/template.rs::two_piece_layer`.
pub fn ring_logo() -> GrayImage {
    let (r_out, r_in) = (40.0f32, 27.0f32);
    let side = (2.0 * r_out) as u32 + 1;
    let c = r_out;
    GrayImage::from_fn(side, side, |x, y| {
        let d = ((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt();
        let bar = (y as f32 - c).abs() <= 5.0 && d <= r_out;
        Luma([if (d <= r_out && d >= r_in) || bar {
            255
        } else {
            0
        }])
    })
}

/// Where photo `k` of [`logo_photos`] gets the logo (top-left corner).
pub const LOGO_AT: [(u32, u32); 4] = [(60, 60), (900, 40), (80, 560), (880, 560)];

/// `negative/clean/00..n` with [`ring_logo`] blended in white at alpha 0.3 at `LOGO_AT[k % 4]`,
/// written into `dir/marked/logo-<k>.png` and their truths into `dir/mask/logo-<k>.png`.
/// Returns each photo's path and truth.
pub fn logo_photos(dir: &Path, n: usize) -> Vec<(PathBuf, GrayImage)> {
    let logo = ring_logo();
    let (marked, mask) = (dir.join("marked"), dir.join("mask"));
    std::fs::create_dir_all(&marked).unwrap();
    std::fs::create_dir_all(&mask).unwrap();
    (0..n)
        .map(|k| {
            let mut img = image::open(synth("negative").join("clean").join(format!("{k:02}.png")))
                .unwrap()
                .to_rgb8();
            let mut truth = GrayImage::new(img.width(), img.height());
            let (x, y) = LOGO_AT[k % LOGO_AT.len()];
            for (lx, ly, p) in logo.enumerate_pixels() {
                if p.0[0] == 0 {
                    continue;
                }
                for c in &mut img.get_pixel_mut(x + lx, y + ly).0 {
                    *c = (*c as f32 * 0.7 + 255.0 * 0.3).round() as u8;
                }
                truth.put_pixel(x + lx, y + ly, Luma([255]));
            }
            let path = marked.join(format!("logo-{k:02}.png"));
            img.save(&path).unwrap();
            truth.save(mask.join(format!("logo-{k:02}.png"))).unwrap();
            (path, truth)
        })
        .collect()
}

/// A set with two watermarks and photos with none (ids in that order).
pub struct Mixed {
    /// The 8 photos of `fixed/marked`.
    pub fixed: Vec<u32>,
    /// 4 photos with [`ring_logo`].
    pub logos: Vec<u32>,
    /// Their truths, in the same order.
    pub logo_truths: Vec<GrayImage>,
    /// `negative/marked` 04 and 05: no watermark.
    pub plain: Vec<u32>,
}

/// Adds `fixed/marked` (8 photos), 4 [`logo_photos`] written into `dir`, and `negative/marked`
/// 04 and 05.
pub fn mixed_set(core: &AppCore, dir: &Path) -> Mixed {
    let names: Vec<String> = (0..8).map(|k| format!("{k:02}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let fixed = add_synth(core, "fixed", &refs);
    let (paths, logo_truths): (Vec<PathBuf>, Vec<GrayImage>) =
        logo_photos(dir, 4).into_iter().unzip();
    let added = core.add_paths(&paths).unwrap();
    assert_eq!(added.added.len(), 4, "{:?}", added.skipped);
    let logos = added.added.iter().map(|p| p.id).collect();
    let plain = add_synth(core, "negative", &["04", "05"]);
    Mixed {
        fixed,
        logos,
        logo_truths,
        plain,
    }
}

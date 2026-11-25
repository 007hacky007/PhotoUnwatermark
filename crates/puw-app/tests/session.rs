//! The session: detection, the saved-watermark shortcut and the step-2 watermark area.
//!
//! Tests that need models read them from `PUW_MODELS_DIR` and return early with a message when
//! it is not set. They run the models as the app ships them (Core ML on Macs, the processor
//! elsewhere) and share one compiled-model cache, so Core ML compiles each model once, not once
//! per test.

mod common;

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use image::{GrayImage, Rgb, RgbImage};
use puw_app::images::jpeg;
use puw_app::{
    AppCore, AppError, DataDirs, LoadPurpose, ProgressEvent, Settings, Suggestion, user_message,
};
use puw_core::{Library, MatchStatus, ModelKind, SavedWatermark, build_template, find};

/// Model tests run one at a time, so their timings and memory do not add up and only one of
/// them writes to the shared compiled-model cache.
static MODEL_TESTS: Mutex<()> = Mutex::new(());

struct Fixture {
    core: AppCore,
    dirs: DataDirs,
    _tmp: tempfile::TempDir,
    _serial: Option<MutexGuard<'static, ()>>,
}

/// A core with its own data folder. `models` replaces the data folder's (empty) models folder.
fn fixture(models: Option<PathBuf>) -> Fixture {
    let serial = models
        .as_ref()
        .map(|_| MODEL_TESTS.lock().unwrap_or_else(PoisonError::into_inner));
    let tmp = tempfile::tempdir().unwrap();
    let mut dirs = DataDirs::at(tmp.path());
    if let Some(m) = models {
        dirs.models = m;
        dirs.cache = common::model_cache_dir();
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
fn model_fixture() -> Option<Fixture> {
    match common::models_dir() {
        Some(m) => Some(fixture(Some(m))),
        None => {
            eprintln!("skipped: PUW_MODELS_DIR not set");
            None
        }
    }
}

fn fixed() -> PathBuf {
    common::synth_dir().join("fixed")
}

/// Adds the 8 marked photos of the synthetic `fixed` set; their ids are 0..8 in name order.
fn add_fixed(core: &AppCore) {
    let added = core.add_paths(&[fixed().join("marked")]).unwrap();
    assert_eq!(added.added.len(), 8, "{:?}", added.skipped);
}

fn name_of(core: &AppCore, id: u32) -> String {
    core.photos()
        .into_iter()
        .find(|p| p.id == id)
        .expect("photo in the list")
        .name
}

fn ground_truth(name: &str) -> GrayImage {
    image::open(fixed().join("mask").join(name))
        .unwrap()
        .to_luma8()
}

/// Share of the ground-truth pixels that `area` (same size, row-major) covers.
fn covered(truth: &GrayImage, area: &[u8]) -> f64 {
    assert_eq!(truth.as_raw().len(), area.len());
    let (mut all, mut hit) = (0u64, 0u64);
    for (t, a) in truth.as_raw().iter().zip(area) {
        if *t != 0 {
            all += 1;
            hit += (*a != 0) as u64;
        }
    }
    assert!(all > 0);
    hit as f64 / all as f64
}

fn noop(_: ProgressEvent) {}

#[derive(Default)]
struct Recorder(Mutex<Vec<ProgressEvent>>);

impl Recorder {
    fn push(&self, e: ProgressEvent) {
        self.0.lock().unwrap().push(e);
    }

    fn events(&self) -> Vec<ProgressEvent> {
        self.0.lock().unwrap().clone()
    }
}

fn load_fractions(events: &[ProgressEvent]) -> Vec<(f32, LoadPurpose)> {
    events
        .iter()
        .filter_map(|e| match e {
            ProgressEvent::LoadingModels { fraction, which } => Some((*fraction, *which)),
            _ => None,
        })
        .collect()
}

#[test]
fn core_is_send_and_sync() {
    fn check<T: Send + Sync>() {}
    check::<AppCore>();
}

#[test]
fn detect_without_loading_at_start() {
    let Some(f) = model_fixture() else { return };
    add_fixed(&f.core);
    let r = f.core.engine().unwrap().report();
    assert!(
        r.segmenter.is_none() && r.lama.is_none() && r.face.is_none(),
        "{r:?}"
    );
}

#[test]
fn detect_fixed_set() {
    let Some(f) = model_fixture() else { return };
    add_fixed(&f.core);
    let rec = Recorder::default();
    let out = f.core.detect(&|e| rec.push(e)).unwrap();

    assert_eq!(out.sample.len(), 3);
    assert!(out.sample.contains(&out.representative));
    let events = rec.events();
    assert!(
        load_fractions(&events)
            .iter()
            .any(|(_, w)| *w == LoadPurpose::Finder)
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ProgressEvent::Reading { .. }))
    );
    assert!(events.contains(&ProgressEvent::Looking { done: 3, total: 3 }));

    let rep = out.representative;
    assert_eq!(f.core.edit_size(rep).unwrap(), (1536, 1024));
    let share = covered(
        &ground_truth(&name_of(&f.core, rep)),
        &f.core.area(rep).unwrap(),
    );
    eprintln!("representative {rep} covers {share:.3} of its watermark");
    assert!(share >= 0.8, "covers {share:.3}");
}

#[test]
fn detect_reports_load_progress_monotonic() {
    let Some(f) = model_fixture() else { return };
    add_fixed(&f.core);
    let rec = Recorder::default();
    f.core.detect(&|e| rec.push(e)).unwrap();

    let loads = load_fractions(&rec.events());
    eprintln!("load fractions: {loads:?}");
    assert!(loads.len() >= 2, "{loads:?}");
    assert!(loads.iter().all(|(_, w)| *w == LoadPurpose::Finder));
    assert!(loads.windows(2).all(|p| p[0].0 <= p[1].0), "{loads:?}");
    assert!(loads.iter().all(|(x, _)| (0.0..=1.0).contains(x)));
    assert_eq!(loads.last().unwrap().0, 1.0);
    assert!(loads[..loads.len() - 1].iter().all(|(x, _)| *x <= 0.95));

    // The measured load times are kept for the next estimate, in memory and on disk.
    for s in [f.core.settings(), Settings::load(&f.dirs.settings)] {
        for kind in [ModelKind::Segmenter, ModelKind::Face] {
            assert!(s.load_seconds.get(&kind).is_some_and(|x| *x > 0.0), "{s:?}");
        }
        assert!(!s.load_seconds.contains_key(&ModelKind::Lama));
    }

    // Loaded models stay: a second detection loads nothing.
    let rec = Recorder::default();
    f.core.detect(&|e| rec.push(e)).unwrap();
    assert!(load_fractions(&rec.events()).is_empty());
}

#[test]
fn second_job_is_busy() {
    let Some(f) = model_fixture() else { return };
    add_fixed(&f.core);
    let core = &f.core;
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    std::thread::scope(|s| {
        let first = s.spawn(|| {
            core.detect(&|e| {
                let _ = tx.lock().unwrap().send(e);
            })
        });
        rx.recv_timeout(Duration::from_secs(60))
            .expect("the first job reports progress");
        assert!(matches!(core.detect(&noop), Err(AppError::Busy)));
        assert!(matches!(core.use_saved("x", &noop), Err(AppError::Busy)));
        assert!(matches!(core.suggest_saved(), Err(AppError::Busy)));
        assert!(matches!(
            core.ensure_models(&[ModelKind::Lama], LoadPurpose::Remover, &noop),
            Err(AppError::Busy)
        ));
        assert!(matches!(core.add_paths(&[]), Err(AppError::Busy)));
        assert!(matches!(core.remove_photo(0), Err(AppError::Busy)));
        assert!(matches!(core.clear_photos(), Err(AppError::Busy)));
        assert!(matches!(core.confirm_set_area(0), Err(AppError::Busy)));
        first.join().unwrap().unwrap();
    });
    // The slot is free again once the job is done.
    assert_eq!(core.suggest_saved().unwrap(), None);
}

#[test]
fn cancel_detect() {
    let Some(f) = model_fixture() else { return };
    add_fixed(&f.core);
    let core = &f.core;
    let err = core
        .detect(&|e| {
            if matches!(e, ProgressEvent::Reading { .. }) {
                core.cancel();
            }
        })
        .unwrap_err();
    assert_eq!(user_message(&err), "Stopped.");

    // The session is still usable and the next job is not cancelled.
    assert_eq!(core.photos().len(), 8);
    let out = core.detect(&noop).unwrap();
    assert_eq!(out.sample.len(), 3);
}

/// Saves "Studio", learned from the ground truth of the fixed set's first photo.
fn save_fixed_watermark(dirs: &DataDirs) -> SavedWatermark {
    let marked = image::open(fixed().join("marked/00.png"))
        .unwrap()
        .to_rgb8();
    let t = build_template(&marked, &ground_truth("00.png"));
    let w = SavedWatermark::new("Studio", t, jpeg(&RgbImage::new(4, 4), 80));
    Library::open(&dirs.watermarks).unwrap().save(&w).unwrap();
    w
}

/// The fixed set's marked photos copied into a folder of their own, so files can be spoiled.
fn copy_fixed() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for e in std::fs::read_dir(fixed().join("marked")).unwrap() {
        let p = e.unwrap().path();
        std::fs::copy(&p, dir.path().join(p.file_name().unwrap())).unwrap();
    }
    dir
}

#[test]
fn use_saved_skips_models() {
    // Without PUW_MODELS_DIR the models folder is empty, so loading any model would fail.
    let f = fixture(common::models_dir());
    add_fixed(&f.core);
    assert_eq!(f.core.suggest_saved().unwrap(), None);

    let w = save_fixed_watermark(&f.dirs);

    assert_eq!(
        f.core.suggest_saved().unwrap(),
        Some(Suggestion {
            id: w.id.clone(),
            name: "Studio".into()
        })
    );
    let rec = Recorder::default();
    let out = f.core.use_saved(&w.id, &|e| rec.push(e)).unwrap();
    assert_eq!(out.sample.len(), 3);
    assert!(out.sample.contains(&out.representative));
    let events = rec.events();
    assert!(load_fractions(&events).is_empty());
    assert!(events.contains(&ProgressEvent::Looking { done: 3, total: 3 }));

    let rep = out.representative;
    let share = covered(
        &ground_truth(&name_of(&f.core, rep)),
        &f.core.area(rep).unwrap(),
    );
    assert!(share >= 0.8, "covers {share:.3}");

    if common::models_dir().is_some() {
        let r = f.core.engine().unwrap().report();
        assert!(
            r.segmenter.is_none() && r.lama.is_none() && r.face.is_none(),
            "{r:?}"
        );
    }
}

#[test]
fn confirm_rejects_empty() {
    let f = fixture(None);
    add_fixed(&f.core);
    // No draft and no area yet.
    assert!(matches!(
        f.core.confirm_set_area(1),
        Err(AppError::NothingToRemove)
    ));
    f.core
        .put_draft(1, 1536, 1024, vec![0; 1536 * 1024])
        .unwrap();
    assert!(matches!(
        f.core.confirm_set_area(1),
        Err(AppError::NothingToRemove)
    ));
    assert!(f.core.confirmed_template().is_none());
}

#[test]
fn confirm_builds_template() {
    let f = fixture(None);
    add_fixed(&f.core);
    let gt0 = ground_truth(&name_of(&f.core, 0));
    f.core
        .put_draft(0, 1536, 1024, gt0.as_raw().clone())
        .unwrap();
    f.core.confirm_set_area(0).unwrap();

    let t = f.core.confirmed_template().expect("set area confirmed");
    assert!(!t.pieces.is_empty());
    assert_eq!((t.ref_w, t.ref_h), (1536, 1024));
    let other = image::open(fixed().join("marked/03.png"))
        .unwrap()
        .to_rgb8();
    assert_eq!(find(&t, &other).status, MatchStatus::Found);

    // The confirmed area is that photo's area now.
    let binary: Vec<u8> = gt0
        .as_raw()
        .iter()
        .map(|&v| if v != 0 { 255 } else { 0 })
        .collect();
    assert!(
        f.core.area(0).unwrap() == binary,
        "the confirmed area is not the draft"
    );

    // Confirming another photo replaces the set area and clears the earlier result.
    let gt2 = ground_truth(&name_of(&f.core, 2));
    f.core
        .put_draft(2, 1536, 1024, gt2.as_raw().clone())
        .unwrap();
    f.core.confirm_set_area(2).unwrap();
    assert!(f.core.area(0).unwrap().iter().all(|&v| v == 0));
    assert!(covered(&gt2, &f.core.area(2).unwrap()) == 1.0);
}

fn write_big_jpeg(dir: &Path) -> PathBuf {
    let img = RgbImage::from_fn(5000, 3000, |x, y| {
        Rgb([(x % 251) as u8, (y % 241) as u8, ((x + y) % 239) as u8])
    });
    let p = dir.join("big.jpg");
    std::fs::write(&p, jpeg(&img, 85)).unwrap();
    p
}

#[test]
fn area_bytes_have_edit_size() {
    let f = fixture(None);
    let photos = tempfile::tempdir().unwrap();
    let big = write_big_jpeg(photos.path());
    let added = f.core.add_paths(&[big]).unwrap().added;
    let id = added[0].id;

    let (ew, eh) = f.core.edit_size(id).unwrap();
    assert_eq!((ew, eh), (4096, 2458));
    let area = f.core.area(id).unwrap();
    assert_eq!(area.len(), (ew * eh) as usize);
    assert!(area.iter().all(|&v| v == 0));

    // A wrong size is refused.
    assert!(f.core.put_draft(id, ew, eh, vec![0; 10]).is_err());
    assert!(
        f.core
            .put_draft(id, 1536, 1024, vec![0; 1536 * 1024])
            .is_err()
    );

    // A painted rectangle survives the trip to full size and back.
    let mut draft = vec![0u8; (ew * eh) as usize];
    for y in 100..300 {
        for x in 200..900 {
            draft[(y * ew + x) as usize] = 1;
        }
    }
    f.core.put_draft(id, ew, eh, draft.clone()).unwrap();
    f.core.confirm_set_area(id).unwrap();
    let area = f.core.area(id).unwrap();
    assert_eq!(area.len(), (ew * eh) as usize);
    assert!(area.iter().all(|&v| v == 0 || v == 255));
    let painted = draft.iter().filter(|&&v| v != 0).count();
    let kept = draft
        .iter()
        .zip(&area)
        .filter(|(d, a)| **d != 0 && **a != 0)
        .count();
    let total = area.iter().filter(|&&v| v != 0).count();
    assert_eq!(kept, painted);
    assert!(total < painted + painted / 10, "{total} vs {painted}");
}

#[test]
fn images_are_jpeg() {
    let f = fixture(None);
    add_fixed(&f.core);
    let edit = f.core.edit_image(4).unwrap();
    let thumb = f.core.thumb_image(4).unwrap();
    for bytes in [&edit, &thumb] {
        assert!(bytes.starts_with(&[0xFF, 0xD8]));
    }
    let edit = image::load_from_memory(&edit).unwrap();
    assert_eq!((edit.width(), edit.height()), (1536, 1024));
    let thumb = image::load_from_memory(&thumb).unwrap();
    assert_eq!(thumb.width().max(thumb.height()), 320);
    // Served from memory the second time, with the same bytes.
    assert_eq!(
        f.core.thumb_image(4).unwrap(),
        f.core.thumb_image(4).unwrap()
    );
    assert!(f.core.thumb_image(99).is_err());
}

#[test]
fn photo_list_changes() {
    let f = fixture(None);
    add_fixed(&f.core);
    f.core.remove_photo(3).unwrap();
    assert_eq!(f.core.photos().len(), 7);
    assert!(f.core.area(3).is_err());
    f.core.clear_photos().unwrap();
    assert!(f.core.photos().is_empty());
    // Ids are never reused.
    add_fixed(&f.core);
    assert!(f.core.photos().iter().all(|p| p.id >= 8));
}

#[test]
fn images_served_during_job() {
    let Some(f) = model_fixture() else { return };
    add_fixed(&f.core);
    let core = &f.core;
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    std::thread::scope(|s| {
        let job = s.spawn(|| {
            core.detect(&|e| {
                if matches!(e, ProgressEvent::Looking { done: 0, .. }) {
                    let _ = tx.lock().unwrap().send(());
                }
            })
        });
        rx.recv_timeout(Duration::from_secs(120))
            .expect("detection reaches the guesses");
        for _ in 0..5 {
            let t = Instant::now();
            assert_eq!(core.photos().len(), 8);
            let photos_took = t.elapsed();
            let t = Instant::now();
            assert_eq!(core.area(0).unwrap().len(), 1536 * 1024);
            let area_took = t.elapsed();
            assert!(
                photos_took < Duration::from_millis(100) && area_took < Duration::from_millis(100),
                "photos {photos_took:?}, area {area_took:?}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!job.is_finished(), "the job was still running");
        job.join().unwrap().unwrap();
    });
}

#[test]
fn new_guesses_drop_stale_drafts() {
    let f = fixture(None);
    add_fixed(&f.core);
    let w = save_fixed_watermark(&f.dirs);
    // A draft on photo 1 (in the sample), far from the watermark.
    let mut draft = vec![0u8; 1536 * 1024];
    for y in 0..100 {
        draft[y * 1536..y * 1536 + 100].fill(255);
    }
    f.core.put_draft(1, 1536, 1024, draft).unwrap();

    let out = f.core.use_saved(&w.id, &noop).unwrap();
    assert!(out.sample.contains(&1));
    let shown = f.core.area(1).unwrap();
    assert!(shown.iter().any(|&v| v != 0));
    assert!(shown[..100].iter().all(|&v| v == 0));

    // Accepting what is shown, without a new draft, confirms the new guess.
    f.core.confirm_set_area(1).unwrap();
    assert!(
        f.core.area(1).unwrap() == shown,
        "the confirmed area is not the one shown"
    );
    let t = f.core.confirmed_template().unwrap();
    let other = image::open(fixed().join("marked/05.png"))
        .unwrap()
        .to_rgb8();
    assert_eq!(find(&t, &other).status, MatchStatus::Found);
}

#[test]
fn unreadable_sample_photo_is_left_out() {
    let f = fixture(None);
    let photos = copy_fixed();
    f.core.add_paths(&[photos.path().to_path_buf()]).unwrap();
    let w = save_fixed_watermark(&f.dirs);
    // The sample of 8 photos is 1, 4 and 6; photo 4 can no longer be read.
    std::fs::write(photos.path().join("04.png"), b"not a photo").unwrap();

    let rec = Recorder::default();
    let out = f.core.use_saved(&w.id, &|e| rec.push(e)).unwrap();
    assert_eq!(out.sample, vec![1, 6]);
    assert!(out.sample.contains(&out.representative));
    assert!(
        rec.events()
            .contains(&ProgressEvent::Looking { done: 3, total: 3 })
    );
    assert!(f.core.area(4).unwrap().iter().all(|&v| v == 0));

    // With no readable photo in the sample the search fails with a plain sentence.
    for name in ["01.png", "06.png"] {
        std::fs::remove_file(photos.path().join(name)).unwrap();
    }
    let err = f.core.use_saved(&w.id, &noop).unwrap_err();
    let m = user_message(&err);
    assert!(
        m == "The photo could not be opened. The file may be damaged."
            || m == "The photo could not be found. It may have been moved or deleted.",
        "{m}"
    );
}

#[test]
fn detect_leaves_out_unreadable_photo() {
    let Some(f) = model_fixture() else { return };
    let photos = copy_fixed();
    f.core.add_paths(&[photos.path().to_path_buf()]).unwrap();
    std::fs::write(photos.path().join("04.png"), b"not a photo").unwrap();
    let out = f.core.detect(&noop).unwrap();
    assert_eq!(out.sample, vec![1, 6]);
    assert!(out.sample.contains(&out.representative));
}

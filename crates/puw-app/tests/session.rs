//! The session: detection, the saved-watermark shortcut, the step-2 watermark area and
//! finding it in every photo (step 3).
//!
//! Tests that need models read them from `PUW_MODELS_DIR` and return early with a message when
//! it is not set. They run the models as the app ships them (Core ML on Macs, the processor
//! elsewhere) and share one compiled-model cache, so Core ML compiles each model once, not once
//! per test.

mod common;

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{
    Recorder, add_synth, confirm_truth, fixture, iou, matching, model_fixture, noop, rect_area,
    state, synth, synth_truth,
};
use image::{GrayImage, Rgb, RgbImage};
use puw_app::images::{GRID_LONG_SIDE, jpeg, thumbnail};
use puw_app::photos::Problem;
use puw_app::{
    AppCore, AppError, AreaStatus, DataDirs, LoadPurpose, MatchSummary, ProgressEvent, Settings,
    Suggestion, user_message,
};
use puw_core::{Library, MatchStatus, ModelKind, SavedWatermark, build_template, find};

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
        &f.core.start_area(rep).unwrap(),
    );
    eprintln!("representative {rep} covers {share:.3} of its watermark");
    assert!(share >= 0.8, "covers {share:.3}");
}

/// The first guess, confirmed as it is (no editing), is found on every photo of the fixed set,
/// and once saved it is offered for the same photos, quickly.
#[test]
fn first_guess_matches_fixed_set_and_is_offered() {
    let Some(f) = model_fixture() else { return };
    add_fixed(&f.core);
    let out = f.core.detect(&noop).unwrap();
    f.core.confirm_set_area(out.representative).unwrap();
    let sum = f.core.match_all(&noop).unwrap();
    eprintln!("first guess as the set area: {sum:?}");
    assert_eq!(sum.found, 8, "{sum:?}");

    let id = f.core.save_watermark(1, "From the guess").unwrap();
    f.core.clear_photos().unwrap();
    add_fixed(&f.core);
    let start = Instant::now();
    let offer = f.core.suggest_saved().unwrap();
    let secs = start.elapsed().as_secs_f32();
    eprintln!("saved-watermark offer in {secs:.1} s");
    assert_eq!(offer.map(|s| s.id), Some(id));
    assert!(secs < 30.0, "{secs:.1} s");
}

/// Model files that are gone are missing app data (download again), not a photo problem.
#[test]
fn missing_models_are_missing_data() {
    let f = fixture(None);
    add_fixed(&f.core);
    let err = f.core.detect(&noop).unwrap_err();
    assert!(matches!(err, AppError::MissingModels(_)), "{err:?}");
    let u = puw_app::UserError::from(err);
    assert!(u.missing_data);
    assert!(!u.message.contains("photo"), "{}", u.message);

    // A file deleted after the engine was made is reported the same way.
    for a in puw_app::download::model_assets() {
        std::fs::write(f.dirs.models.join(a.name), b"x").unwrap();
    }
    f.core.engine().unwrap();
    std::fs::remove_file(f.dirs.models.join("segmenter.onnx")).unwrap();
    let err = f.core.detect(&noop).unwrap_err();
    assert!(matches!(err, AppError::MissingModels(_)), "{err:?}");
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
        assert!(matches!(core.use_saved("x"), Err(AppError::Busy)));
        // Not a job: it answers at once.
        assert!(matches!(
            core.group_sample(),
            Err(AppError::NothingToRemove)
        ));
        assert!(matches!(core.suggest_saved(), Err(AppError::Busy)));
        assert!(matches!(
            core.ensure_models(&[ModelKind::Lama], LoadPurpose::Remover, &noop),
            Err(AppError::Busy)
        ));
        assert!(matches!(core.add_paths(&[]), Err(AppError::Busy)));
        assert!(matches!(core.remove_photo(0), Err(AppError::Busy)));
        assert!(matches!(core.clear_photos(), Err(AppError::Busy)));
        assert!(matches!(core.confirm_set_area(0), Err(AppError::Busy)));
        assert!(matches!(core.confirm_new_group(0), Err(AppError::Busy)));
        assert!(matches!(core.clear_groups(), Err(AppError::Busy)));
        assert!(matches!(core.match_all(&noop), Err(AppError::Busy)));
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

/// A saved watermark starts a group at once; its matching loads no model and the photos step 2
/// shows for it are where it was found.
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
    assert_eq!(f.core.use_saved(&w.id).unwrap(), 1);
    let rec = Recorder::default();
    let sum = f.core.match_all(&|e| rec.push(e)).unwrap();
    assert_eq!(sum.found, 8, "{sum:?}");
    let events = rec.events();
    assert!(load_fractions(&events).is_empty());
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, ProgressEvent::Looking { .. }))
    );

    let out = f.core.group_sample().unwrap();
    assert_eq!(out.sample.len(), 3);
    assert_eq!(out.sample[0], out.representative);
    let rep = out.representative;
    let share = covered(
        &ground_truth(&name_of(&f.core, rep)),
        &f.core.start_area(rep).unwrap(),
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
fn saved_group_drops_stale_drafts() {
    let f = fixture(None);
    add_fixed(&f.core);
    let w = save_fixed_watermark(&f.dirs);
    // A draft on photo 1, far from the watermark.
    let mut draft = vec![0u8; 1536 * 1024];
    for y in 0..100 {
        draft[y * 1536..y * 1536 + 100].fill(255);
    }
    f.core.put_draft(1, 1536, 1024, draft).unwrap();

    assert_eq!(f.core.use_saved(&w.id).unwrap(), 1);
    assert!(f.core.start_area(1).unwrap().iter().all(|&v| v == 0));
    f.core.match_all(&noop).unwrap();
    let shown = f.core.start_area(1).unwrap();
    assert!(shown.iter().any(|&v| v != 0));
    assert!(shown[..100].iter().all(|&v| v == 0));

    // Accepting what is shown, without a new draft, changes nothing: the old draft is gone.
    f.core.confirm_set_area(1).unwrap();
    assert!(
        f.core.area(1).unwrap() == shown,
        "the confirmed area is not the one shown"
    );
    let g = f.core.groups();
    assert!(!g.groups[0].edited && g.matched, "{g:?}");
}

#[test]
fn unreadable_photo_stays_out_of_a_saved_group() {
    let f = fixture(None);
    let photos = copy_fixed();
    f.core.add_paths(&[photos.path().to_path_buf()]).unwrap();
    let w = save_fixed_watermark(&f.dirs);
    std::fs::write(photos.path().join("04.png"), b"not a photo").unwrap();

    assert_eq!(f.core.use_saved(&w.id).unwrap(), 1);
    let sum = f.core.match_all(&noop).unwrap();
    assert_eq!(sum.total, 7, "{sum:?}");
    let st = state(&f.core, 4);
    assert_eq!(st.status, AreaStatus::Unreadable);
    assert!(st.group.is_none() && !st.member, "{st:?}");
    let out = f.core.group_sample().unwrap();
    assert!(!out.sample.contains(&4), "{out:?}");
    assert!(out.sample.contains(&out.representative));
}

/// A photo cut short after it was added, and one deleted: both are "cannot be read" (with
/// why), never ticked, never counted as not found, and they do not hold up removal.
#[test]
fn unreadable_photos_get_their_own_state() {
    let f = fixture(None);
    let photos = copy_fixed();
    f.core.add_paths(&[photos.path().to_path_buf()]).unwrap();
    let bytes = std::fs::read(photos.path().join("02.png")).unwrap();
    std::fs::write(photos.path().join("02.png"), &bytes[..bytes.len() / 3]).unwrap();
    std::fs::remove_file(photos.path().join("05.png")).unwrap();
    // The thumbnail of the cut-short photo fails and says so.
    assert!(f.core.thumb_image(2).is_err());
    let problem = |id: u32| {
        f.core
            .photos()
            .into_iter()
            .find(|p| p.id == id)
            .unwrap()
            .problem
    };
    assert_eq!(problem(2), Some(Problem::Damaged));
    assert_eq!(problem(5), None);

    confirm_truth(&f.core, "fixed", "00", 0);
    let sum = f.core.match_all(&noop).unwrap();
    assert_eq!(problem(2), Some(Problem::Damaged));
    assert_eq!(problem(5), Some(Problem::Gone));
    for id in [2, 5] {
        let st = state(&f.core, id);
        assert_eq!(st.status, AreaStatus::Unreadable);
        assert!(!st.included && !st.has_area);
        assert!(f.core.set_included(id, true).is_err());
    }
    assert_eq!(sum.found + sum.unsure + sum.not_found, 6, "{sum:?}");
    // Photos that cannot be read are in no group, so they are not counted at all.
    assert_eq!(sum.total, 6);
    assert!(!f.core.removal_blockers().contains(&2));
    assert!(!f.core.removal_blockers().contains(&5));
    assert!(!f.core.to_remove().contains(&2));
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

// ---- Step 3: finding the set's watermark in every photo ----

fn synth_names(set: &str) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(synth(set).join("marked"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "png"))
        .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

/// Marks that change the luma under their ground truth by less than this on average are
/// invisible (the same rule as puw-core's `random_set` test; only `random/02` is below it).
const MIN_VISIBLE: f32 = 10.0;

fn visibility(set: &str, name: &str) -> f32 {
    let open = |sub: &str| {
        image::open(synth(set).join(sub).join(format!("{name}.png")))
            .unwrap()
            .to_rgb8()
    };
    let (m, c, g) = (open("marked"), open("clean"), synth_truth(set, name));
    let luma = |p: &Rgb<u8>| 0.299 * p.0[0] as f32 + 0.587 * p.0[1] as f32 + 0.114 * p.0[2] as f32;
    let (mut sum, mut n) = (0f32, 0f32);
    for ((a, b), q) in m.pixels().zip(c.pixels()).zip(g.pixels()) {
        if q.0[0] != 0 {
            sum += (luma(a) - luma(b)).abs();
            n += 1.0;
        }
    }
    sum / n.max(1.0)
}

#[test]
fn match_random_set() {
    let f = fixture(None);
    let names = synth_names("random");
    assert_eq!(names.len(), 12);
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let ids = add_synth(&f.core, "random", &refs);
    let negatives = synth_names("negative");
    let refs: Vec<&str> = negatives.iter().map(String::as_str).collect();
    let neg_ids = add_synth(&f.core, "negative", &refs);
    confirm_truth(&f.core, "random", &names[0], ids[0]);
    assert_eq!(state(&f.core, neg_ids[0]).status, AreaStatus::Pending);

    let rec = Recorder::default();
    let t = Instant::now();
    let summary = f.core.match_all(&|e| rec.push(e)).unwrap();
    eprintln!("match_all of 17 photos took {:.2?}", t.elapsed());

    let m = matching(&rec.events());
    assert_eq!(m.first(), Some(&(0, 17)), "{m:?}");
    assert_eq!(m.last(), Some(&(17, 17)), "{m:?}");
    assert!(m.windows(2).all(|p| p[0].0 + 1 == p[1].0), "{m:?}");

    // The confirmed photo is found with its confirmed area.
    let s0 = state(&f.core, ids[0]);
    assert_eq!(s0.status, AreaStatus::Found);
    assert!(s0.included && s0.has_area);

    let (mut visible, mut found) = (0, 0);
    for (name, &id) in names.iter().zip(&ids).skip(1) {
        let s = state(&f.core, id);
        let v = iou(&synth_truth("random", name), &f.core.area(id).unwrap());
        eprintln!("random {name}: {:?} iou {v:.3}", s.status);
        if s.status == AreaStatus::Found {
            assert!(v >= 0.6, "{name}: found in the wrong place, iou {v:.3}");
        }
        if visibility("random", name) >= MIN_VISIBLE {
            visible += 1;
            found += (s.status == AreaStatus::Found) as u32;
        }
    }
    assert!(visible >= 10, "only {visible} visible");
    assert!(found * 100 >= visible * 95, "found {found} of {visible}");
    for &id in &neg_ids {
        assert_ne!(
            state(&f.core, id).status,
            AreaStatus::Found,
            "negative {id}"
        );
    }

    let states = f.core.photo_states();
    let count = |st: AreaStatus| states.iter().filter(|s| s.status == st).count() as u32;
    assert_eq!(
        summary,
        MatchSummary {
            found: count(AreaStatus::Found),
            unsure: count(AreaStatus::Unsure),
            not_found: count(AreaStatus::NotFound),
            total: 18,
        }
    );
    assert_eq!(count(AreaStatus::Pending), 0);

    // Every photo has a result now: running it again matches nothing.
    let rec = Recorder::default();
    f.core.match_all(&|e| rec.push(e)).unwrap();
    assert_eq!(matching(&rec.events()), vec![(0, 0)]);
}

#[test]
fn match_needs_confirmed_area() {
    let f = fixture(None);
    add_synth(&f.core, "random", &["00"]);
    assert!(matches!(
        f.core.match_all(&noop),
        Err(AppError::NothingToRemove)
    ));
}

#[test]
fn unsure_blocks_until_opened_or_unticked() {
    let f = fixture(None);
    // With the set area from random/05, random/10 scores 0.432: unsure
    // (docs/notes/thresholds.md).
    let ids = add_synth(&f.core, "random", &["05", "10"]);
    let (five, ten) = (ids[0], ids[1]);
    confirm_truth(&f.core, "random", "05", five);
    f.core.match_all(&noop).unwrap();

    let s = state(&f.core, ten);
    assert_eq!(s.status, AreaStatus::Unsure);
    assert!(s.included && !s.reviewed && s.has_area, "{s:?}");
    assert_eq!(f.core.removal_blockers(), vec![ten]);
    // Not a member of the group until somebody has looked at it.
    assert_eq!(f.core.to_remove(), vec![five]);

    // Opening it in the editor is enough.
    f.core.open_photo(ten).unwrap();
    assert!(state(&f.core, ten).reviewed);
    assert!(f.core.removal_blockers().is_empty());
    assert_eq!(f.core.to_remove(), vec![five, ten]);

    // Confirming a changed set area starts over: the photo needs a look again. (The same
    // area again changes nothing, see `unchanged_reconfirm_keeps_step3`.)
    let (w, h) = f.core.edit_size(five).unwrap();
    let mut changed = f.core.area(five).unwrap();
    changed[0] = 255;
    f.core.put_draft(five, w, h, changed).unwrap();
    f.core.confirm_set_area(five).unwrap();
    assert_eq!(state(&f.core, ten).status, AreaStatus::Pending);
    f.core.match_all(&noop).unwrap();
    let s = state(&f.core, ten);
    assert_eq!(s.status, AreaStatus::Unsure);
    assert!(!s.reviewed);
    assert_eq!(f.core.removal_blockers(), vec![ten]);

    // Unticking it is the other way.
    f.core.set_included(ten, false).unwrap();
    assert!(f.core.removal_blockers().is_empty());
    assert_eq!(f.core.to_remove(), vec![five]);
    assert!(f.core.open_photo(99).is_err());
}

#[test]
fn not_found_unticked_by_default() {
    let f = fixture(None);
    let ids = add_synth(&f.core, "random", &["00"]);
    let neg = add_synth(&f.core, "negative", &["00"])[0];
    confirm_truth(&f.core, "random", "00", ids[0]);
    let summary = f.core.match_all(&noop).unwrap();
    assert_eq!(
        summary,
        MatchSummary {
            found: 1,
            unsure: 0,
            not_found: 1,
            total: 2
        }
    );
    let s = state(&f.core, neg);
    assert_eq!(s.status, AreaStatus::NotFound);
    assert!(!s.included && !s.has_area, "{s:?}");
    assert!(f.core.area(neg).unwrap().iter().all(|&v| v == 0));
    // Not found photos do not hold up removal; they are just left out.
    assert!(f.core.removal_blockers().is_empty());
    assert_eq!(f.core.to_remove(), vec![ids[0]]);
}

#[test]
fn painting_not_found_ticks_it() {
    let f = fixture(None);
    let ids = add_synth(&f.core, "random", &["00"]);
    let neg = add_synth(&f.core, "negative", &["00"])[0];
    confirm_truth(&f.core, "random", "00", ids[0]);
    f.core.match_all(&noop).unwrap();
    assert_eq!(state(&f.core, neg).status, AreaStatus::NotFound);
    let template = f.core.confirmed_template().unwrap();

    f.core.open_photo(neg).unwrap();
    let (w, h) = f.core.edit_size(neg).unwrap();
    let painted = rect_area(w, h, 100, 200, 300, 260);
    f.core.put_draft(neg, w, h, painted.clone()).unwrap();
    f.core.save_photo_area(neg).unwrap();

    let s = state(&f.core, neg);
    assert_eq!(s.status, AreaStatus::Edited);
    assert!(s.included && s.reviewed && s.has_area, "{s:?}");
    assert!(
        f.core.area(neg).unwrap() == painted,
        "the saved area is not the painted one"
    );
    assert_eq!(f.core.to_remove(), vec![ids[0], neg]);
    assert!(f.core.removal_blockers().is_empty());
    // The set's watermark is untouched.
    assert!(Arc::ptr_eq(
        &template,
        &f.core.confirmed_template().unwrap()
    ));
}

#[test]
fn photo_edit_does_not_change_set() {
    let f = fixture(None);
    let ids = add_synth(&f.core, "random", &["00", "01", "03", "04", "11"]);
    confirm_truth(&f.core, "random", "00", ids[0]);
    f.core.match_all(&noop).unwrap();
    let template = f.core.confirmed_template().unwrap();

    // Photo 3 (random/04) gets an area of its own somewhere else.
    let three = ids[3];
    let (w, h) = f.core.edit_size(three).unwrap();
    let own = rect_area(w, h, 10, 10, 60, 40);
    f.core.put_draft(three, w, h, own.clone()).unwrap();
    f.core.save_photo_area(three).unwrap();
    assert!(Arc::ptr_eq(
        &template,
        &f.core.confirmed_template().unwrap()
    ));

    // A changed set area makes the other photos wait for matching again, and they hold up
    // removal until then; photo 3 keeps its own area.
    let new = ids[4];
    let (w, h) = f.core.edit_size(ids[0]).unwrap();
    let mut changed = f.core.area(ids[0]).unwrap();
    changed[0] = 255;
    f.core.put_draft(ids[0], w, h, changed).unwrap();
    f.core.confirm_set_area(ids[0]).unwrap();
    assert_eq!(state(&f.core, new).status, AreaStatus::Pending);
    assert_eq!(f.core.removal_blockers(), vec![ids[1], ids[2], new]);

    let rec = Recorder::default();
    f.core.match_all(&|e| rec.push(e)).unwrap();
    assert_eq!(matching(&rec.events()).last(), Some(&(3, 3)));
    // Found with the set's watermark, where random/11 has it, not where photo 3 was edited.
    assert_eq!(state(&f.core, new).status, AreaStatus::Found);
    let v = iou(&synth_truth("random", "11"), &f.core.area(new).unwrap());
    assert!(v >= 0.6, "iou {v:.3}");
    assert_eq!(state(&f.core, three).status, AreaStatus::Edited);
    assert!(f.core.area(three).unwrap() == own);
    assert!(f.core.removal_blockers().is_empty());
}

#[test]
fn reconfirm_clears_matches_keeps_edits() {
    let f = fixture(None);
    let ids = add_synth(&f.core, "random", &["00", "01", "04"]);
    confirm_truth(&f.core, "random", "00", ids[0]);
    f.core.match_all(&noop).unwrap();
    assert_eq!(state(&f.core, ids[1]).status, AreaStatus::Found);

    let (w, h) = f.core.edit_size(ids[1]).unwrap();
    let own = rect_area(w, h, 5, 5, 50, 50);
    f.core.put_draft(ids[1], w, h, own.clone()).unwrap();
    f.core.save_photo_area(ids[1]).unwrap();

    // A new set area from photo 2: earlier results go, the photo's own area stays.
    confirm_truth(&f.core, "random", "04", ids[2]);
    assert_eq!(state(&f.core, ids[0]).status, AreaStatus::Pending);
    assert!(!state(&f.core, ids[0]).included);
    let edited = state(&f.core, ids[1]);
    assert_eq!(edited.status, AreaStatus::Edited);
    assert!(edited.included && edited.reviewed);
    assert!(f.core.area(ids[1]).unwrap() == own);
    assert_eq!(state(&f.core, ids[2]).status, AreaStatus::Found);
    assert_eq!(f.core.removal_blockers(), vec![ids[0]]);

    let rec = Recorder::default();
    f.core.match_all(&|e| rec.push(e)).unwrap();
    assert_eq!(matching(&rec.events()).last(), Some(&(1, 1)));
    assert_eq!(state(&f.core, ids[0]).status, AreaStatus::Found);
    assert!(f.core.area(ids[1]).unwrap() == own);
}

/// Back from step 3 and "Looks right" again with the area unchanged: nothing in step 3 is
/// lost (matches, unticks, looks) and nothing is matched again.
#[test]
fn unchanged_reconfirm_keeps_step3() {
    let f = fixture(None);
    let ids = add_synth(&f.core, "random", &["00", "01", "04"]);
    confirm_truth(&f.core, "random", "00", ids[0]);
    f.core.match_all(&noop).unwrap();
    f.core.set_included(ids[1], false).unwrap();
    f.core.open_photo(ids[2]).unwrap();
    let before = f.core.photo_states();
    let template = f.core.confirmed_template().unwrap();

    // The editor sends the same area again (the app's own area of the photo, at edit size).
    let (w, h) = f.core.edit_size(ids[0]).unwrap();
    let same = f.core.area(ids[0]).unwrap();
    f.core.put_draft(ids[0], w, h, same.clone()).unwrap();
    f.core.confirm_set_area(ids[0]).unwrap();
    assert_eq!(f.core.photo_states(), before);
    assert!(Arc::ptr_eq(
        &template,
        &f.core.confirmed_template().unwrap()
    ));
    let rec = Recorder::default();
    f.core.match_all(&|e| rec.push(e)).unwrap();
    assert_eq!(matching(&rec.events()), [(0, 0)], "matched again");

    // Without a draft (the area as the app has it) it is the same.
    f.core.confirm_set_area(ids[0]).unwrap();
    assert_eq!(f.core.photo_states(), before);

    // A changed area is a new set watermark: step 3 starts over.
    let mut changed = same;
    changed[0] = 255;
    f.core.put_draft(ids[0], w, h, changed).unwrap();
    f.core.confirm_set_area(ids[0]).unwrap();
    assert_eq!(state(&f.core, ids[1]).status, AreaStatus::Pending);
}

/// Leaving the single-photo editor without saving drops its draft, so a later save never uses it.
#[test]
fn dropped_draft_is_gone() {
    let f = fixture(None);
    let ids = add_synth(&f.core, "random", &["00", "01"]);
    confirm_truth(&f.core, "random", "00", ids[0]);
    f.core.match_all(&noop).unwrap();
    let before = f.core.area(ids[1]).unwrap();
    let (w, h) = f.core.edit_size(ids[1]).unwrap();
    f.core
        .put_draft(ids[1], w, h, rect_area(w, h, 5, 5, 50, 50))
        .unwrap();
    f.core.drop_draft(ids[1]);
    // Saving without a draft keeps the area the photo had.
    f.core.save_photo_area(ids[1]).unwrap();
    assert!(f.core.area(ids[1]).unwrap() == before);
}

#[test]
fn empty_area_cannot_be_included() {
    let f = fixture(None);
    let ids = add_synth(&f.core, "random", &["00", "01"]);
    let neg = add_synth(&f.core, "negative", &["00"])[0];
    confirm_truth(&f.core, "random", "00", ids[0]);
    f.core.match_all(&noop).unwrap();

    assert!(matches!(
        f.core.set_included(neg, true),
        Err(AppError::NothingToRemove)
    ));
    assert!(!state(&f.core, neg).included);

    // Erasing the whole area of a found photo leaves it out.
    let one = ids[1];
    let (w, h) = f.core.edit_size(one).unwrap();
    f.core
        .put_draft(one, w, h, vec![0; (w * h) as usize])
        .unwrap();
    f.core.save_photo_area(one).unwrap();
    let s = state(&f.core, one);
    assert_eq!(s.status, AreaStatus::Edited);
    assert!(!s.included && !s.has_area, "{s:?}");
    assert!(matches!(
        f.core.set_included(one, true),
        Err(AppError::NothingToRemove)
    ));
    assert_eq!(f.core.to_remove(), vec![ids[0]]);

    // A photo with an area can be left out and taken back.
    f.core.set_included(ids[0], false).unwrap();
    assert!(f.core.to_remove().is_empty());
    f.core.set_included(ids[0], true).unwrap();
    assert_eq!(f.core.to_remove(), vec![ids[0]]);
    assert!(f.core.set_included(99, false).is_err());
}

#[test]
fn grid_image_is_jpeg() {
    let f = fixture(None);
    let ids = add_synth(&f.core, "random", &["00"]);
    let neg = add_synth(&f.core, "negative", &["00"])[0];
    confirm_truth(&f.core, "random", "00", ids[0]);
    f.core.match_all(&noop).unwrap();

    let path = synth("random").join("marked/00.png");
    let plain = thumbnail(&path, GRID_LONG_SIDE).unwrap();
    let bytes = f.core.grid_image(ids[0]).unwrap();
    assert!(bytes.starts_with(&[0xFF, 0xD8]));
    let grid = image::load_from_memory(&bytes).unwrap().to_rgb8();
    assert_eq!(grid.dimensions(), plain.dimensions());
    assert_eq!(grid.width().max(grid.height()), GRID_LONG_SIDE);

    // The found area is tinted red: inside its box the grid is redder than the photo.
    let gt = synth_truth("random", "00");
    let scale = GRID_LONG_SIDE as f64 / gt.width().max(gt.height()) as f64;
    let redness = |img: &RgbImage| {
        let (mut sum, mut n) = (0f64, 0f64);
        for (x, y, p) in gt.enumerate_pixels() {
            let (gx, gy) = ((x as f64 * scale) as u32, (y as f64 * scale) as u32);
            if p.0[0] != 0 && gx < img.width() && gy < img.height() {
                let q = img.get_pixel(gx, gy).0;
                sum += q[0] as f64 - q[1] as f64;
                n += 1.0;
            }
        }
        sum / n
    };
    let (r_grid, r_plain) = (redness(&grid), redness(&plain));
    assert!(
        r_grid > r_plain + 30.0,
        "grid {r_grid:.1}, photo {r_plain:.1}"
    );

    // The area is drawn on every call: after it is erased, the photo is shown as it is.
    let (w, h) = f.core.edit_size(ids[0]).unwrap();
    f.core
        .put_draft(ids[0], w, h, vec![0; (w * h) as usize])
        .unwrap();
    f.core.save_photo_area(ids[0]).unwrap();
    let erased = image::load_from_memory(&f.core.grid_image(ids[0]).unwrap())
        .unwrap()
        .to_rgb8();
    let r_erased = redness(&erased);
    assert!(
        r_erased < r_plain + 5.0,
        "erased {r_erased:.1}, photo {r_plain:.1}"
    );

    // A photo where nothing was found is shown as it is.
    let neg_grid = image::load_from_memory(&f.core.grid_image(neg).unwrap()).unwrap();
    assert_eq!(neg_grid.width().max(neg_grid.height()), GRID_LONG_SIDE);
    assert!(f.core.grid_image(99).is_err());
}

#[test]
fn cancel_match_keeps_results() {
    // More photos than matching threads, so photos are left when the cancel lands: at that
    // point each thread has at most the photo it is working on.
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()) / 2;
    let to_match = threads.max(1) + 4;
    let names = synth_names("random");
    let dir = tempfile::tempdir().unwrap();
    for k in 0..=to_match {
        let name = &names[k % names.len()];
        std::fs::copy(
            synth("random").join("marked").join(format!("{name}.png")),
            dir.path().join(format!("{k:03}.png")),
        )
        .unwrap();
    }
    let f = fixture(None);
    let added = f.core.add_paths(&[dir.path().to_path_buf()]).unwrap().added;
    assert_eq!(added.len(), to_match + 1);
    confirm_truth(&f.core, "random", &names[0], added[0].id);

    let core = &f.core;
    let err = core
        .match_all(&|e| {
            if matches!(e, ProgressEvent::Matching { done: 1, .. }) {
                core.cancel();
            }
        })
        .unwrap_err();
    assert_eq!(user_message(&err), "Stopped.");

    let pending = |c: &AppCore| {
        c.photo_states()
            .iter()
            .filter(|s| s.status == AreaStatus::Pending)
            .count() as u32
    };
    let left = pending(core);
    assert!(
        left >= 4 && left < to_match as u32,
        "{left} of {to_match} photos left"
    );

    // The results so far are kept; the next run matches only the rest.
    let rec = Recorder::default();
    core.match_all(&|e| rec.push(e)).unwrap();
    assert_eq!(matching(&rec.events()).last(), Some(&(left, left)));
    assert_eq!(pending(core), 0);
}

#[test]
fn confirm_replaces_own_area() {
    let f = fixture(None);
    let ids = add_synth(&f.core, "random", &["00", "01"]);
    confirm_truth(&f.core, "random", "00", ids[0]);
    f.core.match_all(&noop).unwrap();

    // Photo 1 gets an area of its own in step 3...
    let one = ids[1];
    let (w, h) = f.core.edit_size(one).unwrap();
    f.core
        .put_draft(one, w, h, rect_area(w, h, 5, 5, 50, 50))
        .unwrap();
    f.core.save_photo_area(one).unwrap();
    assert_eq!(state(&f.core, one).status, AreaStatus::Edited);

    // ...then step 2 confirms a different area on it: that one counts from now on.
    confirm_truth(&f.core, "random", "01", one);
    let binary: Vec<u8> = synth_truth("random", "01")
        .into_raw()
        .into_iter()
        .map(|v| if v != 0 { 255 } else { 0 })
        .collect();
    assert!(
        f.core.area(one).unwrap() == binary,
        "the area is not the confirmed one"
    );
    let s = state(&f.core, one);
    assert_eq!(s.status, AreaStatus::Found);
    assert!(s.included && s.has_area, "{s:?}");
}

#[test]
fn layers_reread_only_for_template_size() {
    let f = fixture(None);
    let reads = |core: &AppCore| {
        let rec = Recorder::default();
        core.match_all(&|e| rec.push(e)).unwrap();
        rec.events()
            .iter()
            .any(|e| matches!(e, ProgressEvent::Reading { .. }))
    };
    // random/00 and random/04 are 1536x1024, random/01 and random/02 are not. Adding photos
    // clears the groups, so the set area is confirmed again after each change.
    let ids = add_synth(&f.core, "random", &["00", "01"]);
    let confirm = || confirm_truth(&f.core, "random", "00", ids[0]);
    confirm();
    assert!(reads(&f.core), "the set has no layers yet");

    add_synth(&f.core, "negative", &["00"]);
    confirm();
    assert!(!reads(&f.core), "a photo of another size was added");
    add_synth(&f.core, "random", &["02"]);
    confirm();
    assert!(!reads(&f.core), "a photo of another size was added");

    add_synth(&f.core, "random", &["04"]);
    confirm();
    assert!(reads(&f.core), "a photo of the watermark's size was added");
    add_synth(&f.core, "negative", &["01"]);
    confirm();
    assert!(
        !reads(&f.core),
        "the layers were read with photo 04 in the set"
    );
}

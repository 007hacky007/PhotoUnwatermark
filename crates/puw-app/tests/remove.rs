//! Preview on one photo, the removal run (step 4), the before/after images and saved
//! watermarks.
//!
//! Removal writes next to the photos, so every test works on copies in a temp folder, never on
//! `target/synth`. Tests that need models read them from `PUW_MODELS_DIR` and return early (or
//! stop before the part that needs them) with a message when it is not set.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use image::{GrayImage, Rgb, RgbImage};
use puw_app::images::{edit_size, jpeg};
use puw_app::{
    AppCore, AppError, DataDirs, LoadPurpose, PREVIEW_LONG_SIDE, ProgressEvent, Settings,
    default_watermark_name, user_message,
};
use puw_core::{ModelKind, load};
use time::{Date, Month};

/// Model tests run one at a time (see tests/session.rs).
static MODEL_TESTS: Mutex<()> = Mutex::new(());

const OUT: &str = "Without watermark";

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

fn model_fixture() -> Option<Fixture> {
    match common::models_dir() {
        Some(m) => Some(fixture(Some(m))),
        None => {
            eprintln!("skipped: PUW_MODELS_DIR not set");
            None
        }
    }
}

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

fn fixed() -> PathBuf {
    common::synth_dir().join("fixed")
}

/// The first `n` marked photos of the synthetic `fixed` set (1536x1024 PNG), copied into
/// `dir/photos`. Returns the copies in name order.
fn copy_fixed(dir: &Path, n: usize) -> Vec<PathBuf> {
    let to = dir.join("photos");
    fs::create_dir_all(&to).unwrap();
    let mut names: Vec<PathBuf> = fs::read_dir(fixed().join("marked"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    names.sort();
    names
        .iter()
        .take(n)
        .map(|p| {
            let dst = to.join(p.file_name().unwrap());
            fs::copy(p, &dst).unwrap();
            dst
        })
        .collect()
}

fn add(core: &AppCore, paths: &[PathBuf]) -> Vec<u32> {
    let added = core.add_paths(paths).unwrap();
    assert_eq!(added.added.len(), paths.len(), "{:?}", added.skipped);
    added.added.iter().map(|p| p.id).collect()
}

fn truth(name: &str) -> GrayImage {
    image::open(fixed().join("mask").join(name))
        .unwrap()
        .to_luma8()
}

/// Confirms the fixed set's ground truth on photo `id` and finds it on the others.
fn confirm_and_match(core: &AppCore, id: u32) {
    let gt = truth("00.png");
    core.put_draft(id, gt.width(), gt.height(), gt.into_raw())
        .unwrap();
    core.confirm_set_area(id).unwrap();
    let sum = core.match_all(&|_| {}).unwrap();
    assert_eq!(sum.found, sum.total, "{sum:?}");
    assert!(core.removal_blockers().is_empty());
}

/// An edit-size area (row-major, 0/255) with the rectangle `x0..x1` x `y0..y1` painted.
fn rect_area(w: u32, h: u32, x0: u32, y0: u32, x1: u32, y1: u32) -> Vec<u8> {
    let mut v = vec![0u8; (w * h) as usize];
    for y in y0..y1 {
        v[(y * w + x0) as usize..(y * w + x1) as usize].fill(255);
    }
    v
}

/// Gives photo `id` an area of its own: the rectangle `x0..x1` x `y0..y1` at edit size.
fn paint(core: &AppCore, id: u32, rect: (u32, u32, u32, u32)) {
    let (w, h) = core.edit_size(id).unwrap();
    let (x0, y0, x1, y1) = rect;
    core.put_draft(id, w, h, rect_area(w, h, x0, y0, x1, y1))
        .unwrap();
    core.save_photo_area(id).unwrap();
}

/// A smooth photo with a bright block at `block` (upright pixels).
fn photo_with_block(w: u32, h: u32, block: (u32, u32, u32, u32)) -> RgbImage {
    let (x0, y0, x1, y1) = block;
    RgbImage::from_fn(w, h, |x, y| {
        if (x0..x1).contains(&x) && (y0..y1).contains(&y) {
            Rgb([250, 250, 250])
        } else {
            Rgb([
                (60 + x * 100 / w) as u8,
                (90 + y * 80 / h) as u8,
                (120 + (x + y) * 40 / (w + h)) as u8,
            ])
        }
    })
}

/// Files in `dir` (names), sorted; empty when `dir` does not exist.
fn names_in(dir: &Path) -> Vec<String> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<String> = rd
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

fn bytes_of(paths: &[PathBuf]) -> Vec<Vec<u8>> {
    paths.iter().map(|p| fs::read(p).unwrap()).collect()
}

fn decode(bytes: &[u8]) -> RgbImage {
    image::load_from_memory(bytes).unwrap().to_rgb8()
}

fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xFF, 0xD8])
}

fn loading(events: &[ProgressEvent]) -> Vec<LoadPurpose> {
    events
        .iter()
        .filter_map(|e| match e {
            ProgressEvent::LoadingModels { which, .. } => Some(*which),
            _ => None,
        })
        .collect()
}

fn removing(events: &[ProgressEvent]) -> Vec<(u32, u32, u32, f32)> {
    events
        .iter()
        .filter_map(|e| match e {
            ProgressEvent::Removing {
                done,
                total,
                photo,
                photo_fraction,
                ..
            } => Some((*done, *total, *photo, *photo_fraction)),
            _ => None,
        })
        .collect()
}

#[test]
fn default_name_format() {
    let day = |y, m, d| Date::from_calendar_date(y, m, d).unwrap();
    assert_eq!(
        default_watermark_name(day(2026, Month::November, 25)),
        "Watermark from 2026-11-25"
    );
    assert_eq!(
        default_watermark_name(day(2027, Month::January, 5)),
        "Watermark from 2027-01-05"
    );
}

#[test]
fn removes_into_output_folder() {
    let Some(f) = model_fixture() else { return };
    let tmp = tempfile::tempdir().unwrap();
    let paths = copy_fixed(tmp.path(), 2);
    let before = bytes_of(&paths);
    let ids = add(&f.core, &paths);
    confirm_and_match(&f.core, ids[0]);

    assert_eq!(
        f.core.check_output().unwrap(),
        puw_app::OutputCheck {
            ok: true,
            unwritable: vec![]
        }
    );
    let rec = Recorder::default();
    let start = Instant::now();
    let report = f.core.remove(&|e| rec.push(e)).unwrap();
    eprintln!("removing 2 photos took {:.2?}", start.elapsed());
    assert!(!report.cancelled);
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(
        report.done.iter().map(|d| d.id).collect::<Vec<_>>(),
        ids,
        "{report:?}"
    );

    // Results are in the output folder under the same names; the originals are untouched.
    let out = tmp.path().join("photos").join(OUT);
    assert_eq!(names_in(&out), ["00.png", "01.png"]);
    for d in &report.done {
        assert_eq!(
            Path::new(&d.output).parent().unwrap(),
            fs::canonicalize(&out).unwrap()
        );
    }
    assert_eq!(bytes_of(&paths), before, "an original changed");
    assert_eq!(
        names_in(&tmp.path().join("photos")),
        ["00.png", "01.png", OUT]
    );

    // The watermark area changed; pixels away from it did not (PNG keeps them exactly).
    for (i, p) in paths.iter().enumerate() {
        let (w, h) = f.core.edit_size(ids[i]).unwrap();
        let gt = GrayImage::from_raw(w, h, f.core.area(ids[i]).unwrap()).unwrap();
        let orig = load(p).unwrap().rgb;
        let res = load(&out.join(p.file_name().unwrap())).unwrap().rgb;
        assert_eq!(res.dimensions(), orig.dimensions());
        let (mut inside, mut n_inside, mut far_changed) = (0f64, 0f64, 0u32);
        for (x, y, t) in gt.enumerate_pixels() {
            let (a, b) = (orig.get_pixel(x, y), res.get_pixel(x, y));
            let d: i32 = (0..3).map(|c| (a.0[c] as i32 - b.0[c] as i32).abs()).sum();
            if t.0[0] != 0 {
                inside += d as f64;
                n_inside += 1.0;
            } else if d != 0 && !near(&gt, x, y, 6) {
                far_changed += 1;
            }
        }
        assert!(inside / n_inside > 1.0, "photo {i}: area unchanged");
        assert_eq!(
            far_changed, 0,
            "photo {i}: pixels away from the area changed"
        );
    }

    // Progress: one photo at a time, ending with both done; the time left appears after the first.
    let r = removing(&rec.events());
    assert_eq!(r.first().map(|e| (e.0, e.1)), Some((0, 2)));
    assert_eq!(r.last().map(|e| (e.0, e.1, e.3)), Some((2, 2, 1.0)));
    assert!(r.windows(2).all(|w| w[0].0 <= w[1].0));
    let left: Vec<Option<u32>> = rec
        .events()
        .iter()
        .filter_map(|e| match e {
            ProgressEvent::Removing {
                done, seconds_left, ..
            } if *done >= 1 => Some(*seconds_left),
            _ => None,
        })
        .collect();
    assert!(left.iter().all(Option::is_some), "{left:?}");

    // Before/after images at edit size.
    for &id in &ids {
        let (ew, eh) = f.core.edit_size(id).unwrap();
        let before = f.core.original_image(id).unwrap();
        let after = f.core.result_image(id).unwrap();
        assert!(is_jpeg(&before) && is_jpeg(&after));
        assert_eq!(decode(&before).dimensions(), (ew, eh));
        assert_eq!(decode(&after).dimensions(), (ew, eh));
    }
    assert!(f.core.result_image(999).is_err());

    // A second run with nothing changed does nothing again: the results are listed as they are.
    let again = f.core.remove(&|_| {}).unwrap();
    assert_eq!(again.done, report.done);
    assert_eq!(names_in(&out), ["00.png", "01.png"]);

    // With a changed area the photo is done again, and never replaces the first result.
    paint(&f.core, ids[0], (10, 10, 60, 40));
    let first = fs::read(out.join("00.png")).unwrap();
    let start = Instant::now();
    let again = f.core.remove(&|_| {}).unwrap();
    eprintln!(
        "second run (remover loaded): {:.2?} for 1 photo",
        start.elapsed()
    );
    assert_eq!(again.done.len(), 2);
    assert_eq!(names_in(&out), ["00 (2).png", "00.png", "01.png"]);
    assert_eq!(fs::read(out.join("00.png")).unwrap(), first);
    assert_eq!(bytes_of(&paths), before);
}

/// True when a pixel of `m` within `r` of (x, y) is set.
fn near(m: &GrayImage, x: u32, y: u32, r: u32) -> bool {
    let (w, h) = m.dimensions();
    (y.saturating_sub(r)..(y + r + 1).min(h))
        .any(|yy| (x.saturating_sub(r)..(x + r + 1).min(w)).any(|xx| m.get_pixel(xx, yy).0[0] != 0))
}

/// EXIF with Orientation only (see tests/photos.rs).
fn exif_orientation(orientation: u16) -> Vec<u8> {
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

fn write_jpeg(path: &Path, img: &RgbImage, exif: Option<Vec<u8>>) {
    use img_parts::{Bytes, ImageEXIF};
    let mut j = img_parts::jpeg::Jpeg::from_bytes(Bytes::from(jpeg(img, 95))).unwrap();
    j.set_exif(exif.map(Bytes::from));
    let mut out = Vec::new();
    j.encoder().write_to(&mut out).unwrap();
    fs::write(path, out).unwrap();
}

/// Mean absolute difference per channel over the `s` x `s` block at (bx, by).
fn block_diff(a: &RgbImage, b: &RgbImage, bx: u32, by: u32, s: u32) -> f64 {
    let mut sum = 0f64;
    for y in by..by + s {
        for x in bx..bx + s {
            let (p, q) = (a.get_pixel(x, y), b.get_pixel(x, y));
            for c in 0..3 {
                sum += (p.0[c] as f64 - q.0[c] as f64).abs();
            }
        }
    }
    sum / (s * s * 3) as f64
}

#[test]
fn area_lands_on_rotated_photo() {
    let Some(f) = model_fixture() else { return };
    let tmp = tempfile::tempdir().unwrap();
    // Upright 768x1024 with a bright block at the top-left, stored turned a quarter to the left
    // with orientation 6 (shown turned a quarter to the right).
    let block = (40, 40, 200, 120);
    let upright = photo_with_block(768, 1024, block);
    let stored = image::imageops::rotate270(&upright);
    let p = tmp.path().join("portrait.jpg");
    write_jpeg(&p, &stored, Some(exif_orientation(6)));
    let original = fs::read(&p).unwrap();

    let ids = add(&f.core, std::slice::from_ref(&p));
    assert_eq!(f.core.edit_size(ids[0]).unwrap(), (768, 1024));
    paint(&f.core, ids[0], (30, 30, 210, 130));
    let report = f.core.remove(&|_| {}).unwrap();
    assert_eq!(report.done.len(), 1, "{report:?}");
    assert_eq!(fs::read(&p).unwrap(), original);

    let before = load(&p).unwrap().rgb;
    let after = load(Path::new(&report.done[0].output)).unwrap().rgb;
    assert_eq!(after.dimensions(), (768, 1024));
    // 32 px blocks: the ones over the painted area changed a lot, every other one hardly.
    let s = 32;
    let (mut most_inside, mut most_outside) = (0f64, 0f64);
    for by in (0..1024).step_by(s as usize) {
        for bx in (0..768).step_by(s as usize) {
            let d = block_diff(&before, &after, bx, by, s);
            let touches = bx < 210 + 8 && by < 130 + 8;
            if touches {
                most_inside = most_inside.max(d);
            } else {
                most_outside = most_outside.max(d);
            }
        }
    }
    eprintln!("block change: inside {most_inside:.1}, outside {most_outside:.1}");
    assert!(most_inside > 40.0, "the bright block is still there");
    assert!(most_outside < 4.0, "pixels away from the area changed");
}

/// Makes a folder read-only for as long as it lives (Unix permissions).
#[cfg(unix)]
struct ReadOnly(PathBuf);

#[cfg(unix)]
impl ReadOnly {
    fn new(dir: &Path) -> ReadOnly {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o555)).unwrap();
        ReadOnly(dir.to_path_buf())
    }
}

#[cfg(unix)]
impl Drop for ReadOnly {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
    }
}

#[cfg(unix)]
#[test]
fn unwritable_source_asks_for_folder() {
    let models = common::models_dir();
    let f = fixture(models.clone());
    let tmp = tempfile::tempdir().unwrap();
    let card = tmp.path().join("card");
    fs::create_dir(&card).unwrap();
    let paths: Vec<PathBuf> = (0..2)
        .map(|i| {
            let p = card.join(format!("{i}.jpg"));
            write_jpeg(&p, &photo_with_block(300, 200, (20, 20, 80, 60)), None);
            p
        })
        .collect();
    let before = bytes_of(&paths);
    let _guard = ReadOnly::new(&card);
    let ids = add(&f.core, &paths);
    for &id in &ids {
        paint(&f.core, id, (10, 10, 90, 70));
    }

    let check = f.core.check_output().unwrap();
    assert!(!check.ok);
    assert_eq!(
        check.unwritable,
        [fs::canonicalize(&card)
            .unwrap()
            .to_string_lossy()
            .into_owned()]
    );
    let rec = Recorder::default();
    assert!(matches!(
        f.core.remove(&|e| rec.push(e)),
        Err(AppError::NoWritableOutput)
    ));
    assert!(rec.events().is_empty(), "{:?}", rec.events());
    assert_eq!(names_in(&card), ["0.jpg", "1.jpg"]);

    let chosen = tmp.path().join("chosen");
    fs::create_dir(&chosen).unwrap();
    f.core.set_output_override(Some(chosen.clone()));
    assert!(f.core.check_output().unwrap().ok);
    if models.is_none() {
        eprintln!("skipped the run: PUW_MODELS_DIR not set");
        return;
    }
    let report = f.core.remove(&|_| {}).unwrap();
    assert_eq!(report.done.len(), 2, "{report:?}");
    assert_eq!(names_in(&chosen), ["0.jpg", "1.jpg"]);
    assert_eq!(names_in(&card), ["0.jpg", "1.jpg"]);
    assert_eq!(bytes_of(&paths), before);

    // Without the chosen folder the photos' own folder is asked for again.
    f.core.set_output_override(None);
    assert!(!f.core.check_output().unwrap().ok);
}

#[test]
fn vanished_photo_is_a_failure() {
    let Some(f) = model_fixture() else { return };
    let tmp = tempfile::tempdir().unwrap();
    let paths = copy_fixed(tmp.path(), 3);
    let ids = add(&f.core, &paths);
    confirm_and_match(&f.core, ids[1]);
    fs::remove_file(&paths[0]).unwrap();

    let rec = Recorder::default();
    let report = f.core.remove(&|e| rec.push(e)).unwrap();
    assert!(!report.cancelled);
    assert_eq!(
        report.done.iter().map(|d| d.id).collect::<Vec<_>>(),
        [ids[1], ids[2]]
    );
    assert_eq!(report.failed.len(), 1, "{report:?}");
    let failed = &report.failed[0];
    assert_eq!((failed.id, failed.name.as_str()), (ids[0], "00.png"));
    assert!(
        failed.reason.contains("could not be found"),
        "{}",
        failed.reason
    );
    assert_eq!(
        names_in(&tmp.path().join("photos").join(OUT)),
        ["01.png", "02.png"]
    );

    // The missing photo took no time: it does not count toward the time left, which stays
    // unknown until a photo was really worked on.
    let left = |photo: u32| -> Vec<Option<u32>> {
        rec.events()
            .iter()
            .filter_map(|e| match e {
                ProgressEvent::Removing {
                    photo: p,
                    seconds_left,
                    ..
                } if *p == photo => Some(*seconds_left),
                _ => None,
            })
            .collect()
    };
    let second = left(ids[1]);
    assert!(
        second[..second.len() - 1].iter().all(Option::is_none),
        "{second:?}"
    );
    assert!(left(ids[2]).iter().all(Option::is_some));
}

#[test]
fn damaged_photos_are_failures() {
    let Some(f) = model_fixture() else { return };
    let tmp = tempfile::tempdir().unwrap();
    // Each header is fine (the photos are added); the pixel data is not.
    let img = photo_with_block(300, 200, (20, 20, 80, 60));
    let good = tmp.path().join("a.jpg");
    write_jpeg(&good, &img, None);
    // A JPEG cut short, as an interrupted copy leaves it.
    let cut = tmp.path().join("b.jpg");
    let bytes = fs::read(&good).unwrap();
    fs::write(&cut, &bytes[..bytes.len() * 2 / 3]).unwrap();
    // A PNG with scrambled pixel data.
    let png = tmp.path().join("c.png");
    img.save(&png).unwrap();
    let mut bytes = fs::read(&png).unwrap();
    let idat = bytes.windows(4).position(|w| w == b"IDAT").unwrap();
    for b in &mut bytes[idat + 8..idat + 64] {
        *b ^= 0x5A;
    }
    fs::write(&png, &bytes).unwrap();

    let ids = add(&f.core, &[cut, good, png]);
    for &id in &ids {
        paint(&f.core, id, (10, 10, 90, 70));
    }
    let report = f.core.remove(&|_| {}).unwrap();
    assert_eq!(
        report.done.iter().map(|d| d.id).collect::<Vec<_>>(),
        [ids[1]],
        "{report:?}"
    );
    let failed: Vec<(&str, &str)> = report
        .failed
        .iter()
        .map(|x| (x.name.as_str(), x.reason.as_str()))
        .collect();
    assert_eq!(
        failed,
        [
            ("b.jpg", "The photo file is incomplete or damaged."),
            (
                "c.png",
                "The photo could not be opened. The file may be damaged."
            ),
        ]
    );
}

#[test]
fn cancel_discards_current_keeps_completed() {
    let Some(f) = model_fixture() else { return };
    let tmp = tempfile::tempdir().unwrap();
    let paths = copy_fixed(tmp.path(), 3);
    let ids = add(&f.core, &paths);
    confirm_and_match(&f.core, ids[0]);

    // Cancel inside the second photo's work: its first event says it starts, the second one
    // comes from the remover.
    let seen = AtomicU32::new(0);
    let core = &f.core;
    let report = core
        .remove(&|e| {
            if let ProgressEvent::Removing { photo, .. } = e
                && photo == ids[1]
                && seen.fetch_add(1, Ordering::SeqCst) == 1
            {
                core.cancel();
            }
        })
        .unwrap();
    assert!(report.cancelled);
    assert!(report.failed.is_empty());
    assert_eq!(
        report.done.iter().map(|d| d.id).collect::<Vec<_>>(),
        [ids[0]]
    );
    // Only the finished photo is in the output folder, and no half-written file is left.
    assert_eq!(names_in(&tmp.path().join("photos").join(OUT)), ["00.png"]);

    // The next run is not cancelled, and does only the unfinished photos: the finished one is
    // listed with its result, not saved twice.
    let rec = Recorder::default();
    let again = f.core.remove(&|e| rec.push(e)).unwrap();
    assert!(!again.cancelled);
    assert_eq!(again.done.len(), 3);
    assert!(
        removing(&rec.events())
            .iter()
            .all(|(_, _, p, _)| *p != ids[0])
    );
    assert_eq!(
        names_in(&tmp.path().join("photos").join(OUT)),
        ["00.png", "01.png", "02.png"]
    );
    // With everything finished, a new run saves nothing new.
    let third = f.core.remove(&|_| {}).unwrap();
    assert_eq!(third.done, again.done);
    assert_eq!(names_in(&tmp.path().join("photos").join(OUT)).len(), 3);
}

#[test]
fn blockers_refuse_removal() {
    let f = fixture(None);
    let tmp = tempfile::tempdir().unwrap();
    let paths = copy_fixed(tmp.path(), 2);
    let ids = add(&f.core, &paths);
    let gt = truth("00.png");
    f.core
        .put_draft(ids[0], gt.width(), gt.height(), gt.into_raw())
        .unwrap();
    f.core.confirm_set_area(ids[0]).unwrap();
    // Photo 1 was not looked at yet.
    assert_eq!(f.core.removal_blockers(), [ids[1]]);
    let rec = Recorder::default();
    assert!(matches!(
        f.core.remove(&|e| rec.push(e)),
        Err(AppError::NotReady)
    ));
    assert!(rec.events().is_empty());
    assert!(names_in(&tmp.path().join("photos").join(OUT)).is_empty());

    // Nothing ticked: nothing to remove.
    f.core.set_included(ids[0], false).unwrap();
    f.core.remove_photo(ids[1]).unwrap();
    assert!(f.core.removal_blockers().is_empty());
    assert!(matches!(
        f.core.remove(&|_| {}),
        Err(AppError::NothingToRemove)
    ));
}

#[test]
fn stale_partials_removed() {
    // No models: the run stops when the remover cannot load, after the clean-up.
    let f = fixture(None);
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("photos");
    fs::create_dir(&dir).unwrap();
    let p = dir.join("a.jpg");
    write_jpeg(&p, &photo_with_block(300, 200, (20, 20, 80, 60)), None);
    let out = dir.join(OUT);
    fs::create_dir(&out).unwrap();
    let me = std::process::id();
    let other = me.wrapping_add(1);
    let stale = [
        format!(".a.{other}.0.partial"),
        format!(".x.y.{other}.12.partial"),
    ];
    let kept = [
        format!(".a.{me}.3.partial"),
        ".notes.partial".to_string(),
        "a.jpg".to_string(),
        format!("a.{other}.0.partial"),
    ];
    for n in stale.iter().chain(&kept) {
        fs::write(out.join(n), b"x").unwrap();
    }
    let ids = add(&f.core, std::slice::from_ref(&p));
    paint(&f.core, ids[0], (10, 10, 90, 70));
    assert!(f.core.remove(&|_| {}).is_err());
    let mut want: Vec<String> = kept.to_vec();
    want.sort();
    assert_eq!(names_in(&out), want);
}

#[test]
fn preview_loads_remover_once() {
    let Some(f) = model_fixture() else { return };
    let tmp = tempfile::tempdir().unwrap();
    let paths = copy_fixed(tmp.path(), 1);
    let id = add(&f.core, &paths)[0];
    assert!(f.core.preview_image(id).is_err());
    assert!(matches!(
        f.core.preview(id, &|_| {}),
        Err(AppError::NothingToRemove)
    ));
    let gt = truth("00.png");
    f.core
        .put_draft(id, gt.width(), gt.height(), gt.as_raw().clone())
        .unwrap();

    let first = Recorder::default();
    let start = Instant::now();
    f.core.preview(id, &|e| first.push(e)).unwrap();
    eprintln!("first preview (with loading) took {:.2?}", start.elapsed());
    let second = Recorder::default();
    let start = Instant::now();
    f.core.preview(id, &|e| second.push(e)).unwrap();
    eprintln!("second preview took {:.2?}", start.elapsed());

    assert_eq!(
        loading(&first.events()).first(),
        Some(&LoadPurpose::Remover)
    );
    assert!(loading(&second.events()).is_empty());
    for rec in [&first, &second] {
        let fractions: Vec<f32> = rec
            .events()
            .iter()
            .filter_map(|e| match e {
                ProgressEvent::Preview { fraction } => Some(*fraction),
                _ => None,
            })
            .collect();
        assert_eq!(fractions.last(), Some(&1.0), "{fractions:?}");
    }

    // The preview is a JPEG at edit size that differs from the photo only around the area.
    let bytes = f.core.preview_image(id).unwrap();
    assert!(is_jpeg(&bytes));
    let shown = decode(&bytes);
    assert_eq!(shown.dimensions(), edit_size(1536, 1024));
    assert_eq!(PREVIEW_LONG_SIDE, 1024);
    let orig = decode(&f.core.edit_image(id).unwrap());
    let (x, y, w, h) = (680, 768, 393, 78);
    assert!(block_diff(&orig, &shown, x + w / 2 - 16, y + h / 2 - 16, 32) > 0.5);
    assert!(block_diff(&orig, &shown, 100, 100, 32) < 3.0);
    assert!(fs::read_dir(tmp.path().join("photos")).unwrap().count() == 1);
}

#[test]
fn fallback_notice_once() {
    let Some(f) = model_fixture() else { return };
    let tmp = tempfile::tempdir().unwrap();
    let paths = copy_fixed(tmp.path(), 3);
    let ids = add(&f.core, &paths);
    confirm_and_match(&f.core, ids[0]);
    // The remover's next run on the graphics card fails; it continues on the processor.
    f.core
        .engine()
        .unwrap()
        .simulate_gpu_failure(ModelKind::Lama)
        .unwrap();
    let rec = Recorder::default();
    let report = f.core.remove(&|e| rec.push(e)).unwrap();
    assert_eq!(report.done.len(), 3, "{report:?}");
    let notices: Vec<ProgressEvent> = rec
        .events()
        .into_iter()
        .filter(|e| matches!(e, ProgressEvent::FellBackToProcessor { .. }))
        .collect();
    assert_eq!(notices.len(), 1, "{notices:?}");

    // A later run that stays on the processor says nothing.
    let rec = Recorder::default();
    f.core.remove(&|e| rec.push(e)).unwrap();
    assert!(
        !rec.events()
            .iter()
            .any(|e| matches!(e, ProgressEvent::FellBackToProcessor { .. }))
    );
}

#[test]
fn save_watermark_roundtrip() {
    let f = fixture(None);
    let tmp = tempfile::tempdir().unwrap();
    let paths = copy_fixed(tmp.path(), 1);
    let id = add(&f.core, &paths)[0];
    assert!(matches!(
        f.core.save_watermark("x"),
        Err(AppError::NothingToRemove)
    ));
    let gt = truth("00.png");
    f.core
        .put_draft(id, gt.width(), gt.height(), gt.into_raw())
        .unwrap();
    f.core.confirm_set_area(id).unwrap();

    let name = default_watermark_name(Date::from_calendar_date(2026, Month::November, 25).unwrap());
    let saved = f.core.save_watermark(&name).unwrap();
    let list = f.core.list_saved().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(
        (list[0].id.as_str(), list[0].name.as_str()),
        (saved.as_str(), "Watermark from 2026-11-25")
    );
    assert!(!list[0].created.is_empty());

    // The thumbnail: the watermark's surroundings, tinted, at most 256 px long, as PNG.
    let png = f.core.saved_thumbnail(&saved).unwrap();
    assert!(png.starts_with(b"\x89PNG"));
    let thumb = image::load_from_memory(&png).unwrap().to_rgb8();
    let (tw, th) = thumb.dimensions();
    assert!(tw.max(th) <= 256, "{tw}x{th}");
    // 393x78 plus 10 % on each side, scaled to 256 wide.
    assert_eq!(tw, 256);
    assert!(
        (th as i32 - (256.0 * 94.0 / 471.0) as i32).abs() <= 2,
        "{tw}x{th}"
    );
    let reddish = thumb
        .pixels()
        .filter(|p| p.0[0] as i32 - p.0[1] as i32 > 40)
        .count();
    assert!(reddish > 100, "{reddish} tinted pixels");

    // The library file holds what was learned from the photo.
    assert_eq!(
        puw_core::Library::open(&f.dirs.watermarks)
            .unwrap()
            .list()
            .unwrap()[0]
            .template,
        *f.core.confirmed_template().unwrap()
    );

    f.core.rename_saved(&saved, "  Studio  ").unwrap();
    assert_eq!(f.core.list_saved().unwrap()[0].name, "Studio");
    let e = f.core.rename_saved(&saved, "   ").unwrap_err();
    assert!(user_message(&e).contains("name"), "{}", user_message(&e));
    assert!(f.core.save_watermark(" ").is_err());
    assert_eq!(f.core.list_saved().unwrap().len(), 1);

    f.core.delete_saved(&saved).unwrap();
    assert!(f.core.list_saved().unwrap().is_empty());
    assert!(f.core.delete_saved(&saved).is_err());
    assert!(f.core.saved_thumbnail(&saved).is_err());
}

/// A stray brush dot far from the watermark does not stretch the saved picture over the photo.
#[test]
fn saved_picture_leaves_out_a_stray_dot() {
    let f = fixture(None);
    let tmp = tempfile::tempdir().unwrap();
    let paths = copy_fixed(tmp.path(), 1);
    let id = add(&f.core, &paths)[0];
    let mut gt = truth("00.png");
    for y in 30..54 {
        for x in 30..54 {
            gt.put_pixel(x, y, image::Luma([255]));
        }
    }
    f.core
        .put_draft(id, gt.width(), gt.height(), gt.into_raw())
        .unwrap();
    f.core.confirm_set_area(id).unwrap();
    let saved = f.core.save_watermark("Dot").unwrap();
    let png = f.core.saved_thumbnail(&saved).unwrap();
    let (tw, th) = image::load_from_memory(&png)
        .unwrap()
        .to_rgb8()
        .dimensions();
    // The same picture as without the dot: 393x78 plus 10 % on each side.
    assert_eq!(tw, 256);
    assert!(
        (th as i32 - (256.0 * 94.0 / 471.0) as i32).abs() <= 2,
        "{tw}x{th}"
    );
}

/// One 300x200 photo with an area, ready for removal.
fn one_small_photo(core: &AppCore, dir: &Path) -> u32 {
    let p = dir.join("s.jpg");
    write_jpeg(&p, &photo_with_block(300, 200, (20, 20, 80, 60)), None);
    let id = add(core, &[p])[0];
    paint(core, id, (10, 10, 90, 70));
    id
}

fn notices(events: &[ProgressEvent]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, ProgressEvent::FellBackToProcessor { .. }))
        .count()
}

#[test]
fn fallback_in_preview_is_announced_once() {
    let Some(f) = model_fixture() else { return };
    let tmp = tempfile::tempdir().unwrap();
    let id = one_small_photo(&f.core, tmp.path());
    f.core
        .engine()
        .unwrap()
        .simulate_gpu_failure(ModelKind::Lama)
        .unwrap();
    let rec = Recorder::default();
    f.core.preview(id, &|e| rec.push(e)).unwrap();
    assert_eq!(notices(&rec.events()), 1, "{:?}", rec.events());
    // Already said: the run after it does not repeat it.
    let rec = Recorder::default();
    assert_eq!(f.core.remove(&|e| rec.push(e)).unwrap().done.len(), 1);
    assert_eq!(notices(&rec.events()), 0);
}

#[test]
fn fallback_outside_a_run_is_not_missed() {
    let Some(f) = model_fixture() else { return };
    let tmp = tempfile::tempdir().unwrap();
    let id = one_small_photo(&f.core, tmp.path());
    // The remover falls back while the app is not watching (no preview, no run).
    let engine = f.core.engine().unwrap();
    engine.simulate_gpu_failure(ModelKind::Lama).unwrap();
    let img = photo_with_block(64, 64, (0, 0, 8, 8));
    let mut area = GrayImage::new(64, 64);
    area.put_pixel(3, 3, image::Luma([255]));
    puw_core::inpaint::inpaint(&engine, &img, &area, None, puw_core::Progress::none()).unwrap();
    // The next run says it, once.
    let rec = Recorder::default();
    assert_eq!(f.core.remove(&|e| rec.push(e)).unwrap().done.len(), 1);
    assert_eq!(notices(&rec.events()), 1);
    let rec = Recorder::default();
    f.core.preview(id, &|e| rec.push(e)).unwrap();
    assert_eq!(notices(&rec.events()), 0);
}

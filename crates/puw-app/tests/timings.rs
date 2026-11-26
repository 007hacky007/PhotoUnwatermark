//! Dev-only timing harness: `cargo test -p puw-app --release --test timings -- --ignored
//! --nocapture`. It drives `AppCore` the way the UI does and prints aggregate numbers only (no
//! file names). Needs `PUW_MODELS_DIR`; the real photos come from `PUW_REAL_DIR/set-a`. Its
//! outputs and the Core ML cache go under `<workspace>/local/timing`, which is gitignored.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use puw_app::images::jpeg;
use puw_app::{AppCore, DataDirs, Detail, LoadPurpose, ProgressEvent, Settings};
use puw_core::{Library, ModelKind, SavedWatermark, build_template};

fn secs(t: Instant) -> f64 {
    t.elapsed().as_secs_f64()
}

fn core(models: &Path, cache: &Path, tmp: &Path, detail: Detail) -> AppCore {
    fs::create_dir_all(tmp).unwrap();
    let mut dirs = DataDirs::at(tmp);
    dirs.models = models.to_path_buf();
    dirs.cache = cache.to_path_buf();
    Settings {
        prefer_gpu: cfg!(target_os = "macos"),
        detail,
        ..Settings::default()
    }
    .save(&dirs.settings)
    .unwrap();
    AppCore::new(dirs).unwrap()
}

fn photos_in(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_file())
        .collect();
    v.sort();
    v
}

fn add(core: &AppCore, paths: &[PathBuf]) -> Vec<u32> {
    core.add_paths(paths)
        .unwrap()
        .added
        .iter()
        .map(|p| p.id)
        .collect()
}

fn load_times(core: &AppCore) -> (f64, f64) {
    let t = Instant::now();
    core.ensure_models(
        &[ModelKind::Segmenter, ModelKind::Face],
        LoadPurpose::Finder,
        &|_| {},
    )
    .unwrap();
    let finder = secs(t);
    let t = Instant::now();
    core.ensure_models(&[ModelKind::Lama], LoadPurpose::Remover, &|_| {})
        .unwrap();
    (finder, secs(t))
}

fn truth_area(core: &AppCore, id: u32, mask: &Path) {
    let gt = image::open(mask).unwrap().to_luma8();
    core.put_draft(id, gt.width(), gt.height(), gt.into_raw())
        .unwrap();
    core.confirm_set_area(id).unwrap();
}

/// Unticks every ticked unsure photo nobody opened, as the UI asks before the next group.
fn settle(core: &AppCore) {
    for st in core.photo_states() {
        if st.status == puw_app::AreaStatus::Unsure && st.included && !st.reviewed {
            core.set_included(st.id, false).unwrap();
        }
    }
}

#[test]
#[ignore = "timing harness, needs models and local photos"]
fn app_timings() {
    let Some(models) = common::models_dir() else {
        eprintln!("skipped: PUW_MODELS_DIR not set");
        return;
    };
    let work = common::synth_dir()
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .join("local")
        .join("timing");
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).unwrap();
    let cache = work.join("coreml-cache");
    let synth = common::synth_dir();
    let mut lines: Vec<String> = Vec::new();
    let mut say = |s: String| {
        eprintln!("{s}");
        lines.push(s);
    };

    // Model load, cold (empty Core ML cache) then warm (new engine, same cache).
    for label in ["cold", "warm"] {
        let c = core(
            &models,
            &cache,
            &work.join(format!("data-{label}")),
            Detail::Best,
        );
        let (f, r) = load_times(&c);
        say(format!("load {label}: finder {f:.1} s, remover {r:.1} s"));
    }

    // Synthetic fixed: detect, then the area from ground truth.
    let c = core(&models, &cache, &work.join("data-fixed"), Detail::Best);
    let fixed = photos_in(&synth.join("fixed").join("marked"));
    let ids = add(&c, &fixed);
    load_times(&c);
    let t = Instant::now();
    let out = c.detect(&|_| {}).unwrap();
    say(format!(
        "detect synthetic fixed ({} photos, models warm in engine): {:.1} s (sample {})",
        ids.len(),
        secs(t),
        out.sample.len()
    ));

    // Synthetic random: match_all.
    let c = core(&models, &cache, &work.join("data-random"), Detail::Best);
    let random = photos_in(&synth.join("random").join("marked"));
    let ids = add(&c, &random);
    truth_area(
        &c,
        ids[0],
        &synth.join("random").join("mask").join("00.png"),
    );
    let t = Instant::now();
    let sum = c.match_all(&|_| {}).unwrap();
    say(format!(
        "match_all synthetic random ({} photos): {:.1} s ({} found, {} unsure, {} not found)",
        sum.total,
        secs(t),
        sum.found,
        sum.unsure,
        sum.not_found
    ));

    // Synthetic mixed set (Plan 2b): group 1 is done, then the second group by hand.
    let mdir = work.join("mixed");
    let c = core(&models, &cache, &work.join("data-mixed"), Detail::Best);
    let m = common::mixed_set(&c, &mdir);
    load_times(&c);
    truth_area(
        &c,
        m.fixed[0],
        &synth.join("fixed").join("mask").join("00.png"),
    );
    c.match_all(&|_| {}).unwrap();
    settle(&c);
    let t = Instant::now();
    let out = c.detect(&|_| {}).unwrap();
    say(format!(
        "detect leftovers, synthetic mixed set ({} photos left, 3 looked at): {:.1} s (sample {})",
        c.groups().leftovers,
        secs(t),
        out.sample.len()
    ));
    let reference = m.logos[0];
    let truth = &m.logo_truths[0];
    c.put_draft(
        reference,
        truth.width(),
        truth.height(),
        truth.as_raw().clone(),
    )
    .unwrap();
    let t = Instant::now();
    c.place_on(reference, m.logos[1]).unwrap();
    say(format!(
        "place_on, synthetic strip photo, first call: {:.2} s",
        secs(t)
    ));
    let t = Instant::now();
    for id in &m.logos[2..] {
        c.place_on(reference, *id).unwrap();
    }
    say(format!(
        "place_on, next 2 strip photos (drawing already learned): {:.2} s",
        secs(t)
    ));
    c.confirm_new_group(reference).unwrap();
    let t = Instant::now();
    let sum = c.match_all(&|_| {}).unwrap();
    say(format!(
        "match_all group 2, synthetic mixed set ({} photos): {:.1} s ({} found, {} unsure, {} not found)",
        sum.total,
        secs(t),
        sum.found,
        sum.unsure,
        sum.not_found
    ));

    // The same logo as a saved watermark: use_saved, then match_all.
    let c = core(&models, &cache, &work.join("data-saved"), Detail::Best);
    let m = common::mixed_set(&c, &mdir);
    load_times(&c);
    truth_area(
        &c,
        m.fixed[0],
        &synth.join("fixed").join("mask").join("00.png"),
    );
    c.match_all(&|_| {}).unwrap();
    settle(&c);
    let logo = image::open(mdir.join("marked").join("logo-00.png"))
        .unwrap()
        .to_rgb8();
    let w = SavedWatermark::new(
        "Ring",
        build_template(&logo, &m.logo_truths[0]),
        jpeg(&image::RgbImage::new(4, 4), 80),
    );
    let lib = DataDirs::at(&work.join("data-saved")).watermarks;
    Library::open(&lib).unwrap().save(&w).unwrap();
    let t = Instant::now();
    c.use_saved(&w.id).unwrap();
    let used = secs(t);
    let t = Instant::now();
    let sum = c.match_all(&|_| {}).unwrap();
    say(format!(
        "use_saved {used:.2} s, then match_all ({} photos): {:.1} s ({} found, {} unsure, {} not found)",
        sum.total,
        secs(t),
        sum.found,
        sum.unsure,
        sum.not_found
    ));

    // Real set-a.
    let Some(real) = std::env::var_os("PUW_REAL_DIR").map(PathBuf::from) else {
        eprintln!("PUW_REAL_DIR not set, real set skipped");
        fs::write(work.join("timings.txt"), lines.join("\n")).unwrap();
        return;
    };
    let a = photos_in(&real.join("set-a"));
    let c = core(&models, &cache, &work.join("data-real"), Detail::Best);
    let ids = add(&c, &a);
    let info = c.photos();
    let mp: Vec<f64> = info
        .iter()
        .map(|p| p.width as f64 * p.height as f64 / 1e6)
        .collect();
    say(format!(
        "real set-a: {} photos, {:.1}-{:.1} MP",
        ids.len(),
        mp.iter().cloned().fold(f64::MAX, f64::min),
        mp.iter().cloned().fold(0.0, f64::max)
    ));
    let t = Instant::now();
    let out = c.detect(&|_| {}).unwrap();
    say(format!(
        "detect real set-a (finder load included, warm cache): {:.1} s (sample {})",
        secs(t),
        out.sample.len()
    ));
    c.confirm_set_area(out.representative).unwrap();
    let t = Instant::now();
    let sum = c.match_all(&|_| {}).unwrap();
    say(format!(
        "match_all real set-a ({} photos): {:.1} s ({} found, {} unsure, {} not found)",
        sum.total,
        secs(t),
        sum.found,
        sum.unsure,
        sum.not_found
    ));

    // Real set-a and set-b as one set: two groups (group 2 = the set-b photo).
    let b = photos_in(&real.join("set-b"));
    let c2 = core(&models, &cache, &work.join("data-real2"), Detail::Best);
    let mut all = a.clone();
    all.extend(b.iter().cloned());
    let ids2 = add(&c2, &all);
    let out = c2.detect(&|_| {}).unwrap();
    c2.confirm_set_area(out.representative).unwrap();
    c2.match_all(&|_| {}).unwrap();
    settle(&c2);
    let t = Instant::now();
    let out2 = c2.detect(&|_| {}).unwrap();
    say(format!(
        "detect leftovers, real set-a + set-b ({} photos left, 3 looked at): {:.1} s (sample {})",
        c2.groups().leftovers,
        secs(t),
        out2.sample.len()
    ));
    c2.confirm_new_group(out2.representative).unwrap();
    let t = Instant::now();
    let sum = c2.match_all(&|_| {}).unwrap();
    say(format!(
        "match_all group 2, real ({} photos): {:.1} s ({} found, {} unsure, {} not found)",
        sum.total,
        secs(t),
        sum.found,
        sum.unsure,
        sum.not_found
    ));
    let strip = ids2
        .iter()
        .copied()
        .find(|id| *id != out2.representative)
        .unwrap();
    let t = Instant::now();
    let p = c2.place_on(out2.representative, strip).unwrap();
    say(format!(
        "place_on, 24 MP strip photo, first call: {:.1} s ({:?})",
        secs(t),
        p.status
    ));
    let t = Instant::now();
    c2.place_on(out2.representative, strip).unwrap();
    say(format!("place_on, same photo again: {:.2} s", secs(t)));

    // Preview of one photo: remover load, then the preview itself.
    let target = out.representative;
    let events = Mutex::new(Vec::<(f64, ProgressEvent)>::new());
    let start = Instant::now();
    c.preview(target, &|e| events.lock().unwrap().push((secs(start), e)))
        .unwrap();
    let total = secs(start);
    let ev = events.into_inner().unwrap();
    let loaded = ev
        .iter()
        .filter(|(_, e)| matches!(e, ProgressEvent::LoadingModels { .. }))
        .map(|(t, _)| *t)
        .fold(0.0, f64::max);
    say(format!(
        "preview of one photo (remover load first time): total {total:.1} s, of which load about {loaded:.1} s"
    ));
    let start = Instant::now();
    c.preview(target, &|_| {}).unwrap();
    say(format!(
        "preview of the same photo again: {:.1} s",
        secs(start)
    ));

    // Removal per photo at Best and Faster, one photo at a time, remover already loaded.
    let states = c.photo_states();
    let ready: Vec<u32> = states
        .iter()
        .filter(|s| s.has_area)
        .map(|s| s.id)
        .take(2)
        .collect();
    // Unsure photos block removal until opened, as in the UI.
    for id in &ready {
        c.open_photo(*id).unwrap();
    }
    let out_dir = work.join("results");
    for detail in [Detail::Best, Detail::Faster] {
        let mut s = c.settings();
        s.detail = detail;
        c.set_settings(s).unwrap();
        for (i, id) in ready.iter().enumerate() {
            for other in &ids {
                c.set_included(*other, other == id).ok();
            }
            let _ = fs::remove_dir_all(&out_dir);
            c.set_output_override(Some(out_dir.clone()));
            let t = Instant::now();
            let rep = c.remove(&|_| {}).unwrap();
            say(format!(
                "removal at {:?} ({}), photo {}: {:.1} s (done {}, failed {})",
                detail,
                detail.work_long_side(),
                i + 1,
                secs(t),
                rep.done.len(),
                rep.failed.len()
            ));
        }
    }
    let _ = fs::remove_dir_all(&out_dir);
    fs::write(work.join("timings.txt"), lines.join("\n")).unwrap();
}

/// Share of the pixels set in both edit-size areas among those set in either.
fn area_iou(a: &[u8], b: &[u8]) -> f64 {
    let (mut inter, mut uni) = (0u64, 0u64);
    for (p, q) in a.iter().zip(b) {
        let (x, y) = (*p != 0, *q != 0);
        inter += (x && y) as u64;
        uni += (x || y) as u64;
    }
    inter as f64 / uni.max(1) as f64
}

/// The real set-a (one fixed watermark) and set-b (one photo of another size) as one set, with
/// group 1 taken from the finder's own guess as it is: what step 2's strip shows, what a stroke
/// on the drawing costs, how long a stopped placement keeps the next call waiting, and what
/// step 3 finds. Aggregates only. `cargo test -p puw-app --release --test timings real_group --
/// --ignored --nocapture`; needs `PUW_MODELS_DIR` and `PUW_REAL_DIR`.
#[test]
#[ignore = "acceptance run, needs models and local photos"]
fn real_group_from_the_first_guess() {
    let Some(models) = common::models_dir() else {
        eprintln!("skipped: PUW_MODELS_DIR not set");
        return;
    };
    let Some(real) = std::env::var_os("PUW_REAL_DIR").map(PathBuf::from) else {
        eprintln!("skipped: PUW_REAL_DIR not set");
        return;
    };
    let work = common::synth_dir()
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .join("local")
        .join("timing");
    let data = work.join("data-accept");
    let _ = fs::remove_dir_all(&data);
    let c = core(&models, &work.join("coreml-cache"), &data, Detail::Best);
    let a = photos_in(&real.join("set-a"));
    let mut all = a.clone();
    all.extend(photos_in(&real.join("set-b")));
    let ids = add(&c, &all);
    let set_a: Vec<u32> = ids[..a.len()].to_vec();
    let other = ids[a.len()..].to_vec();
    load_times(&c);
    let t = Instant::now();
    let out = c.detect(&|_| {}).unwrap();
    let rep = out.representative;
    eprintln!(
        "detect: {:.1} s; {} photos, {} of the fixed set, {} of another size",
        secs(t),
        ids.len(),
        set_a.len(),
        other.len()
    );
    assert!(
        set_a.contains(&rep),
        "the representative is in the fixed set"
    );
    let (ew, eh) = c.edit_size(rep).unwrap();
    let guess = c.start_area(rep).unwrap();

    // Step 2's strip: the guess as it is, on the other sample photos.
    let close = |p: &puw_app::StripPlacement| {
        let long = ew.max(eh) as f32;
        (p.scale_x - 1.0).abs() < 0.01
            && (p.scale_y - 1.0).abs() < 0.01
            && p.dx.abs() < 0.01 * long
            && p.dy.abs() < 0.01 * long
    };
    let mut strip_ok = 0;
    let mut first = Vec::new();
    for &id in out.sample.iter().filter(|&&id| id != rep) {
        let t = Instant::now();
        let p = c.place_on(rep, id).unwrap();
        first.push(secs(t));
        if p.status != puw_app::Placed::NotFound && close(&p) {
            strip_ok += 1;
        }
        eprintln!(
            "  strip photo: {:?}, at the same place {}",
            p.status,
            close(&p)
        );
    }
    let strip_others = out.sample.len() - 1;
    eprintln!(
        "strip, first look: {strip_ok} of {strip_others} placed at the same place; {:.2} s each at most",
        first.iter().cloned().fold(0.0, f64::max)
    );

    // A stroke inside the drawing (the UI sends the whole drawing again), then every strip
    // photo again: what a switch costs after an edit.
    let mut stroked = guess.clone();
    let w = ew as usize;
    let rows: Vec<usize> = (0..eh as usize)
        .filter(|&y| guess[y * w..(y + 1) * w].iter().any(|v| *v != 0))
        .collect();
    let row = rows[rows.len() / 2];
    let x0 = guess[row * w..(row + 1) * w]
        .iter()
        .position(|v| *v != 0)
        .unwrap();
    for x in x0.saturating_sub(8)..x0 {
        stroked[row * w + x] = 255;
    }
    c.put_draft(rep, ew, eh, stroked).unwrap();
    let mut after = Vec::new();
    for &id in out.sample.iter().filter(|&&id| id != rep) {
        let t = Instant::now();
        c.place_on(rep, id).unwrap();
        after.push(secs(t));
    }
    eprintln!(
        "strip, each photo after a stroke: {:.2}-{:.2} s",
        after.iter().cloned().fold(f64::MAX, f64::min),
        after.iter().cloned().fold(0.0, f64::max)
    );
    c.put_draft(rep, ew, eh, guess.clone()).unwrap();

    // A full search (the photo of another size: the set position cannot help), stopped
    // after half a second: how long the call still runs.
    if let Some(&o) = other.first() {
        let t = Instant::now();
        let p = c.place_on(rep, o).unwrap();
        eprintln!(
            "strip, the photo of another size: {:?} in {:.2} s",
            p.status,
            secs(t)
        );
        std::thread::scope(|s| {
            let job = s.spawn(|| {
                let t = Instant::now();
                let r = c.place_on(rep, o);
                (r.is_err(), secs(t))
            });
            std::thread::sleep(std::time::Duration::from_millis(500));
            let stopped_at = Instant::now();
            c.cancel();
            let (stopped, total) = job.join().unwrap();
            eprintln!(
                "stop during a full search: call ended {:.2} s after the stop (stopped {stopped}, {total:.2} s in all)",
                secs(stopped_at)
            );
        });
    }

    // Step 3: group 1 from the unedited guess.
    c.drop_draft(rep);
    c.confirm_set_area(rep).unwrap();
    let t = Instant::now();
    let sum = c.match_all(&|_| {}).unwrap();
    eprintln!(
        "match_all: {:.1} s; found {} unsure {} not found {}",
        secs(t),
        sum.found,
        sum.unsure,
        sum.not_found
    );
    let saved = c.area(rep).unwrap();
    let (mut good, mut wrong_found, mut ious) = (0, 0, Vec::new());
    for &id in &set_a {
        let st = c.photo_states().into_iter().find(|s| s.id == id).unwrap();
        let q = area_iou(&c.area(id).unwrap(), &saved);
        ious.push(q);
        match st.status {
            puw_app::AreaStatus::Found if q >= 0.8 => good += 1,
            puw_app::AreaStatus::Found => wrong_found += 1,
            puw_app::AreaStatus::Unsure if q >= 0.8 => good += 1,
            _ => {}
        }
    }
    let other_found = other
        .iter()
        .filter(|id| {
            c.photo_states()
                .into_iter()
                .any(|s| s.id == **id && s.status == puw_app::AreaStatus::Found)
        })
        .count();
    eprintln!(
        "group 1: {good} of {} fixed-set photos Found or Unsure at the saved place; {wrong_found} Found elsewhere; IoU min {:.2}; {other_found} of {} other-size photos Found",
        set_a.len(),
        ious.iter().cloned().fold(f64::MAX, f64::min),
        other.len()
    );
    assert!(good >= 6, "{good}");
    assert_eq!(wrong_found, 0);
    assert_eq!(other_found, 0);
    assert_eq!(strip_ok, strip_others);
}

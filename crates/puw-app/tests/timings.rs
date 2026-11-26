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

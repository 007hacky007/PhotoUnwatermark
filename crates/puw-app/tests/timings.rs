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

    // The photo of another size: the set position cannot help there.
    if let Some(&o) = other.first() {
        let t = Instant::now();
        let p = c.place_on(rep, o).unwrap();
        eprintln!(
            "strip, the photo of another size: {:?} in {:.2} s",
            p.status,
            secs(t)
        );
    }

    // A full search: a drawing of two boxes in empty corners (no set position, two pieces, so
    // the pieces are also searched one by one), on a photo of the same size. Once as it is,
    // then stopped after half a second: how long the call still runs.
    let corners = {
        let mut a = vec![0u8; (ew * eh) as usize];
        let (bw, bh) = (ew as usize / 8, eh as usize / 8);
        for y in 0..eh as usize {
            for x in 0..ew as usize {
                let tl = (bw..2 * bw).contains(&x) && (bh..2 * bh).contains(&y);
                let br = (w - 2 * bw..w - bw).contains(&x)
                    && (eh as usize - 2 * bh..eh as usize - bh).contains(&y);
                if tl || br {
                    a[y * w + x] = 255;
                }
            }
        }
        a
    };
    let same_size: Vec<u32> = set_a.iter().copied().filter(|&id| id != rep).collect();
    c.put_draft(rep, ew, eh, corners).unwrap();
    let t = Instant::now();
    let p = c.place_on(rep, same_size[0]).unwrap();
    let full = secs(t);
    eprintln!(
        "full search on a 24 MP photo: {:?} in {full:.2} s",
        p.status
    );
    // Another photo, so the placement is not one kept from before.
    std::thread::scope(|s| {
        let job = s.spawn(|| c.place_on(rep, same_size[1]).is_err());
        std::thread::sleep(std::time::Duration::from_millis(500));
        let stopped_at = Instant::now();
        c.cancel();
        let stopped = job.join().unwrap();
        eprintln!(
            "stop half a second into a {full:.2} s search: the call ended {:.2} s after the stop (stopped {stopped})",
            secs(stopped_at)
        );
    });

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
    assert_eq!(
        good,
        set_a.len(),
        "every photo of the fixed set at the saved place"
    );
    assert_eq!(wrong_found, 0);
    assert_eq!(other_found, 0);
    assert_eq!(strip_ok, strip_others);
}

/// The drawing a person makes of the first guess before switching photos: its specks removed
/// (every group of parts, grouped as the template groups them, with less than a quarter of the
/// largest group's pixels) and a band of 1 % of the photo's height erased across its top.
/// Edit-size bytes in, edit-size bytes out.
fn cleaned_guess(guess: &[u8], ew: u32, eh: u32) -> Vec<u8> {
    let m = puw_core::Mask::from_raw(ew, eh, guess.to_vec()).unwrap();
    let comps = puw_core::mask::components(&m);
    let groups = puw_core::mask::group_near(&comps, 0.02 * ew.max(eh) as f32);
    let size = |g: &Vec<usize>| g.iter().map(|&i| u64::from(comps[i].pixels)).sum::<u64>();
    let most = groups.iter().map(size).max().unwrap();
    let kept: Vec<usize> = groups
        .iter()
        .filter(|g| 4 * size(g) >= most)
        .flatten()
        .copied()
        .collect();
    let mut out = puw_core::Mask::new(ew, eh);
    for c in kept.iter().map(|&i| &comps[i]) {
        let (ox, oy) = (c.bbox.left() as u32, c.bbox.top() as u32);
        for (x, y, p) in c.mask.enumerate_pixels() {
            if p.0[0] != 0 {
                out.put_pixel(ox + x, oy + y, *p);
            }
        }
    }
    let top = puw_core::mask::bbox(&out).unwrap().top() as u32;
    for y in top..(top + eh / 100).min(eh) {
        for x in 0..ew {
            out.put_pixel(x, y, image::Luma([0]));
        }
    }
    out.into_raw()
}

/// Small sets of the real set-a (3 to 7 photos, one fixed watermark): the step-2 strip places
/// the first guess, as it is and cleaned up by hand ([`cleaned_guess`]), at the same place on
/// every other photo, and step 3 finds every photo there. Times each strip placement and
/// counts Found and Unsure per set size. Aggregates only. `cargo test -p puw-app --release
/// --test timings real_small -- --ignored --nocapture`; needs `PUW_MODELS_DIR` and
/// `PUW_REAL_DIR`. `PUW_SET_SIZES=4,5` runs only the sets of those sizes.
#[test]
#[ignore = "acceptance run, needs models and local photos"]
fn real_small_sets_keep_the_set_position() {
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
    let a = photos_in(&real.join("set-a"));
    assert_eq!(a.len(), 8, "set-a has 8 photos");
    let sets: Vec<Vec<usize>> = vec![
        vec![0, 1, 2],
        vec![3, 4, 5],
        vec![5, 6, 7],
        vec![0, 3, 7],
        vec![1, 4, 6],
        vec![2, 5, 7],
        vec![0, 2, 4],
        vec![1, 3, 6],
        vec![0, 1, 2, 3],
        vec![4, 5, 6, 7],
        vec![0, 2, 4, 6, 7],
        vec![1, 3, 5, 6, 7],
        vec![0, 1, 2, 3, 4, 5],
        vec![2, 3, 4, 5, 6, 7],
        vec![0, 1, 2, 3, 4, 5, 6],
        vec![1, 2, 3, 4, 5, 6, 7],
    ];
    let sizes: Option<Vec<usize>> = std::env::var("PUW_SET_SIZES")
        .ok()
        .map(|v| v.split(',').map(|n| n.trim().parse().unwrap()).collect());
    let (mut strip_ok, mut strip_all, mut times) = (0, 0, Vec::new());
    let (mut match_ok, mut match_all, mut wrong_found) = (0, 0, 0);
    let mut statuses = [0usize; 3];
    // Per set size 3..=7: strip Found, strip Unsure, step-3 Found, step-3 Unsure.
    let mut by_size = [[0usize; 4]; 8];
    let mut removed = Vec::new();
    for (k, set) in sets.iter().enumerate() {
        if sizes.as_ref().is_some_and(|z| !z.contains(&set.len())) {
            continue;
        }
        for cleaned in [false, true] {
            let data = work.join("data-small");
            let _ = fs::remove_dir_all(&data);
            let c = core(&models, &work.join("coreml-cache"), &data, Detail::Best);
            let paths: Vec<PathBuf> = set.iter().map(|&i| a[i].clone()).collect();
            let ids = add(&c, &paths);
            let out = c.detect(&|_| {}).unwrap();
            let rep = out.representative;
            let (ew, eh) = c.edit_size(rep).unwrap();
            if cleaned {
                let guess = c.start_area(rep).unwrap();
                let d = cleaned_guess(&guess, ew, eh);
                let px = |v: &[u8]| v.iter().filter(|p| **p != 0).count() as f64;
                removed.push(1.0 - px(&d) / px(&guess));
                c.put_draft(rep, ew, eh, d).unwrap();
            }
            let long = ew.max(eh) as f32;
            let mut line = Vec::new();
            for &id in ids.iter().filter(|&&id| id != rep) {
                let t = Instant::now();
                let p = c.place_on(rep, id).unwrap();
                times.push(secs(t));
                let same = (p.scale_x - 1.0).abs() < 0.01
                    && (p.scale_y - 1.0).abs() < 0.01
                    && p.dx.abs() < 0.01 * long
                    && p.dy.abs() < 0.01 * long;
                strip_all += 1;
                if p.status != puw_app::Placed::NotFound && same {
                    strip_ok += 1;
                    by_size[set.len()][(p.status == puw_app::Placed::Unsure) as usize] += 1;
                }
                line.push(format!("{:?} same place {same}", p.status));
            }
            c.confirm_set_area(rep).unwrap();
            c.match_all(&|_| {}).unwrap();
            let saved = c.area(rep).unwrap();
            for &id in &ids {
                let st = c.photo_states().into_iter().find(|s| s.id == id).unwrap();
                let q = area_iou(&c.area(id).unwrap(), &saved);
                match_all += 1;
                match st.status {
                    puw_app::AreaStatus::Found if q >= 0.99 => {
                        match_ok += 1;
                        statuses[0] += 1;
                        by_size[set.len()][2] += 1;
                    }
                    puw_app::AreaStatus::Found => wrong_found += 1,
                    puw_app::AreaStatus::Unsure if q >= 0.99 => {
                        match_ok += 1;
                        statuses[1] += 1;
                        by_size[set.len()][3] += 1;
                    }
                    _ => statuses[2] += 1,
                }
            }
            eprintln!(
                "set {} ({} photos){}: strip {}",
                k + 1,
                set.len(),
                if cleaned { " cleaned" } else { "" },
                line.join(", ")
            );
        }
    }
    let _ = fs::remove_dir_all(work.join("data-small"));
    times.sort_by(f64::total_cmp);
    eprintln!(
        "strip: {strip_ok} of {strip_all} placed at the same place; {:.2} s median, {:.2} s max",
        times[times.len() / 2],
        times[times.len() - 1]
    );
    for (n, [sf, su, mf, mu]) in by_size
        .iter()
        .enumerate()
        .filter(|(_, c)| c.iter().any(|v| *v > 0))
    {
        eprintln!("{n} photos: strip Found {sf} Unsure {su}; step 3 Found {mf} Unsure {mu}");
    }
    eprintln!(
        "cleaning removed {:.1}-{:.1} % of the guess",
        100.0 * removed.iter().cloned().fold(f64::MAX, f64::min),
        100.0 * removed.iter().cloned().fold(0.0, f64::max)
    );
    eprintln!(
        "match_all: {match_ok} of {match_all} at the saved place (Found {}, Unsure {}), {} other, {wrong_found} Found elsewhere",
        statuses[0], statuses[1], statuses[2]
    );
    assert_eq!(strip_ok, strip_all);
    assert_eq!(match_ok, match_all);
    assert_eq!(wrong_found, 0);
    // Placed at the true place, and at most one set per size left `Unsure` (its other photos,
    // as is and cleaned). One triple is: one part of its layer scores under the Unsure bar on
    // the other photos, which keeps them from `Found` (0.51 over the whole layer).
    for (n, [_, su, _, mu]) in by_size.iter().enumerate() {
        let one_set = 2 * n.saturating_sub(1);
        assert!(
            *su <= one_set && *mu <= one_set,
            "{n} photos: {su} strip and {mu} step-3 Unsure"
        );
    }
}

/// `m` (edit-size bytes, `ew` x `eh`) with every pixel within `r` px of an unset one cleared.
fn shrunk(m: &[u8], ew: u32, eh: u32, r: u32) -> Vec<u8> {
    let inv: Vec<u8> = m.iter().map(|v| if *v == 0 { 255 } else { 0 }).collect();
    let grown = puw_core::mask::dilate(&puw_core::Mask::from_raw(ew, eh, inv).unwrap(), r);
    grown
        .as_raw()
        .iter()
        .map(|v| if *v == 0 { 255 } else { 0 })
        .collect()
}

/// The drawings a person may confirm, made from the first guess `g` (edit-size bytes): as it
/// is; cleaned up ([`cleaned_guess`]); with everything farther than 1 % of the long side from
/// the watermark `mark` (native size) erased; with a band of 3 % of the height erased across its middle;
/// with four brush dots (radius 1 % of the long side) on the scene away from it; grown and
/// shrunk by 8 px.
fn edited_drawings(
    g: &[u8],
    ew: u32,
    eh: u32,
    mark: &puw_core::Mask,
) -> Vec<(&'static str, Vec<u8>)> {
    let m = puw_core::Mask::from_raw(ew, eh, g.to_vec()).unwrap();
    // Everything of the guess away from the mark erased, as a careful person erases the parts
    // of the scene the guess caught.
    let around = puw_core::mask::dilate(&puw_core::mask::resize(mark, ew, eh), ew.max(eh) / 100);
    let only_mark: Vec<u8> = g
        .iter()
        .zip(around.as_raw())
        .map(|(v, a)| if *a != 0 { *v } else { 0 })
        .collect();
    let bb = puw_core::mask::bbox(&m).unwrap();
    let mut erased = g.to_vec();
    let mid = (bb.top() + bb.bottom()) as u32 / 2;
    for y in mid.saturating_sub(eh * 3 / 200)..(mid + eh * 3 / 200).min(eh) {
        for x in 0..ew {
            erased[(y * ew + x) as usize] = 0;
        }
    }
    let mut dots = g.to_vec();
    let r = (ew.max(eh) / 100) as i64;
    let near = puw_core::mask::dilate(&m, (4 * r) as u32);
    for (fx, fy) in [(0.12, 0.2), (0.88, 0.45), (0.15, 0.8), (0.85, 0.12)] {
        let (cx, cy) = ((fx * ew as f32) as i64, (fy * eh as f32) as i64);
        if near.get_pixel(cx as u32, cy as u32).0[0] != 0 {
            continue;
        }
        for y in (cy - r).max(0)..(cy + r).min(eh as i64) {
            for x in (cx - r).max(0)..(cx + r).min(ew as i64) {
                if (x - cx).pow(2) + (y - cy).pow(2) <= r * r {
                    dots[(y as u32 * ew + x as u32) as usize] = 255;
                }
            }
        }
    }
    vec![
        ("as is", g.to_vec()),
        ("cleaned", cleaned_guess(g, ew, eh)),
        ("scene parts erased", only_mark),
        ("middle erased", erased),
        ("brush dots", dots),
        ("grown 8 px", puw_core::mask::dilate(&m, 8).into_raw()),
        ("shrunk 8 px", shrunk(g, ew, eh, 8)),
    ]
}

/// Every 3-photo set of the real set-a (56), each with the drawings of [`edited_drawings`]
/// confirmed as the group's area: step 3 places every photo at the reference's place (`Found`
/// or `Unsure`, never elsewhere or not at all), and counts per drawing where the photos land. `PUW_TRIPLES=0,1,2;3,4,5`
/// runs only those sets, `PUW_DRAWINGS=as is;cleaned` only those drawings. Aggregates and set indices only. `cargo test -p puw-app --release --test
/// timings real_triples -- --ignored --nocapture`; needs `PUW_MODELS_DIR` and `PUW_REAL_DIR`.
#[test]
#[ignore = "acceptance run, needs models and local photos"]
fn real_triples_with_edited_drawings() {
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
    let a = photos_in(&real.join("set-a"));
    assert_eq!(a.len(), 8, "set-a has 8 photos");
    // Where the watermark is: the batch layer of all 8 photos.
    let all = puw_core::build_set_layers(&a, &puw_core::Progress::none()).unwrap();
    let (_, mark) = all.iter().next().expect("set-a has a layer");
    let mark = mark.clone();
    let triples: Vec<Vec<usize>> = match std::env::var("PUW_TRIPLES") {
        Ok(v) => v
            .split(';')
            .map(|t| t.split(',').map(|n| n.trim().parse().unwrap()).collect())
            .collect(),
        Err(_) => (0..8)
            .flat_map(|i| (i + 1..8).flat_map(move |j| (j + 1..8).map(move |k| vec![i, j, k])))
            .collect(),
    };
    let only: Option<Vec<String>> = std::env::var("PUW_DRAWINGS")
        .ok()
        .map(|v| v.split(';').map(str::to_owned).collect());
    // Per drawing: at the saved place Found, Unsure; not found; Unsure elsewhere; Found elsewhere.
    let mut counts: Vec<(&str, [usize; 5])> = Vec::new();
    for (n, triple) in triples.iter().enumerate() {
        let data = work.join("data-triples");
        let _ = fs::remove_dir_all(&data);
        let c = core(&models, &work.join("coreml-cache"), &data, Detail::Best);
        let paths: Vec<PathBuf> = triple.iter().map(|&i| a[i].clone()).collect();
        let ids = add(&c, &paths);
        let out = c.detect(&|_| {}).unwrap();
        let rep = out.representative;
        let (ew, eh) = c.edit_size(rep).unwrap();
        let guess = c.start_area(rep).unwrap();
        let mut line = Vec::new();
        for (name, drawing) in edited_drawings(&guess, ew, eh, &mark) {
            if only.as_ref().is_some_and(|o| !o.iter().any(|d| d == name)) {
                continue;
            }
            c.clear_groups().unwrap();
            c.put_draft(rep, ew, eh, drawing).unwrap();
            c.confirm_set_area(rep).unwrap();
            c.match_all(&|_| {}).unwrap();
            let saved = c.area(rep).unwrap();
            let k = match counts.iter().position(|(d, _)| *d == name) {
                Some(k) => k,
                None => {
                    counts.push((name, [0; 5]));
                    counts.len() - 1
                }
            };
            let mut got = Vec::new();
            for &id in ids.iter().filter(|&&id| id != rep) {
                let st = c.photo_states().into_iter().find(|s| s.id == id).unwrap();
                let same = area_iou(&c.area(id).unwrap(), &saved) >= 0.99;
                let slot = match (st.status, same) {
                    (puw_app::AreaStatus::Found, true) => 0,
                    (puw_app::AreaStatus::Unsure, true) => 1,
                    (puw_app::AreaStatus::Found, false) => 4,
                    (puw_app::AreaStatus::Unsure, false) => 3,
                    _ => 2,
                };
                counts[k].1[slot] += 1;
                got.push(["F", "U", "N", "U-elsewhere", "F-elsewhere"][slot]);
            }
            line.push(format!("{name}: {}", got.join(" ")));
        }
        eprintln!("set {} {:?}: {}", n + 1, triple, line.join("; "));
    }
    let _ = fs::remove_dir_all(work.join("data-triples"));
    for (name, [f, u, nf, ue, fe]) in &counts {
        eprintln!(
            "{name}: at the saved place Found {f} Unsure {u}; not found {nf}; Unsure elsewhere {ue}; Found elsewhere {fe}"
        );
    }
    // Every photo placed at the reference's place, whatever the person did to the drawing;
    // `Unsure` stays allowed there.
    for (name, [_, _, nf, ue, fe]) in &counts {
        assert_eq!((nf, ue, fe), (&0, &0, &0), "{name}");
    }
}

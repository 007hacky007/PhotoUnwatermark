//! Watermark groups: a set with two watermarks is handled one group at a time, and every photo
//! is always counted once (in a group, with no group yet, or as one that cannot be read).
//!
//! The tests of the leftovers' first guess need models: they read them from `PUW_MODELS_DIR`
//! and return early with a message when it is not set. No other test here needs models.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use common::{
    Fixture, Mixed, Recorder, add_synth, confirm_truth, fixture, logo_photos, matching, mixed_set,
    model_fixture, noop, put_truth, rect_area, settle, state, synth, synth_truth,
};
use image::{GrayImage, RgbImage};
use puw_app::images::jpeg;
use puw_app::{
    AppCore, AppError, AreaStatus, DeviceState, GroupInfo, PhotoState, Placed, ProgressEvent,
    StripPlacement, with_ticket,
};
use puw_core::{Library, MatchStatus, SavedWatermark, build_template, find};

/// The members of every group, plus the photos with no group yet, plus the ones that cannot be
/// read, are all the photos. Each count is checked against other calls too: the photo states,
/// the photo problems and, when nothing holds up removal, the photos to remove.
fn assert_counts(core: &AppCore) {
    let g = core.groups();
    let photos = core.photos();
    let members: u32 = g.groups.iter().map(|i| i.photos).sum();
    assert_eq!(
        members + g.leftovers + g.unreadable,
        photos.len() as u32,
        "{g:?}"
    );
    let states = core.photo_states();
    let count = |keep: &dyn Fn(&PhotoState) -> bool| states.iter().filter(|s| keep(s)).count();
    assert_eq!(
        g.leftovers as usize,
        count(&|s| s.status != AreaStatus::Unreadable && !s.member),
        "{g:?}"
    );
    assert_eq!(
        g.unreadable as usize,
        photos.iter().filter(|p| p.problem.is_some()).count(),
        "{g:?}"
    );
    for info in &g.groups {
        assert_eq!(
            info.photos as usize,
            count(&|s| s.member && s.group == Some(info.number)),
            "{g:?}"
        );
    }
    if core.removal_blockers().is_empty() {
        assert_eq!(core.to_remove().len(), members as usize, "{g:?}");
    }
}

fn states_of(core: &AppCore, ids: &[u32]) -> Vec<PhotoState> {
    ids.iter().map(|&id| state(core, id)).collect()
}

/// Group 1 from fixed[0]'s truth, matched and settled.
fn group_one(core: &AppCore, m: &Mixed) {
    confirm_truth(core, "fixed", "00", m.fixed[0]);
    assert_counts(core);
    core.match_all(&noop).unwrap();
    assert_counts(core);
    settle(core);
    assert_counts(core);
}

/// Group 2 from logos[0]'s truth; returns the `Matching` events of its matching.
fn group_two(core: &AppCore, m: &Mixed) -> Vec<(u32, u32)> {
    put_truth(core, m.logos[0], &m.logo_truths[0]);
    core.confirm_new_group(m.logos[0]).unwrap();
    assert_counts(core);
    let rec = Recorder::default();
    core.match_all(&|e| rec.push(e)).unwrap();
    assert_counts(core);
    settle(core);
    assert_counts(core);
    matching(&rec.events())
}

fn two_groups(core: &AppCore, dir: &Path) -> Mixed {
    let m = mixed_set(core, dir);
    group_one(core, &m);
    group_two(core, &m);
    m
}

/// A JPEG cut short: it is added (its header is fine) but cannot be read.
fn broken_jpeg(dir: &Path) -> PathBuf {
    let img = image::RgbImage::from_fn(300, 200, |x, y| image::Rgb([x as u8, y as u8, 90]));
    let bytes = jpeg(&img, 85);
    let p = dir.join("broken.jpg");
    std::fs::write(&p, &bytes[..bytes.len() * 2 / 3]).unwrap();
    p
}

#[test]
fn first_confirm_makes_group_one() {
    let f = fixture(None);
    let dir = tempfile::tempdir().unwrap();
    let m = mixed_set(&f.core, dir.path());
    assert_counts(&f.core);
    let g = f.core.groups();
    assert!(
        g.groups.is_empty() && g.current.is_none() && !g.matched,
        "{g:?}"
    );

    confirm_truth(&f.core, "fixed", "00", m.fixed[0]);
    assert_counts(&f.core);
    let g = f.core.groups();
    assert_eq!(
        g.groups,
        [GroupInfo {
            number: 1,
            photos: 1,
            saved_name: None,
            edited: false,
            can_save: true,
        }]
    );
    assert_eq!(g.current, Some(1));
    assert!(!g.matched);

    f.core.match_all(&noop).unwrap();
    assert_counts(&f.core);
    let g = f.core.groups();
    assert!(g.matched);
    for &id in &m.fixed {
        let st = state(&f.core, id);
        assert!(st.member && st.group == Some(1), "{st:?}");
    }
    assert_eq!(g.leftovers, 6, "{g:?}");
    assert_eq!(g.groups[0].photos, 8);
}

#[test]
fn next_group_matches_only_photos_without_a_group() {
    let f = fixture(None);
    let dir = tempfile::tempdir().unwrap();
    let m = mixed_set(&f.core, dir.path());
    group_one(&f.core, &m);
    let before = states_of(&f.core, &m.fixed);

    put_truth(&f.core, m.logos[0], &m.logo_truths[0]);
    f.core.confirm_new_group(m.logos[0]).unwrap();
    assert_counts(&f.core);
    assert_eq!(f.core.groups().current, Some(2));
    let rec = Recorder::default();
    f.core.match_all(&|e| rec.push(e)).unwrap();
    assert_counts(&f.core);
    let events = matching(&rec.events());
    assert_eq!(events.first(), Some(&(0, 5)), "{events:?}");
    assert_eq!(events.last(), Some(&(5, 5)), "{events:?}");

    assert_eq!(states_of(&f.core, &m.fixed), before);
    for &id in &m.logos[1..] {
        let st = state(&f.core, id);
        assert_eq!(st.status, AreaStatus::Found, "{st:?}");
        assert!(st.member && st.group == Some(2), "{st:?}");
    }
    for &id in &m.plain {
        let st = state(&f.core, id);
        assert_eq!(st.status, AreaStatus::NotFound, "{st:?}");
        assert!(!st.member && st.group == Some(2), "{st:?}");
    }
    let g = f.core.groups();
    assert_eq!(g.leftovers, 2, "{g:?}");
    assert_eq!(
        g.groups.iter().map(|i| i.photos).collect::<Vec<_>>(),
        [8, 4]
    );
}

#[test]
fn unticking_an_earlier_group_photo_drops_it() {
    let f = fixture(None);
    let dir = tempfile::tempdir().unwrap();
    let m = two_groups(&f.core, dir.path());
    let left = f.core.groups().leftovers;

    let id = m.fixed[3];
    f.core.set_included(id, false).unwrap();
    assert_counts(&f.core);
    let st = state(&f.core, id);
    assert_eq!(st.status, AreaStatus::Pending);
    assert!(st.group.is_none() && !st.has_area && !st.member, "{st:?}");
    assert!(matches!(
        f.core.set_included(id, true),
        Err(AppError::NothingToRemove)
    ));
    assert!(!f.core.removal_blockers().contains(&id));
    assert_eq!(f.core.groups().leftovers, left + 1);
    assert_counts(&f.core);
}

#[test]
fn unticking_a_current_group_photo_can_be_undone() {
    let f = fixture(None);
    let dir = tempfile::tempdir().unwrap();
    let m = two_groups(&f.core, dir.path());
    let left = f.core.groups().leftovers;

    let id = m.logos[1];
    f.core.set_included(id, false).unwrap();
    assert_counts(&f.core);
    let st = state(&f.core, id);
    assert!(st.group == Some(2) && !st.member, "{st:?}");
    assert_eq!(f.core.groups().leftovers, left + 1);

    f.core.set_included(id, true).unwrap();
    assert_counts(&f.core);
    assert!(state(&f.core, id).member);
    assert_eq!(f.core.groups().leftovers, left);
}

#[test]
fn next_group_rematches_unticked_photos() {
    let f = fixture(None);
    let dir = tempfile::tempdir().unwrap();
    let m = mixed_set(&f.core, dir.path());
    group_one(&f.core, &m);
    f.core.set_included(m.fixed[2], false).unwrap();
    assert_counts(&f.core);

    let events = group_two(&f.core, &m);
    assert_eq!(events.last(), Some(&(6, 6)), "{events:?}");
    let st = state(&f.core, m.fixed[2]);
    assert_eq!(st.status, AreaStatus::NotFound);
    assert!(st.group == Some(2) && !st.member, "{st:?}");
}

#[test]
fn new_group_waits() {
    // Before group 1's matching.
    let f = fixture(None);
    let dir = tempfile::tempdir().unwrap();
    let m = mixed_set(&f.core, dir.path());
    confirm_truth(&f.core, "fixed", "00", m.fixed[0]);
    put_truth(&f.core, m.logos[0], &m.logo_truths[0]);
    assert!(matches!(
        f.core.confirm_new_group(m.logos[0]),
        Err(AppError::NotReady)
    ));
    assert_eq!(f.core.groups().groups.len(), 1);
    assert_counts(&f.core);

    // While an unsure photo is ticked and nobody opened it. With the set area from random/05,
    // random/10 scores unsure (see `unsure_blocks_until_opened_or_unticked`).
    let f = fixture(None);
    let ids = add_synth(&f.core, "random", &["05", "10"]);
    let (five, ten) = (ids[0], ids[1]);
    let (paths, truths): (Vec<_>, Vec<_>) = logo_photos(dir.path(), 4).into_iter().unzip();
    let logos: Vec<u32> = f
        .core
        .add_paths(&paths)
        .unwrap()
        .added
        .iter()
        .map(|p| p.id)
        .collect();
    confirm_truth(&f.core, "random", "05", five);
    f.core.match_all(&noop).unwrap();
    assert_counts(&f.core);
    let st = state(&f.core, ten);
    assert_eq!(st.status, AreaStatus::Unsure);
    assert!(st.included && !st.reviewed && !st.member, "{st:?}");

    put_truth(&f.core, logos[0], &truths[0]);
    assert!(matches!(
        f.core.confirm_new_group(logos[0]),
        Err(AppError::NotReady)
    ));
    f.core.open_photo(ten).unwrap();
    assert!(state(&f.core, ten).member);
    assert_counts(&f.core);
    f.core.confirm_new_group(logos[0]).unwrap();
    assert_eq!(f.core.groups().current, Some(2));
    assert_counts(&f.core);
}

#[test]
fn unchanged_confirm_keeps_each_group() {
    let f = fixture(None);
    let dir = tempfile::tempdir().unwrap();
    let m = two_groups(&f.core, dir.path());
    f.core.set_included(m.logos[2], false).unwrap();
    assert_counts(&f.core);
    let before = f.core.photo_states();
    let fixed_before = states_of(&f.core, &m.fixed);
    let template = f.core.confirmed_template().unwrap();

    let id = m.logos[0];
    let (w, h) = f.core.edit_size(id).unwrap();
    let same = f.core.area(id).unwrap();
    f.core.put_draft(id, w, h, same.clone()).unwrap();
    f.core.confirm_set_area(id).unwrap();
    assert_eq!(f.core.photo_states(), before);
    assert!(Arc::ptr_eq(
        &template,
        &f.core.confirmed_template().unwrap()
    ));
    assert!(f.core.groups().matched);

    let mut changed = same;
    changed[0] = 255;
    f.core.put_draft(id, w, h, changed).unwrap();
    f.core.confirm_set_area(id).unwrap();
    assert_counts(&f.core);
    assert_eq!(state(&f.core, m.logos[1]).status, AreaStatus::Pending);
    assert_eq!(state(&f.core, m.logos[2]).status, AreaStatus::Pending);
    assert_eq!(state(&f.core, id).status, AreaStatus::Found);
    assert_eq!(states_of(&f.core, &m.fixed), fixed_before);
    let g = f.core.groups();
    assert!(!g.matched && g.current == Some(2), "{g:?}");
    assert!(!Arc::ptr_eq(
        &template,
        &f.core.confirmed_template().unwrap()
    ));
}

#[test]
fn saving_a_photo_joins_the_right_group() {
    let f = fixture(None);
    let dir = tempfile::tempdir().unwrap();
    let m = two_groups(&f.core, dir.path());
    let paint = |id: u32| {
        let (w, h) = f.core.edit_size(id).unwrap();
        f.core
            .put_draft(id, w, h, rect_area(w, h, 100, 100, 300, 200))
            .unwrap();
        f.core.save_photo_area(id).unwrap();
        assert_counts(&f.core);
        state(&f.core, id)
    };

    let st = paint(m.plain[0]);
    assert_eq!(st.status, AreaStatus::Edited);
    assert!(st.group == Some(2) && st.member, "{st:?}");

    let st = paint(m.fixed[1]);
    assert_eq!(st.status, AreaStatus::Edited);
    assert!(st.group == Some(1) && st.member, "{st:?}");

    // An empty area saved on an earlier group's photo drops it from that group.
    let id = m.fixed[4];
    let (w, h) = f.core.edit_size(id).unwrap();
    f.core
        .put_draft(id, w, h, vec![0; (w * h) as usize])
        .unwrap();
    f.core.save_photo_area(id).unwrap();
    assert_counts(&f.core);
    let st = state(&f.core, id);
    assert_eq!(st.status, AreaStatus::Pending);
    assert!(st.group.is_none() && !st.member, "{st:?}");
}

#[test]
fn saving_needs_a_group() {
    let f = fixture(None);
    let ids = add_synth(&f.core, "random", &["00"]);
    let (w, h) = f.core.edit_size(ids[0]).unwrap();
    f.core
        .put_draft(ids[0], w, h, rect_area(w, h, 10, 10, 50, 50))
        .unwrap();
    assert!(matches!(
        f.core.save_photo_area(ids[0]),
        Err(AppError::NotReady)
    ));
    assert_eq!(state(&f.core, ids[0]).status, AreaStatus::Pending);
}

#[test]
fn unreadable_photos_never_join_a_group() {
    let f = fixture(None);
    let dir = tempfile::tempdir().unwrap();
    let m = mixed_set(&f.core, dir.path());
    let broken = f.core.add_paths(&[broken_jpeg(dir.path())]).unwrap().added[0].id;
    assert_counts(&f.core);
    group_one(&f.core, &m);

    let st = state(&f.core, broken);
    assert_eq!(st.status, AreaStatus::Unreadable);
    assert!(st.group.is_none() && !st.member, "{st:?}");
    let g = f.core.groups();
    assert_eq!(g.unreadable, 1);
    assert_eq!(g.leftovers, 6, "{g:?}");

    // The next group does not look at it either.
    let events = group_two(&f.core, &m);
    assert_eq!(events.last(), Some(&(5, 5)), "{events:?}");
    assert!(state(&f.core, broken).group.is_none());
}

#[test]
fn groups_clear_when_the_list_changes() {
    let f = fixture(None);
    let dir = tempfile::tempdir().unwrap();
    let m = two_groups(&f.core, dir.path());
    let all_clear = |core: &AppCore| {
        let g = core.groups();
        assert!(g.groups.is_empty() && g.current.is_none(), "{g:?}");
        for st in core.photo_states() {
            assert_eq!(st.status, AreaStatus::Pending, "{st:?}");
            assert!(!st.included && !st.member && st.group.is_none(), "{st:?}");
        }
        assert!(core.confirmed_template().is_none());
        assert_counts(core);
    };

    add_synth(&f.core, "negative", &["00"]);
    all_clear(&f.core);

    group_one(&f.core, &m);
    group_two(&f.core, &m);
    assert_eq!(f.core.groups().groups.len(), 2);
    f.core.clear_groups().unwrap();
    all_clear(&f.core);

    // Numbers start at 1 again.
    confirm_truth(&f.core, "fixed", "00", m.fixed[0]);
    assert_eq!(f.core.groups().current, Some(1));

    // Removing a photo clears them too.
    f.core.remove_photo(m.plain[1]).unwrap();
    all_clear(&f.core);
}

/// Review Focus 4: unticking, opening, saving and making groups in any order never loses a
/// photo.
#[test]
fn counts_always_add_up() {
    let f = fixture(None);
    let core = &f.core;
    let dir = tempfile::tempdir().unwrap();
    // random/05 and random/10 (unsure with 05's area), the logo photos and a plain photo.
    let ids = add_synth(core, "random", &["05", "10"]);
    let (five, ten) = (ids[0], ids[1]);
    let (paths, truths): (Vec<_>, Vec<_>) = logo_photos(dir.path(), 4).into_iter().unzip();
    let logos: Vec<u32> = core
        .add_paths(&paths)
        .unwrap()
        .added
        .iter()
        .map(|p| p.id)
        .collect();
    let plain = add_synth(core, "negative", &["04"])[0];
    assert_counts(core);

    // Group 1, matched.
    confirm_truth(core, "random", "05", five);
    assert_counts(core);
    core.match_all(&noop).unwrap();
    assert_counts(core);
    assert_eq!(core.groups().groups[0].photos, 1);

    // Untick a member, then tick it again.
    core.set_included(five, false).unwrap();
    assert_counts(core);
    assert_eq!(core.groups().groups[0].photos, 0);
    core.set_included(five, true).unwrap();
    assert_counts(core);

    // Open the unsure photo: it joins group 1.
    assert_eq!(state(core, ten).status, AreaStatus::Unsure);
    core.open_photo(ten).unwrap();
    assert_counts(core);
    assert_eq!(core.groups().groups[0].photos, 2);

    // Save an area of its own on the plain photo: it joins group 1 too.
    let (w, h) = core.edit_size(plain).unwrap();
    core.put_draft(plain, w, h, rect_area(w, h, 20, 20, 120, 80))
        .unwrap();
    core.save_photo_area(plain).unwrap();
    assert_counts(core);
    assert_eq!(core.groups().groups[0].photos, 3);

    // Group 2 from the logo, matched.
    put_truth(core, logos[0], &truths[0]);
    core.confirm_new_group(logos[0]).unwrap();
    assert_counts(core);
    core.match_all(&noop).unwrap();
    assert_counts(core);
    settle(core);
    assert_counts(core);
    assert_eq!(core.groups().groups[1].photos, 4);

    // Untick a photo of group 1: it has no group now.
    core.set_included(ten, false).unwrap();
    assert_counts(core);
    let g = core.groups();
    assert_eq!(g.groups[0].photos, 2);
    assert_eq!(g.leftovers, 1, "{g:?}");

    // A broken file is added: the groups are cleared.
    core.add_paths(&[broken_jpeg(dir.path())]).unwrap();
    assert_counts(core);

    // Group 1 again: the broken photo is counted as one that cannot be read.
    confirm_truth(core, "random", "05", five);
    assert_counts(core);
    core.match_all(&noop).unwrap();
    assert_counts(core);
    let g = core.groups();
    assert_eq!(g.unreadable, 1, "{g:?}");
}

// ---- The shared drawing on the strip photos ----

/// Bounding box of the set pixels as `[x0, y0, x1, y1]`, at the pixel edges.
fn edge_box(m: &GrayImage) -> [f32; 4] {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for (x, y, p) in m.enumerate_pixels() {
        if p.0[0] != 0 {
            (x0, y0) = (x0.min(x), y0.min(y));
            (x1, y1) = (x1.max(x + 1), y1.max(y + 1));
        }
    }
    assert!(x0 < x1, "an empty mask");
    [x0 as f32, y0 as f32, x1 as f32, y1 as f32]
}

/// IoU of `from` (a reference box) moved by `p`, and `to`.
fn placed_iou(p: &StripPlacement, from: [f32; 4], to: [f32; 4]) -> f32 {
    let a = [
        from[0] * p.scale_x + p.dx,
        from[1] * p.scale_y + p.dy,
        from[2] * p.scale_x + p.dx,
        from[3] * p.scale_y + p.dy,
    ];
    let w = (a[2].min(to[2]) - a[0].max(to[0])).max(0.0);
    let h = (a[3].min(to[3]) - a[1].max(to[1])).max(0.0);
    let area = |r: [f32; 4]| (r[2] - r[0]) * (r[3] - r[1]);
    let inter = w * h;
    inter / (area(a) + area(to) - inter)
}

fn not_found() -> StripPlacement {
    StripPlacement {
        status: Placed::NotFound,
        scale_x: 0.0,
        scale_y: 0.0,
        dx: 0.0,
        dy: 0.0,
    }
}

#[test]
fn placement_on_the_reference_is_identity() {
    let f = fixture(None);
    let ids = add_synth(&f.core, "random", &["00", "01"]);
    put_truth(&f.core, ids[0], &synth_truth("random", "00"));
    let identity = StripPlacement {
        status: Placed::Found,
        scale_x: 1.0,
        scale_y: 1.0,
        dx: 0.0,
        dy: 0.0,
    };
    assert_eq!(f.core.place_on(ids[0], ids[0]).unwrap(), identity);
    // Even with no drawing at all, and without building a template.
    assert_eq!(f.core.place_on(ids[1], ids[1]).unwrap(), identity);
    assert_eq!(f.core.strip_templates_built(), 0);
}

#[test]
fn placement_follows_the_drawing() {
    let f = fixture(None);
    let dir = tempfile::tempdir().unwrap();
    let m = mixed_set(&f.core, dir.path());
    put_truth(&f.core, m.logos[0], &m.logo_truths[0]);
    let p = f.core.place_on(m.logos[0], m.logos[1]).unwrap();
    assert_eq!(p.status, Placed::Found, "{p:?}");
    let v = placed_iou(&p, edge_box(&m.logo_truths[0]), edge_box(&m.logo_truths[1]));
    assert!(v >= 0.8, "{p:?}: iou {v}");
}

/// Review Focus 1: a landscape reference placed on portrait photos of two sizes.
#[test]
fn placement_crosses_sizes() {
    let f = fixture(None);
    let ids = add_synth(&f.core, "random", &["00", "01", "03"]);
    let truth = synth_truth("random", "00");
    put_truth(&f.core, ids[0], &truth);
    for (id, name) in [(ids[1], "01"), (ids[2], "03")] {
        let p = f.core.place_on(ids[0], id).unwrap();
        assert_eq!(p.status, Placed::Found, "{name}: {p:?}");
        let v = placed_iou(&p, edge_box(&truth), edge_box(&synth_truth("random", name)));
        assert!(v >= 0.8, "{name}: {p:?}: iou {v}");
    }
}

#[test]
fn empty_drawing_is_not_found_anywhere() {
    let f = fixture(None);
    let dir = tempfile::tempdir().unwrap();
    let m = mixed_set(&f.core, dir.path());
    // No drawing at all (no draft, no guess, no group).
    assert_eq!(
        f.core.place_on(m.logos[0], m.logos[1]).unwrap(),
        not_found()
    );
    // An empty draft.
    let (w, h) = f.core.edit_size(m.logos[0]).unwrap();
    f.core
        .put_draft(m.logos[0], w, h, vec![0; (w * h) as usize])
        .unwrap();
    assert_eq!(
        f.core.place_on(m.logos[0], m.logos[1]).unwrap(),
        not_found()
    );
    // The photo is not read: one that is gone gives no error.
    let gone = dir.path().join("marked").join("logo-02.png");
    std::fs::remove_file(gone).unwrap();
    assert_eq!(
        f.core.place_on(m.logos[0], m.logos[2]).unwrap(),
        not_found()
    );
    assert!(f.core.photos().iter().all(|i| i.problem.is_none()));
}

#[test]
fn placement_on_a_photo_without_it() {
    let f = fixture(None);
    let dir = tempfile::tempdir().unwrap();
    let m = mixed_set(&f.core, dir.path());
    put_truth(&f.core, m.logos[0], &m.logo_truths[0]);
    assert_eq!(
        f.core.place_on(m.logos[0], m.plain[0]).unwrap(),
        not_found()
    );
}

/// An unsure placement keeps where it was found, so the drawing can be shown there.
#[test]
fn placement_unsure_keeps_transform() {
    let f = fixture(None);
    let ids = add_synth(&f.core, "random", &["05", "10"]);
    let truth = synth_truth("random", "05");
    put_truth(&f.core, ids[0], &truth);
    let p = f.core.place_on(ids[0], ids[1]).unwrap();
    assert_eq!(p.status, Placed::Unsure, "{p:?}");
    assert!(p.scale_x > 0.0 && p.scale_y > 0.0, "{p:?}");
    let v = placed_iou(&p, edge_box(&truth), edge_box(&synth_truth("random", "10")));
    assert!(v >= 0.6, "{p:?}: iou {v}");
}

#[test]
fn placement_on_an_unreadable_photo_fails() {
    let f = fixture(None);
    let dir = tempfile::tempdir().unwrap();
    let m = mixed_set(&f.core, dir.path());
    let broken = f.core.add_paths(&[broken_jpeg(dir.path())]).unwrap().added[0].id;
    put_truth(&f.core, m.logos[0], &m.logo_truths[0]);
    assert!(f.core.place_on(m.logos[0], broken).is_err());
    assert_eq!(state(&f.core, broken).status, AreaStatus::Unreadable);
}

#[test]
fn template_is_kept_until_the_drawing_changes() {
    let f = fixture(None);
    let dir = tempfile::tempdir().unwrap();
    let m = mixed_set(&f.core, dir.path());
    put_truth(&f.core, m.logos[0], &m.logo_truths[0]);
    f.core.place_on(m.logos[0], m.logos[1]).unwrap();
    f.core.place_on(m.logos[0], m.logos[2]).unwrap();
    assert_eq!(f.core.strip_templates_built(), 1);

    // The same drawing sent again is the same drawing.
    put_truth(&f.core, m.logos[0], &m.logo_truths[0]);
    f.core.place_on(m.logos[0], m.logos[1]).unwrap();
    assert_eq!(f.core.strip_templates_built(), 1);

    let (w, h) = f.core.edit_size(m.logos[0]).unwrap();
    f.core
        .put_draft(m.logos[0], w, h, rect_area(w, h, 60, 60, 141, 141))
        .unwrap();
    f.core.place_on(m.logos[0], m.logos[1]).unwrap();
    assert_eq!(f.core.strip_templates_built(), 2);

    // Another reference with its own drawing.
    put_truth(&f.core, m.logos[1], &m.logo_truths[1]);
    f.core.place_on(m.logos[1], m.logos[0]).unwrap();
    assert_eq!(f.core.strip_templates_built(), 3);

    // Clearing the groups forgets the template.
    f.core.clear_groups().unwrap();
    f.core.place_on(m.logos[1], m.logos[0]).unwrap();
    assert_eq!(f.core.strip_templates_built(), 4);
}

// ---- Groups from saved watermarks, and the leftovers' first guess ----

/// Saves the watermark learned from the photo file `photo` and its `truth` as `name`; returns
/// its id.
fn save_from(f: &Fixture, name: &str, photo: &Path, truth: &GrayImage) -> String {
    let img = image::open(photo).unwrap().to_rgb8();
    let w = SavedWatermark::new(
        name,
        build_template(&img, truth),
        jpeg(&RgbImage::new(4, 4), 80),
    );
    Library::open(&f.dirs.watermarks).unwrap().save(&w).unwrap();
    w.id
}

/// Saves "Studio", learned from the truth of `fixed/00`.
fn save_fixed(f: &Fixture) -> String {
    let photo = synth("fixed").join("marked").join("00.png");
    save_from(f, "Studio", &photo, &synth_truth("fixed", "00"))
}

/// Saves "Ring", learned from the truth of the first logo photo of [`mixed_set`] in `dir`.
fn save_logo(f: &Fixture, dir: &Path, m: &Mixed) -> String {
    let photo = dir.join("marked").join("logo-00.png");
    save_from(f, "Ring", &photo, &m.logo_truths[0])
}

fn ids_of(core: &AppCore) -> Vec<u32> {
    core.photos().iter().map(|p| p.id).collect()
}

fn has_looking(events: &[ProgressEvent]) -> bool {
    events
        .iter()
        .any(|e| matches!(e, ProgressEvent::Looking { .. }))
}

fn has_loading(events: &[ProgressEvent]) -> bool {
    events
        .iter()
        .any(|e| matches!(e, ProgressEvent::LoadingModels { .. }))
}

fn has_reading(events: &[ProgressEvent]) -> bool {
    events
        .iter()
        .any(|e| matches!(e, ProgressEvent::Reading { .. }))
}

#[test]
fn use_saved_starts_group_one() {
    let f = fixture(None);
    let dir = tempfile::tempdir().unwrap();
    let m = mixed_set(&f.core, dir.path());
    let id = save_fixed(&f);
    assert_eq!(f.core.use_saved(&id).unwrap(), 1);
    assert_eq!(f.core.device_status().state, DeviceState::NotStarted);
    let g = f.core.groups();
    assert_eq!(
        g.groups,
        [GroupInfo {
            number: 1,
            photos: 0,
            saved_name: Some("Studio".into()),
            edited: false,
            can_save: false,
        }]
    );
    assert!(g.current == Some(1) && !g.matched, "{g:?}");
    assert_counts(&f.core);

    f.core.match_all(&noop).unwrap();
    assert_counts(&f.core);
    for &id in &m.fixed {
        assert!(state(&f.core, id).member, "{:?}", state(&f.core, id));
    }
}

#[test]
fn use_saved_makes_the_next_group() {
    let f = fixture(None);
    let dir = tempfile::tempdir().unwrap();
    let m = mixed_set(&f.core, dir.path());
    let id = save_logo(&f, dir.path(), &m);
    group_one(&f.core, &m);
    let fixed_before = states_of(&f.core, &m.fixed);

    assert_eq!(f.core.use_saved(&id).unwrap(), 2);
    assert_counts(&f.core);
    let g = f.core.groups();
    assert_eq!(g.current, Some(2));
    let two = &g.groups[1];
    assert_eq!(two.saved_name.as_deref(), Some("Ring"));
    assert!(!two.can_save && !two.edited && !g.matched, "{g:?}");

    let rec = Recorder::default();
    f.core.match_all(&|e| rec.push(e)).unwrap();
    assert_counts(&f.core);
    let events = rec.events();
    let m_events = matching(&events);
    assert_eq!(m_events.first(), Some(&(0, 6)), "{m_events:?}");
    assert_eq!(m_events.last(), Some(&(6, 6)), "{m_events:?}");
    assert!(!has_looking(&events) && !has_loading(&events), "{events:?}");
    for &id in &m.logos {
        let st = state(&f.core, id);
        assert!(st.member && st.group == Some(2), "{st:?}");
    }
    assert_eq!(states_of(&f.core, &m.fixed), fixed_before);
    assert!(f.core.groups().matched);
}

#[test]
fn use_saved_waits() {
    let f = fixture(None);
    let dir = tempfile::tempdir().unwrap();
    let m = mixed_set(&f.core, dir.path());
    let id = save_logo(&f, dir.path(), &m);
    confirm_truth(&f.core, "fixed", "00", m.fixed[0]);
    let before = f.core.photo_states();
    assert!(matches!(f.core.use_saved(&id), Err(AppError::NotReady)));
    assert_eq!(f.core.groups().groups.len(), 1);
    assert_eq!(f.core.photo_states(), before);

    // Once matching is done it can start the next group.
    f.core.match_all(&noop).unwrap();
    settle(&f.core);
    assert_eq!(f.core.use_saved(&id).unwrap(), 2);
}

#[test]
fn saved_group_sample_is_its_best_photos() {
    let f = fixture(None);
    let dir = tempfile::tempdir().unwrap();
    let m = mixed_set(&f.core, dir.path());
    assert!(matches!(
        f.core.group_sample(),
        Err(AppError::NothingToRemove)
    ));
    let id = save_logo(&f, dir.path(), &m);
    group_one(&f.core, &m);
    f.core.use_saved(&id).unwrap();
    f.core.match_all(&noop).unwrap();

    let out = f.core.group_sample().unwrap();
    assert!(!out.sample.is_empty() && out.sample.len() <= 3, "{out:?}");
    assert!(out.sample.contains(&out.representative), "{out:?}");
    for &id in &out.sample {
        let st = state(&f.core, id);
        assert!(
            st.member && st.group == Some(2) && st.status == AreaStatus::Found,
            "{st:?}"
        );
    }
    assert_eq!(f.core.group_sample().unwrap(), out);

    // It is kept: unticking a sample photo does not change it.
    f.core.set_included(out.sample[0], false).unwrap();
    assert_eq!(f.core.group_sample().unwrap(), out);
}

#[test]
fn unchanged_saved_group_stays_saved() {
    let f = fixture(None);
    let dir = tempfile::tempdir().unwrap();
    let m = mixed_set(&f.core, dir.path());
    let id = save_logo(&f, dir.path(), &m);
    group_one(&f.core, &m);
    f.core.use_saved(&id).unwrap();
    f.core.match_all(&noop).unwrap();
    settle(&f.core);
    let rep = f.core.group_sample().unwrap().representative;
    let before = f.core.photo_states();

    let (w, h) = f.core.edit_size(rep).unwrap();
    let same = f.core.area(rep).unwrap();
    f.core.put_draft(rep, w, h, same.clone()).unwrap();
    f.core.confirm_set_area(rep).unwrap();
    let g = f.core.groups();
    assert!(!g.groups[1].edited && !g.groups[1].can_save, "{g:?}");
    assert!(g.matched, "{g:?}");
    assert_eq!(f.core.photo_states(), before);

    let mut changed = same;
    changed[0] = 255;
    f.core.put_draft(rep, w, h, changed).unwrap();
    f.core.confirm_set_area(rep).unwrap();
    assert_counts(&f.core);
    let g = f.core.groups();
    let two = &g.groups[1];
    assert!(two.edited && two.can_save, "{g:?}");
    assert_eq!(two.saved_name.as_deref(), Some("Ring"));
    assert!(!g.matched && g.current == Some(2), "{g:?}");
}

#[test]
fn suggest_looks_only_at_photos_without_a_group() {
    let f = fixture(None);
    let dir = tempfile::tempdir().unwrap();
    let m = mixed_set(&f.core, dir.path());
    save_fixed(&f);
    group_one(&f.core, &m);
    // Only the fixed watermark is saved, and no photo left has it.
    assert_eq!(f.core.suggest_saved().unwrap(), None);

    let logo_id = save_logo(&f, dir.path(), &m);
    let offer = f.core.suggest_saved().unwrap().expect("an offer");
    assert_eq!(offer.id, logo_id);
    assert_eq!(offer.name, "Ring");
}

/// R18c: the batch layers are read over the photos being matched, and only when one of them
/// of the template's size was not read for them.
#[test]
fn layers_are_read_over_the_photos_being_matched() {
    let f = fixture(None);
    let core = &f.core;
    let dir = tempfile::tempdir().unwrap();
    let m = mixed_set(core, dir.path());
    let all = ids_of(core);

    confirm_truth(core, "fixed", "00", m.fixed[0]);
    let rec = Recorder::default();
    core.match_all(&|e| rec.push(e)).unwrap();
    assert!(has_reading(&rec.events()));
    let matched: Vec<u32> = all.iter().copied().filter(|&id| id != m.fixed[0]).collect();
    assert_eq!(core.layer_photos(), matched);
    settle(core);

    // Unticked, the photo group 1 was learned from has no group; it was never read for the
    // layers and has the size of the saved fixed watermark, so they are read again, over the
    // photos with no group only.
    core.set_included(m.fixed[0], false).unwrap();
    let fixed_id = save_fixed(&f);
    assert_eq!(core.use_saved(&fixed_id).unwrap(), 2);
    let rec = Recorder::default();
    core.match_all(&|e| rec.push(e)).unwrap();
    let events = rec.events();
    assert!(has_reading(&events));
    assert_eq!(matching(&events).last(), Some(&(7, 7)));
    let leftovers: Vec<u32> = all
        .iter()
        .copied()
        .filter(|id| *id == m.fixed[0] || m.logos.contains(id) || m.plain.contains(id))
        .collect();
    assert_eq!(core.layer_photos(), leftovers);
    let st = state(core, m.fixed[0]);
    assert!(st.member && st.group == Some(2), "{st:?}");
    settle(core);

    // Group 3 matches photos that were all read: nothing is read again.
    put_truth(core, m.logos[0], &m.logo_truths[0]);
    core.confirm_new_group(m.logos[0]).unwrap();
    let rec = Recorder::default();
    core.match_all(&|e| rec.push(e)).unwrap();
    assert!(!has_reading(&rec.events()));
    assert_eq!(core.layer_photos(), leftovers);
    assert_counts(core);
}

/// The ids of the logo and plain photos, the leftovers after group 1.
fn leftovers_of(m: &Mixed) -> Vec<u32> {
    m.logos.iter().chain(&m.plain).copied().collect()
}

#[test]
fn next_group_first_guess_costs_one_sample() {
    let Some(f) = model_fixture() else { return };
    let core = &f.core;
    let dir = tempfile::tempdir().unwrap();
    let m = mixed_set(core, dir.path());
    group_one(core, &m);
    let (groups, states) = (core.groups(), core.photo_states());

    let rec = Recorder::default();
    let start = Instant::now();
    let out = core.detect(&|e| rec.push(e)).unwrap();
    let guess_took = start.elapsed();
    let events = rec.events();
    let looking: Vec<(u32, u32)> = events
        .iter()
        .filter_map(|e| match e {
            ProgressEvent::Looking { done, total } => Some((*done, *total)),
            _ => None,
        })
        .collect();
    assert!(looking.iter().all(|&(_, t)| t == 3), "{looking:?}");
    assert_eq!(looking.last(), Some(&(3, 3)), "{looking:?}");
    assert!(has_reading(&events));
    let leftovers = leftovers_of(&m);
    assert_eq!(out.sample.len(), 3, "{out:?}");
    assert!(
        out.sample.iter().all(|id| leftovers.contains(id)),
        "{out:?}"
    );
    assert!(out.sample.contains(&out.representative));
    // The batch layers come from the leftovers only.
    assert_eq!(core.layer_photos(), leftovers);
    assert_eq!(core.groups(), groups);
    assert_eq!(core.photo_states(), states);
    for &id in &out.sample {
        assert!(core.area(id).unwrap().iter().all(|&v| v == 0), "{id}");
    }
    assert!(
        out.sample
            .iter()
            .any(|&id| core.start_area(id).unwrap().iter().any(|&v| v != 0)),
        "no first guess at all"
    );
    assert_eq!(core.last_detect(), Some(out.clone()));

    let pick = out
        .sample
        .iter()
        .copied()
        .find(|id| m.logos.contains(id))
        .unwrap_or(m.logos[0]);
    let k = m.logos.iter().position(|&id| id == pick).unwrap();
    put_truth(core, pick, &m.logo_truths[k]);
    core.confirm_new_group(pick).unwrap();
    let rec = Recorder::default();
    let start = Instant::now();
    core.match_all(&|e| rec.push(e)).unwrap();
    let match_took = start.elapsed();
    let events = rec.events();
    assert_eq!(matching(&events).last(), Some(&(5, 5)), "{events:?}");
    assert!(!has_reading(&events), "the layers were read again");
    eprintln!("leftovers' first guess {guess_took:?}, matching the leftovers {match_took:?}");
    assert_counts(core);
}

/// Review Focus 5. `detect` keeps the batch layers it read before it was stopped (built from
/// the leftovers); that is not undone, as matching reads them again whenever they miss a photo
/// it matches.
#[test]
fn stopped_leftover_guess_changes_nothing() {
    let Some(f) = model_fixture() else { return };
    let core = &f.core;
    let dir = tempfile::tempdir().unwrap();
    let m = mixed_set(core, dir.path());
    group_one(core, &m);
    let first = core.detect(&noop).unwrap();
    let (groups, states, last) = (core.groups(), core.photo_states(), core.last_detect());
    assert_eq!(last, Some(first.clone()));
    let ids = ids_of(core);
    let starts: Vec<Vec<u8>> = ids.iter().map(|&id| core.start_area(id).unwrap()).collect();

    const TICKET: u64 = 41;
    let err = with_ticket(Some(TICKET), || {
        core.detect(&|e| {
            if matches!(e, ProgressEvent::Looking { .. }) {
                core.cancel_ticket(TICKET);
            }
        })
    })
    .unwrap_err();
    assert!(
        matches!(err, AppError::Core(puw_core::Error::Cancelled)),
        "{err:?}"
    );
    assert_eq!(core.photo_states(), states);
    assert_eq!(core.groups(), groups);
    assert_eq!(core.last_detect(), last);
    for (&id, start) in ids.iter().zip(&starts) {
        assert!(core.start_area(id).unwrap() == *start, "photo {id}");
    }

    // Clearing the groups keeps the first guesses and the photos they looked at.
    core.clear_groups().unwrap();
    assert_eq!(core.last_detect(), last);
    for (&id, start) in ids.iter().zip(&starts) {
        if first.sample.contains(&id) {
            assert!(core.start_area(id).unwrap() == *start, "photo {id}");
        }
    }

    // Removing a photo it looked at forgets it.
    core.remove_photo(first.sample[0]).unwrap();
    assert_eq!(core.last_detect(), None);
}

// ---- Removal and saving per group ----

#[test]
fn removal_waits_for_the_current_matching() {
    let f = fixture(None);
    let dir = tempfile::tempdir().unwrap();
    let m = mixed_set(&f.core, dir.path());
    group_one(&f.core, &m);
    put_truth(&f.core, m.logos[0], &m.logo_truths[0]);
    f.core.confirm_new_group(m.logos[0]).unwrap();
    assert!(!f.core.groups().matched);

    let rec = Recorder::default();
    assert!(matches!(
        f.core.remove(&|e| rec.push(e)),
        Err(AppError::NotReady)
    ));
    assert!(rec.events().is_empty());
}

#[test]
fn each_group_saves_its_own_watermark() {
    let f = fixture(None);
    let dir = tempfile::tempdir().unwrap();
    two_groups(&f.core, dir.path());
    let (a, b) = (
        f.core.save_watermark(1, "A").unwrap(),
        f.core.save_watermark(2, "B").unwrap(),
    );
    let lib = Library::open(&f.dirs.watermarks).unwrap();
    let saved = lib.list().unwrap();
    assert_eq!(saved.len(), 2);
    let template = |id: &str| {
        let w = saved.iter().find(|w| w.id == id).unwrap();
        (w.name.clone(), w.template.clone())
    };
    let (an, at) = template(&a);
    let (bn, bt) = template(&b);
    assert_eq!((an.as_str(), bn.as_str()), ("A", "B"));

    let fixed = image::open(synth("fixed").join("marked").join("01.png"))
        .unwrap()
        .to_rgb8();
    assert_eq!(find(&at, &fixed).status, MatchStatus::Found);
    let logo = image::open(dir.path().join("marked").join("logo-01.png"))
        .unwrap()
        .to_rgb8();
    assert_eq!(find(&bt, &logo).status, MatchStatus::Found);

    // Unknown group, and a group that came from a saved watermark: nothing to save.
    assert!(matches!(
        f.core.save_watermark(9, "C"),
        Err(AppError::NothingToRemove)
    ));
    let n = f.core.use_saved(&a).unwrap();
    assert!(matches!(
        f.core.save_watermark(n, "D"),
        Err(AppError::NothingToRemove)
    ));
}

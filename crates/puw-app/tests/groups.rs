//! Watermark groups: a set with two watermarks is handled one group at a time, and every photo
//! is always counted once (in a group, with no group yet, or as one that cannot be read).
//!
//! No test here needs models.

mod common;

use std::path::Path;
use std::sync::Arc;

use common::{
    Mixed, Recorder, add_synth, confirm_truth, fixture, logo_photos, matching, mixed_set, noop,
    put_truth, rect_area, settle, state, synth_truth,
};
use image::GrayImage;
use puw_app::images::jpeg;
use puw_app::{AppCore, AppError, AreaStatus, GroupInfo, PhotoState, Placed, StripPlacement};

/// The members of every group, plus the photos with no group yet, plus the ones that cannot be
/// read, are all the photos.
fn assert_counts(core: &AppCore) {
    let g = core.groups();
    let members: u32 = g.groups.iter().map(|i| i.photos).sum();
    assert_eq!(
        members + g.leftovers + g.unreadable,
        core.photos().len() as u32,
        "{g:?}"
    );
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
fn broken_jpeg(dir: &Path) -> std::path::PathBuf {
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

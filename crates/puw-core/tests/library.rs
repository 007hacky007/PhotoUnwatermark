mod common;

use image::RgbImage;
use puw_core::Error;
use puw_core::library::{Library, SavedWatermark};
use puw_core::mask::Mask;
use puw_core::template::{WatermarkTemplate, build_template};
use std::path::Path;

const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 1, 2, 3];

fn names(set: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(set.join("marked"))
        .expect("read dir")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "png"))
        .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

fn load(set: &Path, sub: &str, name: &str) -> RgbImage {
    image::open(set.join(sub).join(format!("{name}.png")))
        .expect("open image")
        .to_rgb8()
}

fn fixed_template() -> (WatermarkTemplate, Vec<RgbImage>) {
    let set = common::synth_dir().join("fixed");
    let ns = names(&set);
    let imgs: Vec<RgbImage> = ns.iter().map(|n| load(&set, "marked", n)).collect();
    let mask: Mask = image::open(set.join("mask").join(format!("{}.png", ns[0])))
        .expect("open mask")
        .to_luma8();
    (build_template(&imgs[0], &mask), imgs)
}

fn entry(name: &str, created: &str, id: &str, t: &WatermarkTemplate) -> SavedWatermark {
    SavedWatermark {
        id: id.to_string(),
        name: name.to_string(),
        created: created.to_string(),
        template: t.clone(),
        thumbnail_png: PNG.to_vec(),
    }
}

#[test]
fn save_list_rename_delete() {
    let (t, _) = fixed_template();
    let tmp = tempfile::tempdir().unwrap();
    let lib = Library::open(&tmp.path().join("sub/lib")).unwrap();
    assert!(lib.list().unwrap().is_empty());

    let w = SavedWatermark::new("  Studio  ", t.clone(), PNG.to_vec());
    assert!(w.created.ends_with('Z') && w.created.contains('T'));
    lib.save(&w).unwrap();
    let got = lib.list().unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].name, "Studio");
    assert_eq!(got[0].template, t);
    assert_eq!(got[0].thumbnail_png, PNG);

    lib.rename(&w.id, " Studio 2 ").unwrap();
    assert_eq!(lib.list().unwrap()[0].name, "Studio 2");
    assert!(matches!(
        lib.rename(&w.id, "   "),
        Err(Error::InvalidInput(_))
    ));
    assert_eq!(lib.list().unwrap()[0].name, "Studio 2");

    let unknown = "11111111-2222-4333-8444-555555555555";
    for r in [lib.rename(unknown, "x"), lib.delete(unknown)] {
        match r {
            Err(Error::Io(e)) => assert_eq!(e.kind(), std::io::ErrorKind::NotFound),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    lib.delete(&w.id).unwrap();
    assert!(lib.list().unwrap().is_empty());
    // No temp files left behind.
    let left: Vec<_> = std::fs::read_dir(tmp.path().join("sub/lib"))
        .unwrap()
        .collect();
    assert!(left.is_empty());
}

#[test]
fn corrupt_file_skipped() {
    let (t, _) = fixed_template();
    let tmp = tempfile::tempdir().unwrap();
    let lib = Library::open(tmp.path()).unwrap();
    lib.save(&SavedWatermark::new("ok", t, PNG.to_vec()))
        .unwrap();
    std::fs::write(tmp.path().join("garbage.json"), b"{ not json").unwrap();
    std::fs::write(
        tmp.path().join("11111111-2222-4333-8444-555555555555.json"),
        b"\xff\xfe",
    )
    .unwrap();
    std::fs::write(tmp.path().join("notes.txt"), b"hi").unwrap();
    let got = lib.list().unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].name, "ok");
}

#[test]
fn newest_first_order() {
    let (t, _) = fixed_template();
    let tmp = tempfile::tempdir().unwrap();
    let lib = Library::open(tmp.path()).unwrap();
    let a = "aaaaaaaa-0000-4000-8000-000000000000";
    let b = "bbbbbbbb-0000-4000-8000-000000000000";
    let c = "cccccccc-0000-4000-8000-000000000000";
    lib.save(&entry("old", "2026-01-01T00:00:00Z", c, &t))
        .unwrap();
    lib.save(&entry("tie-b", "2026-06-01T00:00:00Z", b, &t))
        .unwrap();
    lib.save(&entry("tie-a", "2026-06-01T00:00:00Z", a, &t))
        .unwrap();
    lib.save(&entry(
        "new",
        "2026-12-01T00:00:00Z",
        "dddddddd-0000-4000-8000-000000000000",
        &t,
    ))
    .unwrap();
    let order: Vec<String> = lib.list().unwrap().into_iter().map(|w| w.name).collect();
    assert_eq!(order, ["new", "tie-a", "tie-b", "old"]);
}

#[test]
fn ids_cannot_escape_dir() {
    let (t, _) = fixed_template();
    let tmp = tempfile::tempdir().unwrap();
    let lib_dir = tmp.path().join("lib");
    let lib = Library::open(&lib_dir).unwrap();
    let outside = tmp.path().join("x.json");
    std::fs::write(&outside, b"keep").unwrap();

    assert!(matches!(
        lib.rename("../x", "name"),
        Err(Error::InvalidInput(_))
    ));
    assert!(lib.delete("../x").is_err());
    assert!(
        lib.save(&entry("n", "2026-01-01T00:00:00Z", "../x", &t))
            .is_err()
    );
    assert!(lib.delete("..\\x").is_err());
    assert!(lib.delete("").is_err());
    assert_eq!(std::fs::read(&outside).unwrap(), b"keep");
    assert!(lib.list().unwrap().is_empty());
}

#[test]
fn suggest_matches_fixed_set() {
    let (t, imgs) = fixed_template();
    let tmp = tempfile::tempdir().unwrap();
    let lib = Library::open(tmp.path()).unwrap();
    let sample: Vec<&RgbImage> = imgs.iter().collect();
    assert_eq!(lib.suggest(&sample).unwrap(), None, "empty library");
    let w = SavedWatermark::new("mark", t, PNG.to_vec());
    lib.save(&w).unwrap();
    let (id, conf) = lib.suggest(&sample).unwrap().expect("a suggestion");
    assert_eq!(id, w.id);
    assert!(conf >= puw_core::template::FOUND, "conf {conf}");
    assert_eq!(lib.suggest(&[]).unwrap(), None, "empty sample");
}

#[test]
fn suggest_none_for_negatives() {
    let (t, _) = fixed_template();
    let neg_set = common::synth_dir().join("negative");
    let neg: Vec<RgbImage> = names(&neg_set)
        .iter()
        .map(|n| load(&neg_set, "marked", n))
        .collect();
    let tmp = tempfile::tempdir().unwrap();
    let lib = Library::open(tmp.path()).unwrap();
    lib.save(&SavedWatermark::new("mark", t, PNG.to_vec()))
        .unwrap();
    let sample: Vec<&RgbImage> = neg.iter().collect();
    assert_eq!(lib.suggest(&sample).unwrap(), None);
}

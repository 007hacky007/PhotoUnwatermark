use std::fs;
use std::path::{Path, PathBuf};

use image::{GrayImage, Luma, Rgb, RgbImage};
use puw_app::images::{
    EDIT_LONG_SIDE, area_from_edit, area_to_edit, edit_image, edit_size, jpeg, overlay, thumbnail,
};
use puw_app::photos::PhotoList;
use puw_app::{AppError, OutputRule, Result};
use puw_core::Error;
use puw_core::image_io::load;

const OUT: &str = "Without watermark";

fn next_to(name: &str) -> OutputRule {
    OutputRule::NextToOriginals {
        folder_name: name.into(),
    }
}

fn exif_orientation(orientation: u16, thumb: Option<&[u8]>) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(b"II*\0");
    b.extend_from_slice(&8u32.to_le_bytes());
    // IFD0: Orientation only
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&0x0112u16.to_le_bytes());
    b.extend_from_slice(&3u16.to_le_bytes());
    b.extend_from_slice(&1u32.to_le_bytes());
    b.extend_from_slice(&orientation.to_le_bytes());
    b.extend_from_slice(&[0, 0]);
    match thumb {
        None => b.extend_from_slice(&0u32.to_le_bytes()),
        Some(t) => {
            b.extend_from_slice(&26u32.to_le_bytes());
            // IFD1: JPEGInterchangeFormat and its length; data follows at 56
            b.extend_from_slice(&2u16.to_le_bytes());
            for (tag, val) in [(0x0201u16, 56u32), (0x0202, t.len() as u32)] {
                b.extend_from_slice(&tag.to_le_bytes());
                b.extend_from_slice(&4u16.to_le_bytes());
                b.extend_from_slice(&1u32.to_le_bytes());
                b.extend_from_slice(&val.to_le_bytes());
            }
            b.extend_from_slice(&0u32.to_le_bytes());
            assert_eq!(b.len(), 56);
            b.extend_from_slice(t);
        }
    }
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

fn plain(w: u32, h: u32) -> RgbImage {
    RgbImage::from_pixel(w, h, Rgb([120, 130, 140]))
}

fn names(list: &PhotoList) -> Vec<String> {
    list.infos().into_iter().map(|i| i.name).collect()
}

#[test]
fn adds_files_and_folders() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("trip");
    fs::create_dir(&folder).unwrap();
    write_jpeg(&folder.join("b.JPG"), &plain(40, 30), None);
    write_jpeg(&folder.join("a.jpeg"), &plain(40, 30), None);
    fs::write(folder.join("notes.txt"), "x").unwrap();
    fs::create_dir(folder.join("deeper")).unwrap();
    write_jpeg(&folder.join("deeper").join("c.jpg"), &plain(40, 30), None);
    let single = dir.path().join("single.png");
    plain(20, 10).save(&single).unwrap();

    let mut list = PhotoList::default();
    let r = list.add_paths(&[folder, single], &next_to(OUT));
    assert!(r.skipped.is_empty(), "{:?}", r.skipped);
    assert_eq!(names(&list), ["a.jpeg", "b.JPG", "single.png"]);
    let infos = list.infos();
    assert_eq!(infos[0].folder, "trip");
    assert_eq!((infos[2].width, infos[2].height), (20, 10));
    let ids: Vec<_> = infos.iter().map(|i| i.id).collect();
    assert_eq!(ids.len(), 3);
    assert!(ids[0] != ids[1] && ids[1] != ids[2]);
    assert_eq!(list.paths().len(), 3);
    assert_eq!(list.get(ids[1]).unwrap().info.name, "b.JPG");

    list.remove(ids[1]);
    assert_eq!(names(&list), ["a.jpeg", "single.png"]);
    assert!(list.get(ids[1]).is_none());
    list.clear();
    assert!(list.infos().is_empty());
}

fn skipped_names(r: &puw_app::photos::AddResult) -> Vec<String> {
    r.skipped.iter().map(|s| s.name.clone()).collect()
}

#[test]
fn scan_skips_output_folder() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join(OUT);
    fs::create_dir(&out).unwrap();
    write_jpeg(&out.join("done.jpg"), &plain(10, 10), None);
    write_jpeg(&dir.path().join("keep.jpg"), &plain(10, 10), None);

    // A scanned folder lists only its own files; the output folder inside it is not opened.
    let mut list = PhotoList::default();
    let r = list.add_paths(&[dir.path().to_path_buf()], &next_to(OUT));
    assert_eq!(names(&list), ["keep.jpg"]);
    assert!(r.skipped.is_empty());

    // Named directly, a result or the output folder is left out with a reason.
    let mut list = PhotoList::default();
    let r = list.add_paths(&[out.join("done.jpg"), out.clone()], &next_to(OUT));
    assert!(r.added.is_empty());
    assert_eq!(skipped_names(&r), ["done.jpg", OUT]);
    assert!(
        r.skipped[0].reason.contains("finished photos"),
        "{:?}",
        r.skipped
    );
    assert!(r.skipped[0].reason.contains(OUT));
    assert!(r.skipped[1].reason.starts_with("This folder was left out"));
    assert!(list.infos().is_empty());

    let nested = out.join("x");
    fs::create_dir(&nested).unwrap();
    write_jpeg(&nested.join("a.jpg"), &plain(10, 10), None);
    let r = list.add_paths(&[nested.join("a.jpg"), nested.clone()], &next_to(OUT));
    assert!(r.added.is_empty());
    assert_eq!(skipped_names(&r), ["a.jpg", "x"]);

    #[cfg(unix)]
    {
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&out, &link).unwrap();
        let r = list.add_paths(&[link], &next_to(OUT));
        assert!(r.added.is_empty());
        assert_eq!(skipped_names(&r), ["link"]);
    }

    if cfg!(any(target_os = "macos", windows)) {
        let r = list.add_paths(&[out.join("done.jpg")], &next_to("WITHOUT WATERMARK"));
        assert!(r.added.is_empty());
        assert_eq!(r.skipped.len(), 1);
    }
    for s in &r.skipped {
        assert!(s.reason.is_ascii());
    }
}

/// "Always save to <folder>": only what is inside that very folder is left out, never photos
/// in other folders of the same name.
#[test]
fn chosen_folder_skips_only_itself() {
    let dir = tempfile::tempdir().unwrap();
    let chosen = dir.path().join("home").join("Pictures");
    let elsewhere = dir.path().join("card").join("Pictures");
    let below = dir.path().join("home").join("Pictures").join("trip");
    for d in [&chosen, &elsewhere, &below] {
        fs::create_dir_all(d).unwrap();
    }
    write_jpeg(&chosen.join("done.jpg"), &plain(10, 10), None);
    write_jpeg(&elsewhere.join("photo.jpg"), &plain(10, 10), None);
    write_jpeg(&below.join("deep.jpg"), &plain(10, 10), None);
    let rule = OutputRule::Folder {
        path: chosen.clone(),
    };

    let mut list = PhotoList::default();
    let r = list.add_paths(
        &[
            elsewhere.clone(),
            chosen.join("done.jpg"),
            below.clone(),
            chosen.clone(),
        ],
        &rule,
    );
    assert_eq!(names(&list), ["photo.jpg"]);
    assert_eq!(skipped_names(&r), ["done.jpg", "trip", "Pictures"]);
    assert!(
        r.skipped
            .iter()
            .all(|s| s.reason.contains("finished photos are saved"))
    );

    // The same folder spelled another way is still the chosen one.
    let other = dir
        .path()
        .join("home")
        .join("..")
        .join("home")
        .join("Pictures");
    let r = list.add_paths(&[other.join("done.jpg")], &rule);
    assert_eq!(skipped_names(&r), ["done.jpg"]);

    // A chosen folder that does not exist yet leaves nothing out.
    let r = list.add_paths(
        &[below.clone()],
        &OutputRule::Folder {
            path: dir.path().join("not yet"),
        },
    );
    assert_eq!(names(&list), ["photo.jpg", "deep.jpg"]);
    assert!(r.skipped.is_empty());

    // Next to the originals, a folder named like the output folder is left out by name.
    let mut list = PhotoList::default();
    let r = list.add_paths(&[elsewhere.join("photo.jpg")], &next_to("pictures"));
    if cfg!(any(target_os = "macos", windows)) {
        assert_eq!(skipped_names(&r), ["photo.jpg"]);
    }
}

#[test]
fn shown_paths_have_no_verbatim_prefix() {
    use puw_app::photos::plain_path;
    use std::path::Path;
    let p = |s: &str| plain_path(Path::new(s)).to_string_lossy().into_owned();
    assert_eq!(p(r"\\?\C:\bench\t2"), r"C:\bench\t2");
    assert_eq!(p(r"\\?\UNC\server\share\a.jpg"), r"\\server\share\a.jpg");
    assert_eq!(p(r"C:\already\plain"), r"C:\already\plain");
    assert_eq!(p("/Users/a/b.jpg"), "/Users/a/b.jpg");
    // Kept where only the prefixed form works.
    assert_eq!(p(r"\\?\C:\dir\CON"), r"\\?\C:\dir\CON");
    assert_eq!(p(r"\\?\C:\dir\name."), r"\\?\C:\dir\name.");
    let long = format!(r"\\?\C:\{}", "a".repeat(300));
    assert_eq!(p(&long), long);
    assert_eq!(p(r"\\?\Volume{x}\a"), r"\\?\Volume{x}\a");
}

#[cfg(windows)]
#[test]
fn real_paths_on_windows_are_plain() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.jpg");
    write_jpeg(&a, &plain(10, 10), None);
    let real = puw_app::photos::real_path(&a).unwrap();
    assert!(!real.to_string_lossy().starts_with(r"\\?\"), "{real:?}");
    let mut list = PhotoList::default();
    list.add_paths(&[a], &next_to(OUT));
    assert!(!list.paths()[0].to_string_lossy().starts_with(r"\\?\"));
}

#[test]
fn dedupes_same_file() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("s");
    fs::create_dir(&sub).unwrap();
    let a = sub.join("a.jpg");
    write_jpeg(&a, &plain(10, 10), None);
    let other_spelling = sub.join("..").join("s").join("a.jpg");

    let mut list = PhotoList::default();
    list.add_paths(&[a.clone(), other_spelling.clone()], &next_to(OUT));
    assert_eq!(list.infos().len(), 1);
    let r = list.add_paths(&[other_spelling, sub], &next_to(OUT));
    assert!(r.added.is_empty());
    assert_eq!(list.infos().len(), 1);
}

#[test]
fn skips_unsupported() {
    let dir = tempfile::tempdir().unwrap();
    let txt = dir.path().join("a.txt");
    fs::write(&txt, "hello").unwrap();
    let heic = dir.path().join("b.heic");
    fs::write(&heic, "x").unwrap();
    let good = dir.path().join("good.jpg");
    write_jpeg(&good, &plain(64, 64), None);
    let bytes = fs::read(&good).unwrap();
    let broken = dir.path().join("broken.jpg");
    fs::write(&broken, &bytes[..20]).unwrap();
    let gone = dir.path().join("gone.jpg");

    let mut list = PhotoList::default();
    let r = list.add_paths(&[txt, heic, broken, good, gone], &next_to(OUT));
    assert_eq!(names(&list), ["good.jpg"]);
    let reason = |n: &str| {
        r.skipped
            .iter()
            .find(|s| s.name == n)
            .unwrap_or_else(|| panic!("{n} not skipped"))
            .reason
            .clone()
    };
    assert!(reason("a.txt").contains("not supported"));
    assert!(reason("b.heic").starts_with("HEIC photos are not supported yet."));
    assert!(reason("broken.jpg").contains("could not be read"));
    assert!(reason("gone.jpg").contains("could not be found"));
    for s in &r.skipped {
        assert!(s.reason.is_ascii());
    }
}

#[test]
fn skips_heic_by_content_in_a_jpg_name() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("x.jpg");
    let mut b = vec![0, 0, 0, 24];
    b.extend_from_slice(b"ftypheic");
    b.extend_from_slice(&[0; 16]);
    fs::write(&p, b).unwrap();
    let mut list = PhotoList::default();
    let r = list.add_paths(&[p], &next_to(OUT));
    assert!(r.skipped[0].reason.contains("HEIC"));
}

/// 600x400 stored with orientation 6 (shown rotated 90 degrees clockwise: upright 400x600);
/// the upright top-left corner is stored bottom-left.
fn rotated_fixture(path: &Path, thumb: Option<&[u8]>) {
    let mut img = plain(600, 400);
    for y in 300..400 {
        for x in 0..100 {
            img.put_pixel(x, y, Rgb([255, 0, 0]));
        }
    }
    write_jpeg(path, &img, Some(exif_orientation(6, thumb)));
}

fn is_red(p: &Rgb<u8>) -> bool {
    p.0[0] > 200 && p.0[1] < 80 && p.0[2] < 80
}

#[test]
fn edit_image_is_upright() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("r.jpg");
    rotated_fixture(&p, None);
    let mut list = PhotoList::default();
    list.add_paths(std::slice::from_ref(&p), &next_to(OUT));
    let info = &list.infos()[0];
    assert_eq!((info.width, info.height), (400, 600));
    let photo = load(&p).unwrap();
    let edit = edit_image(&photo);
    assert_eq!(edit.dimensions(), (400, 600));
    assert!(is_red(edit.get_pixel(20, 20)));
    assert!(!is_red(edit.get_pixel(380, 580)));
}

#[test]
fn edit_size_caps() {
    assert_eq!(edit_size(6000, 4000), (4096, 2731));
    assert_eq!(edit_size(4000, 6000), (2731, 4096));
    assert_eq!(edit_size(800, 600), (800, 600));
    assert_eq!(edit_size(EDIT_LONG_SIDE, 10), (EDIT_LONG_SIDE, 10));
    assert_eq!(edit_size(100_000, 3), (4096, 1));
    let big = puw_core::image_io::LoadedPhoto {
        path: PathBuf::new(),
        rgb: plain(5000, 2500),
        alpha: None,
        format: image::ImageFormat::Jpeg,
        exif: None,
        icc: None,
    };
    assert_eq!(edit_image(&big).dimensions(), (4096, 2048));
}

#[test]
fn area_roundtrip_is_conservative() {
    let (w, h, ew, eh) = (6000, 4000, 4096, 2731);
    let mut full = GrayImage::new(w, h);
    full.put_pixel(3001, 1777, Luma([255]));
    let small = area_to_edit(&full, ew, eh);
    assert_eq!(small.len(), (ew * eh) as usize);
    assert!(small.iter().all(|&v| v == 0 || v == 255));
    assert!(small.contains(&255));
    let back = area_from_edit(&small, ew, eh, w, h).unwrap();
    assert_eq!(back.dimensions(), (w, h));
    assert_eq!(back.get_pixel(3001, 1777).0[0], 255);
    assert!(back.as_raw().iter().all(|&v| v == 0 || v == 255));
}

#[test]
fn area_from_edit_rejects_wrong_length() {
    let r: Result<_> = area_from_edit(&[0; 10], 4, 4, 8, 8);
    assert!(matches!(r, Err(AppError::Core(Error::InvalidInput(_)))));
}

#[test]
fn thumbnail_is_upright() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("r.jpg");
    rotated_fixture(&p, None);
    let t = thumbnail(&p, 320).unwrap();
    assert_eq!(t.dimensions(), (213, 320));
    assert!(is_red(t.get_pixel(10, 10)));
    assert!(!is_red(t.get_pixel(200, 300)));
}

#[test]
fn thumbnail_uses_embedded_when_large_enough() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("r.jpg");
    // Embedded thumbnail (stored 480x320, all green) differs from the photo, so it shows which won.
    let green = RgbImage::from_pixel(480, 320, Rgb([0, 200, 0]));
    rotated_fixture(&p, Some(&jpeg(&green, 90)));
    let t = thumbnail(&p, 320).unwrap();
    assert_eq!(t.dimensions(), (213, 320));
    let px = t.get_pixel(100, 100);
    assert!(px.0[1] > 150 && px.0[0] < 60, "{px:?}");
    // Too small for the request: decode the photo instead.
    let t = thumbnail(&p, 600).unwrap();
    assert_eq!(t.dimensions(), (400, 600));
    assert!(is_red(t.get_pixel(10, 10)));
}

#[test]
fn overlay_tints_only_area() {
    let img = RgbImage::from_pixel(8, 8, Rgb([0, 0, 0]));
    let mut area = GrayImage::new(4, 4);
    area.put_pixel(0, 0, Luma([255]));
    let out = overlay(&img, &area);
    // Area resized to the image: top-left 2x2 covered.
    assert_eq!(out.get_pixel(0, 0).0, [115, 27, 22]);
    assert_eq!(out.get_pixel(1, 1).0, [115, 27, 22]);
    assert_eq!(out.get_pixel(2, 2).0, [0, 0, 0]);
    assert_eq!(out.get_pixel(7, 7).0, [0, 0, 0]);
}

#[test]
fn jpeg_bytes_decode() {
    let b = jpeg(&plain(16, 8), 80);
    assert_eq!(&b[..2], &[0xFF, 0xD8]);
    assert_eq!(image::load_from_memory(&b).unwrap().width(), 16);
}

use std::fs;
use std::path::Path;

use image::{GrayImage, ImageFormat, Rgb, RgbImage, Rgba, RgbaImage};
use puw_core::Error;
use puw_core::image_io::{load, output_dir_for, save_result};

/// Minimal little-endian TIFF/EXIF block with Make and Orientation.
fn exif_blob(orientation: u16) -> Vec<u8> {
    let make = b"PUWCam\0\0"; // 8 bytes, stored after the IFD
    let mut b = Vec::new();
    b.extend_from_slice(b"II*\0");
    b.extend_from_slice(&8u32.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    // Make (0x010F), ASCII, count 7, offset 38
    b.extend_from_slice(&0x010Fu16.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&7u32.to_le_bytes());
    b.extend_from_slice(&38u32.to_le_bytes());
    // Orientation (0x0112), SHORT, count 1
    b.extend_from_slice(&0x0112u16.to_le_bytes());
    b.extend_from_slice(&3u16.to_le_bytes());
    b.extend_from_slice(&1u32.to_le_bytes());
    b.extend_from_slice(&orientation.to_le_bytes());
    b.extend_from_slice(&[0, 0]);
    b.extend_from_slice(&0u32.to_le_bytes());
    assert_eq!(b.len(), 38);
    b.extend_from_slice(make);
    b
}

fn read_exif(bytes: &[u8]) -> exif::Exif {
    exif::Reader::new().read_raw(bytes.to_vec()).unwrap()
}

fn orientation_of(exif_bytes: &[u8]) -> u32 {
    read_exif(exif_bytes)
        .get_field(exif::Tag::Orientation, exif::In::PRIMARY)
        .unwrap()
        .value
        .get_uint(0)
        .unwrap()
}

fn gradient(w: u32, h: u32) -> RgbImage {
    RgbImage::from_fn(w, h, |x, y| {
        Rgb([(x * 255 / (w - 1)) as u8, (y * 255 / (h - 1)) as u8, 90])
    })
}

fn jpeg_bytes(img: &RgbImage) -> Vec<u8> {
    let mut out = Vec::new();
    let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 98);
    image::ImageEncoder::write_image(
        enc,
        img.as_raw(),
        img.width(),
        img.height(),
        image::ExtendedColorType::Rgb8,
    )
    .unwrap();
    out
}

fn with_meta(jpeg: Vec<u8>, exif: Option<Vec<u8>>, icc: Option<Vec<u8>>) -> Vec<u8> {
    use img_parts::{Bytes, ImageEXIF, ImageICC};
    let mut j = img_parts::jpeg::Jpeg::from_bytes(Bytes::from(jpeg)).unwrap();
    j.set_exif(exif.map(Bytes::from));
    j.set_icc_profile(icc.map(Bytes::from));
    let mut out = Vec::new();
    j.encoder().write_to(&mut out).unwrap();
    out
}

fn max_diff(a: &RgbImage, b: &RgbImage) -> u8 {
    assert_eq!(a.dimensions(), b.dimensions());
    a.pixels()
        .zip(b.pixels())
        .flat_map(|(p, q)| p.0.iter().zip(q.0).map(|(x, y)| x.abs_diff(y)))
        .max()
        .unwrap()
}

#[test]
fn orientation_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    // `stored` is what the camera wrote; `upright` what a viewer shows.
    let upright_src = gradient(40, 24);
    for (orientation, transform) in [
        (
            3u16,
            image::imageops::rotate180 as fn(&RgbImage) -> RgbImage,
        ),
        (6, image::imageops::rotate270), // stored = upright rotated CCW
        (8, image::imageops::rotate90),  // stored = upright rotated CW
    ] {
        let stored = transform(&upright_src);
        let path = dir.path().join(format!("o{orientation}.jpg"));
        fs::write(
            &path,
            with_meta(jpeg_bytes(&stored), Some(exif_blob(orientation)), None),
        )
        .unwrap();

        let photo = load(&path).unwrap();
        assert_eq!(
            photo.rgb.dimensions(),
            upright_src.dimensions(),
            "o{orientation}"
        );
        assert!(
            max_diff(&photo.rgb, &upright_src) < 24,
            "o{orientation} upright"
        );

        let out = save_result(&photo, &photo.rgb, &dir.path().join("out")).unwrap();
        let saved_bytes = fs::read(&out).unwrap();
        let saved = load(&out).unwrap();
        assert_eq!(orientation_of(saved.exif.as_ref().unwrap()), 1);
        assert_eq!(saved.rgb.dimensions(), upright_src.dimensions());
        assert!(
            max_diff(&saved.rgb, &photo.rgb) < 16,
            "o{orientation} saved"
        );
        // EXIF reader on the whole file agrees.
        let mut cur = std::io::Cursor::new(saved_bytes);
        let whole = exif::Reader::new().read_from_container(&mut cur).unwrap();
        let o = whole
            .get_field(exif::Tag::Orientation, exif::In::PRIMARY)
            .unwrap();
        assert_eq!(o.value.get_uint(0), Some(1));
    }
}

#[test]
fn exif_and_icc_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("m.jpg");
    let icc = fake_icc(b"RGB ");
    fs::write(
        &path,
        with_meta(
            jpeg_bytes(&gradient(16, 16)),
            Some(exif_blob(1)),
            Some(icc.clone()),
        ),
    )
    .unwrap();
    let photo = load(&path).unwrap();
    assert_eq!(photo.icc.as_deref(), Some(icc.as_slice()));
    let out = save_result(&photo, &photo.rgb, &dir.path().join("out")).unwrap();
    let saved = load(&out).unwrap();
    assert_eq!(saved.icc.as_deref(), Some(icc.as_slice()));
    let make = read_exif(saved.exif.as_ref().unwrap())
        .get_field(exif::Tag::Make, exif::In::PRIMARY)
        .unwrap()
        .display_value()
        .to_string();
    assert!(make.contains("PUWCam"), "{make}");
}

#[test]
fn loads_unusual_formats() {
    let dir = tempfile::tempdir().unwrap();

    // Grayscale JPEG.
    let gray_path = dir.path().join("gray.jpg");
    let gray = GrayImage::from_fn(20, 10, |x, _| image::Luma([(x * 12) as u8]));
    gray.save_with_format(&gray_path, ImageFormat::Jpeg)
        .unwrap();
    let p = load(&gray_path).unwrap();
    assert_eq!(p.rgb.dimensions(), (20, 10));
    assert!(p.alpha.is_none());

    // CMYK JPEG (built with jpeg-encoder: image cannot encode CMYK).
    let cmyk_path = dir.path().join("cmyk.jpg");
    let (w, h) = (16u16, 8u16);
    let data: Vec<u8> = (0..w * h).flat_map(|_| [0u8, 0, 0, 0]).collect(); // no ink = white
    let mut out = Vec::new();
    jpeg_encoder::Encoder::new(&mut out, 95)
        .encode(&data, w, h, jpeg_encoder::ColorType::Cmyk)
        .unwrap();
    fs::write(&cmyk_path, out).unwrap();
    let p = load(&cmyk_path).unwrap();
    assert_eq!(p.rgb.dimensions(), (16, 8));
    // No ink must come out near-white (mind Adobe's inverted CMYK).
    for px in p.rgb.pixels() {
        assert!(px.0.iter().all(|&c| c > 235), "not white: {px:?}");
    }

    // 16-bit PNG.
    let png_path = dir.path().join("deep.png");
    let deep: image::ImageBuffer<Rgb<u16>, Vec<u16>> =
        image::ImageBuffer::from_fn(8, 8, |x, _| Rgb([(x * 8000) as u16, 40000, 65535]));
    deep.save_with_format(&png_path, ImageFormat::Png).unwrap();
    let p = load(&png_path).unwrap();
    assert_eq!(p.format, ImageFormat::Png);
    assert_eq!(p.rgb.get_pixel(0, 0).0[2], 255);
}

#[test]
fn heic_and_garbage_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let heic = dir.path().join("x.heic");
    let mut b = vec![0, 0, 0, 24];
    b.extend_from_slice(b"ftypheic");
    b.extend_from_slice(&[0; 12]);
    fs::write(&heic, b).unwrap();
    assert!(matches!(load(&heic), Err(Error::UnsupportedFormat(s)) if s == "heic"));

    let junk = dir.path().join("junk.jpg");
    fs::write(&junk, b"not an image at all").unwrap();
    assert!(matches!(load(&junk), Err(Error::Image(_))));
}

#[test]
fn png_alpha_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.png");
    RgbaImage::from_fn(10, 6, |x, y| Rgba([200, 100, 50, (x * 25 + y) as u8]))
        .save(&path)
        .unwrap();
    let photo = load(&path).unwrap();
    let alpha = photo.alpha.as_ref().expect("alpha");
    assert_eq!(alpha.get_pixel(4, 2).0[0], 102);

    let out = save_result(&photo, &photo.rgb, &dir.path().join("out")).unwrap();
    let saved = image::open(&out).unwrap().to_rgba8();
    for (x, y, p) in saved.enumerate_pixels() {
        assert_eq!(p.0[3], (x * 25 + y) as u8);
        assert_eq!(&p.0[..3], &[200, 100, 50]);
    }
}

fn hash(p: &Path) -> Vec<u8> {
    fs::read(p).unwrap()
}

#[test]
fn save_is_atomic_and_unique() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("a.jpg");
    fs::write(&src, jpeg_bytes(&gradient(16, 16))).unwrap();
    let before = hash(&src);
    let photo = load(&src).unwrap();

    let out_dir = dir.path().join("deep/new/out");
    fs::create_dir_all(&out_dir).unwrap();
    fs::write(out_dir.join("a.jpg"), b"existing").unwrap();

    let first = save_result(&photo, &photo.rgb, &out_dir).unwrap();
    assert_eq!(first.file_name().unwrap(), "a (2).jpg");
    let second = save_result(&photo, &photo.rgb, &out_dir).unwrap();
    assert_eq!(second.file_name().unwrap(), "a (3).jpg");
    assert_eq!(fs::read(out_dir.join("a.jpg")).unwrap(), b"existing");

    let leftovers: Vec<_> = fs::read_dir(&out_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains("partial"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    for f in [&first, &second] {
        assert!(fs::metadata(f).unwrap().len() > 0, "empty output {f:?}");
    }
    assert_eq!(hash(&src), before);

    // Missing out_dir is created.
    let made = save_result(&photo, &photo.rgb, &dir.path().join("fresh")).unwrap();
    assert_eq!(made.file_name().unwrap(), "a.jpg");
}

#[test]
fn output_dir_name() {
    assert_eq!(
        output_dir_for(Path::new("/photos/trip/a.jpg")),
        Path::new("/photos/trip/Without watermark")
    );
}

#[test]
fn webp_and_tiff_roundtrip_with_metadata() {
    use img_parts::{Bytes, ImageEXIF, ImageICC};
    let dir = tempfile::tempdir().unwrap();
    let img = gradient(12, 12);

    let webp = dir.path().join("w.webp");
    let mut buf = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img.clone())
        .write_to(&mut buf, ImageFormat::WebP)
        .unwrap();
    let mut w = img_parts::webp::WebP::from_bytes(Bytes::from(buf.into_inner())).unwrap();
    w.set_exif(Some(Bytes::from(exif_blob(6))));
    w.set_icc_profile(Some(Bytes::from(fake_icc(b"RGB "))));
    let mut raw = Vec::new();
    w.encoder().write_to(&mut raw).unwrap();
    fs::write(&webp, raw).unwrap();

    let photo = load(&webp).unwrap();
    assert_eq!(photo.rgb.dimensions(), (12, 12));
    let out = save_result(&photo, &photo.rgb, &dir.path().join("out")).unwrap();
    let saved = load(&out).unwrap();
    assert_eq!(saved.format, ImageFormat::WebP);
    assert_eq!(saved.icc, Some(fake_icc(b"RGB ")));
    assert_eq!(orientation_of(saved.exif.as_ref().unwrap()), 1);

    let tiff = dir.path().join("t.tiff");
    img.save_with_format(&tiff, ImageFormat::Tiff).unwrap();
    let photo = load(&tiff).unwrap();
    let out = save_result(&photo, &photo.rgb, &dir.path().join("out")).unwrap();
    assert_eq!(out.file_name().unwrap(), "t.tiff");
    assert_eq!(max_diff(&load(&out).unwrap().rgb, &img), 0);
}

/// A syntactically plausible ICC header with the given colour space.
fn fake_icc(space: &[u8; 4]) -> Vec<u8> {
    let mut v = vec![0u8; 128];
    v[16..20].copy_from_slice(space);
    v.extend_from_slice(b"profile-body");
    v
}

#[test]
fn icc_dropped_when_colour_model_changes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("g.jpg");
    let gray = GrayImage::from_fn(16, 16, |x, _| image::Luma([(x * 15) as u8]));
    let mut raw = Vec::new();
    image::ImageEncoder::write_image(
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut raw, 95),
        gray.as_raw(),
        16,
        16,
        image::ExtendedColorType::L8,
    )
    .unwrap();
    fs::write(&path, with_meta(raw, None, Some(fake_icc(b"GRAY")))).unwrap();
    let photo = load(&path).unwrap();
    assert!(photo.icc.is_some());
    let out = save_result(&photo, &photo.rgb, &dir.path().join("out")).unwrap();
    assert!(load(&out).unwrap().icc.is_none());

    // An RGB profile on an RGB source is kept.
    let rgb_path = dir.path().join("c.jpg");
    fs::write(
        &rgb_path,
        with_meta(jpeg_bytes(&gradient(16, 16)), None, Some(fake_icc(b"RGB "))),
    )
    .unwrap();
    let photo = load(&rgb_path).unwrap();
    let out = save_result(&photo, &photo.rgb, &dir.path().join("out")).unwrap();
    assert_eq!(load(&out).unwrap().icc, Some(fake_icc(b"RGB ")));
}

fn tiff_with_tags(img: &RgbImage, orientation: u16, icc: &[u8]) -> Vec<u8> {
    use tiff::encoder::{TiffEncoder, colortype};
    use tiff::tags::Tag;
    let mut out = std::io::Cursor::new(Vec::new());
    {
        let mut enc = TiffEncoder::new(&mut out).unwrap();
        let mut im = enc
            .new_image::<colortype::RGB8>(img.width(), img.height())
            .unwrap();
        im.encoder()
            .write_tag(Tag::Orientation, orientation)
            .unwrap();
        im.encoder().write_tag(Tag::IccProfile, icc).unwrap();
        im.write_data(img.as_raw()).unwrap();
    }
    out.into_inner()
}

#[test]
fn tiff_orientation_and_icc() {
    let dir = tempfile::tempdir().unwrap();
    let upright = gradient(40, 24);
    let stored = image::imageops::rotate270(&upright); // orientation 6
    let icc = fake_icc(b"RGB ");
    let path = dir.path().join("t.tif");
    fs::write(&path, tiff_with_tags(&stored, 6, &icc)).unwrap();

    let photo = load(&path).unwrap();
    assert_eq!(photo.rgb.dimensions(), (40, 24));
    assert_eq!(max_diff(&photo.rgb, &upright), 0);
    assert_eq!(photo.icc.as_deref(), Some(icc.as_slice()));

    let out = save_result(&photo, &photo.rgb, &dir.path().join("out")).unwrap();
    let saved = load(&out).unwrap();
    assert_eq!(saved.rgb.dimensions(), (40, 24));
    assert_eq!(max_diff(&saved.rgb, &upright), 0);
    assert_eq!(saved.icc.as_deref(), Some(icc.as_slice()));
    let mut dec =
        tiff::decoder::Decoder::new(std::io::Cursor::new(fs::read(&out).unwrap())).unwrap();
    let o = dec
        .find_tag_unsigned::<u16>(tiff::tags::Tag::Orientation)
        .unwrap();
    assert!(matches!(o, None | Some(1)), "{o:?}");
}

#[test]
fn tiff_alpha_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.tiff");
    RgbaImage::from_fn(9, 5, |x, y| Rgba([10, 20, 30, (x * 20 + y) as u8]))
        .save_with_format(&path, ImageFormat::Tiff)
        .unwrap();
    let photo = load(&path).unwrap();
    assert!(photo.alpha.is_some());
    let out = save_result(&photo, &photo.rgb, &dir.path().join("out")).unwrap();
    let saved = image::open(&out).unwrap().to_rgba8();
    for (x, y, p) in saved.enumerate_pixels() {
        assert_eq!(p.0, [10, 20, 30, (x * 20 + y) as u8]);
    }
}

/// Camera-style EXIF: IFD0 (Make, Orientation, Exif IFD pointer), an Exif IFD with
/// PixelXDimension (LONG) and PixelYDimension (SHORT), and an IFD1 with a JPEG thumbnail.
fn camera_exif(le: bool, orientation: u16, px: u32, py: u16, thumb: &[u8]) -> Vec<u8> {
    let mut b = Vec::new();
    let u16b = |v: u16| if le { v.to_le_bytes() } else { v.to_be_bytes() };
    let u32b = |v: u32| if le { v.to_le_bytes() } else { v.to_be_bytes() };
    // (tag, type, count, value bytes left-justified in 4)
    let entry = |b: &mut Vec<u8>, tag: u16, ty: u16, count: u32, value: [u8; 4]| {
        b.extend_from_slice(&u16b(tag));
        b.extend_from_slice(&u16b(ty));
        b.extend_from_slice(&u32b(count));
        b.extend_from_slice(&value);
    };
    let short = |v: u16| {
        let s = u16b(v);
        [s[0], s[1], 0, 0]
    };
    b.extend_from_slice(if le { b"II" } else { b"MM" });
    b.extend_from_slice(&u16b(42));
    b.extend_from_slice(&u32b(8));
    // IFD0 at 8: 3 entries, ends at 50; Make data at 50..58; Exif IFD at 58..88; IFD1 at 88..130.
    b.extend_from_slice(&u16b(3));
    entry(&mut b, 0x010F, 2, 7, u32b(50));
    entry(&mut b, 0x0112, 3, 1, short(orientation));
    entry(&mut b, 0x8769, 4, 1, u32b(58));
    b.extend_from_slice(&u32b(88));
    assert_eq!(b.len(), 50);
    b.extend_from_slice(b"PUWCam\0\0");
    b.extend_from_slice(&u16b(2));
    entry(&mut b, 0xA002, 4, 1, u32b(px));
    entry(&mut b, 0xA003, 3, 1, short(py));
    b.extend_from_slice(&u32b(0));
    assert_eq!(b.len(), 88);
    b.extend_from_slice(&u16b(3));
    entry(&mut b, 0x0103, 3, 1, short(6));
    entry(&mut b, 0x0201, 4, 1, u32b(130));
    entry(&mut b, 0x0202, 4, 1, u32b(thumb.len() as u32));
    b.extend_from_slice(&u32b(0));
    assert_eq!(b.len(), 130);
    b.extend_from_slice(thumb);
    b
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn saved_exif_has_no_thumbnail_and_upright_dimensions() {
    let dir = tempfile::tempdir().unwrap();
    let upright = gradient(40, 24);
    let thumb = jpeg_bytes(&gradient(8, 6));
    for (le, orientation) in [(true, 6u16), (false, 6), (true, 8), (false, 1), (true, 3)] {
        let stored = match orientation {
            6 => image::imageops::rotate270(&upright),
            8 => image::imageops::rotate90(&upright),
            3 => image::imageops::rotate180(&upright),
            _ => upright.clone(),
        };
        let (sw, sh) = stored.dimensions();
        let blob = camera_exif(le, orientation, sw, sh as u16, &thumb);
        // The source really has a thumbnail that a reader finds.
        let src_exif = read_exif(&blob);
        assert!(
            src_exif.fields().any(|f| f.ifd_num == exif::In::THUMBNAIL),
            "source thumbnail"
        );
        let path = dir.path().join(format!("t{le}{orientation}.jpg"));
        fs::write(&path, with_meta(jpeg_bytes(&stored), Some(blob), None)).unwrap();

        let photo = load(&path).unwrap();
        assert_eq!(photo.rgb.dimensions(), (40, 24));
        let out = save_result(&photo, &photo.rgb, &dir.path().join("out")).unwrap();
        let bytes = fs::read(&out).unwrap();
        assert!(
            !contains(&bytes, &thumb),
            "le {le} o{orientation}: thumbnail bytes kept"
        );
        let saved = load(&out).unwrap();
        let e = read_exif(saved.exif.as_ref().unwrap());
        let case = format!("le {le} o{orientation}");
        assert!(
            !e.fields().any(|f| f.ifd_num == exif::In::THUMBNAIL),
            "{case}: IFD1 kept"
        );
        let get = |t| {
            e.get_field(t, exif::In::PRIMARY)
                .and_then(|f| f.value.get_uint(0))
        };
        assert_eq!(get(exif::Tag::Orientation), Some(1), "{case}");
        assert_eq!(get(exif::Tag::PixelXDimension), Some(40), "{case}");
        assert_eq!(get(exif::Tag::PixelYDimension), Some(24), "{case}");
        let make = e.get_field(exif::Tag::Make, exif::In::PRIMARY).unwrap();
        assert_eq!(make.display_value().to_string(), "\"PUWCam\"", "{case}");
        assert_eq!(saved.rgb.dimensions(), (40, 24));
    }
}

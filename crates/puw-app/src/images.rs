//! Edit-size images, thumbnails and conversions of the watermark area between resolutions.

use std::fs;
use std::io::Cursor;
use std::path::Path;

use image::codecs::jpeg::JpegEncoder;
use image::metadata::Orientation;
use image::{ImageEncoder, RgbImage, imageops};
use puw_core::Error;
use puw_core::image_io::{LoadedPhoto, load};
use puw_core::mask::{self, Mask};

use crate::Result;

/// Long side of the image the area is drawn on.
pub const EDIT_LONG_SIDE: u32 = 4096;
/// Long side of small thumbnails.
pub const THUMB_LONG_SIDE: u32 = 320;
/// Long side of grid thumbnails.
pub const GRID_LONG_SIDE: u32 = 480;

/// Size of the edit image for a photo of `w` x `h`: the long side capped at [`EDIT_LONG_SIDE`].
pub fn edit_size(w: u32, h: u32) -> (u32, u32) {
    fit_long_side(w, h, EDIT_LONG_SIDE)
}

fn fit_long_side(w: u32, h: u32, long: u32) -> (u32, u32) {
    let max = w.max(h);
    if max <= long {
        return (w, h);
    }
    let scale = |v: u32| {
        ((u64::from(v) * u64::from(long) + u64::from(max) / 2) / u64::from(max)).max(1) as u32
    };
    (scale(w), scale(h))
}

fn shrink(img: &RgbImage, long: u32) -> RgbImage {
    let (w, h) = fit_long_side(img.width(), img.height(), long);
    if (w, h) == img.dimensions() {
        img.clone()
    } else {
        imageops::resize(img, w, h, imageops::FilterType::Triangle)
    }
}

/// Upright thumbnail whose long side is at most `long_side`. Uses the thumbnail embedded in the
/// file when it is large enough, otherwise decodes the photo.
pub fn thumbnail(path: &Path, long_side: u32) -> Result<RgbImage> {
    if let Some(img) = embedded_thumbnail(path, long_side) {
        return Ok(shrink(&img, long_side));
    }
    let photo = load(path)?;
    Ok(shrink(&photo.rgb, long_side))
}

fn embedded_thumbnail(path: &Path, long_side: u32) -> Option<RgbImage> {
    let bytes = fs::read(path).ok()?;
    if !bytes.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let exif = exif::Reader::new()
        .read_from_container(&mut Cursor::new(&bytes))
        .ok()?;
    let uint = |tag| {
        exif.get_field(tag, exif::In::THUMBNAIL)
            .and_then(|f| f.value.get_uint(0))
    };
    let off = uint(exif::Tag::JPEGInterchangeFormat)? as usize;
    let len = uint(exif::Tag::JPEGInterchangeFormatLength)? as usize;
    let data = exif.buf().get(off..off.checked_add(len)?)?;
    let img = image::load_from_memory_with_format(data, image::ImageFormat::Jpeg).ok()?;
    if img.width().max(img.height()) < long_side {
        return None;
    }
    let orientation = exif
        .get_field(exif::Tag::Orientation, exif::In::PRIMARY)
        .and_then(|f| f.value.get_uint(0))
        .and_then(|v| u8::try_from(v).ok())
        .and_then(Orientation::from_exif)
        .unwrap_or(Orientation::NoTransforms);
    let mut img = img;
    img.apply_orientation(orientation);
    Some(img.into_rgb8())
}

/// The image the watermark area is drawn on: the upright photo, capped at [`EDIT_LONG_SIDE`].
pub fn edit_image(photo: &LoadedPhoto) -> RgbImage {
    shrink(&photo.rgb, EDIT_LONG_SIDE)
}

/// `img` with the area tinted red (#FF3B30, 45 %). The area is resized to `img` first.
pub fn overlay(img: &RgbImage, area: &Mask) -> RgbImage {
    let resized;
    let area = if area.dimensions() == img.dimensions() {
        area
    } else {
        resized = mask::resize_any(area, img.width(), img.height());
        &resized
    };
    let mut out = img.clone();
    for (p, a) in out.pixels_mut().zip(area.pixels()) {
        if a.0[0] != 0 {
            let red = [0xFFu32, 0x3B, 0x30];
            for (c, r) in p.0.iter_mut().zip(red) {
                *c = ((u32::from(*c) * 55 + r * 45 + 50) / 100) as u8;
            }
        }
    }
    out
}

/// Full-size area to edit size. Any covered source pixel covers the target; result is 0/255.
pub fn area_to_edit(full: &Mask, ew: u32, eh: u32) -> Vec<u8> {
    mask::binarize(&mask::resize_any(full, ew, eh)).into_raw()
}

/// Edit-size area bytes (`ew` x `eh`) to a full-size area (`w` x `h`), covering every target
/// pixel touched by a covered source pixel. Fails when the length does not match.
pub fn area_from_edit(bytes: &[u8], ew: u32, eh: u32, w: u32, h: u32) -> Result<Mask> {
    let small = Mask::from_raw(ew, eh, bytes.to_vec()).ok_or_else(|| {
        Error::InvalidInput(format!(
            "area has {} values, expected {}x{}",
            bytes.len(),
            ew,
            eh
        ))
    })?;
    Ok(mask::binarize(&mask::resize_any(&small, w, h)))
}

/// JPEG bytes of `img` at `quality` (1-100).
pub fn jpeg(img: &RgbImage, quality: u8) -> Vec<u8> {
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, quality.clamp(1, 100))
        .write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            image::ExtendedColorType::Rgb8,
        )
        .expect("encoding to memory cannot fail");
    out
}

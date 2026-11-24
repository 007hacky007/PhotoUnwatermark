//! Photo loading (upright pixels, metadata kept aside) and safe result saving.

use std::fs::{self, File};
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use image::codecs::jpeg::JpegEncoder;
use image::{
    DynamicImage, GrayImage, ImageEncoder, ImageFormat, Rgb, RgbImage, RgbaImage,
    metadata::Orientation,
};
use img_parts::{Bytes, ImageEXIF, ImageICC};

use crate::{Error, Result};

const JPEG_QUALITY: u8 = 95;
const OUTPUT_DIR_NAME: &str = "Without watermark";

/// A photo as the engine sees it: upright pixels plus the metadata needed to save it again.
#[derive(Debug, Clone)]
pub struct LoadedPhoto {
    pub path: PathBuf,
    /// Pixels with the EXIF orientation already applied.
    pub rgb: RgbImage,
    /// Transparency (upright), when the source had any.
    pub alpha: Option<GrayImage>,
    pub format: ImageFormat,
    /// Raw EXIF (TIFF structure) as found in the file.
    pub exif: Option<Vec<u8>>,
    pub icc: Option<Vec<u8>>,
}

/// Folder next to the source where results are written.
pub fn output_dir_for(src: &Path) -> PathBuf {
    src.parent()
        .unwrap_or_else(|| Path::new(""))
        .join(OUTPUT_DIR_NAME)
}

fn is_heif(bytes: &[u8]) -> bool {
    bytes.len() >= 12
        && &bytes[4..8] == b"ftyp"
        && matches!(
            &bytes[8..12],
            b"heic" | b"heix" | b"hevc" | b"hevx" | b"heim" | b"heis" | b"mif1" | b"msf1" | b"heif"
        )
}

fn image_err(e: impl std::fmt::Display) -> Error {
    Error::Image(e.to_string())
}

/// Metadata (EXIF, ICC) of the formats we carry through; `(None, None)` for others.
fn read_metadata(bytes: &[u8], format: ImageFormat) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    fn pick<T: ImageEXIF + ImageICC>(img: &T) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
        (
            img.exif().map(|b| b.to_vec()),
            img.icc_profile().map(|b| b.to_vec()),
        )
    }
    let data = Bytes::copy_from_slice(bytes);
    match format {
        ImageFormat::Jpeg => img_parts::jpeg::Jpeg::from_bytes(data)
            .map(|i| pick(&i))
            .unwrap_or_default(),
        ImageFormat::Png => img_parts::png::Png::from_bytes(data)
            .map(|i| pick(&i))
            .unwrap_or_default(),
        ImageFormat::WebP => img_parts::webp::WebP::from_bytes(data)
            .map(|i| pick(&i))
            .unwrap_or_default(),
        _ => (None, None),
    }
}

fn exif_orientation(exif: &[u8]) -> Orientation {
    exif::Reader::new()
        .read_raw(exif.to_vec())
        .ok()
        .and_then(|e| {
            e.get_field(exif::Tag::Orientation, exif::In::PRIMARY)
                .and_then(|f| f.value.get_uint(0))
        })
        .and_then(|v| u8::try_from(v).ok())
        .and_then(Orientation::from_exif)
        .unwrap_or(Orientation::NoTransforms)
}

/// Load a photo, applying its EXIF orientation so `rgb` is upright.
pub fn load(path: &Path) -> Result<LoadedPhoto> {
    let bytes = fs::read(path)?;
    if is_heif(&bytes) {
        return Err(Error::UnsupportedFormat("heic".into()));
    }
    let format = image::guess_format(&bytes).map_err(image_err)?;
    let mut img = image::load_from_memory_with_format(&bytes, format).map_err(image_err)?;
    let (exif, icc) = read_metadata(&bytes, format);

    let orientation = exif
        .as_deref()
        .map(exif_orientation)
        .unwrap_or(Orientation::NoTransforms);
    let alpha_plane = img.color().has_alpha().then(|| extract_alpha(&img));
    img.apply_orientation(orientation);
    let alpha = alpha_plane.map(|a| {
        let mut d = DynamicImage::ImageLuma8(a);
        d.apply_orientation(orientation);
        d.into_luma8()
    });

    Ok(LoadedPhoto {
        path: path.to_path_buf(),
        rgb: img.into_rgb8(),
        alpha,
        format,
        exif,
        icc,
    })
}

fn extract_alpha(img: &DynamicImage) -> GrayImage {
    let la = img.to_luma_alpha8();
    GrayImage::from_fn(la.width(), la.height(), |x, y| {
        image::Luma([la.get_pixel(x, y).0[1]])
    })
}

/// Set the EXIF Orientation tag to 1 in place; leaves the blob alone if it has none.
fn normalise_orientation(exif: &mut [u8]) {
    let le = match exif.get(..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return,
    };
    let u16_at = |b: &[u8], o: usize| -> Option<u16> {
        let s: [u8; 2] = b.get(o..o + 2)?.try_into().ok()?;
        Some(if le {
            u16::from_le_bytes(s)
        } else {
            u16::from_be_bytes(s)
        })
    };
    let Some(ifd) = exif
        .get(4..8)
        .and_then(|s| <[u8; 4]>::try_from(s).ok())
        .map(|s| if le { u32::from_le_bytes(s) } else { u32::from_be_bytes(s) } as usize)
    else {
        return;
    };
    let Some(count) = u16_at(exif, ifd) else {
        return;
    };
    for i in 0..count as usize {
        let e = ifd + 2 + i * 12;
        if u16_at(exif, e) == Some(0x0112) {
            let one = if le {
                1u16.to_le_bytes()
            } else {
                1u16.to_be_bytes()
            };
            if let Some(slot) = exif.get_mut(e + 8..e + 10) {
                slot.copy_from_slice(&one);
            }
            return;
        }
    }
}

fn encode(src: &LoadedPhoto, rgb: &RgbImage) -> Result<Vec<u8>> {
    let (w, h) = rgb.dimensions();
    let mut out = Cursor::new(Vec::new());
    let alpha = match (&src.alpha, src.format) {
        (Some(a), ImageFormat::Png | ImageFormat::WebP | ImageFormat::Tiff) => {
            if a.dimensions() != (w, h) {
                return Err(Error::SizeMismatch);
            }
            Some(a)
        }
        _ => None,
    };
    let rgba = alpha.map(|a| {
        RgbaImage::from_fn(w, h, |x, y| {
            let Rgb([r, g, b]) = *rgb.get_pixel(x, y);
            image::Rgba([r, g, b, a.get_pixel(x, y).0[0]])
        })
    });
    let dynimg = match rgba {
        Some(i) => DynamicImage::ImageRgba8(i),
        None => DynamicImage::ImageRgb8(rgb.clone()),
    };
    match src.format {
        ImageFormat::Jpeg => {
            JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)
                .write_image(rgb.as_raw(), w, h, image::ExtendedColorType::Rgb8)
                .map_err(image_err)?;
        }
        ImageFormat::Png | ImageFormat::WebP | ImageFormat::Tiff => {
            dynimg.write_to(&mut out, src.format).map_err(image_err)?;
        }
        other => {
            return Err(Error::UnsupportedFormat(
                format!("{other:?}").to_lowercase(),
            ));
        }
    }
    Ok(out.into_inner())
}

fn attach_metadata(
    encoded: Vec<u8>,
    format: ImageFormat,
    exif: Option<Vec<u8>>,
    icc: Option<&[u8]>,
) -> Result<Vec<u8>> {
    fn apply<T: ImageEXIF + ImageICC>(img: &mut T, exif: Option<Vec<u8>>, icc: Option<&[u8]>) {
        if let Some(e) = exif {
            img.set_exif(Some(Bytes::from(e)));
        }
        if let Some(i) = icc {
            img.set_icc_profile(Some(Bytes::copy_from_slice(i)));
        }
    }
    if exif.is_none() && icc.is_none() {
        return Ok(encoded);
    }
    let data = Bytes::from(encoded);
    let mut out = Vec::new();
    match format {
        ImageFormat::Jpeg => {
            let mut i = img_parts::jpeg::Jpeg::from_bytes(data).map_err(image_err)?;
            apply(&mut i, exif, icc);
            i.encoder().write_to(&mut out)?;
        }
        ImageFormat::Png => {
            let mut i = img_parts::png::Png::from_bytes(data).map_err(image_err)?;
            apply(&mut i, exif, icc);
            i.encoder().write_to(&mut out)?;
        }
        ImageFormat::WebP => {
            let mut i = img_parts::webp::WebP::from_bytes(data).map_err(image_err)?;
            apply(&mut i, exif, icc);
            i.encoder().write_to(&mut out)?;
        }
        _ => return Ok(data.to_vec()),
    }
    Ok(out)
}

fn unique_target(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let name = |n: u32| match (n, ext.is_empty()) {
        (1, true) => stem.to_string(),
        (1, false) => format!("{stem}.{ext}"),
        (n, true) => format!("{stem} ({n})"),
        (n, false) => format!("{stem} ({n}).{ext}"),
    };
    (1..)
        .map(|n| dir.join(name(n)))
        .find(|p| !p.exists())
        .expect("unbounded range")
}

/// Write `rgb` (upright, same size as the source) next to the originals' output folder.
///
/// Keeps the source stem and format; never touches the source file. Writes a hidden
/// `.partial` file in `out_dir`, fsyncs it, then renames it to a unique final name.
pub fn save_result(src: &LoadedPhoto, rgb: &RgbImage, out_dir: &Path) -> Result<PathBuf> {
    let encoded = encode(src, rgb)?;
    let exif = src.exif.clone().map(|mut e| {
        normalise_orientation(&mut e);
        e
    });
    let bytes = attach_metadata(encoded, src.format, exif, src.icc.as_deref())?;

    fs::create_dir_all(out_dir)?;
    let stem = src
        .path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "image".into());
    let ext = src
        .path
        .extension()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();

    let target = unique_target(out_dir, &stem, &ext);
    let file_name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let partial = out_dir.join(format!(".{file_name}.partial"));
    let write = || -> std::io::Result<()> {
        let mut f = File::create(&partial)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
        fs::rename(&partial, &target)
    };
    if let Err(e) = write() {
        let _ = fs::remove_file(&partial);
        return Err(e.into());
    }
    Ok(target)
}

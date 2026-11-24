//! Photo loading (upright pixels, metadata kept aside) and safe result saving.

use std::fs::{self, File};
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

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

struct Metadata {
    exif: Option<Vec<u8>>,
    icc: Option<Vec<u8>>,
    /// Orientation stored outside an EXIF blob (TIFF keeps it in its own IFD).
    tiff_orientation: Option<Orientation>,
}

/// Metadata of the formats we carry through; empty for others.
fn read_metadata(bytes: &[u8], format: ImageFormat) -> Metadata {
    fn pick<T: ImageEXIF + ImageICC>(img: &T) -> Metadata {
        Metadata {
            exif: img.exif().map(|b| b.to_vec()),
            icc: img.icc_profile().map(|b| b.to_vec()),
            tiff_orientation: None,
        }
    }
    let data = Bytes::copy_from_slice(bytes);
    let found = match format {
        ImageFormat::Jpeg => img_parts::jpeg::Jpeg::from_bytes(data).map(|i| pick(&i)),
        ImageFormat::Png => img_parts::png::Png::from_bytes(data).map(|i| pick(&i)),
        ImageFormat::WebP => img_parts::webp::WebP::from_bytes(data).map(|i| pick(&i)),
        ImageFormat::Tiff => return read_tiff_metadata(bytes),
        _ => return Metadata::none(),
    };
    found.unwrap_or_else(|_| Metadata::none())
}

impl Metadata {
    fn none() -> Self {
        Metadata {
            exif: None,
            icc: None,
            tiff_orientation: None,
        }
    }
}

fn read_tiff_metadata(bytes: &[u8]) -> Metadata {
    let Ok(mut dec) = tiff::decoder::Decoder::new(Cursor::new(bytes)) else {
        return Metadata::none();
    };
    let tiff_orientation = dec
        .find_tag_unsigned::<u8>(tiff::tags::Tag::Orientation)
        .ok()
        .flatten()
        .and_then(Orientation::from_exif);
    let icc = dec
        .find_tag(tiff::tags::Tag::IccProfile)
        .ok()
        .flatten()
        .and_then(|v| v.into_u8_vec().ok());
    Metadata {
        exif: None,
        icc,
        tiff_orientation,
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
    let Metadata {
        exif,
        icc,
        tiff_orientation,
    } = read_metadata(&bytes, format);

    let orientation = tiff_orientation
        .or_else(|| exif.as_deref().map(exif_orientation))
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

/// Byte-order-aware access to a TIFF structure (an EXIF blob). Out-of-range reads give `None`
/// and out-of-range writes are skipped, so a malformed blob is left as it is.
struct Tiff<'a> {
    b: &'a mut Vec<u8>,
    le: bool,
}

impl Tiff<'_> {
    fn u16_at(&self, o: usize) -> Option<u16> {
        let s: [u8; 2] = self.b.get(o..o.checked_add(2)?)?.try_into().ok()?;
        Some(if self.le {
            u16::from_le_bytes(s)
        } else {
            u16::from_be_bytes(s)
        })
    }

    fn u32_at(&self, o: usize) -> Option<u32> {
        let s: [u8; 4] = self.b.get(o..o.checked_add(4)?)?.try_into().ok()?;
        Some(if self.le {
            u32::from_le_bytes(s)
        } else {
            u32::from_be_bytes(s)
        })
    }

    fn put(&mut self, o: usize, bytes: &[u8]) {
        if let Some(slot) = o
            .checked_add(bytes.len())
            .and_then(|end| self.b.get_mut(o..end))
        {
            slot.copy_from_slice(bytes);
        }
    }

    fn put_u16(&mut self, o: usize, v: u16) {
        let b = if self.le {
            v.to_le_bytes()
        } else {
            v.to_be_bytes()
        };
        self.put(o, &b);
    }

    fn put_u32(&mut self, o: usize, v: u32) {
        let b = if self.le {
            v.to_le_bytes()
        } else {
            v.to_be_bytes()
        };
        self.put(o, &b);
    }

    /// Entries of the IFD at `ifd` as (entry offset, tag); empty when it is out of range.
    fn entries(&self, ifd: usize) -> Vec<(usize, u16)> {
        let n = self.u16_at(ifd).unwrap_or(0) as usize;
        (0..n)
            .map(|i| ifd + 2 + 12 * i)
            .map_while(|e| Some((e, self.u16_at(e)?)))
            .collect()
    }

    /// Value of a one-count SHORT or LONG entry.
    fn uint(&self, e: usize) -> Option<u32> {
        if self.u32_at(e + 4)? != 1 {
            return None;
        }
        match self.u16_at(e + 2)? {
            3 => self.u16_at(e + 8).map(u32::from),
            4 => self.u32_at(e + 8),
            _ => None,
        }
    }

    /// Makes entry `e` a one-count LONG holding `v`.
    fn set_long(&mut self, e: usize, v: u32) {
        self.put_u16(e + 2, 4);
        self.put_u32(e + 4, 1);
        self.put_u32(e + 8, v);
    }
}

const TAG_ORIENTATION: u16 = 0x0112;
const TAG_EXIF_IFD: u16 = 0x8769;
const TAG_PIXEL_X: u16 = 0xA002;
const TAG_PIXEL_Y: u16 = 0xA003;
const TAG_STRIP_OFFSETS: u16 = 0x0111;
const TAG_STRIP_BYTES: u16 = 0x0117;
const TAG_JPEG_OFFSET: u16 = 0x0201;
const TAG_JPEG_BYTES: u16 = 0x0202;

/// Prepares the source EXIF for a result with upright pixels:
/// - Orientation (IFD0) becomes 1;
/// - for orientations 5-8 (a quarter turn), PixelXDimension and PixelYDimension swap;
/// - the thumbnail (IFD1) is removed: unlinked from IFD0, its table and image data zeroed, and
///   cut off the blob when it is at the end. It shows the original, watermark included.
///
/// A blob that is not a TIFF structure is left alone.
fn clean_exif(exif: &mut Vec<u8>) {
    let le = match exif.get(..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return,
    };
    let mut t = Tiff { b: exif, le };
    let Some(ifd0) = t.u32_at(4).map(|o| o as usize) else {
        return;
    };
    let entries = t.entries(ifd0);
    let mut exif_ifd = None;
    let mut quarter_turn = false;
    for &(e, tag) in &entries {
        match tag {
            TAG_ORIENTATION => {
                quarter_turn = matches!(t.u16_at(e + 8), Some(5..=8));
                t.put_u16(e + 8, 1);
            }
            TAG_EXIF_IFD => exif_ifd = t.uint(e).map(|o| o as usize),
            _ => {}
        }
    }
    if quarter_turn && let Some(sub) = exif_ifd {
        let find = |t: &Tiff, tag| t.entries(sub).into_iter().find(|&(_, g)| g == tag);
        if let (Some((ex, _)), Some((ey, _))) = (find(&t, TAG_PIXEL_X), find(&t, TAG_PIXEL_Y))
            && let (Some(px), Some(py)) = (t.uint(ex), t.uint(ey))
        {
            t.set_long(ex, py);
            t.set_long(ey, px);
        }
    }

    let next_at = ifd0 + 2 + 12 * entries.len();
    let Some(ifd1) = t.u32_at(next_at).filter(|&o| o != 0).map(|o| o as usize) else {
        return;
    };
    t.put_u32(next_at, 0);
    // Regions of the thumbnail that are zeroed: the image data, then the IFD1 table.
    let mut regions = Vec::new();
    let ifd1_entries = t.entries(ifd1);
    let value = |tag| {
        ifd1_entries
            .iter()
            .find(|&&(_, g)| g == tag)
            .and_then(|&(e, _)| t.uint(e))
            .map(|v| v as usize)
    };
    for (off, len) in [
        (TAG_JPEG_OFFSET, TAG_JPEG_BYTES),
        (TAG_STRIP_OFFSETS, TAG_STRIP_BYTES),
    ] {
        if let (Some(o), Some(n)) = (value(off), value(len)) {
            regions.push((o, o.saturating_add(n)));
        }
    }
    regions.push((ifd1, ifd1 + 2 + 12 * ifd1_entries.len() + 4));
    let len = t.b.len();
    for &(s, e) in &regions {
        if s < len {
            t.b[s..e.min(len)].fill(0);
        }
    }
    // Cut off what was zeroed at the end of the blob (repeatedly: the table usually sits right
    // before the image data).
    while let Some(&(s, _)) = regions
        .iter()
        .find(|&&(s, e)| s < t.b.len() && e >= t.b.len())
    {
        t.b.truncate(s);
    }
}

fn encode(src: &LoadedPhoto, rgb: &RgbImage, icc: Option<&[u8]>) -> Result<Vec<u8>> {
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
        ImageFormat::Tiff => encode_tiff(&mut out, rgb, alpha, icc)?,
        ImageFormat::Png | ImageFormat::WebP => {
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

fn encode_tiff(
    out: &mut Cursor<Vec<u8>>,
    rgb: &RgbImage,
    alpha: Option<&GrayImage>,
    icc: Option<&[u8]>,
) -> Result<()> {
    use tiff::encoder::{TiffEncoder, colortype};
    use tiff::tags::Tag;
    let (w, h) = rgb.dimensions();
    let mut enc = TiffEncoder::new(out).map_err(image_err)?;
    // No Orientation tag is written: the pixels are upright.
    macro_rules! write {
        ($ct:ty, $data:expr, $alpha:expr) => {{
            let mut img = enc.new_image::<$ct>(w, h).map_err(image_err)?;
            if let Some(icc) = icc {
                img.encoder()
                    .write_tag(Tag::IccProfile, icc)
                    .map_err(image_err)?;
            }
            if $alpha {
                // 2 = unassociated alpha
                img.encoder()
                    .write_tag(Tag::ExtraSamples, &[2u16][..])
                    .map_err(image_err)?;
            }
            img.write_data($data).map_err(image_err)
        }};
    }
    match alpha {
        Some(a) => {
            let data: Vec<u8> = rgb
                .pixels()
                .zip(a.pixels())
                .flat_map(|(p, a)| [p.0[0], p.0[1], p.0[2], a.0[0]])
                .collect();
            write!(colortype::RGBA8, &data, true)
        }
        None => write!(colortype::RGB8, rgb.as_raw(), false),
    }
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

/// Publish the finished `partial` as the first free name `stem.ext`, `stem (2).ext`, ...
///
/// A hard link never replaces an existing file (`AlreadyExists` moves on to the next
/// suffix) and the final name only ever appears with complete content. The partial is
/// removed afterwards; failing to remove it does not fail the publish. Filesystems that
/// refuse hard links use [`copy_no_clobber`].
fn publish_no_clobber(
    partial: &Path,
    dir: &Path,
    stem: &str,
    ext: &str,
) -> std::io::Result<PathBuf> {
    let name = |n: u32| match (n, ext.is_empty()) {
        (1, true) => stem.to_string(),
        (1, false) => format!("{stem}.{ext}"),
        (n, true) => format!("{stem} ({n})"),
        (n, false) => format!("{stem} ({n}).{ext}"),
    };
    for n in 1.. {
        let target = dir.join(name(n));
        match fs::hard_link(partial, &target) {
            Ok(()) => {
                remove_partial(partial);
                return Ok(target);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => match copy_no_clobber(partial, &target) {
                Ok(()) => {
                    remove_partial(partial);
                    return Ok(target);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            },
        }
    }
    unreachable!("unbounded range")
}

/// Removes the partial after the result was published. A failure only leaves a hidden file
/// behind, so it is logged and does not fail the save.
fn remove_partial(partial: &Path) {
    if let Err(e) = fs::remove_file(partial) {
        tracing::warn!("could not remove a partial file after saving: {e}");
    }
}

/// Last resort for filesystems without hard links: `create_new` + copy + fsync.
/// NOT atomic: the target exists with partial content while it is being written, and
/// is removed again if the copy fails.
fn copy_no_clobber(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut dst = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(to)?;
    let copied = File::open(from)
        .and_then(|mut src| std::io::copy(&mut src, &mut dst))
        .and_then(|_| dst.sync_all());
    if copied.is_err() {
        drop(dst);
        let _ = fs::remove_file(to);
    }
    copied
}

/// Write `rgb` (upright, same size as the source) into `out_dir`.
///
/// Keeps the source stem and format; never touches the source file or an existing
/// file in `out_dir`. Writes a hidden `.partial` file in `out_dir`, fsyncs it, then
/// publishes it under a unique final name that never replaces an existing file.
///
/// EXIF is carried with Orientation set to 1, PixelXDimension / PixelYDimension swapped for
/// a source that was turned a quarter, and the embedded thumbnail removed (it shows the
/// original, watermark included).
///
/// Known limitation: for TIFF the ICC profile is carried but the EXIF sub-IFD is not
/// (the saved TIFF has no Orientation tag, as its pixels are upright).
/// The ICC profile is only attached when its colour space is RGB, because the output
/// is always RGB (grayscale and CMYK sources are converted on load).
pub fn save_result(src: &LoadedPhoto, rgb: &RgbImage, out_dir: &Path) -> Result<PathBuf> {
    let icc = src
        .icc
        .as_deref()
        .filter(|i| i.get(16..20) == Some(b"RGB ".as_slice()));
    let encoded = encode(src, rgb, icc)?;
    let exif = src.exif.clone().map(|mut e| {
        clean_exif(&mut e);
        e
    });
    let bytes = attach_metadata(encoded, src.format, exif, icc)?;

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

    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let partial = out_dir.join(format!(
        ".{stem}.{}.{}.partial",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let write_partial = || -> std::io::Result<()> {
        let mut f = File::create(&partial)?;
        f.write_all(&bytes)?;
        f.sync_all()
    };
    if let Err(e) = write_partial() {
        let _ = fs::remove_file(&partial);
        return Err(e.into());
    }
    publish_no_clobber(&partial, out_dir, &stem, &ext)
        .inspect_err(|_| {
            let _ = fs::remove_file(&partial);
        })
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::{copy_no_clobber, publish_no_clobber};
    use std::fs;

    #[test]
    fn publish_never_replaces_and_picks_next_suffix() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.jpg"), b"keep").unwrap();
        let mut seen = Vec::new();
        for body in [&b"one"[..], b"two"] {
            let partial = dir.path().join(".a.partial");
            fs::write(&partial, body).unwrap();
            let p = publish_no_clobber(&partial, dir.path(), "a", "jpg").unwrap();
            assert!(!partial.exists());
            assert_eq!(fs::read(&p).unwrap(), body);
            seen.push(p.file_name().unwrap().to_string_lossy().into_owned());
        }
        assert_eq!(seen, ["a (2).jpg", "a (3).jpg"]);
        assert_eq!(fs::read(dir.path().join("a.jpg")).unwrap(), b"keep");

        let partial = dir.path().join(".b.partial");
        fs::write(&partial, b"x").unwrap();
        let p = publish_no_clobber(&partial, dir.path(), "b", "").unwrap();
        assert_eq!(p.file_name().unwrap(), "b");
    }

    #[test]
    fn copy_fallback_refuses_existing_target() {
        let dir = tempfile::tempdir().unwrap();
        let (from, to) = (dir.path().join("f"), dir.path().join("t"));
        fs::write(&from, b"new").unwrap();
        fs::write(&to, b"old").unwrap();
        let e = copy_no_clobber(&from, &to).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&to).unwrap(), b"old");
        fs::remove_file(&to).unwrap();
        copy_no_clobber(&from, &to).unwrap();
        assert_eq!(fs::read(&to).unwrap(), b"new");
    }
}

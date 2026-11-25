//! Saved watermarks: keeping the confirmed watermark for later sets, and the list of saved
//! ones. The locking rules of [`AppCore`] (see `core.rs`) apply here too.

use std::io::Cursor;

use image::{ImageFormat, imageops};
use puw_core::mask;
use puw_core::template::main_bbox;
use puw_core::{Library, SavedWatermark, load};
use serde::Serialize;
use time::Date;

use super::{AppCore, not_found};
use crate::images::{overlay, shrink};
use crate::{AppError, Result};

/// Long side, at most, of a saved watermark's picture.
const SAVED_THUMB_LONG_SIDE: u32 = 256;
/// Room around the watermark in that picture, as a share of the watermark's width and height
/// on each side.
const SAVED_THUMB_MARGIN: f32 = 0.1;

/// A saved watermark as the list shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedInfo {
    pub id: String,
    pub name: String,
    /// When it was saved (RFC 3339, UTC).
    pub created: String,
}

/// The name a saved watermark gets unless the person types another: "Watermark from
/// YYYY-MM-DD".
pub fn default_watermark_name(today: Date) -> String {
    format!(
        "Watermark from {:04}-{:02}-{:02}",
        today.year(),
        u8::from(today.month()),
        today.day()
    )
}

impl AppCore {
    /// Saves the set's confirmed watermark under `name` (trimmed; an empty name is refused)
    /// and returns its id. Its picture is the confirmed photo around the watermark (the parts
    /// matching looks for, see [`main_bbox`], plus 10 % on each side) with the area tinted, at most 256 px long, as PNG. Fails with
    /// [`AppError::NothingToRemove`] before a set area is confirmed.
    pub fn save_watermark(&self, name: &str) -> Result<String> {
        let _job = self.jobs.start()?;
        let (template, confirmed) = {
            let s = self.session();
            (s.template.clone(), s.confirmed.clone())
        };
        let (Some(template), Some(confirmed)) = (template, confirmed) else {
            return Err(AppError::NothingToRemove);
        };
        let rgb = load(&confirmed.path)?.rgb;
        let (w, h) = rgb.dimensions();
        let area = confirmed.area.resized(w, h);
        // The picture shows the watermark itself, without a stray dot far from it.
        // (left, top, width, height) in photo pixels.
        let (rx, ry, rw, rh) = match main_bbox(&template) {
            Some([x, y, bw, bh]) => {
                let px = |v: f32, n: u32| ((v * n as f32).round().max(0.0) as u32).min(n - 1);
                let (x0, y0) = (px(x, w), px(y, h));
                let (x1, y1) = (px(x + bw, w).max(x0 + 1), px(y + bh, h).max(y0 + 1));
                (x0, y0, x1 - x0, y1 - y0)
            }
            None => {
                let r = mask::bbox(&area).ok_or(AppError::NothingToRemove)?;
                (r.left() as u32, r.top() as u32, r.width(), r.height())
            }
        };
        let margin = |v: u32| (v as f32 * SAVED_THUMB_MARGIN).round() as u32;
        let (mx, my) = (margin(rw), margin(rh));
        let x0 = rx.saturating_sub(mx);
        let y0 = ry.saturating_sub(my);
        let x1 = (rx + rw + mx).min(w);
        let y1 = (ry + rh + my).min(h);
        let crop = imageops::crop_imm(&rgb, x0, y0, x1 - x0, y1 - y0).to_image();
        let crop_area = imageops::crop_imm(&area, x0, y0, x1 - x0, y1 - y0).to_image();
        drop((rgb, area));
        let thumb = shrink(&overlay(&crop, &crop_area), SAVED_THUMB_LONG_SIDE);
        let mut png = Vec::new();
        thumb
            .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .map_err(|e| puw_core::Error::Image(e.to_string()))?;

        let saved = SavedWatermark::new(name, (*template).clone(), png);
        Library::open(&self.dirs.watermarks)?.save(&saved)?;
        Ok(saved.id)
    }

    /// The saved watermarks, newest first.
    pub fn list_saved(&self) -> Result<Vec<SavedInfo>> {
        Ok(Library::open(&self.dirs.watermarks)?
            .list()?
            .into_iter()
            .map(|w| SavedInfo {
                id: w.id,
                name: w.name,
                created: w.created,
            })
            .collect())
    }

    /// Renames saved watermark `id` (trimmed; an empty name is refused).
    pub fn rename_saved(&self, id: &str, name: &str) -> Result<()> {
        Ok(Library::open(&self.dirs.watermarks)?.rename(id, name)?)
    }

    pub fn delete_saved(&self, id: &str) -> Result<()> {
        Ok(Library::open(&self.dirs.watermarks)?.delete(id)?)
    }

    /// The picture of saved watermark `id` (PNG).
    pub fn saved_thumbnail(&self, id: &str) -> Result<Vec<u8>> {
        Library::open(&self.dirs.watermarks)?
            .list()?
            .into_iter()
            .find(|w| w.id == id)
            .map(|w| w.thumbnail_png)
            .ok_or_else(|| not_found("no such saved watermark"))
    }
}

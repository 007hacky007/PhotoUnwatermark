//! Step 2's strip: where the watermark area drawn on one photo lands on another, so the editor
//! can show the one shared drawing on every photo of the strip. The locking rules of
//! [`AppCore`] (see `core.rs`) apply here too.

use std::sync::Arc;

use puw_core::mask;
use puw_core::{MatchStatus, SetLayers, Transform, build_template, load, match_photo};
use serde::Serialize;

use super::{AppCore, no_photo};
use crate::images::{self, area_from_edit};
use crate::photos::PhotoId;
use crate::session::Area;
use crate::{AppError, Result};

/// How sure a [`StripPlacement`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Placed {
    Found,
    Unsure,
    NotFound,
}

/// Where the shared drawing lands on a photo: maps the reference photo's edit-image pixels
/// onto this photo's edit image, `(x * scale_x + dx, y * scale_y + dy)`, with `(0, 0)` at the
/// top-left corner of the first pixel. An unsure placement keeps where it was found; scale
/// and shift are 0 for [`Placed::NotFound`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StripPlacement {
    pub status: Placed,
    pub scale_x: f32,
    pub scale_y: f32,
    pub dx: f32,
    pub dy: f32,
}

impl StripPlacement {
    const IDENTITY: StripPlacement = StripPlacement {
        status: Placed::Found,
        scale_x: 1.0,
        scale_y: 1.0,
        dx: 0.0,
        dy: 0.0,
    };

    const NOT_FOUND: StripPlacement = StripPlacement {
        status: Placed::NotFound,
        scale_x: 0.0,
        scale_y: 0.0,
        dx: 0.0,
        dy: 0.0,
    };
}

/// `t` (native pixels of a `reference` photo onto native pixels of a `photo`, both upright
/// sizes) as a map between their edit images.
fn on_edit_images(
    status: Placed,
    t: Transform,
    reference: (u32, u32),
    photo: (u32, u32),
) -> StripPlacement {
    let (rw, rh) = reference;
    let (pw, ph) = photo;
    let (erw, erh) = images::edit_size(rw, rh);
    let (epw, eph) = images::edit_size(pw, ph);
    let ratio = |e: u32, n: u32| e as f32 / n as f32;
    StripPlacement {
        status,
        scale_x: t.scale_x / ratio(erw, rw) * ratio(epw, pw),
        scale_y: t.scale_y / ratio(erh, rh) * ratio(eph, ph),
        dx: t.dx * ratio(epw, pw),
        dy: t.dy * ratio(eph, ph),
    }
}

impl AppCore {
    /// Where the shared drawing (the draft of photo `reference`, else its step-2 area, see
    /// [`AppCore::start_area`]) lands on photo `id`, found as step 3 finds it: the watermark
    /// learned from the drawing on `reference`, then looked for on `id` with the set's batch
    /// layers (if read). On `reference` itself it is [`Placed::Found`] where it is, with no
    /// photo read. An empty drawing is [`Placed::NotFound`] everywhere, again with no photo
    /// read. The learned watermark is kept until the reference or the drawing changes, or the
    /// groups are cleared. A photo that cannot be read gets its [`crate::photos::Problem`] and
    /// the error is returned.
    pub fn place_on(&self, reference: PhotoId, id: PhotoId) -> Result<StripPlacement> {
        let job = self.jobs.start()?;
        let (ref_path, (w, h), draft, start, path, cached, layers) = {
            let s = self.session();
            let r = s.photos.get(reference).ok_or_else(|| no_photo(reference))?;
            let p = s.photos.get(id).ok_or_else(|| no_photo(id))?;
            (
                r.path.clone(),
                (r.info.width, r.info.height),
                s.data.get(&reference).and_then(|d| d.draft.clone()),
                s.start_area(reference),
                p.path.clone(),
                s.strip.clone(),
                s.layers.clone(),
            )
        };
        if id == reference {
            return Ok(StripPlacement::IDENTITY);
        }
        let drawing = match (draft, start) {
            (Some(d), _) => Arc::new(Area::from_mask(&area_from_edit(&d.bytes, d.w, d.h, w, h)?)),
            (None, Some(a)) => a,
            (None, None) => return Ok(StripPlacement::NOT_FOUND),
        };
        if drawing.is_empty() {
            return Ok(StripPlacement::NOT_FOUND);
        }

        let template = match cached {
            Some((r, a, t)) if r == reference && *a == *drawing => t,
            _ => {
                job.check()?;
                let photo =
                    self.noting_problem(reference, load(&ref_path).map_err(AppError::from))?;
                let (pw, ph) = photo.rgb.dimensions();
                let full = drawing.to_full();
                let area = if full.dimensions() == (pw, ph) {
                    full
                } else {
                    mask::binarize(&mask::resize_any(&full, pw, ph))
                };
                let t = Arc::new(build_template(&photo.rgb, &area));
                let mut s = self.session();
                s.strip = Some((reference, drawing, t.clone()));
                s.strip_built += 1;
                t
            }
        };
        if template.pieces.is_empty() {
            return Ok(StripPlacement::NOT_FOUND);
        }

        job.check()?;
        let photo = self.noting_problem(id, load(&path).map_err(AppError::from))?;
        let layers = layers.unwrap_or_else(|| Arc::new(SetLayers::none()));
        let r = match_photo(&template, &photo.rgb, &layers);
        let status = match r.status {
            MatchStatus::Found => Placed::Found,
            MatchStatus::Unsure => Placed::Unsure,
            MatchStatus::NotFound => return Ok(StripPlacement::NOT_FOUND),
        };
        let Some(t) = r.transform else {
            return Ok(StripPlacement::NOT_FOUND);
        };
        Ok(on_edit_images(
            status,
            t,
            (template.ref_w, template.ref_h),
            photo.rgb.dimensions(),
        ))
    }

    /// How many times [`AppCore::place_on`] learned the watermark from a drawing.
    #[doc(hidden)]
    pub fn strip_templates_built(&self) -> usize {
        self.session().strip_built
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    /// A point of the reference's edit image, mapped by the edit placement, lands where the
    /// same point in native pixels lands under the native transform.
    #[test]
    fn edit_placement_matches_the_native_transform() {
        // An 8000 x 6000 reference (edit 4096 x 3072) onto a 3000 x 6000 photo (edit 2048 x
        // 4096) and onto a small one that is its own edit image.
        let t = Transform {
            scale_x: 0.7,
            scale_y: 0.72,
            dx: 120.0,
            dy: -40.0,
        };
        for photo in [(3000, 6000), (900, 1200)] {
            let p = on_edit_images(Placed::Unsure, t, (8000, 6000), photo);
            assert_eq!(p.status, Placed::Unsure);
            let (erw, erh) = images::edit_size(8000, 6000);
            let (epw, eph) = images::edit_size(photo.0, photo.1);
            for (x, y) in [(0.0, 0.0), (1000.0, 2500.0), (8000.0, 6000.0)] {
                let (nx, ny) = t.apply(x, y);
                let (ex, ey) = (x * erw as f32 / 8000.0, y * erh as f32 / 6000.0);
                let got = (ex * p.scale_x + p.dx, ey * p.scale_y + p.dy);
                let want = (
                    nx * epw as f32 / photo.0 as f32,
                    ny * eph as f32 / photo.1 as f32,
                );
                assert!(
                    close(got.0, want.0) && close(got.1, want.1),
                    "{photo:?} ({x}, {y}): {got:?} != {want:?}"
                );
            }
        }
    }

    #[test]
    fn placement_is_camel_case() {
        let json = serde_json::to_value(StripPlacement::NOT_FOUND).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "status": "notFound", "scaleX": 0.0, "scaleY": 0.0, "dx": 0.0, "dy": 0.0
            })
        );
    }
}

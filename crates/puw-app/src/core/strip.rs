//! Step 2's strip: where the watermark area drawn on one photo lands on another, so the editor
//! can show the one shared drawing on every photo of the strip. The locking rules of
//! [`AppCore`] (see `core.rs`) apply here too.

use std::collections::HashMap;
use std::sync::Arc;

use puw_core::mask::{self, Mask};
use puw_core::template::main_bbox;
use puw_core::{
    MatchStatus, PositionPrior, SetLayers, Transform, WatermarkTemplate, build_template, load,
    match_with,
};
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

/// How far, as a share of the reference photo's width or height, each side of the drawing's
/// main box may move before a strip photo's placement is looked for again (see
/// [`AppCore::place_on`]). 1 % is 40 px on a 4000 px side: a stroke along the edge of the
/// drawing, not a drawing moved to another place.
const KEEP_PLACEMENT: f32 = 0.01;

/// What [`AppCore::place_on`] keeps between calls: the watermark learned from one drawing on
/// one reference photo, and where it was found on the strip photos.
#[derive(Debug, Clone)]
pub(crate) struct StripCache {
    pub reference: PhotoId,
    /// The drawing at the reference's edit size, as the editor sends it.
    pub drawing: Arc<Area>,
    /// The batch layers the placements were looked for with.
    pub layers: Option<Arc<SetLayers>>,
    pub template: Arc<WatermarkTemplate>,
    pub prior: Option<Arc<PositionPrior>>,
    /// The template's main box ([`main_bbox`]), as fractions of the reference photo.
    pub main: Option<[f32; 4]>,
    /// Per strip photo: where it was placed, and the main box it was placed with. Kept across
    /// drawings on the same reference and layers.
    pub placed: HashMap<PhotoId, ([f32; 4], StripPlacement)>,
}

fn same_layers(a: &Option<Arc<SetLayers>>, b: &Option<Arc<SetLayers>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    }
}

fn near(a: &[f32; 4], b: &[f32; 4]) -> bool {
    let sides = |r: &[f32; 4]| [r[0], r[1], r[0] + r[2], r[1] + r[3]];
    sides(a)
        .iter()
        .zip(sides(b))
        .all(|(x, y)| (x - y).abs() <= KEEP_PLACEMENT)
}

impl AppCore {
    /// Where the shared drawing (the draft of photo `reference`, else its step-2 area, see
    /// [`AppCore::start_area`]) lands on photo `id`, found as step 3 finds it: the watermark
    /// learned from the drawing on `reference`, then looked for on `id` with the set's batch
    /// layers (if read) and their position prior. On `reference` itself it is
    /// [`Placed::Found`] where it is, with no photo read. An empty drawing is
    /// [`Placed::NotFound`] everywhere, again with no photo read. A photo that cannot be read
    /// gets its [`crate::photos::Problem`] and the error is returned.
    ///
    /// The learned watermark is kept until the reference or the drawing (compared at edit size)
    /// changes, or the groups are cleared. A `Found` or `Unsure` placement on a photo is kept
    /// too, across drawings on the same reference, while the main box of the drawing (the parts
    /// matching looks for, see [`main_bbox`]) stays within 1 % of where it was: a stroke on the
    /// drawing then costs one read of the reference, and no photo is looked at again. A
    /// `NotFound` is always looked for again, as the new drawing may be what finds it.
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
        let (ew, eh) = images::edit_size(w, h);
        // The drawing at edit size: what the editor drew, compared without upsampling it.
        let drawing = match (&draft, &start) {
            (Some(d), _) => {
                let m = Mask::from_raw(d.w, d.h, d.bytes.clone()).ok_or_else(|| {
                    AppError::from(puw_core::Error::InvalidInput(
                        "draft of the wrong size".into(),
                    ))
                })?;
                Arc::new(Area::from_mask(&mask::binarize(&m)))
            }
            (None, Some(a)) => Arc::new(Area::from_mask(&a.resized(ew, eh))),
            (None, None) => return Ok(StripPlacement::NOT_FOUND),
        };
        if drawing.is_empty() {
            return Ok(StripPlacement::NOT_FOUND);
        }

        let kept = cached.filter(|c| c.reference == reference && same_layers(&c.layers, &layers));
        let strip = match kept {
            Some(c) if *c.drawing == *drawing => c,
            kept => {
                job.check()?;
                let photo =
                    self.noting_problem(reference, load(&ref_path).map_err(AppError::from))?;
                let (pw, ph) = photo.rgb.dimensions();
                let area = match (draft, start) {
                    (Some(d), _) => area_from_edit(&d.bytes, d.w, d.h, pw, ph)?,
                    (None, Some(a)) => a.resized(pw, ph),
                    (None, None) => unreachable!("an empty drawing returned above"),
                };
                let template = Arc::new(build_template(&photo.rgb, &area));
                drop(photo);
                let no_layers = SetLayers::none();
                let prior = PositionPrior::new(&template, layers.as_deref().unwrap_or(&no_layers))
                    .map(Arc::new);
                let c = StripCache {
                    reference,
                    drawing,
                    layers: layers.clone(),
                    main: main_bbox(&template),
                    template,
                    prior,
                    placed: kept.map(|k| k.placed).unwrap_or_default(),
                };
                let mut s = self.session();
                s.strip = Some(c.clone());
                s.strip_built += 1;
                c
            }
        };
        if strip.template.pieces.is_empty() {
            return Ok(StripPlacement::NOT_FOUND);
        }
        let Some(main) = strip.main else {
            return Ok(StripPlacement::NOT_FOUND);
        };
        if let Some((at, p)) = strip.placed.get(&id)
            && p.status != Placed::NotFound
            && near(at, &main)
        {
            return Ok(*p);
        }

        job.check()?;
        let photo = self.noting_problem(id, load(&path).map_err(AppError::from))?;
        let t = &strip.template;
        let r = match_with(t, strip.prior.as_deref(), &photo.rgb, job.flag());
        job.check()?;
        let placed = match (r.status, r.transform) {
            (MatchStatus::Found, Some(tr)) => on_edit_images(
                Placed::Found,
                tr,
                (t.ref_w, t.ref_h),
                photo.rgb.dimensions(),
            ),
            (MatchStatus::Unsure, Some(tr)) => on_edit_images(
                Placed::Unsure,
                tr,
                (t.ref_w, t.ref_h),
                photo.rgb.dimensions(),
            ),
            _ => StripPlacement::NOT_FOUND,
        };
        let mut s = self.session();
        s.strip_matched += 1;
        if let Some(c) = s.strip.as_mut().filter(|c| c.reference == reference) {
            c.placed.insert(id, (main, placed));
        }
        Ok(placed)
    }

    /// How many times [`AppCore::place_on`] learned the watermark from a drawing.
    #[doc(hidden)]
    pub fn strip_templates_built(&self) -> usize {
        self.session().strip_built
    }

    /// How many times [`AppCore::place_on`] looked for the watermark on a strip photo.
    #[doc(hidden)]
    pub fn strip_photos_matched(&self) -> usize {
        self.session().strip_matched
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

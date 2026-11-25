//! Step 3: finding the set's watermark in every photo, the per-photo states and edits, the rule
//! that keeps removal from starting until unsure photos are handled, and the grid images. The
//! locking rules of [`AppCore`] (see `core.rs`) apply here too.

use std::sync::{Arc, Mutex};

use puw_core::{MatchStatus, SetLayers, WatermarkTemplate, load, match_photo};
use rayon::prelude::*;
use serde::Serialize;

use super::{AppCore, lock, no_photo};
use crate::images::{area_from_edit, jpeg, overlay};
use crate::jobs::{Events, Job, ProgressEvent};
use crate::photos::PhotoId;
use crate::session::{Area, Matched, Unmatched};
use crate::{AppError, Result};

const GRID_QUALITY: u8 = 85;

/// How finding the set's watermark went, over all photos. A photo with an area of its own
/// counts as found, or as not found when that area is empty; a photo not looked for yet
/// counts only in `total`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchSummary {
    pub found: u32,
    pub unsure: u32,
    pub not_found: u32,
    pub total: u32,
}

/// What step 3 shows about a photo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AreaStatus {
    Found,
    Unsure,
    NotFound,
    /// The person saved an area for this photo alone.
    Edited,
    /// Not looked for yet (no set area, or added after matching).
    Pending,
}

/// One photo as step 3 shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotoState {
    pub id: PhotoId,
    pub status: AreaStatus,
    /// Ticked for removal.
    pub included: bool,
    /// Opened in the editor since its result came.
    pub reviewed: bool,
    /// Its current area is not empty.
    pub has_area: bool,
}

impl PhotoState {
    /// Not looked for yet, or ticked while unsure and not opened.
    pub(crate) fn blocks_removal(&self) -> bool {
        self.status == AreaStatus::Pending
            || (self.status == AreaStatus::Unsure && self.included && !self.reviewed)
    }

    /// Ticked with an area.
    pub(crate) fn is_to_remove(&self) -> bool {
        self.included && self.has_area
    }
}

impl AppCore {
    /// Finds the set's watermark on every photo that has neither a result nor an area of its
    /// own, reporting [`ProgressEvent::Matching`] (starting with `done: 0`). The set is read
    /// first ([`ProgressEvent::Reading`]) when it has no batch layers yet (a saved watermark
    /// was used) or photos of the confirmed template's size were added since they were read
    /// (matching uses only the layer of that size).
    ///
    /// Photos are matched in parallel on at most half the cores; each worker reads, matches
    /// and drops one photo before the next. Every result is stored as soon as it is ready, so a
    /// cancelled run keeps the photos it finished. A photo that cannot be read counts as not
    /// found. Fails with [`AppError::NothingToRemove`] before a set area is confirmed.
    pub fn match_all(&self, ev: Events) -> Result<MatchSummary> {
        let job = self.jobs.start()?;
        let (template, layers, todo) = {
            let s = self.session();
            let template = s.template.clone().ok_or(AppError::NothingToRemove)?;
            let layers = if s.layers_stale((template.ref_w, template.ref_h)) {
                None
            } else {
                s.layers.clone()
            };
            (template, layers, s.unmatched())
        };
        let layers = match layers {
            Some(l) => l,
            None if todo.is_empty() => Arc::new(SetLayers::none()),
            None => self.read_layers(&job, ev)?,
        };
        self.match_photos(&job, &template, &layers, &todo, ev)?;
        Ok(self.summary())
    }

    fn match_photos(
        &self,
        job: &Job,
        template: &WatermarkTemplate,
        layers: &SetLayers,
        todo: &[Unmatched],
        ev: Events,
    ) -> Result<()> {
        let total = todo.len() as u32;
        // Held while the event is sent, so events go out one at a time with `done` growing.
        let done = Mutex::new(0u32);
        ev(ProgressEvent::Matching { done: 0, total });
        let one = |(id, path, (w, h)): &Unmatched| -> Result<()> {
            job.check()?;
            let matched = match load(path) {
                Ok(photo) => Matched::new(&match_photo(template, &photo.rgb, layers)),
                // Nothing can be removed from a photo that cannot be read.
                Err(_) => Matched {
                    status: MatchStatus::NotFound,
                    area: Arc::new(Area::empty(*w, *h)),
                },
            };
            self.session().set_match(*id, matched);
            let mut done = lock(&done);
            *done += 1;
            ev(ProgressEvent::Matching { done: *done, total });
            Ok(())
        };
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get()) / 2;
        match rayon::ThreadPoolBuilder::new()
            .num_threads(threads.max(1))
            .build()
        {
            Ok(pool) => pool.install(|| todo.par_iter().try_for_each(one)),
            Err(_) => todo.iter().try_for_each(&one),
        }
    }

    /// See [`MatchSummary`].
    fn summary(&self) -> MatchSummary {
        let states = self.photo_states();
        let mut sum = MatchSummary {
            found: 0,
            unsure: 0,
            not_found: 0,
            total: states.len() as u32,
        };
        for st in &states {
            match st.status {
                AreaStatus::Found => sum.found += 1,
                AreaStatus::Edited if st.has_area => sum.found += 1,
                AreaStatus::Unsure => sum.unsure += 1,
                AreaStatus::NotFound | AreaStatus::Edited => sum.not_found += 1,
                AreaStatus::Pending => {}
            }
        }
        sum
    }

    /// Every photo's step-3 state, in list order.
    pub fn photo_states(&self) -> Vec<PhotoState> {
        self.session().states()
    }

    /// Notes that photo `id` was opened in the editor, which settles an unsure result.
    pub fn open_photo(&self, id: PhotoId) -> Result<()> {
        let mut s = self.session();
        s.photos.get(id).ok_or_else(|| no_photo(id))?;
        s.data.entry(id).or_default().reviewed = true;
        Ok(())
    }

    /// Makes the draft of photo `id` (or, without one, its current area) the area of this
    /// photo alone: status [`AreaStatus::Edited`], reviewed, and ticked when the area is not
    /// empty. The set's watermark stays as it is.
    pub fn save_photo_area(&self, id: PhotoId) -> Result<()> {
        let ((w, h), draft, current) = {
            let s = self.session();
            let info = &s.photos.get(id).ok_or_else(|| no_photo(id))?.info;
            let size = (info.width, info.height);
            let draft = s.data.get(&id).and_then(|d| d.draft.clone());
            (size, draft, s.area(id))
        };
        // The full-size area is made without the lock; the draft is cleared once it is saved.
        let area = match (draft, current) {
            (Some(d), _) => Arc::new(Area::from_mask(&area_from_edit(&d.bytes, d.w, d.h, w, h)?)),
            (None, Some(a)) => a,
            (None, None) => Arc::new(Area::empty(w, h)),
        };
        let mut s = self.session();
        s.photos.get(id).ok_or_else(|| no_photo(id))?;
        s.save_own(id, area);
        Ok(())
    }

    /// Ticks or unticks photo `id` for removal. A photo whose area is empty cannot be ticked
    /// ([`AppError::NothingToRemove`]).
    pub fn set_included(&self, id: PhotoId, included: bool) -> Result<()> {
        let mut s = self.session();
        s.photos.get(id).ok_or_else(|| no_photo(id))?;
        if included && !s.has_area(id) {
            return Err(AppError::NothingToRemove);
        }
        s.data.entry(id).or_default().included = included;
        Ok(())
    }

    /// The photos that keep removal from starting, in list order: ticked unsure photos nobody
    /// opened, and photos the watermark was not looked for on yet.
    pub fn removal_blockers(&self) -> Vec<PhotoId> {
        self.photo_states()
            .into_iter()
            .filter(PhotoState::blocks_removal)
            .map(|st| st.id)
            .collect()
    }

    /// The ticked photos with an area, in list order.
    pub fn to_remove(&self) -> Vec<PhotoId> {
        self.photo_states()
            .into_iter()
            .filter(PhotoState::is_to_remove)
            .map(|st| st.id)
            .collect()
    }

    /// JPEG of photo `id` for the step-3 grid (long side [`crate::images::GRID_LONG_SIDE`])
    /// with its current area tinted. The plain thumbnail is kept in memory; the area is drawn
    /// on every call, as it may have changed.
    pub fn grid_image(&self, id: PhotoId) -> Result<Vec<u8>> {
        let (path, area) = {
            let s = self.session();
            let path = s.photos.get(id).ok_or_else(|| no_photo(id))?.path.clone();
            (path, s.area(id))
        };
        let thumb = self.images.grid(id, &path)?;
        Ok(match area {
            Some(a) if !a.is_empty() => jpeg(
                &overlay(&thumb, &a.resized(thumb.width(), thumb.height())),
                GRID_QUALITY,
            ),
            _ => jpeg(&thumb, GRID_QUALITY),
        })
    }
}

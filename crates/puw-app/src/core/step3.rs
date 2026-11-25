//! Step 3: finding the current group's watermark in the photos with no group, the per-photo
//! states and edits, the groups, the rule that keeps removal (and the next group) from starting
//! until unsure photos are handled, and the grid images. The locking rules of [`AppCore`] (see
//! `core.rs`) apply here too.

use std::sync::{Arc, Mutex};

use puw_core::{MatchStatus, SetLayers, WatermarkTemplate, load, match_photo};
use rayon::prelude::*;
use serde::Serialize;

use super::{AppCore, lock, no_photo};
use crate::images::{area_from_edit, jpeg, overlay};
use crate::jobs::{Events, Job, ProgressEvent};
use crate::photos::{PhotoId, Problem};
use crate::session::{Area, Matched, Unmatched};
use crate::{AppError, Result};

const GRID_QUALITY: u8 = 85;

/// How finding the current group's watermark went, over the photos whose result (or own
/// area) came from that group. A photo with an area of its own counts as found, or as not
/// found when that area is empty. Photos of other groups, photos not looked for yet and
/// photos that cannot be read are not counted, not even in `total`.
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
    /// The photo cannot be read (gone or damaged): never ticked, opened or counted as found
    /// or not found. [`crate::photos::PhotoInfo::problem`] says why.
    Unreadable,
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
    /// The group its result or own area came from.
    pub group: Option<u32>,
    /// It is in group `group`: ticked, readable, with a non-empty area, and not an unsure
    /// photo nobody opened.
    pub member: bool,
}

/// A watermark group as step 3 and Done show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupInfo {
    pub number: u32,
    /// Its members.
    pub photos: u32,
    /// The saved watermark it was started from.
    pub saved_name: Option<String>,
    /// That saved watermark's area was changed in step 2.
    pub edited: bool,
    /// It can be saved as a watermark for next time (it has a confirmed area).
    pub can_save: bool,
}

/// Every group, and where the photos in none stand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupsState {
    pub groups: Vec<GroupInfo>,
    /// The number of the current group (the last one made).
    pub current: Option<u32>,
    /// The current group's matching has finished.
    pub matched: bool,
    /// Readable photos in no group.
    pub leftovers: u32,
    /// Photos that cannot be read.
    pub unreadable: u32,
}

impl PhotoState {
    /// Ticked while unsure and not opened, or not looked for yet while `matching_open` (the
    /// current group's matching is not done; once it is, such a photo just has no group).
    pub(crate) fn blocks_removal(&self, matching_open: bool) -> bool {
        (self.status == AreaStatus::Pending && matching_open)
            || (self.status == AreaStatus::Unsure && self.included && !self.reviewed)
    }

    /// Ticked with an area.
    pub(crate) fn is_to_remove(&self) -> bool {
        self.included && self.has_area
    }
}

/// See [`MatchSummary`]: `states` counted for group `group`.
fn summary(states: &[PhotoState], group: u32) -> MatchSummary {
    let mut sum = MatchSummary {
        found: 0,
        unsure: 0,
        not_found: 0,
        total: 0,
    };
    for st in states.iter().filter(|st| st.group == Some(group)) {
        match st.status {
            AreaStatus::Found => sum.found += 1,
            AreaStatus::Edited if st.has_area => sum.found += 1,
            AreaStatus::Unsure => sum.unsure += 1,
            AreaStatus::NotFound | AreaStatus::Edited => sum.not_found += 1,
            AreaStatus::Pending | AreaStatus::Unreadable => continue,
        }
        sum.total += 1;
    }
    sum
}

impl AppCore {
    /// Finds the current group's watermark on every readable photo that has neither a result
    /// nor an area of its own, reporting [`ProgressEvent::Matching`] (starting with `done: 0`).
    /// Once this has finished for the group, it matches nothing more until the group's area
    /// changes. The photos to match are read first ([`ProgressEvent::Reading`]) when there
    /// are no batch layers yet (a saved watermark was used) or one of them of the template's
    /// size was not read for them (matching uses only the layer of that size).
    ///
    /// Photos are matched in parallel on at most half the cores; each worker reads, matches
    /// and drops one photo before the next. Every result is stored as soon as it is ready, so a
    /// cancelled run keeps the photos it finished. A photo that cannot be read becomes
    /// [`AreaStatus::Unreadable`] and gets no group. Fails with [`AppError::NothingToRemove`]
    /// while there is no group.
    pub fn match_all(&self, ev: Events) -> Result<MatchSummary> {
        let job = self.jobs.start()?;
        let (number, template, layers, todo) = {
            let s = self.session();
            let group = s.current().ok_or(AppError::NothingToRemove)?;
            let (number, template) = (group.number, group.template.clone());
            // `Pending` photos only: never one that cannot be read.
            let todo = if group.matched {
                Vec::new()
            } else {
                s.unmatched()
            };
            let ids: Vec<PhotoId> = todo.iter().map(|t| t.0).collect();
            let layers = if s.layers_stale((template.ref_w, template.ref_h), &ids) {
                None
            } else {
                s.layers.clone()
            };
            (number, template, layers, todo)
        };
        let layers = match layers {
            Some(l) => l,
            None if todo.is_empty() => Arc::new(SetLayers::none()),
            None => {
                let (ids, paths): (Vec<PhotoId>, Vec<_>) =
                    todo.iter().map(|(id, p, _)| (*id, p.clone())).unzip();
                self.read_layers(&job, ev, &ids, &paths)?
            }
        };
        self.match_photos(&job, number, &template, &layers, &todo, ev)?;
        let mut s = self.session();
        if let Some(g) = s.groups.iter_mut().find(|g| g.number == number) {
            g.matched = true;
        }
        Ok(summary(&s.states(), number))
    }

    fn match_photos(
        &self,
        job: &Job,
        group: u32,
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
            match load(path) {
                Ok(photo) => {
                    let matched = Matched::new(&match_photo(template, &photo.rgb, layers));
                    self.session().set_match(*id, matched, Some(group));
                }
                // Nothing can be removed from a photo that cannot be read.
                Err(e) => {
                    let mut s = self.session();
                    s.set_match(
                        *id,
                        Matched {
                            status: MatchStatus::NotFound,
                            area: Arc::new(Area::empty(*w, *h)),
                            confidence: 0.0,
                        },
                        None,
                    );
                    let problem = Problem::of(&e.into()).unwrap_or(Problem::Damaged);
                    s.photos.set_problem(*id, problem);
                }
            }
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

    /// Every group, and where the photos in none stand.
    pub fn groups(&self) -> GroupsState {
        self.session().groups_state()
    }

    /// Removes every group, and every photo's result, tick, opened state and own area. The
    /// step-2 guesses, previews and removal results stay.
    pub fn clear_groups(&self) -> Result<()> {
        let _job = self.jobs.start()?;
        self.session().clear_groups();
        Ok(())
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
    /// empty. The photo keeps an earlier group it is in, else it joins the current one; an
    /// empty area on a photo of an earlier group drops it from that group. The groups'
    /// watermarks stay as they are. Fails with [`AppError::NotReady`] while there is no group.
    pub fn save_photo_area(&self, id: PhotoId) -> Result<()> {
        let ((w, h), draft, current) = {
            let s = self.session();
            let info = &s.photos.get(id).ok_or_else(|| no_photo(id))?.info;
            if s.current().is_none() {
                return Err(AppError::NotReady);
            }
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
        if s.current().is_none() {
            return Err(AppError::NotReady);
        }
        s.save_own(id, area);
        Ok(())
    }

    /// Ticks or unticks photo `id` for removal. Only a photo in a group with a non-empty area
    /// can be ticked ([`AppError::NothingToRemove`]). Unticking a photo of an earlier group
    /// drops it from that group at once (it waits for the next group); a photo of the current
    /// group keeps its result and can be ticked again.
    pub fn set_included(&self, id: PhotoId, included: bool) -> Result<()> {
        let mut s = self.session();
        s.photos.get(id).ok_or_else(|| no_photo(id))?;
        let grouped = s.data.get(&id).is_some_and(|d| d.group.is_some());
        if included && !(grouped && s.has_area(id)) {
            return Err(AppError::NothingToRemove);
        }
        s.set_included(id, included);
        Ok(())
    }

    /// The photos that keep removal (and the next group) from starting, in list order: ticked
    /// unsure photos nobody opened, and, while the current group's matching is not done, the
    /// readable photos the watermark was not looked for on yet. Once it is done, such a photo
    /// just has no group.
    pub fn removal_blockers(&self) -> Vec<PhotoId> {
        self.session().removal_blockers()
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
        let thumb = self.noting_problem(id, self.images.grid(id, &path))?;
        Ok(match area {
            Some(a) if !a.is_empty() => jpeg(
                &overlay(&thumb, &a.resized(thumb.width(), thumb.height())),
                GRID_QUALITY,
            ),
            _ => jpeg(&thumb, GRID_QUALITY),
        })
    }
}

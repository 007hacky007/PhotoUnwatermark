//! What the app knows about the current set of photos. Plain data: `AppCore` locks it only to
//! read a job's inputs and to write its results, never while a job works.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use puw_core::{Mask, MatchResult, MatchStatus, SetLayers, WatermarkTemplate};

use crate::core::{AreaStatus, DetectOutcome, GroupInfo, GroupsState, PhotoState};
use crate::photos::{PhotoId, PhotoList};
use crate::{Detail, OutputRule};

/// An area as the editor sent it, at the photo's edit size, not yet confirmed.
#[derive(Debug, Clone)]
pub(crate) struct Draft {
    pub w: u32,
    pub h: u32,
    pub bytes: Vec<u8>,
}

/// A binary area of a photo, kept as its bounding box and one bit per pixel inside it, so a
/// watermark-sized area of a 36 MP photo takes kilobytes, not the 36 MB of a full-size [`Mask`].
/// An empty area keeps no pixels at all. Expanded only when needed ([`Area::resized`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Area {
    /// Size of the photo (upright pixels).
    w: u32,
    h: u32,
    /// The bounding box: top-left corner and size; zero size when the area is empty.
    x: u32,
    y: u32,
    bw: u32,
    bh: u32,
    /// Row by row, `bw.div_ceil(8)` bytes per row; bit `i % 8` of byte `i / 8` is pixel `x + i`.
    bits: Vec<u8>,
}

impl Area {
    /// The area of the non-zero pixels of `full`.
    pub fn from_mask(full: &Mask) -> Area {
        let (w, h) = full.dimensions();
        let Some(r) = puw_core::mask::bbox(full) else {
            return Area::empty(w, h);
        };
        let (x, y, bw, bh) = (r.left() as u32, r.top() as u32, r.width(), r.height());
        let stride = bw.div_ceil(8) as usize;
        let mut bits = vec![0u8; stride * bh as usize];
        let src = full.as_raw();
        for (j, out) in bits.chunks_exact_mut(stride).enumerate() {
            let start = (y as usize + j) * w as usize + x as usize;
            for (i, &v) in src[start..start + bw as usize].iter().enumerate() {
                if v != 0 {
                    out[i / 8] |= 1 << (i % 8);
                }
            }
        }
        Area {
            w,
            h,
            x,
            y,
            bw,
            bh,
            bits,
        }
    }

    /// No area on a photo of `w` x `h`.
    pub fn empty(w: u32, h: u32) -> Area {
        Area {
            w,
            h,
            x: 0,
            y: 0,
            bw: 0,
            bh: 0,
            bits: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.bw == 0 || self.bh == 0
    }

    /// The area at full size, 0/255.
    pub fn to_full(&self) -> Mask {
        self.resized(self.w, self.h)
    }

    /// The area at `tw` x `th`, 0/255: the same as [`puw_core::mask::resize_any`] of
    /// [`Area::to_full`] (a target pixel is set when any covered photo pixel overlaps it), but
    /// without making the full-size mask.
    pub fn resized(&self, tw: u32, th: u32) -> Mask {
        let mut out = Mask::new(tw, th);
        if self.is_empty() || tw == 0 || th == 0 {
            return out;
        }
        // Target pixels [t0, t1] overlapped by photo pixel `s` of `n`, mapped onto `t` pixels.
        let span = |s: u32, n: u32, t: u32| -> (usize, usize) {
            let (s, n, t) = (u64::from(s), u64::from(n), u64::from(t));
            let t0 = s * t / n;
            let t1 = ((s + 1) * t).div_ceil(n) - 1;
            (t0 as usize, t1 as usize)
        };
        let cols: Vec<(usize, usize)> =
            (0..self.bw).map(|i| span(self.x + i, self.w, tw)).collect();
        let stride = self.bw.div_ceil(8) as usize;
        let tw = tw as usize;
        let mut row = vec![false; tw];
        let dst: &mut [u8] = &mut out;
        for (j, line) in self.bits.chunks_exact(stride).enumerate() {
            if line.iter().all(|&b| b == 0) {
                continue;
            }
            row.fill(false);
            for (i, &(t0, t1)) in cols.iter().enumerate() {
                if line[i / 8] & (1 << (i % 8)) != 0 {
                    row[t0..=t1].fill(true);
                }
            }
            let (ty0, ty1) = span(self.y + j as u32, self.h, th);
            for ty in ty0..=ty1 {
                for (d, &r) in dst[ty * tw..(ty + 1) * tw].iter_mut().zip(&row) {
                    if r {
                        *d = 255;
                    }
                }
            }
        }
        out
    }

    /// Bytes this area takes in memory.
    #[cfg(test)]
    pub fn stored_bytes(&self) -> usize {
        std::mem::size_of::<Area>() + self.bits.capacity()
    }
}

/// Where a group's watermark was found on a photo (an empty area for `NotFound`).
#[derive(Debug, Clone)]
pub(crate) struct Matched {
    pub status: MatchStatus,
    pub area: Arc<Area>,
    /// The match score.
    #[allow(
        dead_code,
        reason = "picks a saved group's step-2 photos (group_sample)"
    )]
    pub confidence: f32,
}

impl Matched {
    pub fn new(r: &MatchResult) -> Matched {
        Matched {
            status: r.status,
            area: Arc::new(Area::from_mask(&r.mask)),
            confidence: r.confidence,
        }
    }
}

/// Everything known about one photo.
#[derive(Debug, Default)]
pub(crate) struct PhotoData {
    /// The automatic step-2 guess (detection or a saved watermark's search).
    pub guess: Option<Arc<Area>>,
    /// Where the watermark of group `group` was found on this photo.
    pub matched: Option<Matched>,
    /// An area the person saved for this photo alone (may be empty).
    pub own: Option<Arc<Area>>,
    /// The group the photo's result (`matched`) or own area came from.
    pub group: Option<u32>,
    pub draft: Option<Draft>,
    /// Ticked for removal.
    pub included: bool,
    /// Opened in the editor (or saved there) since its result came.
    pub reviewed: bool,
    /// The last preview, JPEG at edit size.
    pub preview: Option<Arc<Vec<u8>>>,
    /// The file the last removal run saved for this photo, and how it was made: the area it
    /// removed, where results went and how much detail was kept.
    pub result: Option<PathBuf>,
    pub result_made: Option<Made>,
}

/// How a result was made; a photo is done again when any of it changes.
#[derive(Debug, Clone)]
pub(crate) struct Made {
    pub area: Arc<Area>,
    pub output: OutputRule,
    pub detail: Detail,
}

impl Made {
    /// The same area (the very one, not a copy that happens to be equal), output rule and
    /// detail.
    pub fn same(&self, area: &Arc<Area>, output: &OutputRule, detail: Detail) -> bool {
        Arc::ptr_eq(&self.area, area) && self.output == *output && self.detail == detail
    }
}

/// The photo a group's watermark area was confirmed on, and that area.
#[derive(Debug, Clone)]
pub(crate) struct Confirmed {
    pub path: PathBuf,
    pub area: Arc<Area>,
}

/// The saved watermark a group was started from.
#[derive(Debug, Clone)]
pub(crate) struct SavedOrigin {
    #[allow(dead_code, reason = "set when a group starts from a saved watermark")]
    pub id: String,
    pub name: String,
}

/// One watermark of the set, and the photos it was looked for on.
#[derive(Debug, Clone)]
pub(crate) struct Group {
    /// 1, 2, 3 in creation order, never reused while the groups last.
    pub number: u32,
    /// What the watermark looks like, used to find it on the photos with no group.
    pub template: Arc<WatermarkTemplate>,
    /// Where the template was learned (kept when that photo is removed from the list). `None`
    /// only for a saved watermark used as it is.
    pub confirmed: Option<Confirmed>,
    pub saved: Option<SavedOrigin>,
    /// A saved watermark whose area was changed in step 2.
    pub edited: bool,
    /// The photos step 2 shows for this group.
    pub outcome: Option<DetectOutcome>,
    /// This group's matching over the photos with no group has finished.
    pub matched: bool,
}

/// A photo waiting for matching: id, file and upright size.
pub(crate) type Unmatched = (PhotoId, PathBuf, (u32, u32));

#[derive(Debug, Default)]
pub(crate) struct Session {
    pub photos: PhotoList,
    /// The set's batch layers, once the set was read.
    pub layers: Option<Arc<SetLayers>>,
    /// The photos `layers` was built from.
    pub layers_from: HashSet<PhotoId>,
    /// The watermark groups, in creation order; the last one is the current group.
    pub groups: Vec<Group>,
    /// The photos the last first guess (or saved-watermark search) looked at.
    pub last_detect: Option<DetectOutcome>,
    pub data: HashMap<PhotoId, PhotoData>,
    /// The folder the person chose for the results of this set, when the usual one cannot
    /// be written.
    pub output_override: Option<PathBuf>,
    /// The template the strip placements use: learned from this photo and drawing (full
    /// size), kept until either changes.
    pub strip: Option<(PhotoId, Arc<Area>, Arc<WatermarkTemplate>)>,
    /// How many times `strip` was built.
    pub strip_built: usize,
}

impl Session {
    /// The current group (the last one made), if any.
    pub fn current(&self) -> Option<&Group> {
        self.groups.last()
    }

    fn current_mut(&mut self) -> Option<&mut Group> {
        self.groups.last_mut()
    }

    /// The number of the current group.
    pub fn current_number(&self) -> Option<u32> {
        self.current().map(|g| g.number)
    }

    /// There is a current group and its matching over the photos with no group is not done.
    /// Until it is, a photo not looked for yet holds up removal and the next group.
    pub fn matching_open(&self) -> bool {
        self.current().is_some_and(|g| !g.matched)
    }

    /// The photo's area: its own, else where its group's watermark was found. `None` when it
    /// has neither (the step-2 guess is not an area yet, see [`Session::start_area`]).
    pub fn area(&self, id: PhotoId) -> Option<Arc<Area>> {
        let d = self.data.get(&id)?;
        if let Some(a) = &d.own {
            return Some(a.clone());
        }
        d.matched.as_ref().map(|m| m.area.clone())
    }

    /// The area step 2 starts from: the photo's step-2 guess when it has one and is in no
    /// group, else [`Session::area`].
    pub fn start_area(&self, id: PhotoId) -> Option<Arc<Area>> {
        if let Some(g) = self.data.get(&id).and_then(|d| d.guess.clone())
            && !self.is_member(id)
        {
            return Some(g);
        }
        self.area(id)
    }

    /// The photo's area is not empty, and the photo can be read.
    pub fn has_area(&self, id: PhotoId) -> bool {
        !self.unreadable(id) && self.area(id).is_some_and(|a| !a.is_empty())
    }

    /// Reading photo `id` failed (it is gone or cannot be opened).
    pub fn unreadable(&self, id: PhotoId) -> bool {
        self.photos
            .get(id)
            .is_some_and(|e| e.info.problem.is_some())
    }

    /// `Unreadable` for a photo that cannot be read, else `Edited` with an own area, else the
    /// match status, else `Pending`.
    pub fn status(&self, id: PhotoId) -> AreaStatus {
        if self.unreadable(id) {
            return AreaStatus::Unreadable;
        }
        let Some(d) = self.data.get(&id) else {
            return AreaStatus::Pending;
        };
        if d.own.is_some() {
            return AreaStatus::Edited;
        }
        match d.matched.as_ref().map(|m| m.status) {
            Some(MatchStatus::Found) => AreaStatus::Found,
            Some(MatchStatus::Unsure) => AreaStatus::Unsure,
            Some(MatchStatus::NotFound) => AreaStatus::NotFound,
            None => AreaStatus::Pending,
        }
    }

    /// Photo `id` is in its group: ticked, readable, with a non-empty area from that (existing)
    /// group, and not an unsure photo nobody opened.
    pub fn is_member(&self, id: PhotoId) -> bool {
        let Some(d) = self.data.get(&id) else {
            return false;
        };
        let Some(n) = d.group else {
            return false;
        };
        d.included
            && self.groups.iter().any(|g| g.number == n)
            && self.has_area(id)
            && !(self.status(id) == AreaStatus::Unsure && !d.reviewed)
    }

    /// Photos with neither an own area nor a match result, in list order.
    pub fn unmatched(&self) -> Vec<Unmatched> {
        self.listed(|id| self.status(id) == AreaStatus::Pending)
    }

    /// The photos that can be read and are in no group, in list order.
    pub fn ungrouped(&self) -> Vec<Unmatched> {
        self.listed(|id| !self.unreadable(id) && !self.is_member(id))
    }

    fn listed(&self, keep: impl Fn(PhotoId) -> bool) -> Vec<Unmatched> {
        self.photos
            .infos()
            .into_iter()
            .zip(self.photos.paths())
            .filter(|(i, _)| keep(i.id))
            .map(|(i, p)| (i.id, p, (i.width, i.height)))
            .collect()
    }

    /// Keeps `layers`, built from the photos `from`.
    pub fn set_layers(&mut self, layers: Arc<SetLayers>, from: &[PhotoId]) {
        self.layers = Some(layers);
        self.layers_from = from.iter().copied().collect();
    }

    /// True when there are no layers yet, or a photo of `size` among `todo` (the photos about
    /// to be matched) was not read for them. Matching uses only the layer of the template's
    /// size, so other sizes never count.
    pub fn layers_stale(&self, size: (u32, u32), todo: &[PhotoId]) -> bool {
        if self.layers.is_none() {
            return true;
        }
        todo.iter().any(|&id| {
            !self.layers_from.contains(&id)
                && self
                    .photos
                    .get(id)
                    .is_some_and(|e| (e.info.width, e.info.height) == size)
        })
    }

    /// Replaces every step-2 guess with `guesses` and drops every draft, so a draft drawn on
    /// an earlier guess can never stand in for the area shown now.
    pub fn set_guesses(&mut self, guesses: Vec<(PhotoId, Mask)>) {
        self.drop_guesses();
        for (id, mask) in guesses {
            self.data.entry(id).or_default().guess = Some(Arc::new(Area::from_mask(&mask)));
        }
    }

    /// Drops every step-2 guess and every draft.
    fn drop_guesses(&mut self) {
        for d in self.data.values_mut() {
            d.guess = None;
            d.draft = None;
        }
    }

    /// Stores where group `group`'s watermark was found on photo `id` (`None` for a photo that
    /// could not be read: it gets no group). Unless the photo has its own area, the result
    /// becomes its area: it is ticked when found or unsure with a non-empty area, and needs a
    /// new look.
    pub fn set_match(&mut self, id: PhotoId, m: Matched, group: Option<u32>) {
        let d = self.data.entry(id).or_default();
        if d.own.is_none() {
            d.included = m.status != MatchStatus::NotFound && !m.area.is_empty();
            d.reviewed = false;
            d.group = group;
        }
        d.matched = Some(m);
    }

    /// Forgets photo `id`'s result, own area and group: it waits for matching again, unticked
    /// and not opened.
    pub fn reset(&mut self, id: PhotoId) {
        if let Some(d) = self.data.get_mut(&id) {
            d.matched = None;
            d.own = None;
            d.group = None;
            d.included = false;
            d.reviewed = false;
        }
    }

    /// Photo `id`'s result or own area came from a group other than the current one (so an
    /// earlier one).
    fn in_earlier_group(&self, id: PhotoId) -> bool {
        let group = self.data.get(&id).and_then(|d| d.group);
        group.is_some() && group != self.current_number()
    }

    /// True when confirming `area` (full size) on photo `id` would change nothing: the photo
    /// has no area of its own, the current group found it with this very area, and the
    /// group's watermark was learned from that photo and area (or it is a saved watermark
    /// used as it is). Learning from another photo makes another watermark, even with the
    /// area it was found with there.
    pub fn confirms_nothing_new(&self, id: PhotoId, area: &Area) -> bool {
        let (Some(d), Some(g), Some(e)) = (self.data.get(&id), self.current(), self.photos.get(id))
        else {
            return false;
        };
        let learned_here = g
            .confirmed
            .as_ref()
            .is_none_or(|c| c.path == e.path && *c.area == *area);
        d.own.is_none()
            && d.group == Some(g.number)
            && learned_here
            && d.matched
                .as_ref()
                .is_some_and(|m| m.status == MatchStatus::Found && *m.area == *area)
    }

    /// The current group's matching has finished and nothing holds up removal, so the next
    /// group can start. True when there is no group.
    pub fn ready_for_next_group(&self) -> bool {
        self.current().is_none() || (!self.matching_open() && self.removal_blockers().is_empty())
    }

    /// Starts the next group (group 1 when there is none) and returns its number. Every photo
    /// in no group is reset first (result, own area and group gone, unticked, not opened);
    /// the photos of earlier groups stay as they are. Every step-2 guess and draft is dropped.
    pub fn begin_group(
        &mut self,
        template: Arc<WatermarkTemplate>,
        confirmed: Option<Confirmed>,
        saved: Option<SavedOrigin>,
        outcome: Option<DetectOutcome>,
    ) -> u32 {
        let leftovers: Vec<PhotoId> = self.ungrouped().into_iter().map(|u| u.0).collect();
        for id in leftovers {
            self.reset(id);
        }
        self.drop_guesses();
        let number = self.current_number().map_or(1, |n| n + 1);
        self.groups.push(Group {
            number,
            template,
            confirmed,
            saved,
            edited: false,
            outcome,
            matched: false,
        });
        number
    }

    /// The photos step 2 showed for a group confirmed on photo `id`: the last first guess when
    /// it looked at `id`, else `id` alone.
    fn outcome_for(&self, id: PhotoId) -> DetectOutcome {
        match &self.last_detect {
            Some(o) if o.sample.contains(&id) => o.clone(),
            _ => DetectOutcome {
                sample: vec![id],
                representative: id,
            },
        }
    }

    /// Makes `area` (full size, binary) of photo `id` the watermark of a new group, which
    /// becomes the current one ([`Session::begin_group`]). The photo counts as found in it,
    /// replacing any own area it had. Returns the group's number.
    pub fn confirm_new(&mut self, id: PhotoId, template: WatermarkTemplate, area: &Mask) -> u32 {
        let area = Arc::new(Area::from_mask(area));
        let confirmed = self.confirmed_on(id, &area);
        let outcome = self.outcome_for(id);
        let number = self.begin_group(Arc::new(template), confirmed, None, Some(outcome));
        self.found_by_confirm(id, area, number);
        number
    }

    /// Makes `area` (full size, binary) of photo `id` the current group's watermark instead of
    /// the one it had (a new group when there is none). Every photo whose result came from
    /// this group, the photo it was confirmed on before included, waits for matching again
    /// (unticked, not opened); areas saved for single photos stay, and so do earlier groups.
    /// Photo `id` counts as found with the new area, replacing any own area it had. Every
    /// step-2 guess and draft is dropped.
    pub fn confirm_current(&mut self, id: PhotoId, template: WatermarkTemplate, area: &Mask) {
        let Some(number) = self.current_number() else {
            self.confirm_new(id, template, area);
            return;
        };
        let area = Arc::new(Area::from_mask(area));
        let stale: Vec<PhotoId> = self
            .data
            .iter()
            .filter(|(pid, d)| **pid != id && d.own.is_none() && d.group == Some(number))
            .map(|(pid, _)| *pid)
            .collect();
        for pid in stale {
            self.reset(pid);
        }
        self.drop_guesses();
        let confirmed = self.confirmed_on(id, &area);
        let outcome = match self.current().and_then(|g| g.outcome.clone()) {
            Some(o) if o.sample.contains(&id) => o,
            _ => self.outcome_for(id),
        };
        if let Some(g) = self.current_mut() {
            g.template = Arc::new(template);
            g.confirmed = confirmed;
            g.edited |= g.saved.is_some();
            g.outcome = Some(outcome);
            g.matched = false;
        }
        self.found_by_confirm(id, area, number);
    }

    fn confirmed_on(&self, id: PhotoId, area: &Arc<Area>) -> Option<Confirmed> {
        self.photos.get(id).map(|e| Confirmed {
            path: e.path.clone(),
            area: area.clone(),
        })
    }

    /// Photo `id` was confirmed with `area` for group `number`: found there, ticked, and its
    /// own area and draft are gone.
    fn found_by_confirm(&mut self, id: PhotoId, area: Arc<Area>, number: u32) {
        if let Some(d) = self.data.get_mut(&id) {
            d.own = None;
            d.draft = None;
        }
        let matched = Matched {
            status: MatchStatus::Found,
            area,
            confidence: 1.0,
        };
        self.set_match(id, matched, Some(number));
    }

    /// Makes `area` photo `id`'s own area: reviewed, and ticked when it is not empty. The
    /// photo keeps an earlier group it is in, else it joins the current one; an empty area on
    /// a photo of an earlier group drops it from that group instead ([`Session::reset`]).
    /// Does nothing when there is no group.
    pub fn save_own(&mut self, id: PhotoId, area: Arc<Area>) {
        let Some(current) = self.current_number() else {
            return;
        };
        let earlier = self.in_earlier_group(id);
        if earlier && area.is_empty() {
            self.reset(id);
            return;
        }
        let d = self.data.entry(id).or_default();
        if !earlier {
            d.group = Some(current);
        }
        d.included = !area.is_empty();
        d.reviewed = true;
        d.own = Some(area);
        d.draft = None;
    }

    /// Ticks or unticks photo `id`. Unticking a photo of an earlier group drops it from that
    /// group at once (its area goes and it waits for the next group's matching); a photo of
    /// the current group keeps its result, so it can be ticked again.
    pub fn set_included(&mut self, id: PhotoId, included: bool) {
        if !included && self.in_earlier_group(id) {
            self.reset(id);
            return;
        }
        self.data.entry(id).or_default().included = included;
    }

    /// The photos that keep removal (and the next group) from starting, in list order: ticked
    /// unsure photos nobody opened, and, while the current group's matching is not done, the
    /// readable photos not looked for yet.
    pub fn removal_blockers(&self) -> Vec<PhotoId> {
        let open = self.matching_open();
        self.states()
            .into_iter()
            .filter(|st| st.blocks_removal(open))
            .map(|st| st.id)
            .collect()
    }

    /// Every photo's state, in list order.
    pub fn states(&self) -> Vec<PhotoState> {
        self.photos
            .infos()
            .iter()
            .map(|i| {
                let d = self.data.get(&i.id);
                PhotoState {
                    id: i.id,
                    status: self.status(i.id),
                    included: d.is_some_and(|d| d.included) && i.problem.is_none(),
                    reviewed: d.is_some_and(|d| d.reviewed),
                    has_area: self.has_area(i.id),
                    group: d.and_then(|d| d.group),
                    member: self.is_member(i.id),
                }
            })
            .collect()
    }

    /// The groups with their member counts, and how many photos are in none.
    pub fn groups_state(&self) -> GroupsState {
        let mut members: HashMap<u32, u32> = HashMap::new();
        let (mut leftovers, mut unreadable) = (0, 0);
        for i in self.photos.infos() {
            if self.unreadable(i.id) {
                unreadable += 1;
            } else if self.is_member(i.id) {
                let n = self.data[&i.id].group.expect("a member has a group");
                *members.entry(n).or_default() += 1;
            } else {
                leftovers += 1;
            }
        }
        GroupsState {
            groups: self
                .groups
                .iter()
                .map(|g| GroupInfo {
                    number: g.number,
                    photos: members.get(&g.number).copied().unwrap_or(0),
                    saved_name: g.saved.as_ref().map(|s| s.name.clone()),
                    edited: g.edited,
                    can_save: g.confirmed.is_some(),
                })
                .collect(),
            current: self.current_number(),
            matched: self.current().is_some_and(|g| g.matched),
            leftovers,
            unreadable,
        }
    }

    /// Removes every group, and every photo's result, tick, opened state and own area, and
    /// forgets the strip template. The step-2 guesses, the last first guess, previews and
    /// removal results stay.
    pub fn clear_groups(&mut self) {
        self.groups.clear();
        self.strip = None;
        let ids: Vec<PhotoId> = self.data.keys().copied().collect();
        for id in ids {
            self.reset(id);
        }
    }

    pub fn remove(&mut self, id: PhotoId) {
        self.photos.remove(id);
        self.data.remove(&id);
    }

    /// Forgets the photos and everything learned from them. Ids are not reused.
    pub fn clear(&mut self) {
        self.photos.clear();
        self.layers = None;
        self.layers_from.clear();
        self.groups.clear();
        self.last_detect = None;
        self.data.clear();
        self.output_override = None;
        self.strip = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use puw_core::MatchBasis;
    use puw_core::mask::{binarize, resize_any};

    /// A `w` x `h` mask with a speckled block, the same for the same `seed`.
    fn speckles(w: u32, h: u32, seed: u32) -> Mask {
        Mask::from_fn(w, h, |x, y| {
            let v =
                (x.wrapping_mul(7919) ^ y.wrapping_mul(104_729) ^ seed).wrapping_mul(2_654_435_761);
            let inside = x > w / 5 && x < w / 2 + seed % 7 && y > h / 3 && y < h / 2 + 3;
            image::Luma([if inside && !v.is_multiple_of(5) {
                200
            } else {
                0
            }])
        })
    }

    #[test]
    fn area_resizes_like_the_full_mask() {
        for (w, h, seed) in [(37, 23, 1), (100, 61, 2), (13, 77, 3), (64, 64, 4)] {
            let full = speckles(w, h, seed);
            let area = Area::from_mask(&full);
            assert!(!area.is_empty());
            assert_eq!(area.to_full(), binarize(&full), "{w}x{h}");
            for (tw, th) in [(w / 3, h / 2), (w * 2 + 1, h * 3), (5, 9), (1, 1)] {
                assert_eq!(
                    area.resized(tw, th),
                    resize_any(&full, tw, th),
                    "{w}x{h} -> {tw}x{th}"
                );
            }
        }
        let none = Area::from_mask(&Mask::new(30, 20));
        assert!(none.is_empty());
        assert_eq!(none.resized(7, 5), Mask::new(7, 5));
        assert_eq!(none.stored_bytes(), std::mem::size_of::<Area>());
    }

    #[test]
    fn match_on_a_big_photo_is_stored_small() {
        // A 6000x4000 photo (24 MB as a full-size mask) with a 1200x300 watermark found on it.
        let mut mask = Mask::new(6000, 4000);
        for y in 3500u32..3800 {
            for x in 4500..5700 {
                if !(x + y).is_multiple_of(3) {
                    mask.put_pixel(x, y, image::Luma([255]));
                }
            }
        }
        let r = MatchResult {
            status: MatchStatus::Found,
            confidence: 0.9,
            mask,
            basis: MatchBasis::Search,
            transform: None,
        };
        let mut s = Session::default();
        s.set_match(7, Matched::new(&r), Some(1));
        let stored = s.data[&7].matched.as_ref().unwrap().area.stored_bytes();
        assert!(stored < 50_000, "{stored} bytes");
        assert!(s.area(7).unwrap().to_full() == r.mask, "the area changed");
        assert_eq!(s.status(7), AreaStatus::Found);
        assert!(s.data[&7].included);

        // Even an area over the whole photo takes an eighth of the full-size mask.
        let whole = Area::from_mask(&Mask::from_pixel(6000, 4000, image::Luma([255])));
        assert!(whole.stored_bytes() < 3_100_000, "{}", whole.stored_bytes());

        // Nothing found keeps no pixels.
        let none = Matched::new(&MatchResult {
            status: MatchStatus::NotFound,
            confidence: 0.1,
            mask: Mask::new(6000, 4000),
            basis: MatchBasis::Search,
            transform: None,
        });
        assert_eq!(none.area.stored_bytes(), std::mem::size_of::<Area>());
    }
}

//! What the app knows about the current set of photos. Plain data: `AppCore` locks it only to
//! read a job's inputs and to write its results, never while a job works.

use std::collections::HashMap;
use std::ops::Deref;
use std::sync::Arc;

use puw_core::{Mask, MatchBasis, MatchResult, MatchStatus, SetLayers, WatermarkTemplate};

use crate::photos::{PhotoId, PhotoList};

/// An area as the editor sent it, at the photo's edit size, not yet confirmed.
#[derive(Debug, Clone)]
pub(crate) struct Draft {
    pub w: u32,
    pub h: u32,
    pub bytes: Vec<u8>,
}

/// Everything known about one photo. Areas are full size (the upright photo's pixels).
#[derive(Debug, Default)]
pub(crate) struct PhotoData {
    /// The automatic step-2 guess (detection or a saved watermark's search).
    pub guess: Option<Arc<Mask>>,
    /// Where the set's watermark was found on this photo.
    pub matched: Option<Arc<MatchResult>>,
    /// An area the person drew for this photo alone.
    pub own: Option<Arc<Mask>>,
    pub draft: Option<Draft>,
}

/// A shared reference to a full-size area, cheap to take while the session is locked.
pub(crate) enum Area {
    Mask(Arc<Mask>),
    Matched(Arc<MatchResult>),
}

impl Deref for Area {
    type Target = Mask;

    fn deref(&self) -> &Mask {
        match self {
            Area::Mask(m) => m,
            Area::Matched(r) => &r.mask,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct Session {
    pub photos: PhotoList,
    /// The set's batch layers, once detection has read the set.
    pub layers: Option<Arc<SetLayers>>,
    /// What the confirmed step-2 area looks like, used to find it on the other photos.
    pub template: Option<Arc<WatermarkTemplate>>,
    pub data: HashMap<PhotoId, PhotoData>,
}

impl Session {
    /// The photo's current area: its own, else where the set's watermark was found, else the
    /// step-2 guess. `None` when there is none of these.
    pub fn area(&self, id: PhotoId) -> Option<Area> {
        let d = self.data.get(&id)?;
        if let Some(m) = &d.own {
            return Some(Area::Mask(m.clone()));
        }
        if let Some(r) = &d.matched {
            return Some(Area::Matched(r.clone()));
        }
        d.guess.clone().map(Area::Mask)
    }

    /// Replaces every step-2 guess with `guesses`.
    pub fn set_guesses(&mut self, guesses: Vec<(PhotoId, Mask)>) {
        for d in self.data.values_mut() {
            d.guess = None;
        }
        for (id, mask) in guesses {
            self.data.entry(id).or_default().guess = Some(Arc::new(mask));
        }
    }

    /// Makes `area` (full size, binary) of photo `id` the set's watermark: earlier matches of
    /// photos without their own area are cleared, and this photo counts as found with it.
    pub fn confirm(&mut self, id: PhotoId, template: WatermarkTemplate, area: Mask) {
        self.template = Some(Arc::new(template));
        for d in self.data.values_mut() {
            if d.own.is_none() {
                d.matched = None;
            }
        }
        let d = self.data.entry(id).or_default();
        d.draft = None;
        d.matched = Some(Arc::new(MatchResult {
            status: MatchStatus::Found,
            confidence: 1.0,
            mask: area,
            basis: MatchBasis::Search,
        }));
    }

    pub fn remove(&mut self, id: PhotoId) {
        self.photos.remove(id);
        self.data.remove(&id);
    }

    /// Forgets the photos and everything learned from them. Ids are not reused.
    pub fn clear(&mut self) {
        self.photos.clear();
        self.layers = None;
        self.template = None;
        self.data.clear();
    }
}

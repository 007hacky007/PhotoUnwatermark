//! What the app knows about the current set of photos. Plain data: `AppCore` locks it only to
//! read a job's inputs and to write its results, never while a job works.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use puw_core::{Mask, MatchResult, MatchStatus, SetLayers, WatermarkTemplate};

use crate::core::{AreaStatus, PhotoState};
use crate::photos::{PhotoId, PhotoList};

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
#[derive(Debug, Clone)]
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

/// Where the set's watermark was found on a photo (an empty area for `NotFound`).
#[derive(Debug, Clone)]
pub(crate) struct Matched {
    pub status: MatchStatus,
    pub area: Arc<Area>,
}

impl Matched {
    pub fn new(r: &MatchResult) -> Matched {
        Matched {
            status: r.status,
            area: Arc::new(Area::from_mask(&r.mask)),
        }
    }
}

/// Everything known about one photo.
#[derive(Debug, Default)]
pub(crate) struct PhotoData {
    /// The automatic step-2 guess (detection or a saved watermark's search).
    pub guess: Option<Arc<Area>>,
    /// Where the set's watermark was found on this photo.
    pub matched: Option<Matched>,
    /// An area the person saved for this photo alone (may be empty).
    pub own: Option<Arc<Area>>,
    pub draft: Option<Draft>,
    /// Ticked for removal.
    pub included: bool,
    /// Opened in the editor (or saved there) since its result came.
    pub reviewed: bool,
    /// The last preview, JPEG at edit size.
    pub preview: Option<Arc<Vec<u8>>>,
    /// The file the last removal run saved for this photo.
    pub result: Option<PathBuf>,
}

/// The photo the set's watermark area was confirmed on, and that area.
#[derive(Debug, Clone)]
pub(crate) struct Confirmed {
    pub path: PathBuf,
    pub area: Arc<Area>,
}

/// A photo waiting for matching: id, file and upright size.
pub(crate) type Unmatched = (PhotoId, PathBuf, (u32, u32));

#[derive(Debug, Default)]
pub(crate) struct Session {
    pub photos: PhotoList,
    /// The set's batch layers, once the set was read.
    pub layers: Option<Arc<SetLayers>>,
    /// The photos that were in the set when `layers` was built.
    pub layers_from: HashSet<PhotoId>,
    /// What the confirmed step-2 area looks like, used to find it on the other photos.
    pub template: Option<Arc<WatermarkTemplate>>,
    /// Where `template` was learned (kept when that photo is removed from the list).
    pub confirmed: Option<Confirmed>,
    pub data: HashMap<PhotoId, PhotoData>,
    /// The folder the person chose for the results of this set, when the usual one cannot
    /// be written.
    pub output_override: Option<PathBuf>,
}

impl Session {
    /// The photo's current area: its own, else where the set's watermark was found, else the
    /// step-2 guess. `None` when there is none of these.
    pub fn area(&self, id: PhotoId) -> Option<Arc<Area>> {
        let d = self.data.get(&id)?;
        if let Some(a) = &d.own {
            return Some(a.clone());
        }
        if let Some(m) = &d.matched {
            return Some(m.area.clone());
        }
        d.guess.clone()
    }

    pub fn has_area(&self, id: PhotoId) -> bool {
        self.area(id).is_some_and(|a| !a.is_empty())
    }

    /// `Edited` with an own area, else the match status, else `Pending`.
    pub fn status(&self, id: PhotoId) -> AreaStatus {
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

    /// Photos with neither an own area nor a match result, in list order.
    pub fn unmatched(&self) -> Vec<Unmatched> {
        self.photos
            .infos()
            .into_iter()
            .zip(self.photos.paths())
            .filter(|(i, _)| self.status(i.id) == AreaStatus::Pending)
            .map(|(i, p)| (i.id, p, (i.width, i.height)))
            .collect()
    }

    /// Keeps `layers`, built from the photos `from`.
    pub fn set_layers(&mut self, layers: Arc<SetLayers>, from: &[PhotoId]) {
        self.layers = Some(layers);
        self.layers_from = from.iter().copied().collect();
    }

    /// True when there are no layers yet, or a photo of `size` was added after they were
    /// built. Matching uses only the layer of the template's size, so other sizes never count.
    pub fn layers_stale(&self, size: (u32, u32)) -> bool {
        if self.layers.is_none() {
            return true;
        }
        self.photos
            .infos()
            .iter()
            .any(|i| !self.layers_from.contains(&i.id) && (i.width, i.height) == size)
    }

    /// Replaces every step-2 guess with `guesses` and drops every draft, so a draft drawn on
    /// an earlier guess can never stand in for the area shown now.
    pub fn set_guesses(&mut self, guesses: Vec<(PhotoId, Mask)>) {
        for d in self.data.values_mut() {
            d.guess = None;
            d.draft = None;
        }
        for (id, mask) in guesses {
            self.data.entry(id).or_default().guess = Some(Arc::new(Area::from_mask(&mask)));
        }
    }

    /// Stores where the set's watermark was found on photo `id`. Unless the photo has its own
    /// area, it is ticked when found or unsure with a non-empty area, and needs a new look.
    pub fn set_match(&mut self, id: PhotoId, m: Matched) {
        let d = self.data.entry(id).or_default();
        if d.own.is_none() {
            d.included = m.status != MatchStatus::NotFound && !m.area.is_empty();
            d.reviewed = false;
        }
        d.matched = Some(m);
    }

    /// Makes `area` (full size, binary) of photo `id` the set's watermark: earlier results of
    /// photos without their own area are cleared (they wait for matching again, unticked and
    /// not reviewed), and this photo counts as found with it, replacing any own area it had.
    pub fn confirm(&mut self, id: PhotoId, template: WatermarkTemplate, area: &Mask) {
        self.template = Some(Arc::new(template));
        if let Some(d) = self.data.get_mut(&id) {
            d.own = None;
        }
        for d in self.data.values_mut() {
            if d.own.is_none() {
                d.matched = None;
                d.included = false;
                d.reviewed = false;
            }
        }
        let area = Arc::new(Area::from_mask(area));
        self.confirmed = self.photos.get(id).map(|e| Confirmed {
            path: e.path.clone(),
            area: area.clone(),
        });
        let matched = Matched {
            status: MatchStatus::Found,
            area,
        };
        self.set_match(id, matched);
        if let Some(d) = self.data.get_mut(&id) {
            d.draft = None;
        }
    }

    /// Makes `area` photo `id`'s own area: reviewed, and ticked when it is not empty.
    pub fn save_own(&mut self, id: PhotoId, area: Arc<Area>) {
        let d = self.data.entry(id).or_default();
        d.included = !area.is_empty();
        d.reviewed = true;
        d.own = Some(area);
        d.draft = None;
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
                    included: d.is_some_and(|d| d.included),
                    reviewed: d.is_some_and(|d| d.reviewed),
                    has_area: self.has_area(i.id),
                }
            })
            .collect()
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
        self.template = None;
        self.confirmed = None;
        self.data.clear();
        self.output_override = None;
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
        };
        let mut s = Session::default();
        s.set_match(7, Matched::new(&r));
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
        });
        assert_eq!(none.area.stored_bytes(), std::mem::size_of::<Area>());
    }
}

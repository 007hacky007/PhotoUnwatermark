//! Watermark template learnt from one confirmed photo, and matching it in the other photos of
//! a set.
//!
//! The template keeps, per piece of the confirmed mask, the mask shape and the Sobel edge
//! magnitude of the photo inside the piece's bounding box. Matching compares edge maps with a
//! masked normalised cross-correlation ([`masked_ncc`]) that only counts watermark pixels.
//!
//! All matching happens in a "reference space" where the photo's long side is 1536 px. In it, a
//! mark that scales with the photo has the same size in every photo; `k` below is the remaining
//! scale of the mark relative to the template (1.0 = the same relative size as in the reference
//! photo, i.e. `scale0` in native pixels). Searching at scale `k` resamples the target by `1/k`,
//! so the template is always used at its stored resolution.

use crate::batch::{luma_row, sobel_magnitude};
use crate::mask::{self, Mask};
use crate::ncc::masked_ncc;
use image::codecs::png::PngEncoder;
use image::imageops::{self, FilterType};
use image::{ExtendedColorType, GrayImage, ImageBuffer, ImageEncoder, ImageFormat, Luma, RgbImage};
use serde::{Deserialize, Serialize};

/// Minimum score for [`MatchStatus::Found`]. Synthetic sets: visible marks score >= 0.515 at
/// their true placement, and photos without the mark score <= 0.33 with any reference. One
/// exception reaches 0.601: an invisible-mark template on a crop of the same scene. On a real set
/// with a faint mark over a busy background, wrong placements reached 0.455, so the bar sits
/// above that. See `docs/notes/thresholds.md`.
pub const FOUND: f32 = 0.48;
/// Minimum score for [`MatchStatus::Unsure`]; below it the result is [`MatchStatus::NotFound`].
pub const UNSURE: f32 = 0.35;

/// Long side of the reference space in which edges are stored and compared.
const REF_LONG: f32 = 1536.0;
/// Coarse search runs at long side 1536 / 4 = 384.
const COARSE_DOWN: f32 = 4.0;
/// Coarse scales, relative to `scale0`: 0.7, 0.8, 0.9, 1.0, 1.1, 1.25, 1.4 and the midpoints
/// between them. The coarse match only tolerates about 3-4 % of scale error, so without the
/// midpoints a mark at e.g. 0.85 or 1.33 is missed.
const SCALES: [f32; 13] = [
    0.7, 0.75, 0.8, 0.85, 0.9, 0.95, 1.0, 1.05, 1.1, 1.175, 1.25, 1.325, 1.4,
];
/// Coarse peaks kept per scale; each gets one fine look, and the best `CANDIDATES` of those
/// are refined.
const PEAKS_PER_SCALE: usize = 3;
const CANDIDATES: usize = 2;
/// Refinement: +-8 px in 1 % scale steps over +-5 %, then +-2 px in 0.25 % steps over +-1 %.
const REFINE_PX: i32 = 8;
const FINE_PX: i32 = 2;
const REFINE_STEPS: [f32; 2] = [0.01, 0.0025];
const REFINE_SPAN: [i32; 2] = [5, 4];
/// The fixed fast path looks this far around the same relative position.
const FAST_PX: i32 = 2;
/// Independently searched pieces whose scale differs from the median by more than this leave
/// the result `Unsure` at most.
const MAX_SCALE_SPREAD: f32 = 0.1;
/// Mask components below this fraction of the mask's pixels are dropped.
const MIN_PIECE_FRAC: f64 = 0.001;
/// Components whose bounding boxes are closer than this fraction of the long side merge.
const MERGE_FRAC: f32 = 0.02;
/// A piece with fewer pixels than this share of the largest piece is not searched for: it is
/// carried along at its saved offset from the largest piece. Such a piece is a stray brush dot
/// or a speck of scene the first guess caught, far from the mark (pieces are at least
/// `MERGE_FRAC` apart). Searched on its own it would stretch the group template over most of
/// the photo and pull every score down to its own. The smallest real part of a multi-piece
/// mark in the tests (a ring logo beside the fixed set's text) holds about 18 % of the text's
/// pixels; a dot of the editor's default brush beside that text holds about 2 %.
const CARRY_FRAC: f32 = 0.1;

type FImage = ImageBuffer<Luma<f32>, Vec<f32>>;

/// One connected part of the watermark.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Piece {
    /// x, y, w, h of the piece's bounding box as fractions of the reference image.
    pub rel_bbox: [f32; 4],
    /// PNG of the binary mask cropped to the bounding box, at the reference's native resolution.
    pub shape: Vec<u8>,
    /// Sobel edge magnitude inside the bounding box at reference long side 1536, row-major.
    pub edges: Vec<f32>,
    /// Size of `edges`: the bounding box at reference long side 1536.
    pub edges_w: u32,
    pub edges_h: u32,
}

/// What [`find`] looks for: the pieces of one confirmed watermark.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WatermarkTemplate {
    pub pieces: Vec<Piece>,
    /// Native size of the reference photo.
    pub ref_w: u32,
    pub ref_h: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatchStatus {
    Found,
    Unsure,
    NotFound,
}

/// Where a [`MatchResult`]'s placement came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchBasis {
    /// Found by [`find`]'s template search.
    Search,
    /// The template's saved position, trusted because the set's batch layer supports it (see
    /// `pipeline::match_photo`).
    SetPosition,
}

#[derive(Debug, Clone)]
pub struct MatchResult {
    pub status: MatchStatus,
    /// Match score, 0..=1: the masked NCC of the placement that `mask` shows, the minimum over
    /// the pieces when there are several. For [`MatchBasis::Search`] it is the best placement
    /// found anywhere (for `NotFound`, the best of a rejected search); for
    /// [`MatchBasis::SetPosition`] it is the score at the saved position.
    pub confidence: f32,
    /// Image-size mask with the piece shapes at the found placement(s); empty (all 0) for
    /// [`MatchStatus::NotFound`].
    pub mask: Mask,
    pub basis: MatchBasis,
}

/// Learn the watermark under `mask` (non-zero = mark) in `img`.
///
/// Pieces are the 8-connected components of the mask holding at least 0.1 % of its pixels,
/// merged (transitively) when their bounding boxes are closer than 2 % of the long side. A mask
/// of a different size than `img` is resized to it first. An empty mask gives no pieces.
pub fn build_template(img: &RgbImage, mask: &Mask) -> WatermarkTemplate {
    let (w, h) = img.dimensions();
    let resized;
    let mask = if mask.dimensions() == (w, h) {
        mask
    } else {
        resized = mask::resize(mask, w, h);
        &resized
    };
    let mut t = WatermarkTemplate {
        pieces: Vec::new(),
        ref_w: w,
        ref_h: h,
    };
    if w == 0 || h == 0 {
        return t;
    }

    let comps = mask::components(mask);
    let total: u64 = comps.iter().map(|c| c.pixels as u64).sum();
    let comps: Vec<_> = comps
        .into_iter()
        .filter(|c| c.pixels as f64 >= MIN_PIECE_FRAC * total as f64)
        .collect();
    if comps.is_empty() {
        return t;
    }

    let near = MERGE_FRAC * w.max(h) as f32;
    let groups = mask::group_near(&comps, near);

    let (ww, wh) = ref_dims(w, h);
    let edges = sobel_magnitude(&working_luma(img, ww, wh), ww, wh);
    for group in groups {
        let members: Vec<_> = group.iter().map(|&i| &comps[i]).collect();
        let x0 = members.iter().map(|c| c.bbox.left()).min().unwrap() as u32;
        let y0 = members.iter().map(|c| c.bbox.top()).min().unwrap() as u32;
        let x1 = members.iter().map(|c| c.bbox.right()).max().unwrap() as u32;
        let y1 = members.iter().map(|c| c.bbox.bottom()).max().unwrap() as u32;
        let (bw, bh) = (x1 - x0 + 1, y1 - y0 + 1);
        let mut shape = GrayImage::new(bw, bh);
        for c in &members {
            let (ox, oy) = (c.bbox.left() as u32 - x0, c.bbox.top() as u32 - y0);
            for (x, y, p) in c.mask.enumerate_pixels() {
                if p.0[0] != 0 {
                    shape.put_pixel(ox + x, oy + y, Luma([255]));
                }
            }
        }
        let rel_bbox = [
            x0 as f32 / w as f32,
            y0 as f32 / h as f32,
            bw as f32 / w as f32,
            bh as f32 / h as f32,
        ];
        let (ex, ey, ew, eh) = edge_rect(rel_bbox, w, h);
        let mut crop = Vec::with_capacity(ew * eh);
        for y in ey..ey + eh {
            crop.extend_from_slice(&edges[y * ww + ex..y * ww + ex + ew]);
        }
        t.pieces.push(Piece {
            rel_bbox,
            shape: encode_png(&shape),
            edges: crop,
            edges_w: ew as u32,
            edges_h: eh as u32,
        });
    }
    t
}

/// Look for the template's watermark in `img`.
///
/// 1. Fast path: every piece at the same relative position and `scale0` (+-2 px); `Found` when
///    the score reaches [`FOUND`].
/// 2. Group search: all pieces as one rigid template, coarse at long side 384 over scales
///    `scale0 * {0.7 .. 1.4}` (see `SCALES`), the best peaks refined at long side 1536 within
///    +-8 px and +-5 % scale. Each piece is then re-scored on its own at that placement.
/// 3. When the best so far scores below [`UNSURE`] and there are several pieces, each piece is
///    searched on its own the same way, largest first, and may not overlap the placement of an
///    earlier piece. If the pieces' scales differ by more than 10 % from their median, the
///    result is `Unsure` at most.
///
/// With several pieces every score is the minimum over the pieces, the fast path included, so
/// one matching piece cannot carry a misplaced one into a wrong `Found`. The best-scoring
/// outcome gives the status. A damaged template (undecodable shape, wrong edge size) gives
/// `NotFound`.
pub fn find(t: &WatermarkTemplate, img: &RgbImage) -> MatchResult {
    let (w, h) = img.dimensions();
    let not_found = |confidence: f32| MatchResult {
        status: MatchStatus::NotFound,
        confidence,
        mask: GrayImage::new(w, h),
        basis: MatchBasis::Search,
    };
    if w == 0 || h == 0 || t.ref_w == 0 || t.ref_h == 0 || t.pieces.is_empty() {
        return not_found(0.0);
    }
    let Some(all) = t
        .pieces
        .iter()
        .map(|p| Prepared::new(p, t.ref_w, t.ref_h))
        .collect::<Option<Vec<_>>>()
    else {
        return not_found(0.0);
    };
    let (pieces, carried) = split_carried(all);
    let target = Target::new(img);

    let singles: Vec<Tmpl> = (0..pieces.len())
        .map(|i| Tmpl::new(&pieces, &[i]))
        .collect();

    let mut best = fast_path(t, &pieces, &singles, &target);
    if best.as_ref().is_none_or(|b| b.score < FOUND) {
        let group = if pieces.len() == 1 {
            search(&target, &singles[0], &[]).map(|p| Outcome::from_places(vec![(0, p)]))
        } else {
            let all: Vec<usize> = (0..pieces.len()).collect();
            let group = Tmpl::new(&pieces, &all);
            search(&target, &group, &[])
                .map(|p| check_pieces(&target, &singles, group.place_pieces(&pieces, &p)))
        };
        keep_better(&mut best, group);
        if pieces.len() >= 2 && best.as_ref().is_none_or(|b| b.score < UNSURE) {
            keep_better(&mut best, Some(search_each(&target, &singles)));
        }
    }

    let Some(best) = best else {
        return not_found(0.0);
    };
    let confidence = best.score.clamp(0.0, 1.0);
    let status = if confidence >= FOUND && !best.unsure_at_most {
        MatchStatus::Found
    } else if confidence >= UNSURE {
        MatchStatus::Unsure
    } else {
        return not_found(confidence);
    };
    let mut out = GrayImage::new(w, h);
    let size = ((target.nw, target.nh), (target.w, target.h));
    for (i, p) in &best.places {
        paste(&mut out, &pieces[*i], p, size.0, size.1);
    }
    // Carried pieces keep their offset from the largest piece, wherever that was placed.
    let anchor = best
        .places
        .iter()
        .max_by_key(|(i, _)| pieces[*i].pixels)
        .map(|(i, p)| (&pieces[*i], *p));
    if let Some((a, ap)) = anchor {
        for c in &carried {
            let pl = Placement {
                x: ap.x + c.ox - a.ox,
                y: ap.y + c.oy - a.oy,
                ..ap
            };
            paste(&mut out, c, &pl, size.0, size.1);
        }
    }
    MatchResult {
        status,
        confidence,
        mask: out,
        basis: MatchBasis::Search,
    }
}

/// Splits pieces into the ones searched for and the ones carried along (see `CARRY_FRAC`),
/// keeping their order. The largest piece is always searched for.
fn split_carried(all: Vec<Prepared>) -> (Vec<Prepared>, Vec<Prepared>) {
    let largest = all.iter().map(|p| p.pixels).max().unwrap_or(0);
    all.into_iter()
        .partition(|p| p.pixels as f32 >= CARRY_FRAC * largest as f32)
}

/// Bounding box `[x, y, w, h]`, as fractions of the reference photo, of the pieces [`find`]
/// searches for: the watermark without stray specks far from it (see `CARRY_FRAC`). `None`
/// for a template without pieces or a damaged one.
pub fn main_bbox(t: &WatermarkTemplate) -> Option<[f32; 4]> {
    let all = t
        .pieces
        .iter()
        .map(|p| Prepared::new(p, t.ref_w, t.ref_h).map(|q| (q.pixels, p.rel_bbox)))
        .collect::<Option<Vec<_>>>()?;
    let largest = all.iter().map(|(n, _)| *n).max()?;
    let main = all
        .iter()
        .filter(|(n, _)| *n as f32 >= CARRY_FRAC * largest as f32)
        .map(|(_, b)| *b);
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for [x, y, w, h] in main {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x + w);
        y1 = y1.max(y + h);
    }
    Some([x0, y0, x1 - x0, y1 - y0])
}

/// Score of the template at its saved position in `img` (same size as the reference): the
/// fast path of [`find`] alone (+-2 px, scale 1, minimum over the pieces it searches for). `None` for a damaged
/// template or a photo of another size.
pub fn saved_score(t: &WatermarkTemplate, img: &RgbImage) -> Option<f32> {
    if img.dimensions() != (t.ref_w, t.ref_h) || t.pieces.is_empty() {
        return None;
    }
    let all = t
        .pieces
        .iter()
        .map(|p| Prepared::new(p, t.ref_w, t.ref_h))
        .collect::<Option<Vec<_>>>()?;
    let (pieces, _) = split_carried(all);
    let singles: Vec<Tmpl> = (0..pieces.len())
        .map(|i| Tmpl::new(&pieces, &[i]))
        .collect();
    let target = Target::new(img);
    fast_path(t, &pieces, &singles, &target).map(|o| o.score.clamp(0.0, 1.0))
}

/// The template's pieces drawn at their saved position and scale 1 into a `t.ref_w` x `t.ref_h`
/// mask. `None` for a damaged template.
pub(crate) fn saved_placement(t: &WatermarkTemplate) -> Option<Mask> {
    let (w, h) = (t.ref_w, t.ref_h);
    if w == 0 || h == 0 || t.pieces.is_empty() {
        return None;
    }
    let (ww, wh) = ref_dims(w, h);
    let mut out = GrayImage::new(w, h);
    for p in &t.pieces {
        let p = Prepared::new(p, w, h)?;
        let pl = Placement {
            k: 1.0,
            x: p.ox,
            y: p.oy,
            score: 1.0,
        };
        paste(&mut out, &p, &pl, (w, h), (ww, wh));
    }
    Some(out)
}

/// Size of an image of native size `w` x `h` in the reference space (long side 1536).
fn ref_dims(w: u32, h: u32) -> (usize, usize) {
    let s = REF_LONG / w.max(h) as f32;
    (
        ((w as f32 * s).round() as usize).max(1),
        ((h as f32 * s).round() as usize).max(1),
    )
}

/// Rect (x, y, w, h) of a piece's edge crop in the reference space of a `ref_w` x `ref_h`
/// reference, clamped to it.
fn edge_rect(rel: [f32; 4], ref_w: u32, ref_h: u32) -> (usize, usize, usize, usize) {
    let (ww, wh) = ref_dims(ref_w, ref_h);
    let x = ((rel[0] * ww as f32).round().max(0.0) as usize).min(ww - 1);
    let y = ((rel[1] * wh as f32).round().max(0.0) as usize).min(wh - 1);
    let w = ((rel[2] * ww as f32).round() as usize).clamp(1, ww - x);
    let h = ((rel[3] * wh as f32).round() as usize).clamp(1, wh - y);
    (x, y, w, h)
}

/// ITU-R 601 luma of `img` resized (antialiased) to `ww` x `wh`.
fn working_luma(img: &RgbImage, ww: usize, wh: usize) -> Vec<f32> {
    let resized;
    let src = if img.dimensions() == (ww as u32, wh as u32) {
        img
    } else {
        resized = imageops::resize(img, ww as u32, wh as u32, FilterType::Triangle);
        &resized
    };
    let mut luma = vec![0f32; ww * wh];
    for (dst, row) in luma
        .chunks_exact_mut(ww)
        .zip(src.as_raw().chunks_exact(ww * 3))
    {
        luma_row(row, dst);
    }
    luma
}

fn encode_png(m: &Mask) -> Vec<u8> {
    let mut out = Vec::new();
    PngEncoder::new(&mut out)
        .write_image(m.as_raw(), m.width(), m.height(), ExtendedColorType::L8)
        .expect("PNG encoding into memory cannot fail");
    out
}

/// Antialiased (triangle filter) resize of a row-major image of non-negative floats.
fn resize_f(data: &[f32], w: usize, h: usize, nw: usize, nh: usize) -> Vec<f32> {
    if (w, h) == (nw, nh) {
        return data.to_vec();
    }
    // `imageops::resize` clamps float pixels to 0..=1; the triangle filter has no overshoot,
    // so scaling into that range and back is exact up to rounding.
    let max = data.iter().copied().fold(0f32, f32::max);
    let scale = if max > 1.0 { max } else { 1.0 };
    let img = FImage::from_raw(w as u32, h as u32, data.iter().map(|v| v / scale).collect())
        .expect("sized buffer");
    let mut out = imageops::resize(&img, nw as u32, nh as u32, FilterType::Triangle).into_raw();
    if scale != 1.0 {
        out.iter_mut().for_each(|v| *v *= scale);
    }
    out
}

/// Root-mean-square pooling of a row-major image over each output pixel's source area. Used
/// for the coarse edge maps: plain averaging washes the thin edges of a mark out.
fn pool_rms(data: &[f32], w: usize, h: usize, nw: usize, nh: usize) -> Vec<f32> {
    let mut out = vec![0f32; nw * nh];
    for j in 0..nh {
        let y0 = j * h / nh;
        let y1 = ((j + 1) * h).div_ceil(nh).clamp(y0 + 1, h);
        for i in 0..nw {
            let x0 = i * w / nw;
            let x1 = ((i + 1) * w).div_ceil(nw).clamp(x0 + 1, w);
            let mut acc = 0f32;
            for y in y0..y1 {
                acc += data[y * w + x0..y * w + x1]
                    .iter()
                    .map(|v| v * v)
                    .sum::<f32>();
            }
            out[j * nw + i] = (acc / ((y1 - y0) * (x1 - x0)) as f32).sqrt();
        }
    }
    out
}

/// A piece decoded and checked for matching.
struct Prepared {
    shape: Mask,
    edges: Vec<f32>,
    /// Shape resized to the edge crop, as 0..1 weights.
    weights: Vec<f32>,
    w: usize,
    h: usize,
    /// Edge crop origin in reference space, and the exact (unrounded) bbox origin.
    ox: i32,
    oy: i32,
    ox_f: f32,
    oy_f: f32,
    /// Reference-space pixels per native reference pixel.
    s_ref: f32,
    /// Set pixels of `shape`.
    pixels: u32,
}

impl Prepared {
    fn new(p: &Piece, ref_w: u32, ref_h: u32) -> Option<Self> {
        let (w, h) = (p.edges_w as usize, p.edges_h as usize);
        if w == 0 || h == 0 || p.edges.len() != w * h || p.edges.iter().any(|v| !v.is_finite()) {
            return None;
        }
        if !p.rel_bbox.iter().all(|v| v.is_finite()) {
            return None;
        }
        let shape = image::load_from_memory_with_format(&p.shape, ImageFormat::Png)
            .ok()?
            .to_luma8();
        if shape.width() == 0 || shape.height() == 0 {
            return None;
        }
        let unit: Vec<f32> = shape
            .as_raw()
            .iter()
            .map(|&v| (v != 0) as u8 as f32)
            .collect();
        let pixels = unit.iter().filter(|&&v| v != 0.0).count() as u32;
        let weights = resize_f(&unit, shape.width() as usize, shape.height() as usize, w, h);
        let (ww, wh) = ref_dims(ref_w, ref_h);
        let (ox, oy, _, _) = edge_rect(p.rel_bbox, ref_w, ref_h);
        Some(Self {
            shape,
            edges: p.edges.clone(),
            weights,
            w,
            h,
            ox: ox as i32,
            oy: oy as i32,
            ox_f: p.rel_bbox[0] * ww as f32,
            oy_f: p.rel_bbox[1] * wh as f32,
            s_ref: REF_LONG / ref_w.max(ref_h) as f32,
            pixels,
        })
    }
}

/// One or several pieces composed into a single matching template (reference space), weighted
/// by the piece shapes, and a coarse copy (RMS-pooled edges, weighted by the piece bounding
/// boxes: at 1/4 scale the mark's own edges are nearly uniform inside its shape, so it is the
/// contrast between the mark and its surroundings that locates it).
struct Tmpl {
    members: Vec<usize>,
    /// Origin of the template in the reference photo's reference space.
    x0: i32,
    y0: i32,
    w: usize,
    h: usize,
    edges: Vec<f32>,
    weights: Vec<f32>,
    cw: usize,
    ch: usize,
    c_edges: Vec<f32>,
    c_weights: Vec<f32>,
}

impl Tmpl {
    fn new(pieces: &[Prepared], members: &[usize]) -> Self {
        let span = |f: &dyn Fn(&Prepared) -> (i32, i32)| {
            let lo = members.iter().map(|&i| f(&pieces[i]).0).min().unwrap_or(0);
            let hi = members.iter().map(|&i| f(&pieces[i]).1).max().unwrap_or(1);
            (lo, hi)
        };
        let (x0, x1) = span(&|p| (p.ox, p.ox + p.w as i32));
        let (y0, y1) = span(&|p| (p.oy, p.oy + p.h as i32));
        let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
        let mut edges = vec![0f32; w * h];
        let mut weights = vec![0f32; w * h];
        let mut boxes = vec![0f32; w * h];
        for &i in members {
            let p = &pieces[i];
            let (dx, dy) = ((p.ox - x0) as usize, (p.oy - y0) as usize);
            for y in 0..p.h {
                boxes[(dy + y) * w + dx..(dy + y) * w + dx + p.w].fill(1.0);
                for x in 0..p.w {
                    let (s, d) = (y * p.w + x, (dy + y) * w + dx + x);
                    if p.weights[s] >= weights[d] {
                        weights[d] = p.weights[s];
                        edges[d] = p.edges[s];
                    }
                }
            }
        }
        let cw = ((w as f32 / COARSE_DOWN).round() as usize).max(1);
        let ch = ((h as f32 / COARSE_DOWN).round() as usize).max(1);
        Self {
            members: members.to_vec(),
            x0,
            y0,
            w,
            h,
            c_edges: pool_rms(&edges, w, h, cw, ch),
            c_weights: resize_f(&boxes, w, h, cw, ch),
            edges,
            weights,
            cw,
            ch,
        }
    }

    /// Placement of each member piece's edge crop for a placement of the whole template.
    fn place_pieces(&self, pieces: &[Prepared], p: &Placement) -> Vec<(usize, Placement)> {
        self.members
            .iter()
            .map(|&i| {
                let q = Placement {
                    x: p.x + pieces[i].ox - self.x0,
                    y: p.y + pieces[i].oy - self.y0,
                    ..*p
                };
                (i, q)
            })
            .collect()
    }
}

/// Template top-left `(x, y)` in the target's k-space (its reference space resampled by `1/k`).
#[derive(Debug, Clone, Copy)]
struct Placement {
    k: f32,
    x: i32,
    y: i32,
    score: f32,
}

/// Placements of pieces (by index) and the overall score: the minimum over the pieces.
struct Outcome {
    score: f32,
    places: Vec<(usize, Placement)>,
    /// The pieces' geometry is inconsistent: the result may be `Unsure` but never `Found`.
    unsure_at_most: bool,
}

impl Outcome {
    fn from_places(places: Vec<(usize, Placement)>) -> Self {
        let score = places
            .iter()
            .map(|(_, p)| p.score)
            .fold(f32::INFINITY, f32::min);
        Self {
            score,
            places,
            unsure_at_most: false,
        }
    }
}

/// Bounding box `[x, y, w, h]` of `tm` placed at `p`, in the target's reference space.
fn placed_rect(tm: &Tmpl, p: &Placement) -> [f32; 4] {
    let k = p.k;
    [
        p.x as f32 * k,
        p.y as f32 * k,
        tm.w as f32 * k,
        tm.h as f32 * k,
    ]
}

/// Whether two `[x, y, w, h]` rects share any area.
fn overlaps(a: &[f32; 4], b: &[f32; 4]) -> bool {
    a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
}

/// The photo being searched, in reference space.
struct Target {
    nw: u32,
    nh: u32,
    w: usize,
    h: usize,
    luma: Vec<f32>,
    edges: Vec<f32>,
}

impl Target {
    fn new(img: &RgbImage) -> Self {
        let (nw, nh) = img.dimensions();
        let (w, h) = ref_dims(nw, nh);
        let luma = working_luma(img, w, h);
        let edges = sobel_magnitude(&luma, w, h);
        Self {
            nw,
            nh,
            w,
            h,
            luma,
            edges,
        }
    }

    /// Size of k-space.
    fn k_dims(&self, k: f32) -> (i32, i32) {
        (
            (self.w as f32 / k).floor() as i32,
            (self.h as f32 / k).floor() as i32,
        )
    }

    /// Edge magnitude of the k-space pixels `x0..x0+rw` x `y0..y0+rh`: bilinear resampling of
    /// the luma, then Sobel with a 1 px margin so the region's border sees its real neighbours.
    fn region_edges(&self, k: f32, x0: i32, y0: i32, rw: usize, rh: usize) -> Vec<f32> {
        let m = 1; // Sobel needs one neighbour on each side
        let (pw, ph) = (rw + 2 * m, rh + 2 * m);
        let (mw, mh) = ((self.w - 1) as f32, (self.h - 1) as f32);
        let l = |x: usize, y: usize| self.luma[y * self.w + x];
        let mut luma = vec![0f32; pw * ph];
        for j in 0..ph {
            let sy = (((y0 - m as i32 + j as i32) as f32 + 0.5) * k - 0.5).clamp(0.0, mh);
            let (yi, fy) = (sy.floor() as usize, sy.fract());
            let yj = (yi + 1).min(self.h - 1);
            for i in 0..pw {
                let sx = (((x0 - m as i32 + i as i32) as f32 + 0.5) * k - 0.5).clamp(0.0, mw);
                let (xi, fx) = (sx.floor() as usize, sx.fract());
                let xj = (xi + 1).min(self.w - 1);
                let top = l(xi, yi) + (l(xj, yi) - l(xi, yi)) * fx;
                let bot = l(xi, yj) + (l(xj, yj) - l(xi, yj)) * fx;
                luma[j * pw + i] = top + (bot - top) * fy;
            }
        }
        let mag = sobel_magnitude(&luma, pw, ph);
        let mut out = Vec::with_capacity(rw * rh);
        for j in m..m + rh {
            out.extend_from_slice(&mag[j * pw + m..j * pw + m + rw]);
        }
        out
    }

    /// Best placement of `tm` at scale `k` with its top-left within `r` px of `(x, y)` whose
    /// bounding box does not overlap any `claimed` rect (reference space).
    fn score_window(
        &self,
        tm: &Tmpl,
        k: f32,
        (x, y): (i32, i32),
        r: i32,
        claimed: &[[f32; 4]],
    ) -> Option<Placement> {
        let (kw, kh) = self.k_dims(k);
        let (x0, y0) = ((x - r).max(0), (y - r).max(0));
        let x1 = (x + tm.w as i32 + r).min(kw);
        let y1 = (y + tm.h as i32 + r).min(kh);
        if x1 - x0 < tm.w as i32 || y1 - y0 < tm.h as i32 {
            return None;
        }
        let (rw, rh) = ((x1 - x0) as usize, (y1 - y0) as usize);
        let reg = self.region_edges(k, x0, y0, rw, rh);
        let map = masked_ncc(&reg, rw, rh, &tm.edges, &tm.weights, tm.w, tm.h);
        let ow = rw - tm.w + 1;
        map.iter()
            .enumerate()
            .map(|(i, &score)| Placement {
                k,
                x: x0 + (i % ow) as i32,
                y: y0 + (i / ow) as i32,
                score,
            })
            .filter(|p| {
                let r = placed_rect(tm, p);
                !claimed.iter().any(|c| overlaps(&r, c))
            })
            .max_by(|a, b| a.score.total_cmp(&b.score))
    }
}

/// Step 1: each piece near its own relative position (the group anchored at the same relative
/// position as in the reference), k = 1.
fn fast_path(
    t: &WatermarkTemplate,
    pieces: &[Prepared],
    singles: &[Tmpl],
    target: &Target,
) -> Option<Outcome> {
    let (rw, rh) = ref_dims(t.ref_w, t.ref_h);
    let gx0 = pieces.iter().map(|p| p.ox).min()?;
    let gy0 = pieces.iter().map(|p| p.oy).min()?;
    let tx = (gx0 as f32 / rw as f32 * target.w as f32).round() as i32;
    let ty = (gy0 as f32 / rh as f32 * target.h as f32).round() as i32;
    let places = pieces
        .iter()
        .zip(singles)
        .enumerate()
        .map(|(i, (p, tm))| {
            let (x, y) = (tx + p.ox - gx0, ty + p.oy - gy0);
            target
                .score_window(tm, 1.0, (x, y), FAST_PX, &[])
                .map(|pl| (i, pl))
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Outcome::from_places(places))
}

/// Re-score each piece on its own (+-2 px) at a group placement, so that a group match carried
/// by one piece does not hide another piece that is somewhere else.
fn check_pieces(target: &Target, singles: &[Tmpl], places: Vec<(usize, Placement)>) -> Outcome {
    let places = places
        .into_iter()
        .map(|(i, p)| {
            let q = target
                .score_window(&singles[i], p.k, (p.x, p.y), FINE_PX, &[])
                .unwrap_or(Placement {
                    score: f32::NEG_INFINITY,
                    ..p
                });
            (i, q)
        })
        .collect();
    Outcome::from_places(places)
}

fn keep_better(best: &mut Option<Outcome>, o: Option<Outcome>) {
    if let Some(o) = o
        && best.as_ref().is_none_or(|b| o.score > b.score)
    {
        *best = Some(o);
    }
}

/// Step 3: each piece on its own, largest first. A piece may not overlap the placement of an
/// earlier one, so identical or similar pieces cannot all land on one instance of the mark.
/// Pieces whose scales differ by more than 10 % from their median leave the result `Unsure`
/// at most.
fn search_each(target: &Target, singles: &[Tmpl]) -> Outcome {
    let mut order: Vec<usize> = (0..singles.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(singles[i].w * singles[i].h));
    let mut claimed: Vec<[f32; 4]> = Vec::new();
    let mut places = Vec::new();
    for i in order {
        match search(target, &singles[i], &claimed) {
            Some(p) => {
                claimed.push(placed_rect(&singles[i], &p));
                places.push((i, p));
            }
            None => {
                return Outcome {
                    score: f32::NEG_INFINITY,
                    places,
                    unsure_at_most: true,
                };
            }
        }
    }
    let mut ks: Vec<f32> = places.iter().map(|(_, p)| p.k).collect();
    ks.sort_by(f32::total_cmp);
    let median = ks[ks.len() / 2];
    let mut o = Outcome::from_places(places);
    o.unsure_at_most = ks
        .iter()
        .any(|k| (k / median - 1.0).abs() > MAX_SCALE_SPREAD);
    o
}

/// Steps 2/3: coarse search over scales, then refinement of the best few peaks. Placements
/// overlapping a `claimed` rect (reference space) are skipped.
fn search(target: &Target, tm: &Tmpl, claimed: &[[f32; 4]]) -> Option<Placement> {
    // (score, k, template centre x, y in k-space)
    let mut cands: Vec<(f32, f32, f32, f32)> = Vec::new();
    for k in SCALES {
        let cw = ((target.w as f32 / (COARSE_DOWN * k)).round() as usize).max(1);
        let ch = ((target.h as f32 / (COARSE_DOWN * k)).round() as usize).max(1);
        if cw < tm.cw || ch < tm.ch {
            continue;
        }
        let coarse = pool_rms(&target.edges, target.w, target.h, cw, ch);
        let mut map = masked_ncc(&coarse, cw, ch, &tm.c_edges, &tm.c_weights, tm.cw, tm.ch);
        let (ow, oh) = (cw - tm.cw + 1, ch - tm.ch + 1);
        let (kw, kh) = target.k_dims(k);
        let (sx, sy) = (kw as f32 / cw as f32, kh as f32 / ch as f32);
        let (rx, ry) = ((tm.cw / 2) as i32, (tm.ch / 2) as i32);
        if !claimed.is_empty() {
            for (i, v) in map.iter_mut().enumerate() {
                let p = Placement {
                    k,
                    x: ((i % ow) as f32 * sx).round() as i32,
                    y: ((i / ow) as f32 * sy).round() as i32,
                    score: 0.0,
                };
                let r = placed_rect(tm, &p);
                if claimed.iter().any(|c| overlaps(&r, c)) {
                    *v = f32::NEG_INFINITY;
                }
            }
        }
        for _ in 0..PEAKS_PER_SCALE {
            let Some((i, &score)) = map.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)) else {
                break;
            };
            if score <= 0.0 {
                break;
            }
            let (px, py) = ((i % ow) as i32, (i / ow) as i32);
            cands.push((
                score,
                k,
                (px as f32 + tm.cw as f32 / 2.0) * sx,
                (py as f32 + tm.ch as f32 / 2.0) * sy,
            ));
            // Suppress this peak's neighbourhood before looking for the next one.
            for y in (py - ry).max(0)..=(py + ry).min(oh as i32 - 1) {
                for x in (px - rx).max(0)..=(px + rx).min(ow as i32 - 1) {
                    map[y as usize * ow + x as usize] = f32::NEG_INFINITY;
                }
            }
        }
    }
    // Coarse scores are not comparable across scales (more placements at small k, different
    // resampling), so every coarse peak gets one fine look at its own scale and the best few
    // by fine score are refined.
    let mut checked: Vec<Placement> = cands
        .into_iter()
        .filter_map(|(_, k, cx, cy)| {
            let x = (cx - tm.w as f32 / 2.0).round() as i32;
            let y = (cy - tm.h as f32 / 2.0).round() as i32;
            target.score_window(tm, k, (x, y), REFINE_PX, claimed)
        })
        .collect();
    checked.sort_by(|a, b| b.score.total_cmp(&a.score));
    checked.truncate(CANDIDATES);
    checked
        .into_iter()
        .map(|p| refine(target, tm, p, claimed))
        .max_by(|a, b| a.score.total_cmp(&b.score))
}

/// Refine a placement over position and scale: +-8 px over +-5 % in 1 % steps, then +-2 px over
/// +-1 % in 0.25 % steps. Scale steps keep the template centre fixed.
fn refine(target: &Target, tm: &Tmpl, start: Placement, claimed: &[[f32; 4]]) -> Placement {
    let (half_w, half_h) = (tm.w as f32 / 2.0, tm.h as f32 / 2.0);
    let mut best = start;
    let mut r = REFINE_PX;
    for (step, span) in REFINE_STEPS.into_iter().zip(REFINE_SPAN) {
        let (k0, cx0, cy0) = (best.k, best.x as f32 + half_w, best.y as f32 + half_h);
        for j in -span..=span {
            let k2 = k0 * (1.0 + step * j as f32);
            let x = (cx0 * k0 / k2 - half_w).round() as i32;
            let y = (cy0 * k0 / k2 - half_h).round() as i32;
            if let Some(p) = target.score_window(tm, k2, (x, y), r, claimed)
                && p.score > best.score
            {
                best = p;
            }
        }
        r = FINE_PX;
    }
    best
}

/// Paste piece `p`'s shape for an edge-crop placement `pl` into the image-size mask `out`.
fn paste(
    out: &mut Mask,
    p: &Prepared,
    pl: &Placement,
    (nw, nh): (u32, u32),
    (tw, th): (usize, usize),
) {
    // k-space -> target reference space (x k) -> native (x native / reference size).
    let (fx, fy) = (pl.k * nw as f32 / tw as f32, pl.k * nh as f32 / th as f32);
    let x = ((pl.x as f32 + p.ox_f - p.ox as f32) * fx).round() as i64;
    let y = ((pl.y as f32 + p.oy_f - p.oy as f32) * fy).round() as i64;
    let w = (p.shape.width() as f32 * p.s_ref * fx).round().max(1.0) as u32;
    let h = (p.shape.height() as f32 * p.s_ref * fy).round().max(1.0) as u32;
    let shape = mask::resize(&p.shape, w, h);
    let (ow, oh) = (out.width() as i64, out.height() as i64);
    for (sx, sy, v) in shape.enumerate_pixels() {
        let (tx, ty) = (x + sx as i64, y + sy as i64);
        if v.0[0] != 0 && (0..ow).contains(&tx) && (0..oh).contains(&ty) {
            out.put_pixel(tx as u32, ty as u32, Luma([255]));
        }
    }
}

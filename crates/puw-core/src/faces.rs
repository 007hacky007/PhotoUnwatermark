//! Face guard: keeps the automatic watermark guess away from eyes.
//!
//! In a fully automatic run the guess sometimes marks an eye, and inpainting then replaces it
//! with a dark blob. [`detect_faces`] finds faces with YuNet. [`drop_eye_components`] removes
//! guessed components that reach an eye, and [`guard`] clears what is left in the eye circles
//! (after growth), each unless a more reliable layer (`keep`) supports them.

use image::imageops::{self, FilterType};
use image::{GrayImage, Luma, RgbImage};
use imageproc::rect::Rect;

use crate::engine::Engine;
use crate::mask::Mask;
use crate::{Error, Result};

/// Side of the YuNet input square.
const SIDE: u32 = 640;
const STRIDES: [u32; 3] = [8, 16, 32];
/// Minimum detection score (sqrt of class and objectness).
const MIN_SCORE: f32 = 0.7;
const NMS_IOU: f32 = 0.3;
/// Cleared radius around an eye, as a fraction of the face box width.
const EYE_RADIUS: f32 = 0.12;

/// A detected face in photo pixel coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct Face {
    /// Face box, clamped to the photo.
    pub bbox: Rect,
    /// Width of the face as detected, before clamping: a face cut by the photo's edge keeps
    /// its full width here. [`guard`] scales its radius with this.
    pub width: f32,
    /// Right eye, then left eye, in YuNet's naming: the subject's right eye normally appears on
    /// the image's left, so the first point usually has the smaller x.
    pub eyes: [(f32, f32); 2],
    pub score: f32,
}

/// A detection in the coordinates of the model input (or, after [`Candidate::to_photo`], of
/// the photo); the box is unclamped.
#[derive(Debug, Clone, PartialEq)]
struct Candidate {
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    eyes: [(f32, f32); 2],
    score: f32,
}

impl Candidate {
    /// From model-input coordinates of a view that was scaled by `scale` and starts at
    /// (`x0`, `y0`) in the photo.
    fn to_photo(&self, scale: f32, x0: f32, y0: f32) -> Candidate {
        let inv = 1.0 / scale;
        Candidate {
            x1: self.x1 * inv + x0,
            y1: self.y1 * inv + y0,
            x2: self.x2 * inv + x0,
            y2: self.y2 * inv + y0,
            eyes: self.eyes.map(|(x, y)| (x * inv + x0, y * inv + y0)),
            score: self.score,
        }
    }
}

fn iou(a: &Candidate, b: &Candidate) -> f32 {
    let iw = (a.x2.min(b.x2) - a.x1.max(b.x1)).max(0.0);
    let ih = (a.y2.min(b.y2) - a.y1.max(b.y1)).max(0.0);
    let inter = iw * ih;
    let union = (a.x2 - a.x1) * (a.y2 - a.y1) + (b.x2 - b.x1) * (b.y2 - b.y1) - inter;
    if union <= 0.0 { 0.0 } else { inter / union }
}

/// Finds faces with YuNet (score >= 0.7, NMS at IoU 0.3).
///
/// The whole photo is letterboxed (aspect kept, padded) into the model's fixed 640x640 input,
/// so a face smaller than about 10 px at that scale is missed.
pub fn detect_faces(engine: &Engine, img: &RgbImage) -> Result<Vec<Face>> {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Ok(Vec::new());
    }
    let cands = full_frame(engine, img)?;
    Ok(to_faces(nms(cands), w, h))
}

/// Candidates of one pass over the whole photo, letterboxed into the model input.
fn full_frame(engine: &Engine, img: &RgbImage) -> Result<Vec<Candidate>> {
    let (w, h) = img.dimensions();
    let scale = SIDE as f32 / w.max(h) as f32;
    let nw = ((w as f32 * scale).round() as u32).clamp(1, SIDE);
    let nh = ((h as f32 * scale).round() as u32).clamp(1, SIDE);
    let small = imageops::resize(img, nw, nh, FilterType::Triangle);
    let cands = run_square(engine, &small)?;
    Ok(cands.iter().map(|c| c.to_photo(scale, 0.0, 0.0)).collect())
}

/// YuNet on `small` (at most 640 on each side) placed in the top-left corner of the input
/// square; candidates in `small`'s coordinates.
fn run_square(engine: &Engine, small: &RgbImage) -> Result<Vec<Candidate>> {
    // Planar BGR, 0..255, padding black.
    let plane = (SIDE * SIDE) as usize;
    let mut nchw = vec![0f32; 3 * plane];
    for (x, y, p) in small.enumerate_pixels() {
        if x >= SIDE || y >= SIDE {
            continue;
        }
        let i = (y * SIDE + x) as usize;
        nchw[i] = f32::from(p.0[2]);
        nchw[plane + i] = f32::from(p.0[1]);
        nchw[2 * plane + i] = f32::from(p.0[0]);
    }
    let out = engine.run_face(&nchw, SIDE, SIDE)?;
    decode(&out)
}

/// Candidates from the raw model outputs, in model-input coordinates. Detections scoring below
/// [`MIN_SCORE`] (or NaN) and detections with a non-finite or empty box or a non-finite
/// keypoint are dropped.
fn decode(out: &[Vec<f32>]) -> Result<Vec<Candidate>> {
    if out.len() != 12 {
        return Err(Error::Model(format!(
            "face model returned {} outputs, expected 12",
            out.len()
        )));
    }
    // Decoding follows OpenCV's FaceDetectorYN::postProcess.
    let mut cands = Vec::new();
    for (k, &stride) in STRIDES.iter().enumerate() {
        let (cls, obj, bbox, kps) = (&out[k], &out[3 + k], &out[6 + k], &out[9 + k]);
        let cols = SIDE / stride;
        let n = (cols * cols) as usize;
        if cls.len() < n || obj.len() < n || bbox.len() < 4 * n || kps.len() < 10 * n {
            return Err(Error::Model(format!(
                "face output for stride {stride} too short"
            )));
        }
        let s = stride as f32;
        for i in 0..n {
            let score = (cls[i].clamp(0.0, 1.0) * obj[i].clamp(0.0, 1.0)).sqrt();
            if score.is_nan() || score < MIN_SCORE {
                continue;
            }
            let (col, row) = ((i as u32 % cols) as f32, (i as u32 / cols) as f32);
            let b = &bbox[4 * i..4 * i + 4];
            let cx = (col + b[0]) * s;
            let cy = (row + b[1]) * s;
            let bw = b[2].exp() * s;
            let bh = b[3].exp() * s;
            let k = &kps[10 * i..10 * i + 10];
            let pt = |n: usize| ((col + k[2 * n]) * s, (row + k[2 * n + 1]) * s);
            let c = Candidate {
                x1: cx - bw / 2.0,
                y1: cy - bh / 2.0,
                x2: cx + bw / 2.0,
                y2: cy + bh / 2.0,
                eyes: [pt(0), pt(1)],
                score,
            };
            let finite = [c.x1, c.y1, c.x2, c.y2]
                .into_iter()
                .chain(c.eyes.iter().flat_map(|&(x, y)| [x, y]))
                .all(f32::is_finite);
            if finite && c.x2 > c.x1 && c.y2 > c.y1 {
                cands.push(c);
            }
        }
    }
    Ok(cands)
}

/// Non-maximum suppression: the highest scores first, dropping any candidate overlapping a
/// kept one by more than [`NMS_IOU`].
fn nms(mut cands: Vec<Candidate>) -> Vec<Candidate> {
    cands.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut kept: Vec<Candidate> = Vec::new();
    for c in cands {
        if kept.iter().all(|k| iou(k, &c) <= NMS_IOU) {
            kept.push(c);
        }
    }
    kept
}

/// Faces from photo-coordinate candidates: the box is clamped to the `w` x `h` photo, the
/// width is kept as detected.
fn to_faces(cands: Vec<Candidate>, w: u32, h: u32) -> Vec<Face> {
    cands
        .into_iter()
        .filter_map(|c| {
            let x1 = c.x1.clamp(0.0, w as f32);
            let y1 = c.y1.clamp(0.0, h as f32);
            let x2 = c.x2.clamp(0.0, w as f32);
            let y2 = c.y2.clamp(0.0, h as f32);
            let (bw, bh) = ((x2 - x1).round() as u32, (y2 - y1).round() as u32);
            if bw == 0 || bh == 0 {
                return None;
            }
            Some(Face {
                bbox: Rect::at(x1.round() as i32, y1.round() as i32).of_size(bw, bh),
                width: c.x2 - c.x1,
                eyes: c.eyes,
                score: c.score,
            })
        })
        .collect()
}

/// Eye circles of `faces` as (x, y, radius), skipping any with a non-finite or non-positive
/// value.
fn eye_circles(faces: &[Face]) -> Vec<(f32, f32, f32)> {
    let mut v = Vec::new();
    for face in faces {
        let r = EYE_RADIUS * face.width;
        if !(r.is_finite() && r > 0.0) {
            continue;
        }
        for &(ex, ey) in &face.eyes {
            if ex.is_finite() && ey.is_finite() {
                v.push((ex, ey, r));
            }
        }
    }
    v
}

/// Removes every 8-connected component of `mask` that reaches into an eye circle (radius
/// 0.12 x [`Face::width`]), unless `keep` supports it: `keep` is set (non-zero) on one of the
/// component's pixels inside an eye circle. The whole component goes, not only its pixels in
/// the circle, so later growth cannot bring it back into the eye. `keep` is ignored where it
/// does not cover; faces with a non-finite eye or width are skipped.
pub fn drop_eye_components(mask: &Mask, faces: &[Face], keep: Option<&Mask>) -> Mask {
    let circles = eye_circles(faces);
    let mut out: GrayImage = mask.clone();
    let (w, h) = mask.dimensions();
    if circles.is_empty() || w == 0 || h == 0 {
        return out;
    }
    let in_circle = |x: u32, y: u32| {
        circles.iter().any(|&(ex, ey, r)| {
            let (dx, dy) = (x as f32 - ex, y as f32 - ey);
            dx * dx + dy * dy <= r * r
        })
    };
    let kept = |x: u32, y: u32| {
        keep.is_some_and(|k| x < k.width() && y < k.height() && k.get_pixel(x, y).0[0] != 0)
    };
    let set = |x: u32, y: u32| mask.get_pixel(x, y).0[0] != 0;
    let mut seen = vec![false; w as usize * h as usize];
    let idx = |x: u32, y: u32| y as usize * w as usize + x as usize;
    for &(ex, ey, r) in &circles {
        let x0 = ((ex - r).floor().max(0.0)) as u32;
        let y0 = ((ey - r).floor().max(0.0)) as u32;
        let x1 = (((ex + r).ceil() + 1.0).min(w as f32)).max(0.0) as u32;
        let y1 = (((ey + r).ceil() + 1.0).min(h as f32)).max(0.0) as u32;
        for y in y0..y1 {
            for x in x0..x1 {
                if !set(x, y) || seen[idx(x, y)] || !in_circle(x, y) {
                    continue;
                }
                // Flood-fill the component that reaches the eye.
                seen[idx(x, y)] = true;
                let mut stack = vec![(x, y)];
                let mut pixels = Vec::new();
                let mut supported = false;
                while let Some((cx, cy)) = stack.pop() {
                    pixels.push((cx, cy));
                    supported |= !supported && in_circle(cx, cy) && kept(cx, cy);
                    for (dx, dy) in [
                        (-1, -1),
                        (0, -1),
                        (1, -1),
                        (-1, 0),
                        (1, 0),
                        (-1, 1),
                        (0, 1),
                        (1, 1),
                    ] {
                        let (nx, ny) = (cx as i64 + dx, cy as i64 + dy);
                        if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                            continue;
                        }
                        let (nx, ny) = (nx as u32, ny as u32);
                        if set(nx, ny) && !seen[idx(nx, ny)] {
                            seen[idx(nx, ny)] = true;
                            stack.push((nx, ny));
                        }
                    }
                }
                if !supported {
                    for (px, py) in pixels {
                        out.put_pixel(px, py, Luma([0]));
                    }
                }
            }
        }
    }
    out
}

/// Clears mask pixels in a circle of radius 0.12 x [`Face::width`] around each eye, except
/// where `keep` is set (non-zero). `keep = None` clears everything in the circles. Returns a
/// mask of the input's size; `keep` is ignored where it does not cover (a smaller `keep` counts
/// as unset there). Faces with a non-finite eye or width are skipped.
pub fn guard(mask: &Mask, faces: &[Face], keep: Option<&Mask>) -> Mask {
    let (w, h) = mask.dimensions();
    let mut out: GrayImage = mask.clone();
    for (ex, ey, r) in eye_circles(faces) {
        let x0 = ((ex - r).floor().max(0.0)) as u32;
        let y0 = ((ey - r).floor().max(0.0)) as u32;
        let x1 = (((ex + r).ceil() + 1.0).min(w as f32)).max(0.0) as u32;
        let y1 = (((ey + r).ceil() + 1.0).min(h as f32)).max(0.0) as u32;
        for y in y0..y1 {
            for x in x0..x1 {
                let (dx, dy) = (x as f32 - ex, y as f32 - ey);
                if dx * dx + dy * dy > r * r {
                    continue;
                }
                let kept = keep.is_some_and(|k| {
                    x < k.width() && y < k.height() && k.get_pixel(x, y).0[0] != 0
                });
                if !kept {
                    out.put_pixel(x, y, Luma([0]));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Raw outputs with no detection: every score 0.
    fn empty_outputs() -> Vec<Vec<f32>> {
        let ns: Vec<usize> = STRIDES
            .iter()
            .map(|s| ((SIDE / s) * (SIDE / s)) as usize)
            .collect();
        let mut out = Vec::new();
        for per in [1, 1, 4, 10] {
            for n in &ns {
                out.push(vec![0.0; per * n]);
            }
        }
        out
    }

    /// Puts a detection at stride-8 anchor `i`: a 40x48 box centred on the anchor's cell,
    /// eyes 10 px either side of the centre.
    fn put(out: &mut [Vec<f32>], i: usize, cls: f32, eye_x: f32) {
        out[0][i] = cls;
        out[3][i] = 1.0;
        out[6][4 * i..4 * i + 4].copy_from_slice(&[0.5, 0.5, 5f32.ln(), 6f32.ln()]);
        let k = &mut out[9][10 * i..10 * i + 10];
        k[0] = 0.5 - 10.0 / 8.0 + eye_x;
        k[1] = 0.0;
        k[2] = 0.5 + 10.0 / 8.0;
        k[3] = 0.0;
    }

    #[test]
    fn decode_drops_nan_scores_boxes_and_keypoints() {
        let mut out = empty_outputs();
        put(&mut out, 100, 1.0, 0.0); // valid
        put(&mut out, 200, f32::NAN, 0.0); // NaN score
        put(&mut out, 300, 1.0, f32::NAN); // NaN eye
        put(&mut out, 400, 1.0, 0.0);
        out[6][4 * 400 + 2] = f32::INFINITY; // infinite box
        put(&mut out, 500, 0.4, 0.0); // score sqrt(0.4) = 0.63, too low
        let c = decode(&out).expect("decode");
        assert_eq!(c.len(), 1, "{c:?}");
        // Anchor 100 of an 80-column grid: column 20, row 1; centre (164, 12) at stride 8.
        let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
        assert!(near(c[0].x1, 144.0) && near(c[0].x2, 184.0), "{c:?}");
        assert!(
            near(c[0].eyes[0].0, 154.0) && near(c[0].eyes[0].1, 8.0),
            "{c:?}"
        );
    }

    #[test]
    fn faces_cut_by_the_edge_keep_their_width() {
        let c = Candidate {
            x1: -30.0,
            y1: 10.0,
            x2: 70.0,
            y2: 130.0,
            eyes: [(5.0, 50.0), (45.0, 50.0)],
            score: 0.9,
        };
        let f = to_faces(vec![c], 200, 100);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].bbox, Rect::at(0, 10).of_size(70, 90));
        assert_eq!(f[0].width, 100.0);
    }
}

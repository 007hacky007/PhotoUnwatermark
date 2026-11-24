//! Face guard: keeps the automatic watermark guess away from eyes.
//!
//! In a fully automatic run the guess sometimes marks an eye, and inpainting then replaces it
//! with a dark blob. [`detect_faces`] finds faces with YuNet, and [`guard`] clears guessed
//! pixels around each eye unless a more reliable layer (`keep`) supports them.

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
    pub bbox: Rect,
    /// Right eye, then left eye, in YuNet's naming: the subject's right eye normally appears on
    /// the image's left, so the first point usually has the smaller x.
    pub eyes: [(f32, f32); 2],
    pub score: f32,
}

struct Candidate {
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    eyes: [(f32, f32); 2],
    score: f32,
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
/// so a face smaller than about 10 px at that scale is missed. That is acceptable for v1: the
/// guard matters for faces large enough to be damaged visibly.
pub fn detect_faces(engine: &Engine, img: &RgbImage) -> Result<Vec<Face>> {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Ok(Vec::new());
    }
    let scale = SIDE as f32 / w.max(h) as f32;
    let nw = ((w as f32 * scale).round() as u32).clamp(1, SIDE);
    let nh = ((h as f32 * scale).round() as u32).clamp(1, SIDE);
    let small = imageops::resize(img, nw, nh, FilterType::Triangle);

    // Planar BGR, 0..255, padding black, photo in the top-left corner.
    let plane = (SIDE * SIDE) as usize;
    let mut nchw = vec![0f32; 3 * plane];
    for (x, y, p) in small.enumerate_pixels() {
        let i = (y * SIDE + x) as usize;
        nchw[i] = f32::from(p.0[2]);
        nchw[plane + i] = f32::from(p.0[1]);
        nchw[2 * plane + i] = f32::from(p.0[0]);
    }
    let out = engine.run_face(&nchw, SIDE, SIDE)?;
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
            if score < MIN_SCORE {
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
            cands.push(Candidate {
                x1: cx - bw / 2.0,
                y1: cy - bh / 2.0,
                x2: cx + bw / 2.0,
                y2: cy + bh / 2.0,
                eyes: [pt(0), pt(1)],
                score,
            });
        }
    }

    cands.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut kept: Vec<Candidate> = Vec::new();
    for c in cands {
        if kept.iter().all(|k| iou(k, &c) <= NMS_IOU) {
            kept.push(c);
        }
    }

    // Back from the letterboxed square to photo coordinates.
    let inv = 1.0 / scale;
    let faces = kept
        .into_iter()
        .filter_map(|c| {
            let x1 = (c.x1 * inv).clamp(0.0, w as f32);
            let y1 = (c.y1 * inv).clamp(0.0, h as f32);
            let x2 = (c.x2 * inv).clamp(0.0, w as f32);
            let y2 = (c.y2 * inv).clamp(0.0, h as f32);
            let (bw, bh) = ((x2 - x1).round() as u32, (y2 - y1).round() as u32);
            if bw == 0 || bh == 0 {
                return None;
            }
            Some(Face {
                bbox: Rect::at(x1.round() as i32, y1.round() as i32).of_size(bw, bh),
                eyes: c.eyes.map(|(x, y)| (x * inv, y * inv)),
                score: c.score,
            })
        })
        .collect();
    Ok(faces)
}

/// Clears mask pixels in a circle of radius 0.12 x face width around each eye, except where
/// `keep` is 255. `keep = None` clears everything in the circles. Returns a mask of the input's
/// size; `keep` is ignored where it does not cover (a smaller `keep` counts as unset there).
pub fn guard(mask: &Mask, faces: &[Face], keep: Option<&Mask>) -> Mask {
    let (w, h) = mask.dimensions();
    let mut out: GrayImage = mask.clone();
    for face in faces {
        let r = EYE_RADIUS * face.bbox.width() as f32;
        for &(ex, ey) in &face.eyes {
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
                        x < k.width() && y < k.height() && k.get_pixel(x, y).0[0] == 255
                    });
                    if !kept {
                        out.put_pixel(x, y, Luma([0]));
                    }
                }
            }
        }
    }
    out
}

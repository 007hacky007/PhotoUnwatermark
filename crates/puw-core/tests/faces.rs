mod common;

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

use image::Luma;
use imageproc::rect::Rect;
use puw_core::engine::{Engine, ModelPaths};
use puw_core::faces::{Face, detect_faces, detect_faces_near, drop_eye_components, guard};
use puw_core::mask::Mask;

fn engine() -> Option<&'static Engine> {
    static ENGINE: OnceLock<Option<Engine>> = OnceLock::new();
    ENGINE
        .get_or_init(|| {
            let Some(dir) = common::models_dir() else {
                eprintln!("skipped: PUW_MODELS_DIR not set");
                return None;
            };
            let paths = ModelPaths::from_dir(&dir).expect("model files present");
            Some(Engine::new(&paths, common::shipping_options()).expect("engine loads"))
        })
        .as_ref()
}

fn face() -> Face {
    Face {
        bbox: Rect::at(40, 40).of_size(100, 120),
        width: 100.0,
        eyes: [(70.0, 80.0), (110.0, 80.0)],
        score: 0.9,
    }
}

fn full(w: u32, h: u32) -> Mask {
    Mask::from_pixel(w, h, Luma([255]))
}

#[test]
fn guard_clears_eye_region() {
    let out = guard(&full(200, 200), &[face()], None);
    assert_eq!(out.dimensions(), (200, 200));
    assert_eq!(out.get_pixel(70, 80).0[0], 0);
    assert_eq!(out.get_pixel(110, 80).0[0], 0);
    // Radius is 0.12 x 100 = 12 px.
    assert_eq!(out.get_pixel(70 + 11, 80).0[0], 0);
    assert_eq!(out.get_pixel(70 + 14, 80).0[0], 255);
    assert_eq!(out.get_pixel(5, 5).0[0], 255);
    assert_eq!(out.get_pixel(90, 150).0[0], 255);
}

#[test]
fn guard_respects_keep() {
    let mut keep = Mask::new(200, 200);
    for y in 70..90 {
        for x in 60..80 {
            keep.put_pixel(x, y, Luma([255]));
        }
    }
    let out = guard(&full(200, 200), &[face()], Some(&keep));
    assert_eq!(
        out.get_pixel(70, 80).0[0],
        255,
        "kept pixel at the right eye"
    );
    assert_eq!(out.get_pixel(110, 80).0[0], 0, "left eye not in keep");
    // Inside the circle but outside keep.
    assert_eq!(out.get_pixel(70, 90 + 1).0[0], 0);
}

#[test]
fn guard_handles_eyes_near_the_border() {
    let f = Face {
        bbox: Rect::at(0, 0).of_size(50, 50),
        width: 50.0,
        eyes: [(-3.0, 2.0), (400.0, 400.0)],
        score: 0.8,
    };
    let out = guard(&full(20, 20), &[f], None);
    assert_eq!(out.get_pixel(0, 2).0[0], 0);
    assert_eq!(out.get_pixel(19, 19).0[0], 255);
}

#[test]
fn guard_radius_uses_full_face_width() {
    // A face cut by the left edge: its box is clamped to 50 px, the face is 100 px wide.
    let f = Face {
        bbox: Rect::at(0, 40).of_size(50, 120),
        width: 100.0,
        eyes: [(10.0, 80.0), (45.0, 80.0)],
        score: 0.9,
    };
    let out = guard(&full(200, 200), &[f], None);
    // Radius 0.12 x 100 = 12 px, not 0.12 x 50 = 6 px.
    assert_eq!(out.get_pixel(45 + 10, 80).0[0], 0);
    assert_eq!(out.get_pixel(45 + 14, 80).0[0], 255);
}

#[test]
fn guard_ignores_non_finite_faces() {
    let bad_eyes = Face {
        eyes: [(f32::NAN, 80.0), (110.0, f32::INFINITY)],
        ..face()
    };
    let bad_width = Face {
        width: f32::NAN,
        ..face()
    };
    let out = guard(&full(200, 200), &[bad_eyes, bad_width], None);
    assert!(out.pixels().all(|p| p.0[0] == 255));
}

#[test]
fn components_reaching_an_eye_are_dropped_whole() {
    let mut m = Mask::new(200, 200);
    // A bar from the right eye (70, 80) to x = 160, and a separate square far away.
    for y in 78..83 {
        for x in 70..160 {
            m.put_pixel(x, y, Luma([255]));
        }
    }
    for y in 170..180 {
        for x in 10..20 {
            m.put_pixel(x, y, Luma([255]));
        }
    }
    let out = drop_eye_components(&m, &[face()], None);
    assert_eq!(
        out.get_pixel(150, 80).0[0],
        0,
        "bar outside the circle kept"
    );
    assert_eq!(
        out.get_pixel(15, 175).0[0],
        255,
        "unrelated component dropped"
    );

    // A keep pixel on the bar inside the circle supports the whole bar.
    let mut keep = Mask::new(200, 200);
    keep.put_pixel(75, 80, Luma([255]));
    let out = drop_eye_components(&m, &[face()], Some(&keep));
    assert_eq!(out, m);
    // Keep pixels outside the circles do not.
    let mut far = Mask::new(200, 200);
    far.put_pixel(150, 80, Luma([255]));
    let out = drop_eye_components(&m, &[face()], Some(&far));
    assert_eq!(out.get_pixel(150, 80).0[0], 0);
}

/// scikit-image's astronaut photo as a PNG in a temp folder.
fn astronaut() -> image::RgbImage {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    let dir = DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("puw-astronaut-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let status = Command::new("uv")
            .args(["run", "--project", "tools", "python", "-c"])
            .arg(
                "import sys; from skimage import data; from PIL import Image; \
                 Image.fromarray(data.astronaut()).save(sys.argv[1])",
            )
            .arg(dir.join("astronaut.png"))
            .current_dir(root)
            .status()
            .expect("run uv");
        assert!(status.success(), "astronaut generation failed");
        dir
    });
    image::open(dir.join("astronaut.png"))
        .expect("open")
        .to_rgb8()
}

#[test]
fn detect_faces_on_astronaut() {
    let Some(engine) = engine() else { return };
    let img = astronaut();
    let faces = detect_faces(engine, &img).expect("detect");
    eprintln!("astronaut faces: {faces:?}");
    assert_eq!(faces.len(), 1, "{faces:?}");
    let f = &faces[0];
    let (l, t) = (f.bbox.left() as f32, f.bbox.top() as f32);
    let (r, b) = (l + f.bbox.width() as f32, t + f.bbox.height() as f32);
    for (x, y) in f.eyes {
        assert!(
            x > l && x < r && y > t && y < b,
            "eye ({x},{y}) outside {:?}",
            f.bbox
        );
    }
    assert!(
        f.eyes[0].0 < f.eyes[1].0,
        "right eye is left of left eye in x"
    );
    assert!(f.score >= 0.7);
}

#[test]
fn small_face_in_large_frame_found_near_the_guess() {
    let Some(engine) = engine() else { return };
    let src = astronaut();
    let big = detect_faces(engine, &src).expect("detect");
    assert_eq!(big.len(), 1);
    // The astronaut shrunk so the face is about 80 px wide, in a 6000x4000 frame.
    let k = 80.0 / big[0].width;
    let side = (src.width() as f32 * k).round() as u32;
    let small = image::imageops::resize(&src, side, side, image::imageops::FilterType::Lanczos3);
    let mut frame = image::RgbImage::from_fn(6000, 4000, |x, y| {
        image::Rgb([
            (90 + x / 100) as u8,
            (100 + y / 80) as u8,
            (110 + (x + y) % 7) as u8,
        ])
    });
    let (ox, oy) = (4000u32, 2500u32);
    image::imageops::replace(&mut frame, &small, ox as i64, oy as i64);
    let fb = &big[0].bbox;
    let face_box = (
        ox as f32 + fb.left() as f32 * k,
        oy as f32 + fb.top() as f32 * k,
        ox as f32 + (fb.left() as f32 + fb.width() as f32) * k,
        oy as f32 + (fb.top() as f32 + fb.height() as f32) * k,
    );
    let inside = |f: &Face| {
        f.eyes
            .iter()
            .all(|&(x, y)| x > face_box.0 && x < face_box.2 && y > face_box.1 && y < face_box.3)
    };

    // One full-frame pass misses it: the face is under 9 px wide at the model's input.
    let full = detect_faces(engine, &frame).expect("detect");
    eprintln!("full frame: {full:?}");
    assert!(!full.iter().any(inside), "full frame already finds it");

    // A guessed component just right of the face.
    let mut near = Mask::new(6000, 4000);
    let gx = face_box.2.round() as u32 + 10;
    let gy = ((face_box.1 + face_box.3) / 2.0).round() as u32;
    for y in gy..gy + 10 {
        for x in gx..gx + 40 {
            near.put_pixel(x, y, Luma([255]));
        }
    }
    let found = detect_faces_near(engine, &frame, &near, 24).expect("detect near");
    eprintln!("near the guess: {found:?}");
    let hits: Vec<&Face> = found.iter().filter(|f| inside(f)).collect();
    assert_eq!(hits.len(), 1, "{found:?}");
    assert!(
        (hits[0].width - 80.0).abs() < 25.0,
        "width {}",
        hits[0].width
    );

    // No guess, no crop passes: the same as the full frame.
    let none = detect_faces_near(engine, &frame, &Mask::new(6000, 4000), 24).expect("detect");
    assert_eq!(none, full);
}

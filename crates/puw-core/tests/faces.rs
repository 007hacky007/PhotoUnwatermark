mod common;

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

use image::Luma;
use imageproc::rect::Rect;
use puw_core::engine::{Engine, ModelPaths};
use puw_core::faces::{Face, detect_faces, guard};
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
            Some(Engine::new(&paths, false, None).expect("engine loads"))
        })
        .as_ref()
}

fn face() -> Face {
    Face {
        bbox: Rect::at(40, 40).of_size(100, 120),
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
        eyes: [(-3.0, 2.0), (400.0, 400.0)],
        score: 0.8,
    };
    let out = guard(&full(20, 20), &[f], None);
    assert_eq!(out.get_pixel(0, 2).0[0], 0);
    assert_eq!(out.get_pixel(19, 19).0[0], 255);
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

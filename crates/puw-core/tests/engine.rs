mod common;

use image::imageops::FilterType;
use puw_core::engine::{Backend, CpuReason, Engine, ModelPaths};

const SIDE: usize = 512;
const PLANE: usize = SIDE * SIDE;
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];

fn engine(prefer_gpu: bool) -> Option<Engine> {
    let Some(dir) = common::models_dir() else {
        eprintln!("skipped: PUW_MODELS_DIR not set");
        return None;
    };
    let paths = ModelPaths::from_dir(&dir).expect("model files present");
    Some(Engine::new(&paths, prefer_gpu, None).expect("engine loads"))
}

/// First marked image of the synthetic styles set, resized to 512x512, as planar 0..1 RGB.
fn sample_rgb01() -> Vec<f32> {
    let dir = common::synth_dir().join("styles").join("marked");
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .expect("read styles/marked")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "png"))
        .collect();
    names.sort();
    let img = image::open(&names[0]).expect("open sample").to_rgb8();
    let img = image::imageops::resize(&img, SIDE as u32, SIDE as u32, FilterType::Triangle);
    let mut out = vec![0.0f32; 3 * PLANE];
    for (i, px) in img.pixels().enumerate() {
        for c in 0..3 {
            out[c * PLANE + i] = f32::from(px[c]) / 255.0;
        }
    }
    out
}

fn imagenet(rgb01: &[f32]) -> Vec<f32> {
    rgb01
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let c = i / PLANE;
            (v - MEAN[c]) / STD[c]
        })
        .collect()
}

fn centred_hole(side: usize) -> Vec<f32> {
    let start = (SIDE - side) / 2;
    let mut mask = vec![0.0f32; PLANE];
    for y in start..start + side {
        mask[y * SIDE + start..y * SIDE + start + side].fill(1.0);
    }
    mask
}

/// A partly supported model is reloaded on the CPU, so only `Gpu` means a GPU session.
fn gpu_active(backend: &Backend) -> bool {
    matches!(backend, Backend::Gpu { .. })
}

#[test]
fn engine_loads_on_cpu() {
    let Some(engine) = engine(false) else { return };
    let report = engine.report();
    for backend in [&report.segmenter, &report.lama, &report.face] {
        assert_eq!(
            backend,
            &Backend::Cpu {
                reason: CpuReason::NotRequested
            }
        );
    }
    let prob = engine
        .run_segmenter(&vec![0.0; 3 * PLANE])
        .expect("segmenter runs");
    assert_eq!(prob.len(), PLANE);
    assert!(prob.iter().all(|p| (0.0..=1.0).contains(p)));
}

#[test]
fn engine_reports_gpu_honestly() {
    let Some(engine) = engine(true) else { return };
    let report = engine.report();
    eprintln!("device report with prefer_gpu = true: {report:?}");
    for backend in [&report.segmenter, &report.lama, &report.face] {
        match backend {
            Backend::Gpu { name } => assert!(!name.is_empty()),
            Backend::Cpu { reason } => match reason {
                CpuReason::NotRequested => panic!("GPU was requested"),
                CpuReason::ProviderUnavailable(why)
                | CpuReason::LoadFailed(why)
                | CpuReason::GpuNotBeneficial(why) => {
                    assert!(!why.trim().is_empty(), "empty reason")
                }
                CpuReason::PartialGpu {
                    gpu_nodes,
                    total_nodes,
                } => assert!(0 < *gpu_nodes && gpu_nodes < total_nodes),
            },
        }
        #[cfg(target_os = "macos")]
        assert_eq!(
            backend,
            &Backend::Cpu {
                reason: CpuReason::GpuNotBeneficial(puw_core::engine::MAC_GPU_REASON.to_string())
            }
        );
    }
}

#[test]
fn lama_output_range() {
    let Some(engine) = engine(false) else { return };
    let out = engine
        .run_lama(&vec![0.0; 3 * PLANE], &vec![0.0; PLANE])
        .expect("lama runs");
    assert_eq!(out.len(), 3 * PLANE);
    assert!(
        out.iter().all(|v| (0.0..=255.0).contains(v)),
        "out of range"
    );
}

#[test]
fn face_outputs_in_graph_order() {
    let Some(engine) = engine(false) else { return };
    let outs = engine
        .run_face(&vec![0.0; 3 * 640 * 640], 640, 640)
        .expect("face model runs");
    // cls, obj, bbox, kps for strides 8, 16, 32 (YuNet 2023mar)
    let lens: Vec<usize> = outs.iter().map(Vec::len).collect();
    assert_eq!(
        lens,
        [
            6400, 1600, 400, 6400, 1600, 400, 25600, 6400, 1600, 64000, 16000, 4000
        ]
    );
    assert!(
        engine
            .run_face(&vec![0.0; 3 * 320 * 320], 320, 320)
            .is_err()
    );
}

#[test]
fn wrong_input_length_is_an_error() {
    let Some(engine) = engine(false) else { return };
    assert!(engine.run_segmenter(&[0.0; 10]).is_err());
    assert!(engine.run_lama(&vec![0.0; 3 * PLANE], &[0.0; 10]).is_err());
}

#[test]
fn missing_model_file_is_an_error() {
    let dir = std::env::temp_dir().join(format!("puw-no-models-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let err = ModelPaths::from_dir(&dir);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(matches!(err, Err(puw_core::Error::Io(_))));
}

#[test]
fn gpu_matches_cpu() {
    let Some(gpu) = engine(true) else { return };
    let report = gpu.report().clone();
    let check_seg = gpu_active(&report.segmenter);
    let check_lama = gpu_active(&report.lama);
    if !check_seg && !check_lama {
        eprintln!("skipped: no GPU backend active ({report:?})");
        return;
    }
    let cpu = engine(false).expect("models available");
    let rgb = sample_rgb01();

    if check_seg {
        let input = imagenet(&rgb);
        let a = gpu.run_segmenter(&input).expect("gpu segmenter");
        let b = cpu.run_segmenter(&input).expect("cpu segmenter");
        let max_diff = a
            .iter()
            .zip(&b)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0f32, f32::max);
        eprintln!("segmenter GPU vs CPU max prob diff: {max_diff}");
        assert!(max_diff < 0.02, "segmenter max diff {max_diff}");
    } else {
        eprintln!("segmenter on CPU ({:?}), not compared", report.segmenter);
    }

    if check_lama {
        let mask = centred_hole(128);
        let a = gpu.run_lama(&rgb, &mask).expect("gpu lama");
        let b = cpu.run_lama(&rgb, &mask).expect("cpu lama");
        let (mut sum, mut n) = (0.0f64, 0usize);
        for c in 0..3 {
            for (i, m) in mask.iter().enumerate() {
                if *m > 0.5 {
                    let d = f64::from(a[c * PLANE + i] - b[c * PLANE + i]);
                    sum += d * d;
                    n += 1;
                }
            }
        }
        let mse = sum / n as f64;
        let psnr = if mse == 0.0 {
            f64::INFINITY
        } else {
            10.0 * (255.0f64 * 255.0 / mse).log10()
        };
        eprintln!("LaMa GPU vs CPU PSNR in hole: {psnr:.2} dB");
        assert!(psnr > 40.0, "LaMa PSNR {psnr:.2} dB");
    } else {
        eprintln!("LaMa on CPU ({:?}), not compared", report.lama);
    }
}

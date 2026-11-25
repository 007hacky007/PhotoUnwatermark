mod common;

use image::imageops::FilterType;
use std::time::{Duration, Instant};

use puw_core::engine::{Backend, CpuReason, Engine, EngineOptions, ModelKind, ModelPaths};

const SIDE: usize = 512;
const PLANE: usize = SIDE * SIDE;
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];

fn engine_with(opts: EngineOptions) -> Option<Engine> {
    let Some(dir) = common::models_dir() else {
        eprintln!("skipped: PUW_MODELS_DIR not set");
        return None;
    };
    let paths = ModelPaths::from_dir(&dir).expect("model files present");
    Some(Engine::new(&paths, opts).expect("engine created"))
}

fn engine(prefer_gpu: bool) -> Option<Engine> {
    engine_with(EngineOptions {
        prefer_gpu,
        ..EngineOptions::default()
    })
}

/// Every model loaded, so the report has an entry for each.
fn loaded_engine(prefer_gpu: bool) -> Option<Engine> {
    let engine = engine(prefer_gpu)?;
    for kind in [ModelKind::Segmenter, ModelKind::Lama, ModelKind::Face] {
        engine.load(kind).expect("model loads");
    }
    Some(engine)
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

fn face_input() -> Vec<f32> {
    vec![0.0; 3 * 640 * 640]
}

#[test]
fn new_is_instant() {
    let Some(dir) = common::models_dir() else {
        eprintln!("skipped: PUW_MODELS_DIR not set");
        return;
    };
    let paths = ModelPaths::from_dir(&dir).expect("model files present");
    let start = Instant::now();
    let engine = Engine::new(&paths, EngineOptions::default()).expect("engine created");
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "{:?}",
        start.elapsed()
    );
    let report = engine.report();
    assert_eq!(
        [report.segmenter, report.lama, report.face],
        [None, None, None]
    );
}

#[test]
fn load_is_idempotent() {
    let Some(engine) = engine(false) else { return };
    let first = engine.load(ModelKind::Face).expect("face loads");
    let second = engine.load(ModelKind::Face).expect("face loads again");
    assert_eq!(first, second);
    assert_eq!(engine.load_count(ModelKind::Face), 1);
    assert_eq!(engine.backend(ModelKind::Face), Some(first));
    assert_eq!(engine.backend(ModelKind::Lama), None);
}

#[test]
fn run_loads_on_demand() {
    let Some(engine) = engine(false) else { return };
    assert_eq!(engine.backend(ModelKind::Face), None);
    let outs = engine.run_face(&face_input(), 640, 640).expect("face runs");
    assert_eq!(outs.len(), 12);
    assert_eq!(
        engine.backend(ModelKind::Face),
        Some(Backend::Cpu {
            reason: CpuReason::NotRequested
        })
    );
    assert_eq!(engine.backend(ModelKind::Segmenter), None);
}

#[test]
fn concurrent_load_loads_once() {
    let Some(engine) = engine(false) else { return };
    let backends: Vec<Backend> = std::thread::scope(|s| {
        let a = s.spawn(|| engine.load(ModelKind::Face).expect("face loads"));
        let b = s.spawn(|| engine.load(ModelKind::Face).expect("face loads"));
        vec![a.join().expect("thread a"), b.join().expect("thread b")]
    });
    assert_eq!(backends[0], backends[1]);
    assert_eq!(engine.load_count(ModelKind::Face), 1);
}

#[test]
fn gpu_run_failure_falls_back() {
    let Some(engine) = engine(false) else { return };
    let expected = engine.run_face(&face_input(), 640, 640).expect("face runs");
    engine
        .simulate_gpu_failure(ModelKind::Face)
        .expect("face loads");
    assert!(matches!(
        engine.backend(ModelKind::Face),
        Some(Backend::Gpu { .. })
    ));
    let outs = engine
        .run_face(&face_input(), 640, 640)
        .expect("face runs after the GPU failed");
    assert_eq!(outs, expected);
    match engine.backend(ModelKind::Face) {
        Some(Backend::Cpu {
            reason: CpuReason::RunFailed(why),
        }) => assert!(!why.trim().is_empty(), "empty reason"),
        other => panic!("expected Cpu {{ RunFailed }}, got {other:?}"),
    }
    assert_eq!(engine.load_count(ModelKind::Face), 2);
}

#[test]
fn wrong_input_does_not_fall_back() {
    let Some(engine) = engine(false) else { return };
    engine
        .simulate_gpu_failure(ModelKind::Face)
        .expect("face loads");
    assert!(engine.run_face(&[0.0; 10], 640, 640).is_err());
    assert!(matches!(
        engine.backend(ModelKind::Face),
        Some(Backend::Gpu { .. })
    ));
}

#[test]
fn intra_threads_applied() {
    let Some(engine) = engine_with(EngineOptions {
        intra_threads: Some(1),
        ..EngineOptions::default()
    }) else {
        return;
    };
    assert_eq!(
        engine.load(ModelKind::Face).expect("face loads"),
        Backend::Cpu {
            reason: CpuReason::NotRequested
        }
    );
    let outs = engine.run_face(&face_input(), 640, 640).expect("face runs");
    assert_eq!(outs.len(), 12);
}

#[test]
fn model_kind_serialises_lowercase() {
    let json = serde_json::to_string(&[ModelKind::Segmenter, ModelKind::Lama, ModelKind::Face])
        .expect("serialise");
    assert_eq!(json, r#"["segmenter","lama","face"]"#);
    let back: ModelKind = serde_json::from_str(r#""lama""#).expect("deserialise");
    assert_eq!(back, ModelKind::Lama);
}

#[test]
fn engine_loads_on_cpu() {
    let Some(engine) = loaded_engine(false) else {
        return;
    };
    let report = engine.report();
    for backend in [report.segmenter, report.lama, report.face] {
        assert_eq!(
            backend,
            Some(Backend::Cpu {
                reason: CpuReason::NotRequested
            })
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
    let Some(engine) = loaded_engine(true) else {
        return;
    };
    let report = engine.report();
    eprintln!("device report with prefer_gpu = true: {report:?}");
    for backend in [&report.segmenter, &report.lama, &report.face] {
        let backend = backend.as_ref().expect("model loaded");
        match backend {
            Backend::Gpu { name } => assert!(!name.is_empty()),
            Backend::Cpu { reason } => match reason {
                CpuReason::NotRequested => panic!("GPU was requested"),
                CpuReason::ProviderUnavailable(why)
                | CpuReason::LoadFailed(why)
                | CpuReason::GpuNotBeneficial(why)
                | CpuReason::RunFailed(why) => {
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
    let check_seg = gpu_active(&gpu.load(ModelKind::Segmenter).expect("segmenter loads"));
    let check_lama = gpu_active(&gpu.load(ModelKind::Lama).expect("lama loads"));
    let report = gpu.report();
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

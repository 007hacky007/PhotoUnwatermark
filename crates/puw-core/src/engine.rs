//! ONNX Runtime sessions for the three models, and an honest report of where each one runs.
//!
//! Models load one at a time, on first use (see [`Engine`]). With `prefer_gpu` on Windows, each
//! model is first loaded with the DirectML execution provider. ONNX Runtime's own session log is
//! captured while the model loads and read for how many graph nodes the provider took. The report says `Gpu` only when that log shows the
//! provider took every node; anything else is reported as `Cpu` with the real reason, never a
//! guess. macOS never uses Core ML (see [`MAC_GPU_REASON`] and `docs/notes/coreml.md`).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Instant;

use ort::environment::Environment;
use ort::ep::ExecutionProviderDispatch;
use ort::logging::LogLevel;
use ort::session::Session;
use ort::session::builder::SessionBuilder;
use ort::value::TensorRef;
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// Locations of the three model files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelPaths {
    pub segmenter: PathBuf,
    pub lama: PathBuf,
    pub face: PathBuf,
}

impl ModelPaths {
    /// `segmenter.onnx`, `lama.onnx` and `face.onnx` inside `dir`; fails with [`Error::Io`]
    /// (`NotFound`) when one of them is missing.
    pub fn from_dir(dir: &Path) -> Result<Self> {
        let file = |name: &str| {
            let path = dir.join(name);
            if path.is_file() {
                Ok(path)
            } else {
                Err(Error::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("model file {} not found", path.display()),
                )))
            }
        };
        Ok(Self {
            segmenter: file("segmenter.onnx")?,
            lama: file("lama.onnx")?,
            face: file("face.onnx")?,
        })
    }
}

/// Where one model runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Backend {
    /// The whole model runs through the named GPU provider (`"DirectML"`).
    Gpu { name: String },
    /// The model runs on the processor, for the given reason.
    Cpu { reason: CpuReason },
}

/// Why a model runs on the processor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CpuReason {
    /// The caller asked for the processor (`prefer_gpu = false`).
    NotRequested,
    /// No usable GPU provider: none is built for this platform, it could not be set up, it took
    /// no part of the model, or its share could not be confirmed. The text says which.
    ProviderUnavailable(String),
    /// The GPU provider was set up but loading the model with it failed; the text is the error.
    /// The model was then loaded again for the processor.
    LoadFailed(String),
    /// The GPU provider can run only `gpu_nodes` of the model's `total_nodes` compute nodes. Both
    /// are real graph nodes after ONNX Runtime's graph optimisations, counted with the provider's
    /// own graph fusion off; `Shape`, `Size` and `Memcpy*` nodes, which ONNX Runtime keeps on the
    /// processor by design, are not counted. By policy the model is then loaded again for the
    /// processor alone: v1 uses the GPU only for a model it can run completely, so "graphics
    /// card" in the report always means the whole model, and no per-machine guess is made about
    /// whether a split between the two is faster.
    PartialGpu {
        gpu_nodes: usize,
        total_nodes: usize,
    },
    /// A GPU path exists but was measured to be worse than the processor on this platform; the
    /// text states the measured facts.
    GpuNotBeneficial(String),
    /// The model ran on the GPU, but a run failed; the text is the error. The model was then
    /// loaded again for the processor and the run retried there.
    RunFailed(String),
}

/// Why macOS runs every model on the processor. Measured with Core ML through ONNX Runtime 1.31
/// on Apple silicon: segmenter results differed from the processor's by up to 0.04-0.21 in
/// probability with every compute-unit setting, the fp32 models ran as fast or faster on the
/// processor, LaMa only partly fits Core ML, and loading took 14-55 s longer.
pub const MAC_GPU_REASON: &str = "On Macs the graphics path (Core ML) gave different results and was slower than the \
     processor in our tests";

/// Where each loaded model runs; `None` for a model that is not loaded yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceReport {
    pub segmenter: Option<Backend>,
    pub lama: Option<Backend>,
    pub face: Option<Backend>,
}

/// One of the three models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelKind {
    Segmenter,
    Lama,
    Face,
}

impl ModelKind {
    fn label(self) -> &'static str {
        match self {
            ModelKind::Segmenter => "segmenter",
            ModelKind::Lama => "lama",
            ModelKind::Face => "face",
        }
    }
}

impl ModelPaths {
    fn get(&self, kind: ModelKind) -> &Path {
        match kind {
            ModelKind::Segmenter => &self.segmenter,
            ModelKind::Lama => &self.lama,
            ModelKind::Face => &self.face,
        }
    }
}

/// How the engine loads its models.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EngineOptions {
    /// Try the platform's GPU provider first, falling back to the processor on any failure (see
    /// [`Backend`]).
    pub prefer_gpu: bool,
    /// A folder with GPU runtime libraries (`DirectML.dll` on Windows); it is added to the DLL
    /// search path before DirectML is used and ignored on other platforms.
    pub gpu_helper_dir: Option<PathBuf>,
    /// Threads ONNX Runtime may use within one model run; `None` keeps its default (one per
    /// physical core).
    pub intra_threads: Option<usize>,
}

/// The three models, each loaded on first use and kept until the engine is dropped.
///
/// All methods take `&self` and may be called from several threads. Callers asking for a model
/// that is being loaded wait for that one load. A model running on the GPU whose run fails is
/// loaded again for the processor and the run is retried once there; its backend then reports
/// [`CpuReason::RunFailed`].
pub struct Engine {
    paths: ModelPaths,
    opts: EngineOptions,
    /// The GPU provider, or why there is none; resolved on the first load.
    gpu: OnceLock<std::result::Result<GpuProvider, CpuReason>>,
    /// Indexed by `ModelKind as usize`.
    slots: [Slot; 3],
}

/// One model's place in the engine.
#[derive(Default)]
struct Slot {
    /// Held for the whole of a load, so concurrent callers wait for one load.
    loading: Mutex<()>,
    /// The loaded model. Locked only briefly, so asking for a backend never waits for a load.
    current: Mutex<Option<Loaded>>,
    /// Sessions created for this model (tests check that a model loads once).
    loads: AtomicUsize,
}

impl Slot {
    fn get(&self) -> Option<Loaded> {
        lock(&self.current).clone()
    }

    fn set(&self, loaded: Loaded) {
        *lock(&self.current) = Some(loaded);
    }
}

#[derive(Clone)]
struct Loaded {
    model: Arc<Model>,
    backend: Backend,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Engine {
    /// An engine for the models at `paths`. Loads nothing: each model is loaded by [`Engine::load`]
    /// or on its first run. Never fails today; the `Result` leaves room for checks of `opts`.
    pub fn new(paths: &ModelPaths, opts: EngineOptions) -> Result<Engine> {
        Ok(Engine {
            paths: paths.clone(),
            opts,
            gpu: OnceLock::new(),
            slots: Default::default(),
        })
    }

    /// Loads `kind` unless it is already loaded, and returns where it runs. With `prefer_gpu` the
    /// model tries the platform's GPU provider and falls back to the processor on any failure.
    ///
    /// Errors only when the model cannot be loaded even for the processor.
    pub fn load(&self, kind: ModelKind) -> Result<Backend> {
        Ok(self.loaded(kind)?.backend)
    }

    /// Where `kind` runs, or `None` when it is not loaded.
    pub fn backend(&self, kind: ModelKind) -> Option<Backend> {
        self.slot(kind).get().map(|l| l.backend)
    }

    /// Where each loaded model runs.
    pub fn report(&self) -> DeviceReport {
        DeviceReport {
            segmenter: self.backend(ModelKind::Segmenter),
            lama: self.backend(ModelKind::Lama),
            face: self.backend(ModelKind::Face),
        }
    }

    /// Sessions created so far for `kind`: 1 after the first load, one more for each fallback to
    /// the processor after a failed GPU run.
    ///
    /// Public only for the integration tests; not part of the stable API.
    #[doc(hidden)]
    pub fn load_count(&self, kind: ModelKind) -> usize {
        self.slot(kind).loads.load(Ordering::SeqCst)
    }

    /// Loads `kind` if needed, then reports it as running on a GPU and makes its next run fail, so
    /// the fallback after a failed GPU run can be tested on any machine.
    ///
    /// Public only for the integration tests; not part of the stable API.
    #[doc(hidden)]
    pub fn simulate_gpu_failure(&self, kind: ModelKind) -> Result<()> {
        let slot = self.slot(kind);
        let Loaded { model, .. } = self.loaded(kind)?;
        let _guard = lock(&slot.loading);
        model.fail_next_run.store(true, Ordering::SeqCst);
        slot.set(Loaded {
            model,
            backend: Backend::Gpu {
                name: "simulated".to_string(),
            },
        });
        Ok(())
    }

    /// Segmenter on one 512x512 tile: `nchw` is ImageNet-normalised planar RGB
    /// (`3 * 512 * 512` values); returns the `512 * 512` watermark probabilities in 0..1. Loads
    /// the segmenter on first use.
    ///
    /// Public only for the integration tests; not part of the stable API.
    #[doc(hidden)]
    pub fn run_segmenter(&self, nchw: &[f32]) -> Result<Vec<f32>> {
        Ok(self.run(ModelKind::Segmenter, &[nchw])?.swap_remove(0))
    }

    /// LaMa on one 512x512 tile: `img` is planar RGB in 0..1 (`3 * 512 * 512` values), `mask` is
    /// `512 * 512` values with 1 = hole. Returns planar RGB in 0..255. Loads LaMa on first use.
    ///
    /// Public only for the integration tests; not part of the stable API.
    #[doc(hidden)]
    pub fn run_lama(&self, img: &[f32], mask: &[f32]) -> Result<Vec<f32>> {
        Ok(self.run(ModelKind::Lama, &[img, mask])?.swap_remove(0))
    }

    /// YuNet face detector. The model (2023mar) has a fixed input, so `h` and `w` must both be
    /// 640: `nchw` is `3 * 640 * 640` planar **BGR** values in 0..255 (OpenCV convention), with
    /// the photo letterboxed into the square. Returns the raw outputs in graph order:
    /// `cls_8, cls_16, cls_32, obj_8, obj_16, obj_32, bbox_8, bbox_16, bbox_32, kps_8, kps_16,
    /// kps_32` (per anchor: 1, 1, 4 and 10 values; 6400, 1600 and 400 anchors for strides 8, 16
    /// and 32). Loads the face model on first use.
    ///
    /// Public only for the integration tests; not part of the stable API.
    #[doc(hidden)]
    pub fn run_face(&self, nchw: &[f32], h: u32, w: u32) -> Result<Vec<Vec<f32>>> {
        let Loaded { model, .. } = self.loaded(ModelKind::Face)?;
        let shape = match model.inputs.first() {
            Some((_, shape)) if shape.len() == 4 => shape,
            _ => {
                return Err(Error::Model(
                    "face model does not take one NCHW image".to_string(),
                ));
            }
        };
        if shape[2..] != [i64::from(h), i64::from(w)] {
            return Err(Error::Model(format!(
                "face model takes {}x{} input, got {w}x{h}",
                shape[3], shape[2]
            )));
        }
        drop(model);
        self.run(ModelKind::Face, &[nchw])
    }

    fn slot(&self, kind: ModelKind) -> &Slot {
        &self.slots[kind as usize]
    }

    /// The loaded `kind`, loading it first if needed.
    fn loaded(&self, kind: ModelKind) -> Result<Loaded> {
        let slot = self.slot(kind);
        if let Some(loaded) = slot.get() {
            return Ok(loaded);
        }
        let _guard = lock(&slot.loading);
        if let Some(loaded) = slot.get() {
            return Ok(loaded);
        }
        let env = environment()?;
        let gpu = self.gpu.get_or_init(|| {
            if self.opts.prefer_gpu {
                platform::gpu_provider(self.opts.gpu_helper_dir.as_deref())
            } else {
                Err(CpuReason::NotRequested)
            }
        });
        let start = Instant::now();
        let (model, backend) = Model::load(
            env,
            kind.label(),
            self.paths.get(kind),
            gpu,
            self.opts.intra_threads,
        )?;
        slot.loads.fetch_add(1, Ordering::SeqCst);
        tracing::info!(
            model = kind.label(),
            ?backend,
            seconds = start.elapsed().as_secs_f64(),
            "model loaded"
        );
        let loaded = Loaded {
            model: Arc::new(model),
            backend,
        };
        slot.set(loaded.clone());
        Ok(loaded)
    }

    /// Runs `kind` on `data`, loading it first if needed. A failed run on the GPU is retried once
    /// on the processor (see [`Engine`]).
    fn run(&self, kind: ModelKind, data: &[&[f32]]) -> Result<Vec<Vec<f32>>> {
        let Loaded { model, backend } = self.loaded(kind)?;
        // Bad input is the caller's error, not a reason to leave the GPU.
        model.check(data)?;
        let err = match model.run(data) {
            Ok(outputs) => return Ok(outputs),
            Err(err) => err,
        };
        if !matches!(backend, Backend::Gpu { .. }) {
            return Err(err);
        }
        let cpu = self.fall_back(kind, &model, &err)?;
        drop(model);
        cpu.run(data)
    }

    /// Replaces the GPU model `failed` with a processor session after its run failed with `err`,
    /// unless another thread already replaced it, and returns the replacement.
    fn fall_back(&self, kind: ModelKind, failed: &Arc<Model>, err: &Error) -> Result<Arc<Model>> {
        let slot = self.slot(kind);
        let _guard = lock(&slot.loading);
        if let Some(current) = slot.get()
            && !Arc::ptr_eq(&current.model, failed)
        {
            return Ok(current.model);
        }
        let why = match err {
            Error::Model(text) => text.clone(),
            other => other.to_string(),
        };
        tracing::warn!(model = kind.label(), error = %why, "GPU run failed, switching to the processor");
        let (model, backend) = Model::load_cpu(
            environment()?,
            kind.label(),
            self.paths.get(kind),
            CpuReason::RunFailed(why),
            self.opts.intra_threads,
        )?;
        slot.loads.fetch_add(1, Ordering::SeqCst);
        let model = Arc::new(model);
        slot.set(Loaded {
            model: Arc::clone(&model),
            backend,
        });
        Ok(model)
    }
}

fn model_err(context: &str, err: impl std::fmt::Display) -> Error {
    Error::Model(format!("{context}: {err}"))
}

/// The process-wide ONNX Runtime environment, created on first use. Telemetry is off.
fn environment() -> Result<&'static Environment> {
    static ENV: OnceLock<std::result::Result<Environment, String>> = OnceLock::new();
    ENV.get_or_init(|| {
        let env = ort::init()
            .with_name("puw")
            .with_telemetry(false)
            .build()
            .map_err(|e| e.to_string())?;
        env.set_log_level(LogLevel::Warning);
        Ok(env)
    })
    .as_ref()
    .map_err(|e| model_err("could not start ONNX Runtime", e))
}

/// A GPU execution provider this build can try (only DirectML is ever built).
#[cfg_attr(not(windows), allow(dead_code))]
struct GpuProvider {
    /// Name shown to the user.
    name: &'static str,
    /// ONNX Runtime's name for the provider, as it appears in its logs.
    ep_name: &'static str,
    build: fn() -> ExecutionProviderDispatch,
    /// Session config entries for the placement probe only: they turn off the provider's graph
    /// fusion, so the probe's log lists real graph nodes instead of fused partitions.
    probe_config: &'static [(&'static str, &'static str)],
}

/// One loaded model. `Session::run` needs `&mut`, hence the mutex.
struct Model {
    label: &'static str,
    session: Mutex<Session>,
    /// Input names and fixed shapes, in graph order.
    inputs: Vec<(String, Vec<i64>)>,
    /// Set by [`Engine::simulate_gpu_failure`]: the next run fails.
    fail_next_run: AtomicBool,
}

impl Model {
    fn load(
        env: &Environment,
        label: &'static str,
        path: &Path,
        gpu: &std::result::Result<GpuProvider, CpuReason>,
        threads: Option<usize>,
    ) -> Result<(Model, Backend)> {
        let provider = match gpu {
            Ok(provider) => provider,
            Err(reason) => return Self::load_cpu(env, label, path, reason.clone(), threads),
        };
        // Probe: load once with verbose logging and graph fusion off, and read the placement.
        let log = SessionLog::new();
        let mut probe = builder(env, label, Some(&log), threads)?;
        for (key, value) in provider.probe_config {
            probe = probe
                .with_config_entry(*key, *value)
                .map_err(|e| model_err("could not configure ONNX Runtime", e))?;
        }
        let probe = match Self::commit_with(probe, provider, label, path) {
            Ok(session) => session,
            Err(reason) => return Self::load_cpu(env, label, path, reason, threads),
        };
        drop(probe);
        match classify(
            &placement(&log.finish(), provider.ep_name),
            provider.name,
            label,
        ) {
            Ok(()) => {}
            Err(reason) => return Self::load_cpu(env, label, path, reason, threads),
        }
        // The real session keeps the provider's defaults (graph fusion on).
        let session =
            match Self::commit_with(builder(env, label, None, threads)?, provider, label, path) {
                Ok(session) => session,
                Err(reason) => return Self::load_cpu(env, label, path, reason, threads),
            };
        let backend = Backend::Gpu {
            name: provider.name.to_string(),
        };
        Ok((Self::from_session(label, session)?, backend))
    }

    /// Registers `provider` on `builder` and loads the model; any failure is the reason to fall
    /// back to the processor.
    fn commit_with(
        builder: SessionBuilder,
        provider: &GpuProvider,
        label: &str,
        path: &Path,
    ) -> std::result::Result<Session, CpuReason> {
        let builder = builder
            .with_execution_providers([(provider.build)()])
            .map_err(|e| {
                CpuReason::ProviderUnavailable(format!(
                    "{} could not be set up: {}",
                    provider.name,
                    e.to_string().trim()
                ))
            })?;
        { builder }.commit_from_file(path).map_err(|e| {
            CpuReason::LoadFailed(format!(
                "loading the {label} model with {} failed: {}",
                provider.name,
                e.to_string().trim()
            ))
        })
    }

    fn load_cpu(
        env: &Environment,
        label: &'static str,
        path: &Path,
        reason: CpuReason,
        threads: Option<usize>,
    ) -> Result<(Model, Backend)> {
        let session = builder(env, label, None, threads)?
            .commit_from_file(path)
            .map_err(|e| model_err(&format!("could not load {}", path.display()), e))?;
        Ok((Self::from_session(label, session)?, Backend::Cpu { reason }))
    }

    fn from_session(label: &'static str, session: Session) -> Result<Model> {
        let mut inputs = Vec::new();
        for input in session.inputs() {
            let shape = input
                .dtype()
                .tensor_shape()
                .map(|s| s.to_vec())
                .filter(|s| s.iter().all(|d| *d > 0))
                .ok_or_else(|| {
                    Error::Model(format!(
                        "{label} model input `{}` is not a fixed-size tensor",
                        input.name()
                    ))
                })?;
            inputs.push((input.name().to_string(), shape));
        }
        Ok(Model {
            label,
            session: Mutex::new(session),
            inputs,
            fail_next_run: AtomicBool::new(false),
        })
    }

    /// Checks that `data` has one slice of the right length per input.
    fn check(&self, data: &[&[f32]]) -> Result<()> {
        let label = self.label;
        if data.len() != self.inputs.len() {
            return Err(Error::Model(format!(
                "{label} model takes {} inputs, got {}",
                self.inputs.len(),
                data.len()
            )));
        }
        for ((name, shape), values_in) in self.inputs.iter().zip(data) {
            let expected: i64 = shape.iter().product();
            if values_in.len() as i64 != expected {
                return Err(Error::Model(format!(
                    "{label} model input `{name}` needs {expected} values for shape {shape:?}, got {}",
                    values_in.len()
                )));
            }
        }
        Ok(())
    }

    /// Runs the model on `data` (one slice per input, graph order); returns every output in graph
    /// order.
    fn run(&self, data: &[&[f32]]) -> Result<Vec<Vec<f32>>> {
        let label = self.label;
        self.check(data)?;
        if self.fail_next_run.swap(false, Ordering::SeqCst) {
            return Err(Error::Model(format!("{label}: simulated GPU failure")));
        }
        let mut values = Vec::with_capacity(data.len());
        for ((name, shape), values_in) in self.inputs.iter().zip(data) {
            let tensor = TensorRef::from_array_view((shape.clone(), *values_in))
                .map_err(|e| model_err(label, e))?;
            values.push((name.as_str(), tensor));
        }
        let mut session = self.session.lock().unwrap_or_else(PoisonError::into_inner);
        let outputs = session.run(values).map_err(|e| model_err(label, e))?;
        let mut result = Vec::with_capacity(outputs.len());
        for (name, value) in &outputs {
            let (_, out) = value
                .try_extract_tensor::<f32>()
                .map_err(|e| model_err(&format!("{label} model output `{name}`"), e))?;
            result.push(out.to_vec());
        }
        Ok(result)
    }
}

/// Session builder with the crate's logging: ONNX Runtime messages go to `tracing`, and with a
/// `log` the session logs verbosely so its placement messages can be read after loading.
/// `threads` sets the threads used within one run.
fn builder(
    env: &Environment,
    label: &str,
    log: Option<&SessionLog>,
    threads: Option<usize>,
) -> Result<SessionBuilder> {
    let err = |e: ort::Error| model_err("could not configure ONNX Runtime", e);
    let level = if log.is_some() {
        LogLevel::Verbose
    } else {
        LogLevel::Warning
    };
    let lines = log.map(|l| Arc::clone(&l.lines));
    let mut builder = Session::builder(env).map_err(err)?;
    if let Some(threads) = threads {
        builder = builder
            .with_intra_threads(threads)
            .map_err(|e| err(e.into()))?;
    }
    builder
        .with_log_id(format!("puw-{label}"))
        .map_err(|e| err(e.into()))?
        .with_log_level(level)
        .map_err(|e| err(e.into()))?
        .with_logger(Arc::new(move |level, _category, id, location, message| {
            if let Some(lines) = &lines
                && let Some(lines) = lines
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .as_mut()
            {
                lines.push(message.to_string());
            }
            match level {
                LogLevel::Verbose => {
                    tracing::trace!(target: "puw_core::ort", id, location, "{message}")
                }
                LogLevel::Info => {
                    tracing::debug!(target: "puw_core::ort", id, location, "{message}")
                }
                LogLevel::Warning => {
                    tracing::warn!(target: "puw_core::ort", id, location, "{message}")
                }
                LogLevel::Error | LogLevel::Fatal => {
                    tracing::error!(target: "puw_core::ort", id, location, "{message}")
                }
            }
        }))
        .map_err(|e| err(e.into()))
}

/// Messages of one session, recorded from creation until [`SessionLog::finish`].
struct SessionLog {
    lines: Arc<Mutex<Option<Vec<String>>>>,
}

impl SessionLog {
    fn new() -> Self {
        Self {
            lines: Arc::new(Mutex::new(Some(Vec::new()))),
        }
    }

    /// Stops recording and returns what was recorded.
    fn finish(&self) -> Vec<String> {
        self.lines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .unwrap_or_default()
    }
}

/// Node placement read from ONNX Runtime's session log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Placement {
    /// Compute nodes on the GPU provider.
    gpu: usize,
    /// Compute nodes on any other provider (the processor).
    cpu: usize,
}

/// Op types ONNX Runtime places on the processor by design (shape queries and the copies between
/// device and host memory); they do not count as compute nodes.
const CPU_BY_DESIGN: &[&str] = &["Shape", "Size", "MemcpyFromHost", "MemcpyToHost"];

/// Compute-node placement from ONNX Runtime's session log, or `None` if the log does not say.
///
/// ONNX Runtime 1.31 logs at verbose level either `All nodes placed on [X]. Number of nodes: N`,
/// or for each provider ` Node(s) placed on [X]. Number of nodes: N` followed by one
/// `  OpType (node name)` line per node. The per-node lines are used when present so that
/// [`CPU_BY_DESIGN`] nodes can be left out. The log is written after graph partitioning, so a
/// provider that fuses its nodes shows each fused partition as one node; the probe session turns
/// fusion off for that reason (see `GpuProvider::probe_config`).
fn placement(lines: &[String], ep_name: &str) -> Option<Placement> {
    // (provider, count from the header, op types listed under it)
    let mut groups: Vec<(String, usize, Vec<String>)> = Vec::new();
    let mut listing = false;
    for line in lines {
        if let Some(rest) = line.split("placed on [").nth(1)
            && let (Some((ep, _)), Some(n)) = (
                rest.split_once(']'),
                number_after(rest, "Number of nodes: "),
            )
        {
            groups.push((ep.to_string(), n, Vec::new()));
            listing = !line.contains("All nodes placed on");
            continue;
        }
        let node = line.strip_prefix("  ").filter(|l| !l.starts_with(' '));
        match (listing, node, groups.last_mut()) {
            (true, Some(node), Some((_, _, ops))) if node.ends_with(')') => {
                let op = node.split(" (").next().unwrap_or(node);
                ops.push(op.to_string());
            }
            _ => listing = false,
        }
    }
    if groups.is_empty() {
        return None;
    }
    let mut result = Placement { gpu: 0, cpu: 0 };
    for (ep, count, ops) in &groups {
        let compute = if ops.is_empty() {
            *count
        } else {
            ops.iter()
                .filter(|op| !CPU_BY_DESIGN.contains(&op.as_str()))
                .count()
        };
        if ep == ep_name {
            result.gpu += compute;
        } else {
            result.cpu += compute;
        }
    }
    Some(result)
}

/// `Ok` when the GPU provider runs every compute node, otherwise the reason to use the processor.
fn classify(
    placement: &Option<Placement>,
    provider: &str,
    label: &str,
) -> std::result::Result<(), CpuReason> {
    match *placement {
        Some(Placement { gpu, cpu: 0 }) if gpu > 0 => Ok(()),
        Some(Placement { gpu: 0, cpu }) => Err(CpuReason::ProviderUnavailable(format!(
            "{provider} cannot run any of the {cpu} steps of the {label} model"
        ))),
        Some(Placement { gpu, cpu }) => Err(CpuReason::PartialGpu {
            gpu_nodes: gpu,
            total_nodes: gpu + cpu,
        }),
        None => Err(CpuReason::ProviderUnavailable(format!(
            "ONNX Runtime did not report which steps of the {label} model {provider} took, so \
             it is not used"
        ))),
    }
}

fn number_after(text: &str, marker: &str) -> Option<usize> {
    let rest = &text[text.find(marker)? + marker.len()..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{CpuReason, GpuProvider, MAC_GPU_REASON};
    use std::path::Path;

    pub(super) fn gpu_provider(_helper_dir: Option<&Path>) -> Result<GpuProvider, CpuReason> {
        Err(CpuReason::GpuNotBeneficial(MAC_GPU_REASON.to_string()))
    }
}

#[cfg(windows)]
mod platform {
    use super::{CpuReason, GpuProvider};
    use ort::ep::DirectML;
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    pub(super) fn gpu_provider(helper_dir: Option<&Path>) -> Result<GpuProvider, CpuReason> {
        if let Some(dir) = helper_dir {
            use_helper_dir(dir).map_err(CpuReason::ProviderUnavailable)?;
        }
        Ok(GpuProvider {
            name: "DirectML",
            ep_name: "DmlExecutionProvider",
            build: || DirectML::default().build().error_on_failure(),
            // kOrtSessionOptionsConfigDisableDmlGraphFusion in ONNX Runtime.
            probe_config: &[("ep.dml.disable_graph_fusion", "1")],
        })
    }

    const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 0x0000_0008;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetDllDirectoryW(path: *const u16) -> i32;
        fn LoadLibraryExW(file: *const u16, reserved: *mut c_void, flags: u32) -> *mut c_void;
    }

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    /// Adds `dir` to the DLL search path and loads `dir\DirectML.dll` when present, so DirectML
    /// comes from the helper folder rather than from wherever the default search finds it.
    fn use_helper_dir(dir: &Path) -> Result<(), String> {
        // LoadLibraryExW with LOAD_WITH_ALTERED_SEARCH_PATH needs an absolute path; a relative one
        // fails with error 126 (seen in the linking spike).
        let dir = &std::path::absolute(dir)
            .map_err(|e| format!("could not resolve {}: {e}", dir.display()))?;
        // SAFETY: the argument is a NUL-terminated UTF-16 path that outlives the call.
        if unsafe { SetDllDirectoryW(wide(dir).as_ptr()) } == 0 {
            return Err(format!(
                "could not add {} to the DLL search path: {}",
                dir.display(),
                std::io::Error::last_os_error()
            ));
        }
        let dml = dir.join("DirectML.dll");
        if dml.is_file() {
            // SAFETY: as above; the module handle is intentionally kept for the process lifetime.
            let module = unsafe {
                LoadLibraryExW(
                    wide(&dml).as_ptr(),
                    std::ptr::null_mut(),
                    LOAD_WITH_ALTERED_SEARCH_PATH,
                )
            };
            if module.is_null() {
                return Err(format!(
                    "could not load {}: {}",
                    dml.display(),
                    std::io::Error::last_os_error()
                ));
            }
        }
        Ok(())
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod platform {
    use super::{CpuReason, GpuProvider};
    use std::path::Path;

    pub(super) fn gpu_provider(_helper_dir: Option<&Path>) -> Result<GpuProvider, CpuReason> {
        Err(CpuReason::ProviderUnavailable(
            "this build has no graphics card support for this platform".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &[&str]) -> Vec<String> {
        text.iter().map(|s| s.to_string()).collect()
    }

    const DML: &str = "DmlExecutionProvider";

    /// Log text in ONNX Runtime 1.31's format: a header per provider, then one line per node.
    fn placement_log(groups: &[(&str, &[(&str, usize)])]) -> Vec<String> {
        let mut out = vec!["Saving initialized tensors.".to_string()];
        for (ep, ops) in groups {
            let total: usize = ops.iter().map(|(_, n)| n).sum();
            out.push(format!(
                " Node(s) placed on [{ep}]. Number of nodes: {total}"
            ));
            for (op, n) in *ops {
                for i in 0..*n {
                    out.push(format!("  {op} (/model/{op}_{i})"));
                }
            }
        }
        out.push("Done saving initialized tensors".to_string());
        out
    }

    #[test]
    fn all_dml_fused_counts_as_gpu() {
        // Graph fusion on: each DML partition is one node; CPU keeps only shape and copy nodes.
        let log = placement_log(&[
            (DML, &[("DmlFusedNode_0_0", 1), ("DmlFusedNode_0_1", 1)]),
            ("CPUExecutionProvider", &[("Shape", 2), ("MemcpyToHost", 1)]),
        ]);
        let found = placement(&log, DML);
        assert_eq!(found, Some(Placement { gpu: 2, cpu: 0 }));
        assert_eq!(classify(&found, "DirectML", "lama"), Ok(()));
        let single = lines(&[" All nodes placed on [DmlExecutionProvider]. Number of nodes: 1"]);
        assert_eq!(
            classify(&placement(&single, DML), "DirectML", "lama"),
            Ok(())
        );
    }

    #[test]
    fn unfused_dml_with_cpu_shape_nodes_counts_as_gpu() {
        let log = placement_log(&[
            (DML, &[("Conv", 80), ("Relu", 30), ("Add", 10)]),
            ("CPUExecutionProvider", &[("Shape", 3), ("Size", 1)]),
        ]);
        let found = placement(&log, DML);
        assert_eq!(found, Some(Placement { gpu: 120, cpu: 0 }));
        assert_eq!(classify(&found, "DirectML", "segmenter"), Ok(()));
    }

    #[test]
    fn true_split_is_partial_gpu_in_real_nodes() {
        let log = placement_log(&[
            (DML, &[("Conv", 60), ("MemcpyFromHost", 2), ("Mul", 30)]),
            (
                "CPUExecutionProvider",
                &[("Einsum", 4), ("Gather", 1), ("Shape", 2)],
            ),
        ]);
        let found = placement(&log, DML);
        assert_eq!(found, Some(Placement { gpu: 90, cpu: 5 }));
        assert_eq!(
            classify(&found, "DirectML", "lama"),
            Err(CpuReason::PartialGpu {
                gpu_nodes: 90,
                total_nodes: 95
            })
        );
    }

    #[test]
    fn all_cpu_or_no_evidence_is_unavailable() {
        let all_cpu = lines(&[" All nodes placed on [CPUExecutionProvider]. Number of nodes: 42"]);
        let found = placement(&all_cpu, DML);
        assert_eq!(found, Some(Placement { gpu: 0, cpu: 42 }));
        assert!(matches!(
            classify(&found, "DirectML", "face"),
            Err(CpuReason::ProviderUnavailable(why)) if why.contains("42 steps")
        ));
        let none = placement(&lines(&["Session created"]), DML);
        assert_eq!(none, None);
        assert!(matches!(
            classify(&none, "DirectML", "face"),
            Err(CpuReason::ProviderUnavailable(_))
        ));
    }

    #[test]
    fn header_counts_used_when_nodes_are_not_listed() {
        let log = lines(&[
            " Node(s) placed on [DmlExecutionProvider]. Number of nodes: 90",
            " Node(s) placed on [CPUExecutionProvider]. Number of nodes: 10",
        ]);
        assert_eq!(placement(&log, DML), Some(Placement { gpu: 90, cpu: 10 }));
    }
}

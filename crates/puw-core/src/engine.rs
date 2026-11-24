//! ONNX Runtime sessions for the three models, and an honest report of where each one runs.
//!
//! With `prefer_gpu` on Windows, every model is first loaded with the DirectML execution
//! provider. ONNX Runtime's own session log is captured while the model loads and read for how
//! many graph nodes the provider took. The report says `Gpu` only when that log shows the
//! provider took every node; anything else is reported as `Cpu` with the real reason, never a
//! guess. macOS never uses Core ML (see [`MAC_GPU_REASON`] and `docs/notes/coreml.md`).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use ort::environment::Environment;
use ort::ep::ExecutionProviderDispatch;
use ort::logging::LogLevel;
use ort::session::Session;
use ort::session::builder::SessionBuilder;
use ort::value::TensorRef;

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
    /// The GPU provider can take only `gpu_nodes` of the model's `total_nodes` graph nodes, so the
    /// model was loaded again for the processor: a split model hands data back and forth at every
    /// boundary, which measured slower than the processor alone (LaMa with Core ML).
    PartialGpu {
        gpu_nodes: usize,
        total_nodes: usize,
    },
    /// A GPU path exists but was measured to be worse than the processor on this platform; the
    /// text states the measured facts.
    GpuNotBeneficial(String),
}

/// Why macOS runs every model on the processor. Measured with Core ML through ONNX Runtime 1.31
/// on Apple silicon: segmenter results differed from the processor's by up to 0.04-0.21 in
/// probability with every compute-unit setting, the fp32 models ran as fast or faster on the
/// processor, LaMa only partly fits Core ML, and loading took 14-55 s longer.
pub const MAC_GPU_REASON: &str = "On Macs the graphics path (Core ML) gave different results and was slower than the \
     processor in our tests";

/// Backend of each model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceReport {
    pub segmenter: Backend,
    pub lama: Backend,
    pub face: Backend,
}

/// The three loaded models.
pub struct Engine {
    segmenter: Model,
    lama: Model,
    face: Model,
    report: DeviceReport,
}

impl Engine {
    /// Loads all three models. With `prefer_gpu` each model tries the platform's GPU provider and
    /// falls back to the processor on any failure (see [`DeviceReport`]). `gpu_helper_dir` is a
    /// folder with GPU runtime libraries (`DirectML.dll` on Windows); it is added to the DLL search
    /// path before DirectML is used and ignored on other platforms.
    ///
    /// Errors only when a model cannot be loaded even for the processor.
    pub fn new(
        paths: &ModelPaths,
        prefer_gpu: bool,
        gpu_helper_dir: Option<&Path>,
    ) -> Result<Engine> {
        let env = environment()?;
        let gpu = if prefer_gpu {
            platform::gpu_provider(gpu_helper_dir)
        } else {
            Err(CpuReason::NotRequested)
        };
        let (segmenter, seg) = Model::load(env, "segmenter", &paths.segmenter, &gpu)?;
        let (lama, lama_backend) = Model::load(env, "lama", &paths.lama, &gpu)?;
        let (face, face_backend) = Model::load(env, "face", &paths.face, &gpu)?;
        Ok(Engine {
            segmenter,
            lama,
            face,
            report: DeviceReport {
                segmenter: seg,
                lama: lama_backend,
                face: face_backend,
            },
        })
    }

    /// Where each model runs.
    pub fn report(&self) -> &DeviceReport {
        &self.report
    }

    /// Segmenter on one 512x512 tile: `nchw` is ImageNet-normalised planar RGB
    /// (`3 * 512 * 512` values); returns the `512 * 512` watermark probabilities in 0..1.
    ///
    /// Public only for the integration tests; not part of the stable API.
    #[doc(hidden)]
    pub fn run_segmenter(&self, nchw: &[f32]) -> Result<Vec<f32>> {
        Ok(self.segmenter.run(&[nchw])?.swap_remove(0))
    }

    /// LaMa on one 512x512 tile: `img` is planar RGB in 0..1 (`3 * 512 * 512` values), `mask` is
    /// `512 * 512` values with 1 = hole. Returns planar RGB in 0..255.
    ///
    /// Public only for the integration tests; not part of the stable API.
    #[doc(hidden)]
    pub fn run_lama(&self, img: &[f32], mask: &[f32]) -> Result<Vec<f32>> {
        Ok(self.lama.run(&[img, mask])?.swap_remove(0))
    }

    /// YuNet face detector. The model (2023mar) has a fixed input, so `h` and `w` must both be
    /// 640: `nchw` is `3 * 640 * 640` planar **BGR** values in 0..255 (OpenCV convention), with
    /// the photo letterboxed into the square. Returns the raw outputs in graph order:
    /// `cls_8, cls_16, cls_32, obj_8, obj_16, obj_32, bbox_8, bbox_16, bbox_32, kps_8, kps_16,
    /// kps_32` (per anchor: 1, 1, 4 and 10 values; 6400, 1600 and 400 anchors for strides 8, 16
    /// and 32).
    ///
    /// Public only for the integration tests; not part of the stable API.
    #[doc(hidden)]
    pub fn run_face(&self, nchw: &[f32], h: u32, w: u32) -> Result<Vec<Vec<f32>>> {
        let shape = &self.face.inputs[0].1;
        if shape[2..] != [i64::from(h), i64::from(w)] {
            return Err(Error::Model(format!(
                "face model takes {}x{} input, got {w}x{h}",
                shape[3], shape[2]
            )));
        }
        self.face.run(&[nchw])
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
}

/// One loaded model. `Session::run` needs `&mut`, hence the mutex.
struct Model {
    label: &'static str,
    session: Mutex<Session>,
    /// Input names and fixed shapes, in graph order.
    inputs: Vec<(String, Vec<i64>)>,
}

impl Model {
    fn load(
        env: &Environment,
        label: &'static str,
        path: &Path,
        gpu: &std::result::Result<GpuProvider, CpuReason>,
    ) -> Result<(Model, Backend)> {
        let provider = match gpu {
            Ok(provider) => provider,
            Err(reason) => return Self::load_cpu(env, label, path, reason.clone()),
        };
        let log = SessionLog::new();
        let builder = builder(env, label, Some(&log))?;
        let builder = match builder.with_execution_providers([(provider.build)()]) {
            Ok(b) => b,
            Err(e) => {
                let why = format!(
                    "{} could not be set up: {}",
                    provider.name,
                    e.to_string().trim()
                );
                return Self::load_cpu(env, label, path, CpuReason::ProviderUnavailable(why));
            }
        };
        let session = match { builder }.commit_from_file(path) {
            Ok(session) => session,
            Err(e) => {
                let why = format!(
                    "loading the {label} model with {} failed: {}",
                    provider.name,
                    e.to_string().trim()
                );
                return Self::load_cpu(env, label, path, CpuReason::LoadFailed(why));
            }
        };
        let lines = log.finish();
        let backend = match placement(&lines, provider.ep_name) {
            Some((gpu_nodes, total_nodes)) if gpu_nodes == total_nodes && total_nodes > 0 => {
                Backend::Gpu {
                    name: provider.name.to_string(),
                }
            }
            Some((gpu_nodes, total_nodes)) if gpu_nodes > 0 => {
                let reason = CpuReason::PartialGpu {
                    gpu_nodes,
                    total_nodes,
                };
                return Self::load_cpu(env, label, path, reason);
            }
            Some((_, total_nodes)) => {
                let why = format!(
                    "{} cannot run any of the {total_nodes} steps of the {label} model",
                    provider.name
                );
                return Self::load_cpu(env, label, path, CpuReason::ProviderUnavailable(why));
            }
            None => {
                let why = format!(
                    "ONNX Runtime did not report which steps of the {label} model {} took, so it \
                     is not used",
                    provider.name
                );
                return Self::load_cpu(env, label, path, CpuReason::ProviderUnavailable(why));
            }
        };
        Ok((Self::from_session(label, session)?, backend))
    }

    fn load_cpu(
        env: &Environment,
        label: &'static str,
        path: &Path,
        reason: CpuReason,
    ) -> Result<(Model, Backend)> {
        let session = builder(env, label, None)?
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
        })
    }

    /// Runs the model on `data` (one slice per input, graph order); returns every output in graph
    /// order.
    fn run(&self, data: &[&[f32]]) -> Result<Vec<Vec<f32>>> {
        let label = self.label;
        if data.len() != self.inputs.len() {
            return Err(Error::Model(format!(
                "{label} model takes {} inputs, got {}",
                self.inputs.len(),
                data.len()
            )));
        }
        let mut values = Vec::with_capacity(data.len());
        for ((name, shape), values_in) in self.inputs.iter().zip(data) {
            let expected: i64 = shape.iter().product();
            if values_in.len() as i64 != expected {
                return Err(Error::Model(format!(
                    "{label} model input `{name}` needs {expected} values for shape {shape:?}, got {}",
                    values_in.len()
                )));
            }
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
fn builder(env: &Environment, label: &str, log: Option<&SessionLog>) -> Result<SessionBuilder> {
    let err = |e: ort::Error| model_err("could not configure ONNX Runtime", e);
    let level = if log.is_some() {
        LogLevel::Verbose
    } else {
        LogLevel::Warning
    };
    let lines = log.map(|l| Arc::clone(&l.lines));
    Session::builder(env)
        .map_err(err)?
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

/// `(nodes taken by ep_name, total nodes)` from ONNX Runtime's session log, if it says.
///
/// ONNX Runtime 1.31 logs, at verbose level, how many graph nodes ended up on each provider
/// (`All nodes placed on [X]. Number of nodes: N` or one ` Node(s) placed on [X]. Number of
/// nodes: N` line per provider).
fn placement(lines: &[String], ep_name: &str) -> Option<(usize, usize)> {
    let mut found = false;
    let (mut taken, mut total) = (0, 0);
    for line in lines {
        let Some(rest) = line.split("placed on [").nth(1) else {
            continue;
        };
        let (Some((ep, _)), Some(n)) = (
            rest.split_once(']'),
            number_after(rest, "Number of nodes: "),
        ) else {
            continue;
        };
        found = true;
        total += n;
        if ep == ep_name {
            taken += n;
        }
    }
    found.then_some((taken, total))
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

    #[test]
    fn placement_sums_placement_lines() {
        let log = lines(&[
            " Node(s) placed on [DmlExecutionProvider]. Number of nodes: 90",
            " Node(s) placed on [CPUExecutionProvider]. Number of nodes: 10",
        ]);
        assert_eq!(placement(&log, "DmlExecutionProvider"), Some((90, 100)));
        let all_cpu = lines(&["All nodes placed on [CPUExecutionProvider]. Number of nodes: 42"]);
        assert_eq!(placement(&all_cpu, "DmlExecutionProvider"), Some((0, 42)));
    }

    #[test]
    fn placement_unknown_without_evidence() {
        assert_eq!(
            placement(&lines(&["Session created"]), "DmlExecutionProvider"),
            None
        );
    }
}

# PhotoUnwatermark Plan 2: Desktop App

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A working macOS + Windows desktop app (Tauri 2 + Svelte 5) on top of `puw-core` that takes a first-time user from the one-time download through the 5-step wizard to clean copies, with saved watermarks, settings and an honest processing-device chip.

**Architecture:** Three layers. `puw-core` (Plan 1) gets the engine changes the app needs (lazy per-model loading, thread cap, GPU fallback, position prior, batch-layer filter). A new plain-Rust crate `crates/puw-app` holds all app logic (settings, download, photo intake, session, jobs, plain-language messages) with no Tauri dependency, so it is tested with `cargo test`. `app/src-tauri` is a thin Tauri shell (commands, progress channels, a `puw://` image protocol) and `app/src` is the Svelte UI; editor logic lives in pure TypeScript modules tested with Vitest.

**Tech Stack:** Rust 2024 (workspace, MSRV 1.88); `tauri = "2"`, `tauri-build = "2"`, `tauri-plugin-dialog = "2"`, `tauri-plugin-opener = "2"`; `ureq = "3"` (rustls), `sha2 = "0.10"`, `dirs = "6"`, `rayon = "1"`, `serde`/`serde_json`; dev: `tiny_http = "0.12"`, `tempfile = "3"`. Frontend: pnpm, Vite, Svelte 5 (runes), TypeScript, `@tauri-apps/api` 2, `@tauri-apps/plugin-dialog` 2, `@tauri-apps/plugin-opener` 2; tests: Vitest, `@testing-library/svelte`, jsdom, `svelte-check`.

**Spec:** `docs/superpowers/specs/2026-11-23-photounwatermark-design.md` (read it first; the Magnifier and Model loading decisions were confirmed by the owner on 2026-11-25). Plan 1: `docs/superpowers/plans/2026-11-23-plan-1-engine.md`. Measured CPU numbers: `docs/notes/thresholds.md`. Plan 3 (packaging, DMG, portable exe, DirectML delay-load + helper hosting, DLL-planting fix, WebView2-missing message box, final icon, release CI) follows.

## Global Constraints

- Commits as `007hacky007 <007hacky007@users.noreply.github.com>`, `commit.gpgsign false`; one short imperative subject line, no body, no trailers, no AI attribution, no command substitution in messages. Work on `master`. Pushing needs the owner's OK.
- Never commit real photos, crops, masks, overlays or file names derived from them. Real-photo checks read `PUW_REAL_DIR` and write only under the gitignored `local/`.
- Hyphen-minus only in docs and UI strings: no Unicode dashes. The UI may use the arrow `→` and the ellipsis `...`.
- UI never shows the words "mask", "mark", "unmark" (as whole words, any case), nor jargon: "inpaint", "segment", "threshold", "dilat", "ONNX", "tensor". The UI noun is "watermark area". Tools are **Brush**, **Eraser**, **Rectangle**.
- Exact copy from the spec: progress line `1 Add photos > 2 Check the watermark > 3 Check all photos > 4 Remove > 5 Done`; step-2 help "We found the watermark and painted it red. Use the Brush to paint over anything we missed and the Eraser to clean up anything that is not watermark."; buttons "Preview on this photo", "Looks right →", "Remove watermark from N photos", "Open folder", "Save this watermark for next time", "Start a new set"; offer "These look like <name>. Use it?"; summary "Found in X of Y photos. Z need a look."; default saved name "Watermark from <YYYY-MM-DD>"; loading sentence "Getting the watermark remover ready..."; chip "Using graphics card (fast)" / "Using processor (slower) - why?"; fixes "Download GPU helper (N MB)", "Try again", "Use processor anyway".
- Model loading: never at app start, never on a screen where the user works. Detection models (segmenter, face) load when detection starts (step 1 primary button), inside that wait's progress; LaMa loads on the first Preview or Remove, inside that progress. Loaded models stay for the session. Every load shows a bar and a sentence.
- Data folder: `dirs::data_local_dir()/PhotoUnwatermark` (macOS `~/Library/Application Support/PhotoUnwatermark`, Windows `%LOCALAPPDATA%\PhotoUnwatermark`) with `models/`, `watermarks/` (the `Library`), `settings.json`. Tauri identifier `io.github.007hacky007.photounwatermark`, product name `PhotoUnwatermark`.
- Models: release `models-v2`, base URL `https://github.com/007hacky007/PhotoUnwatermark/releases/download/models-v2/`; files and SHA-256 exactly as in `models/manifest.json` of that release: `segmenter.onnx` 480487402 B `2af5fa2a3ae7afd8594489dad3e9042fe1d37cddc77a399e5a6e1f6923d5e77d`, `lama.onnx` 209009184 B `844672871fc83d16c6fa27c230424299423a8e2f2e821d9552a63fa5736939cd`, `face.onnx` 232589 B `8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4`. UI states the size as "about 690 MB".
- Network: only the model download (and, from Plan 3, the GPU helper) plus user-clicked links opened in the system browser. No telemetry, no update check, no remote content in the web view (CSP).
- Output: never modify originals; default subfolder "Without watermark" next to the originals (name configurable); "detail vs speed" maps to the working long side: Best 4096 (default), Balanced 3072, Faster 2048. Preview runs at long side 1536.
- Area contract (Plan 1 R27/R29): every area crossing the UI/app boundary is binarised (non-zero -> 255) before `build_template` or inpainting; the editor works at `EDIT_LONG_SIDE = 4096` (or the photo's own size if smaller), upright pixels.
- Step-3 safety: "Unsure" photos must be opened in the editor or unticked before Remove is enabled; "Not found" photos are unticked by default; enforced in `puw-app` (source of truth) and mirrored in the UI.
- Processing never runs on the UI thread; ONNX Runtime intra-op threads = `max(1, available_parallelism - 1)`; photos in step 4 run one at a time; parallel decode/matching uses at most `max(1, available_parallelism / 2)` threads.
- HEIC is not supported in v1 (builder's decision): skipped with "HEIC photos are not supported yet. Export them as JPEG first."

## Review Focus

1. Adding a folder that already holds a "Without watermark" subfolder (a second run) must not pull results back in: folders are read one level deep and the output folder is skipped. Test: Task 6 `scan_skips_output_folder`.
2. Portrait camera photos (EXIF orientation 6/8): what the user paints on the upright edit image lands on the same pixels of the saved result. Tests: Task 6 `edit_image_is_upright`, Task 9 `area_lands_on_rotated_photo`.
3. A source folder that is read-only (memory card, network share): the app asks for a folder before starting, never fails halfway. Test: Task 9 `unwritable_source_asks_for_folder`.
4. A photo moved or deleted after it was added: that photo is listed as failed with a plain reason; the run continues. Test: Task 9 `vanished_photo_is_a_failure`.
5. Quitting or losing the network during the 690 MB download: the next start resumes from the partial file and still verifies SHA-256; a corrupt file is re-downloaded, never used. Tests: Task 5 `resumes_partial_download`, `corrupt_file_is_redownloaded`.

---

### Task 1: Engine loads models lazily

**Files:**
- Modify: `crates/puw-core/src/engine.rs`, `crates/puw-core/src/error.rs`, `crates/puw-core/src/lib.rs`, `crates/puw-core/src/library.rs` (validation errors), every `Engine::new` caller (`crates/puw-core/tests/*.rs`, `crates/puw-core/examples/device_report.rs`, `crates/puw-dev/src/main.rs`)
- Create: `docs/notes/model-loading.md`
- Test: `crates/puw-core/tests/engine.rs`

**Interfaces:**
- Produces:
  - `pub enum ModelKind { Segmenter, Lama, Face }` (Copy, Eq, Ord, Hash, Debug, Serialize, Deserialize; serialised as `"segmenter"`, `"lama"`, `"face"`).
  - `pub struct EngineOptions { pub prefer_gpu: bool, pub gpu_helper_dir: Option<PathBuf>, pub intra_threads: Option<usize> }` with `Default` (false, None, None).
  - `Engine::new(paths: &ModelPaths, opts: EngineOptions) -> Result<Engine>`: loads nothing, returns in milliseconds.
  - `Engine::load(&self, kind: ModelKind) -> Result<Backend>`: idempotent, thread-safe; concurrent callers for the same kind wait for one load.
  - `Engine::backend(&self, kind: ModelKind) -> Option<Backend>` (None = not loaded); `Engine::report(&self) -> DeviceReport` where `DeviceReport { segmenter, lama, face: Option<Backend> }`.
  - `run_segmenter` / `run_lama` / `run_face` load their model on first use (so `detect`, `inpaint`, `faces` keep working unchanged).
  - `CpuReason::RunFailed(String)`: a GPU model whose run fails is reloaded for the processor, the run is retried once, and the backend becomes `Cpu { RunFailed(<error text>) }`.
  - `Error::InvalidInput(String)` (library validation now uses it instead of `Error::Image`).

- [ ] **Step 1: Write failing tests** in `tests/engine.rs`: `new_is_instant` (`Engine::new` < 1 s and `report()` all `None`), `load_is_idempotent` (`load(Face)` twice returns equal `Backend`; `backend(Face)` is `Some`, `backend(Lama)` is `None`), `run_loads_on_demand` (`run_face` on a fresh engine succeeds and `backend(Face)` becomes `Some`), `concurrent_load_loads_once` (two threads `load(Face)`; a doc-hidden `Engine::load_count(kind)` equals 1), `gpu_run_failure_falls_back` (doc-hidden `Engine::simulate_gpu_failure(ModelKind::Face)` marks the loaded face model as GPU and fails its next run; `run_face` still returns outputs and `backend(Face)` is `Some(Cpu { reason: RunFailed(_) })`), `intra_threads_applied` (`intra_threads: Some(1)` loads and runs). Update the existing tests to the new API.
- [ ] **Step 2: Run** `cargo test -p puw-core --release --test engine` - Expected: FAIL (compile errors on the new API).
- [ ] **Step 3: Implement.** One slot per model (`OnceLock`/`Mutex<Option<Arc<Model>>>` or equivalent); the GPU provider is resolved once per engine on first load; `with_intra_threads` when set. `run_face` checks the input rank before indexing (Plan 1 deferred minor). Library `validate_name` errors become `Error::InvalidInput`.
- [ ] **Step 4: Measure the ONNX Runtime optimized-model cache.** Time a CPU load of `segmenter.onnx` and `lama.onnx` (a) as now, (b) second load of a file saved with `SessionBuilder::with_optimized_model_path` loaded with graph optimisation disabled. Adopt only if (b) <= 60 % of (a) for the segmenter; then add `EngineOptions.cache_dir: Option<PathBuf>`, files named `<label>-<model bytes>-<model mtime secs>-<cpu|dml>.ort.onnx`, stale files of the same label deleted, any cache read failure falls back to the original model. Write the numbers and the decision in `docs/notes/model-loading.md` either way.
- [ ] **Step 5: Run** `cargo test --workspace --release` and `cargo clippy --workspace --all-targets -- -D warnings` - Expected: PASS.
- [ ] **Step 6: Commit** `git commit -m "Load models lazily, one at a time"`

### Task 1b: Core ML on macOS, reported honestly (owner's decision, 2026-11-25)

Why: `ort` 2.0.0-rc.15 sets `AutoDevicePolicy::MaxEfficiency` in every `SessionBuilder::new`, which on macOS places sessions on Core ML. So every Plan 1 Mac measurement, test and threshold ran on Core ML while the report said "processor" (see `docs/notes/model-loading.md`). True processor-only is 6-8x slower. The owner chose: use Core ML on Macs on purpose and say so.

**Files:**
- Modify: `crates/puw-core/Cargo.toml` (`coreml` feature on macOS only), `crates/puw-core/src/engine.rs`, `crates/puw-core/tests/engine.rs`, `docs/notes/coreml.md`, `docs/notes/model-loading.md`, spec ("On macOS the chip reflects whether the CoreML path is actually used..." now true; drop any "Mac is CPU-only" wording)

**Interfaces:**
- Produces:
  - `EngineOptions.cache_dir: Option<PathBuf>` (Core ML compiled-model cache; ignored elsewhere).
  - With `prefer_gpu` on macOS each model is loaded with the Core ML provider registered explicitly (with `with_model_cache_dir` when `cache_dir` is set) and reported as `Backend::Gpu { name: "Core ML" }` only when the session log confirms Core ML runs it (same log-reading approach as DirectML).
  - Every processor session on every platform (`prefer_gpu = false`, and every fallback) sets `with_auto_device(AutoDevicePolicy::PreferCPU)`, so "processor" in the report is always true.
  - `MAC_GPU_REASON` / `CpuReason::GpuNotBeneficial` are no longer produced on macOS (remove them if nothing else uses them).

- [ ] **Step 1: Measure before changing defaults** (write numbers to `docs/notes/coreml.md`, replacing the outdated ones): for each model, (a) today's implicit auto placement, (b) explicit Core ML with default options, (c) explicit Core ML with the options that best match (a), (d) `PreferCPU`: load time cold and with the cache dir warm, per-tile run time, node placement from the session log, and output difference against (a) (segmenter max prob diff, LaMa hole PSNR) and against (d). Choose the explicit configuration whose outputs match (a) (segmenter max prob diff <= 0.01, LaMa hole PSNR >= 40 dB vs (a)), so Plan 1's validated behaviour stays. If no explicit configuration matches (a), stop and report NEEDS_CONTEXT with the numbers.
- [ ] **Step 2: Partial placement on Core ML.** If a model (LaMa is suspected) only partly runs on Core ML: keep the partial Core ML session when it is faster than `PreferCPU` and report it as `Backend::Gpu { name: "Core ML" }` plus the node share (extend `Backend::Gpu` with `share: Option<(usize, usize)>`, `None` for a full placement); otherwise load it for the processor with `CpuReason::PartialGpu`. Windows keeps its whole-model-or-processor policy unchanged.
- [ ] **Step 3: Write failing tests** (macOS-only where noted): `mac_prefers_core_ml` (macOS, `prefer_gpu`: `load(Face)` reports `Gpu { name: "Core ML", .. }`), `cpu_is_really_cpu` (`prefer_gpu = false`: the session log for face shows only the CPU provider; a doc-hidden accessor for the placement is allowed), `core_ml_cache_dir_used` (macOS: after a load with `cache_dir`, the folder is non-empty and a second engine's load is faster or equal), `fallback_uses_cpu_policy` (after `simulate_gpu_failure`, the reloaded session is CPU-only by log). Update the GPU-vs-CPU parity test (Plan 1 R7) so it also runs on macOS with the thresholds chosen in Step 1 against `PreferCPU`, documenting the measured Core ML vs processor difference.
- [ ] **Step 4: Implement**; also fix Task 1 review minors: the module doc and commit wording "one at a time" (different kinds may load in parallel; say so), the over-long doc line, explicit `ModelKind` discriminants or a match for slot indexing, and a pointer from `coreml.md`'s old "Loading delay" heading to `model-loading.md`.
- [ ] **Step 5: Run** `cargo test --workspace --release` (with `PUW_MODELS_DIR`) and clippy - Expected: PASS, with Mac test times recorded in the report.
- [ ] **Step 6: Commit** `git commit -m "Use Core ML on Macs and report it"`

### Task 2: Batch-layer filter and a smaller guess sample

**Files:**
- Modify: `crates/puw-core/src/batch.rs`, `crates/puw-core/src/pipeline.rs`, `docs/notes/thresholds.md`
- Test: `crates/puw-core/tests/batch.rs`, `crates/puw-core/tests/pipeline.rs`

**Interfaces:**
- Produces: `pub fn filter_layer(layer: &Mask) -> Mask` in `puw_core::batch`, applied by `build_set_layers` to every layer before it is stored (a layer that filters to empty is dropped); `REPRESENTATIVE_CAP = 3`.

Why: on a real fixed-mark set the grown batch layer covered about 15 % of the photo (static scene edges); the editor must not open with that. Six guesses cost about 40 s on CPU at 24 MP; three keep the step-1 wait shorter.

- [ ] **Step 1: Write failing tests:** `filter_keeps_watermark` (synthetic `fixed` set: the stored layer still covers >= 90 % of the ground-truth mark pixels of photo 0), `filter_drops_scene_line` (a layer = ground-truth mark of the `fixed` set + a 1 px line across the full width 300 px below it + a 1 px line down the full height: the result keeps >= 90 % of the mark and < 5 % of the lines), `filter_keeps_empty`, `representative_sample_caps_at_three` (`representative_sample(120)` has 3 ascending distinct indexes).
- [ ] **Step 2: Run** `cargo test -p puw-core --release --test batch --test pipeline` - Expected: FAIL (new tests).
- [ ] **Step 3: Implement `filter_layer`.** Approach: group components whose bounding boxes are closer than 2 % of the long side (as `template.rs` merges pieces), drop groups that span more than 60 % of the width or height of the image (scene lines, horizons, frame edges), keep the remaining group with the most pixels plus any other group with at least half its pixel count. Adjust the constants only with evidence from Step 4.
- [ ] **Step 4: Check on the local real sets** (`PUW_REAL_DIR`, `local/real/set-a`, `set-b`): with `puw-dev`, record the grown guess coverage before/after for set-a's representative photo, and write an overlay PNG under `local/` and look at it: the faint mark must still be covered. Target: coverage <= 3 % on set-a. Put aggregate numbers only (no file names) in `docs/notes/thresholds.md`.
- [ ] **Step 5: Run** `cargo test --workspace --release` - Expected: PASS.
- [ ] **Step 6: Commit** `git commit -m "Filter scene lines out of the batch layer"`

### Task 3: Position prior for fixed-position watermarks

**Files:**
- Modify: `crates/puw-core/src/template.rs`, `crates/puw-core/src/pipeline.rs`, `crates/puw-core/src/mask.rs`, `crates/puw-core/src/lib.rs`, `docs/notes/thresholds.md`
- Test: `crates/puw-core/tests/pipeline.rs`, `crates/puw-core/src/mask.rs` (unit)

**Interfaces:**
- Produces:
  - `pub fn binarize(m: &Mask) -> Mask` in `puw_core::mask` (non-zero -> 255).
  - `pub enum MatchBasis { Search, SetPosition }` and a new field `MatchResult.basis: MatchBasis` (`find` sets `Search`).
  - `pub fn match_photo(t: &WatermarkTemplate, img: &RgbImage, layers: &SetLayers) -> MatchResult` in `puw_core::pipeline`, re-exported.

Rule (Plan 1 R21): `match_photo` calls `find`. If the status is not `Found`, `img` has exactly the template's reference size `(t.ref_w, t.ref_h)`, and `layers.get(w, h)` exists, place the template's pieces at their saved position and scale 1; if at least 50 % of those pixels lie inside the layer dilated by `mask::grow_for(w, h, 0.004)`, return `Found` with that placement's mask, the `find` confidence, and `basis: SetPosition`. Otherwise return the `find` result unchanged.

- [ ] **Step 1: Write failing tests:** `binarize_any_nonzero` (values 1, 127, 128, 255 -> 255; 0 -> 0); `prior_finds_faint_fixed_mark` (synthetic `fixed` set re-blended at alpha 0.12 in the test so free search returns non-`Found` for at least one photo: with `build_set_layers` over the set, `match_photo` returns `Found` for all 8 and IoU >= 0.8 with ground truth); `prior_ignored_without_support` (a `SetLayers` whose layer lies elsewhere: result equals `find`'s); `prior_ignored_for_other_size` (a photo of a different size gets `basis == Search`); `negatives_stay_not_found` (photos of the `negative` set resized to the template size with an empty layer set never return `Found`).
- [ ] **Step 2: Run** `cargo test -p puw-core --release --test pipeline` and `cargo test -p puw-core --lib mask` - Expected: FAIL (new tests).
- [ ] **Step 3: Implement** `binarize`, `MatchBasis`, `match_photo` (reuse the piece-placement code `find` already uses to draw a placement mask).
- [ ] **Step 4: Real-set check:** with `puw-dev`, count `Found` on set-a with `find` vs `match_photo`; record the counts (aggregates only) in `docs/notes/thresholds.md`.
- [ ] **Step 5: Run** `cargo test --workspace --release` - Expected: PASS.
- [ ] **Step 6: Commit** `git commit -m "Trust the set position for fixed watermarks"`

### Task 4: App crate, settings and plain-language messages

**Files:**
- Create: `crates/puw-app/Cargo.toml`, `crates/puw-app/src/lib.rs`, `src/paths.rs`, `src/settings.rs`, `src/messages.rs`, `src/estimate.rs`
- Modify: `Cargo.toml` (workspace member)
- Test: unit tests in each module

**Interfaces:**
- Produces:
  - `pub struct DataDirs { pub root: PathBuf, pub models: PathBuf, pub watermarks: PathBuf, pub settings: PathBuf }` with `DataDirs::default_location() -> Result<DataDirs>` and `DataDirs::at(root: &Path) -> DataDirs` (tests).
  - `#[serde(rename_all = "camelCase")] pub struct Settings { pub output: OutputRule, pub detail: Detail, pub prefer_gpu: bool, pub load_seconds: BTreeMap<ModelKind, f32> }`; `pub enum OutputRule { NextToOriginals { folder_name: String }, Folder { path: PathBuf } }` (default `NextToOriginals { "Without watermark" }`); `pub enum Detail { Best, Balanced, Faster }` with `fn work_long_side(self) -> u32` = 4096 / 3072 / 2048; `prefer_gpu` default true.
  - `Settings::load(path) -> Settings` (missing or unreadable file -> defaults, never an error), `Settings::save(&self, path) -> Result<()>` (write to a temp file in the same folder, then rename).
  - `pub fn user_message(err: &AppError) -> String` and `pub enum AppError` (wraps `puw_core::Error`, download errors, `NoWritableOutput`, `NothingToRemove`, `NotReady`, `Busy`), `#[derive(Serialize)] pub struct UserError { pub message: String, pub details: String }` and `impl From<AppError> for UserError`.
  - `pub fn load_fraction(elapsed: f32, expected: f32) -> f32` (`min(0.95, elapsed / expected)`, 0 for `expected <= 0`) and `pub struct Eta` with `Eta::new(total: usize)`, `photo_done(&mut self, seconds: f32)`, `seconds_left(&self, current_elapsed: f32) -> Option<u32>` (None before the first photo finishes; otherwise mean x remaining - current_elapsed, floored at 0).

- [ ] **Step 1: Write failing tests:** `settings_defaults`, `settings_roundtrip`, `settings_garbage_gives_defaults`, `settings_save_is_atomic` (no `.tmp` left behind), `detail_sides`, `load_fraction_caps`, `eta_none_before_first`, `eta_mean_times_remaining`, `messages_are_plain` (every `AppError` variant, including each `puw_core::Error` variant, maps to a non-empty message with no word from the banned list in Global Constraints and no Rust type names), `invalid_input_is_not_image_error` (a library name error reads as a name problem).
- [ ] **Step 2: Run** `cargo test -p puw-app` - Expected: FAIL.
- [ ] **Step 3: Implement.** Messages, for example: `Cancelled` -> "Stopped."; `UnsupportedFormat("heic")` -> the HEIC sentence; `Io NotFound` -> "The photo could not be found. It may have been moved or deleted."; `Io PermissionDenied` -> "The app is not allowed to write to this folder."; `Model` -> "The watermark remover could not run on this photo."; `details` carries the raw error text for a "Show details" toggle.
- [ ] **Step 4: Run** `cargo test -p puw-app` and clippy - Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Add the app crate with settings and messages"`

### Task 5: Model download

**Files:**
- Create: `crates/puw-app/src/download.rs`
- Test: `crates/puw-app/tests/download.rs` (local HTTP server with `tiny_http` that supports `Range` and can cut a response short or corrupt a byte)

**Interfaces:**
- Produces:
  - `pub struct Asset { pub name: &'static str, pub url: String, pub sha256: &'static str, pub bytes: u64 }`; `pub fn model_assets() -> Vec<Asset>` (the three `models-v2` files from Global Constraints); `pub fn model_assets_at(base_url: &str) -> Vec<Asset>` (tests).
  - `pub fn missing(dir: &Path, assets: &[Asset]) -> Vec<Asset>`: an asset is present when `<name>` has the right size and `<name>.sha256` holds its expected hash (written only after a verified download).
  - `#[serde(rename_all = "camelCase")] pub struct DownloadProgress { pub done_bytes: u64, pub total_bytes: u64, pub file: String }`.
  - `pub fn download(dir: &Path, assets: &[Asset], on_progress: &(dyn Fn(DownloadProgress) + Sync), cancel: &AtomicBool) -> Result<(), DownloadError>`; `pub enum DownloadError { Network(String), Verify { name: String }, Disk(String), Cancelled, BadName(String) }` (mapped by `user_message`).
  - `pub fn remove_downloaded_data(dirs: &DataDirs) -> Result<()>` (deletes `models/` and `watermarks/`, keeps `settings.json`).

- [ ] **Step 1: Write failing tests:** `downloads_and_verifies`, `skips_present_files` (no request for a present file), `resumes_partial_download` (server drops the connection after half; second `download` sends `Range: bytes=<n>-` and the result verifies), `server_ignores_range_restarts` (200 instead of 206: file restarted, not appended), `corrupt_file_is_redownloaded` (wrong hash: `.part` deleted, `Verify` error, nothing at the final name; next call downloads again), `rejects_bad_names` (`../x`, `a/b`, `a\b`, empty: `BadName`, nothing written), `cancel_keeps_part` (cancel mid-file: `Cancelled`, `.part` kept for resume), `progress_is_monotonic`, `remove_downloaded_data_keeps_settings`.
- [ ] **Step 2: Run** `cargo test -p puw-app --test download` - Expected: FAIL.
- [ ] **Step 3: Implement** with `ureq` (follows GitHub's redirect), streaming into `<name>.part` while hashing with `sha2`; a resumed file is hashed from disk first. Up to 3 attempts per file with 1 s / 3 s back-off on network errors. Rename to the final name only after the hash matches, then write `<name>.sha256`. No user data in requests (default `ureq` User-Agent or `PhotoUnwatermark/<version>`).
- [ ] **Step 4: Run** tests - Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Download and verify the models with resume"`

### Task 6: Photo intake and images

**Files:**
- Create: `crates/puw-app/src/photos.rs`, `crates/puw-app/src/images.rs`
- Test: `crates/puw-app/tests/photos.rs` (fixtures generated in the test with `image`, EXIF orientation written with the same helper style as `puw-core/tests/image_io.rs`)

**Interfaces:**
- Produces:
  - `pub type PhotoId = u32;` `#[serde(rename_all = "camelCase")] pub struct PhotoInfo { pub id: PhotoId, pub name: String, pub folder: String, pub width: u32, pub height: u32 }` (upright size).
  - `pub struct PhotoList` with `add_paths(&mut self, paths: &[PathBuf], output_folder_name: &str) -> AddResult`, `remove(&mut self, id)`, `clear(&mut self)`, `get(id) -> Option<&PhotoEntry>`, `infos() -> Vec<PhotoInfo>`, `paths() -> Vec<PathBuf>`; `PhotoEntry { info: PhotoInfo, path: PathBuf }`.
  - `#[serde(rename_all = "camelCase")] pub struct AddResult { pub added: Vec<PhotoInfo>, pub skipped: Vec<Skipped> }`, `Skipped { pub name: String, pub reason: String }` (plain sentence).
  - `pub const EDIT_LONG_SIDE: u32 = 4096; pub const THUMB_LONG_SIDE: u32 = 320; pub const GRID_LONG_SIDE: u32 = 480;`
  - `pub fn edit_size(w: u32, h: u32) -> (u32, u32)`; `pub fn thumbnail(path: &Path, long_side: u32) -> Result<RgbImage>` (embedded EXIF thumbnail when it is at least `long_side` on its long side, else full decode; always upright); `pub fn edit_image(photo: &LoadedPhoto) -> RgbImage`; `pub fn overlay(img: &RgbImage, area: &Mask) -> RgbImage` (red `#FF3B30` at 45 % over area pixels, area resized to `img` first); `pub fn area_to_edit(full: &Mask, ew: u32, eh: u32) -> Vec<u8>` and `pub fn area_from_edit(bytes: &[u8], ew: u32, eh: u32, w: u32, h: u32) -> Result<Mask>` (both conservative: any covered source pixel covers the target; result binarised; length check -> `InvalidInput`); `pub fn jpeg(img: &RgbImage, quality: u8) -> Vec<u8>`.

- [ ] **Step 1: Write failing tests:** `adds_files_and_folders` (a folder is read one level deep), `scan_skips_output_folder` (a dropped folder containing `Without watermark/` with images, and a dropped path inside it: none of those added), `dedupes_same_file` (same file via two spellings, `canonicalize`), `skips_unsupported` (`.txt`, `.heic` with the HEIC sentence, a truncated JPEG with "could not be read"), `edit_image_is_upright` (orientation 6 photo 600x400 stored: `PhotoInfo` 400x600, `edit_image` 400x600, a red block painted top-left in the upright source is top-left in the edit image), `edit_size_caps` (6000x4000 -> 4096x2731; 800x600 unchanged), `area_roundtrip_is_conservative` (a 1-pixel area at full size survives `area_to_edit` and back), `area_from_edit_rejects_wrong_length`, `thumbnail_is_upright`, `overlay_tints_only_area`.
- [ ] **Step 2: Run** `cargo test -p puw-app --test photos` - Expected: FAIL.
- [ ] **Step 3: Implement.** Supported extensions (case-insensitive): jpg, jpeg, png, tif, tiff, webp; anything else in a dropped folder is ignored silently, anything dropped as a file is listed in `skipped`. Upright sizes via `puw_core::image_io::upright_size`, read in parallel (rayon, bounded per Global Constraints). Use `mask::resize_any` + `mask::binarize` for the area conversions.
- [ ] **Step 4: Run** tests - Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Add photo intake, thumbnails and edit images"`

### Task 7: Session: detection and step-2 areas

**Files:**
- Create: `crates/puw-app/src/core.rs` (the `AppCore` facade), `crates/puw-app/src/session.rs`, `crates/puw-app/src/jobs.rs`
- Test: `crates/puw-app/tests/session.rs` (needs models via `PUW_MODELS_DIR`; skip with a message when unset, like `puw-core` tests; synthetic sets via the same `synth_dir()` approach as `puw-core/tests/common`)

**Interfaces:**
- Consumes: Tasks 1-6.
- Produces:
  - `pub struct AppCore` (all methods `&self`, internally synchronised, `Send + Sync`): `AppCore::new(dirs: DataDirs) -> Result<AppCore>`; `settings() -> Settings`; `set_settings(Settings) -> Result<()>` (a change of `prefer_gpu` drops the engine; models reload on next use); `engine() -> Result<Arc<Engine>>` (created on first call with `EngineOptions { prefer_gpu, intra_threads: Some(max(1, cores - 1)), .. }`, no model loaded); `cancel()` (sets the current job's flag); `add_paths`, `remove_photo`, `clear_photos`, `photos()`.
  - `#[serde(tag = "phase", rename_all = "camelCase")] pub enum ProgressEvent { LoadingModels { fraction: f32, which: LoadPurpose }, Reading { fraction: f32 }, Looking { done: u32, total: u32 }, Matching { done: u32, total: u32 }, Removing { done: u32, total: u32, photo: PhotoId, photo_fraction: f32, seconds_left: Option<u32> }, Preview { fraction: f32 }, FellBackToProcessor { reason: String } }`; `pub enum LoadPurpose { Finder, Remover }` (UI sentences "Getting the watermark finder ready..." / "Getting the watermark remover ready...").
  - `pub type Events<'a> = &'a (dyn Fn(ProgressEvent) + Sync);`
  - `AppCore::ensure_models(kinds: &[ModelKind], purpose: LoadPurpose, ev: Events) -> Result<()>`: loads missing kinds in order with a ticker thread emitting `LoadingModels` every 200 ms using `load_fraction` (expected seconds from `settings.load_seconds`, else model bytes / 20 MB per s), then 1.0; stores the measured seconds in settings.
  - `AppCore::suggest_saved() -> Result<Option<Suggestion>>`, `Suggestion { id: String, name: String }` (`Library::suggest` on up to 5 photos, no models).
  - `AppCore::detect(ev) -> Result<DetectOutcome>`: `ensure_models([Segmenter, Face], Finder)`, `build_set_layers` (`Reading`), `guess_photo` on `representative_sample(n)` (`Looking`), `pick_representative`. `DetectOutcome { sample: Vec<PhotoId>, representative: PhotoId }`. Layers and guesses are kept in the session.
  - `AppCore::use_saved(id: &str, ev) -> Result<DetectOutcome>`: no model load; `find` with the saved template on the same sample; representative = best `Found`, else the first.
  - `AppCore::edit_size(id) -> Result<(u32, u32)>`; `AppCore::area(id) -> Result<Vec<u8>>` (edit-size bytes of the photo's current area: own area, else match result, else step-2 guess, else empty); `AppCore::put_draft(id, w, h, bytes) -> Result<()>`; `AppCore::confirm_set_area(id) -> Result<()>` (draft -> full size -> `binarize` -> `build_template` on the full photo; clears match results of photos without an own area, then records the confirmed area as that photo's `Found` result; empty area -> `AppError::NothingToRemove`).
  - `AppCore::thumb_image(id) -> Result<Vec<u8>>` (JPEG, `THUMB_LONG_SIDE`, cached in memory, at most 512 entries) and `AppCore::edit_image(id) -> Result<Vec<u8>>` (JPEG quality 90 at edit size; the last 4 cached).
  - Only one job at a time: a second long call while one runs returns `AppError::Busy`. Long jobs lock session state only to read inputs and write results.

- [ ] **Step 1: Write failing tests:** `detect_without_loading_at_start` (`AppCore::new` + `add_paths` leave `engine().report()` all `None` or no engine at all), `detect_fixed_set` (synthetic `fixed` set: events include `LoadingModels{Finder}`, `Reading`, `Looking`; outcome sample len 3; representative's `area` covers >= 80 % of its ground truth at edit size), `detect_reports_load_progress_monotonic`, `second_job_is_busy`, `cancel_detect` (cancel during `Reading` returns the `Cancelled` message, session still usable), `use_saved_skips_models` (save a template built from ground truth into the library, `suggest_saved` returns it, `use_saved` returns an outcome and the engine has no model loaded), `confirm_rejects_empty`, `confirm_builds_template`, `area_bytes_have_edit_size`, `images_served_during_job` (while `detect` runs on another thread, `photos()` and `area()` of another photo return within 100 ms).
- [ ] **Step 2: Run** `cargo test -p puw-app --release --test session` - Expected: FAIL.
- [ ] **Step 3: Implement** `session.rs` (state: photos, layers, guesses, sample, drafts, template, per-photo results/own areas/flags), `jobs.rs` (job slot with cancel flag and `Busy`), `core.rs`.
- [ ] **Step 4: Run** tests - Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Add the session with detection and set area"`

### Task 8: Session: matching and the step-3 rule

**Files:**
- Modify: `crates/puw-app/src/session.rs`, `crates/puw-app/src/core.rs`
- Test: `crates/puw-app/tests/session.rs`

**Interfaces:**
- Produces:
  - `AppCore::match_all(ev) -> Result<MatchSummary>`: builds layers if missing (saved-watermark path), runs `match_photo` (Task 3) on every photo without a result (parallel, bounded), emitting `Matching`. `MatchSummary { found: u32, unsure: u32, not_found: u32, total: u32 }`. The confirmed photo itself is `Found` with its confirmed area.
  - `#[serde(rename_all = "camelCase")] pub struct PhotoState { pub id: PhotoId, pub status: AreaStatus, pub included: bool, pub reviewed: bool, pub has_area: bool }`, `pub enum AreaStatus { Found, Unsure, NotFound, Edited, Pending }`; `AppCore::photo_states() -> Vec<PhotoState>`.
  - Defaults after matching: `Found` included; `Unsure` included but not reviewed; `NotFound` not included.
  - `AppCore::open_photo(id)` (marks reviewed), `AppCore::save_photo_area(id) -> Result<()>` (draft becomes the photo's own area; status `Edited`, reviewed; included iff the area is non-empty; never changes the set template), `AppCore::set_included(id, bool) -> Result<()>` (including a photo with an empty area is refused with `NothingToRemove`).
  - `AppCore::removal_blockers() -> Vec<PhotoId>`: included photos that are `Unsure` and not reviewed, plus `Pending`. `AppCore::to_remove() -> Vec<PhotoId>`.
  - `AppCore::grid_image(id) -> Result<Vec<u8>>`: JPEG of the `GRID_LONG_SIDE` thumbnail with the photo's current area as overlay.

- [ ] **Step 1: Write failing tests:** `match_random_set` (synthetic `random` set confirmed on ground truth of photo 0: >= 95 % of visible-mark photos `Found`, 0 `Found` among photos of the `negative` set added to the same session), `unsure_blocks_until_opened_or_unticked`, `not_found_unticked_by_default`, `painting_not_found_ticks_it`, `photo_edit_does_not_change_set` (after `save_photo_area` on photo 3, re-running `match_all` for a newly added photo uses the set template), `reconfirm_clears_matches_keeps_edits`, `empty_area_cannot_be_included`, `grid_image_is_jpeg`.
- [ ] **Step 2: Run** `cargo test -p puw-app --release --test session` - Expected: FAIL (new tests).
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** tests - Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Match the watermark across the set"`

### Task 9: Session: preview, removal and saving the watermark

**Files:**
- Create: `crates/puw-app/src/output.rs`
- Modify: `crates/puw-app/src/session.rs`, `crates/puw-app/src/core.rs`
- Test: `crates/puw-app/tests/remove.rs`

**Interfaces:**
- Produces:
  - `pub const PREVIEW_LONG_SIDE: u32 = 1536;` `AppCore::preview(id, ev) -> Result<()>`: `ensure_models([Lama], Remover)`, inpaint the photo's draft at `PREVIEW_LONG_SIDE` (`Preview` events), keep the result at edit size; `AppCore::preview_image(id) -> Result<Vec<u8>>` (JPEG).
  - `pub fn output_dirs(photos: &[PathBuf], rule: &OutputRule) -> Result<BTreeMap<PathBuf, PathBuf>, Vec<PathBuf>>` in `output.rs`: source folder -> output folder; `Err` lists source folders where the output folder cannot be created or written (probe file created and deleted).
  - `AppCore::check_output() -> Result<OutputCheck>`, `OutputCheck { ok: bool, unwritable: Vec<String> }`; `AppCore::set_output_override(Option<PathBuf>)` (used for this run when the user picks a folder).
  - `AppCore::remove(ev) -> Result<RemoveReport>`: refuses with `AppError::NotReady` if `removal_blockers()` is non-empty or output is not writable; `ensure_models([Lama], Remover)`; one photo at a time: `load` -> area (own, else match) -> `remove_one` with `settings.detail.work_long_side()`; emits `Removing` with `Eta`; on cancel the current photo is discarded and completed ones kept; a per-photo error is recorded and the run continues. Before starting, deletes `.*.partial` files left in the output folders by other process ids (Plan 1 M6). If the engine report changes a model to `Cpu { RunFailed }` during the run, emits `FellBackToProcessor` once.
  - `#[serde(rename_all = "camelCase")] pub struct RemoveReport { pub done: Vec<Done>, pub failed: Vec<Failed>, pub cancelled: bool }`, `Done { id: PhotoId, output: String }`, `Failed { id: PhotoId, name: String, reason: String }` (plain sentence).
  - `AppCore::result_image(id) -> Result<Vec<u8>>` and `AppCore::original_image(id)` (edit-size JPEGs for the before/after slider).
  - `AppCore::save_watermark(name: &str) -> Result<String>` (saved id; thumbnail = the confirmed photo cropped to the set area's bounding box plus 10 % margin, overlay applied, long side <= 256, PNG); `default_watermark_name(today) -> String` = "Watermark from YYYY-MM-DD"; `list_saved`, `rename_saved`, `delete_saved`, `saved_thumbnail(id) -> Vec<u8>`.

- [ ] **Step 1: Write failing tests:** `removes_into_output_folder` (2 synthetic photos: outputs in `Without watermark`, originals byte-identical before/after), `area_lands_on_rotated_photo` (orientation-6 JPEG, area painted at upright top-left at edit size: the saved result differs from the original only near top-left of the upright image), `unwritable_source_asks_for_folder` (read-only source folder on Unix via permissions, skipped on Windows: `check_output().ok == false` and `remove` refuses before touching any photo; with `set_output_override` it succeeds), `vanished_photo_is_a_failure` (delete one source file after matching: it is in `failed` with the "could not be found" sentence, the others are done), `cancel_discards_current_keeps_completed`, `blockers_refuse_removal`, `stale_partials_removed`, `preview_loads_remover_once` (two previews emit `LoadingModels{Remover}` only the first time), `save_watermark_roundtrip` (save, list shows it with the default name, rename, delete), `default_name_format`.
- [ ] **Step 2: Run** `cargo test -p puw-app --release --test remove` - Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test --workspace --release` - Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Add preview, removal and saved watermarks to the session"`

### Task 10: Tauri shell, image protocol and frontend scaffold

**Files:**
- Create: `app/package.json`, `app/pnpm-lock.yaml`, `app/vite.config.ts`, `app/tsconfig.json`, `app/svelte.config.js`, `app/index.html`, `app/src/main.ts`, `app/src/App.svelte`, `app/src/app.css` (tokens, light + dark via `prefers-color-scheme`), `app/src/lib/api.ts`, `app/src/lib/types.ts`, `app/src/lib/copy.ts`, `app/src/lib/wizard.svelte.ts`, `app/src/lib/components/ProgressLine.svelte`, `app/src/lib/components/Waiting.svelte`, `app/src/test/setup.ts`, `app/src/test/copy.test.ts`, `app/src/test/wizard.test.ts`, `app/src-tauri/Cargo.toml`, `app/src-tauri/build.rs`, `app/src-tauri/tauri.conf.json`, `app/src-tauri/capabilities/default.json`, `app/src-tauri/src/main.rs`, `app/src-tauri/src/commands.rs`, `app/src-tauri/src/protocol.rs`, `app/src-tauri/icons/*` (generated from a simple generic SVG with `pnpm tauri icon`; final icon is Plan 3)
- Modify: `Cargo.toml` (member `app/src-tauri`), `.github/workflows/ci.yml`, `.gitignore` (`app/node_modules`, `app/dist`)

**Interfaces:**
- Consumes: `AppCore` (Tasks 7-9), `download` (Task 5).
- Produces:
  - Tauri commands (snake_case; one `AppCore` in managed state; every long command runs in `tauri::async_runtime::spawn_blocking` and takes `on_event: Channel<ProgressEvent>` or `Channel<DownloadProgress>`; errors are `UserError`): `app_status -> { modelsReady, missingBytes, platform, version }`, `download_models(onEvent)`, `cancel_job`, `add_paths(paths)`, `remove_photo(id)`, `clear_photos`, `list_photos`, `suggest_saved`, `detect(onEvent)`, `use_saved(id, onEvent)`, `edit_size(id)`, `get_area(id)` (raw `tauri::ipc::Response` bytes), `put_draft` (raw body `Uint8Array`, headers `x-photo-id`, `x-width`, `x-height`), `confirm_set_area(id)`, `match_all(onEvent)`, `photo_states`, `open_photo(id)`, `save_photo_area(id)`, `set_included(id, included)`, `removal_blockers`, `preview(id, onEvent)`, `check_output`, `set_output_override(path)`, `remove(onEvent)`, `save_watermark(name)`, `default_watermark_name`, `list_saved`, `rename_saved(id, name)`, `delete_saved(id)`, `get_settings`, `set_settings(settings)`, `remove_downloaded_data`, `device_status`, `device_fix(fix)` (Task 13 fills the last two).
  - Image protocol `puw` (`register_asynchronous_uri_scheme_protocol`, work off the main thread): `thumb/<id>`, `grid/<id>`, `edit/<id>`, `preview/<id>`, `original/<id>`, `result/<id>`, `saved/<id>`; any query string is ignored (cache busting); unknown route or id -> 404. Frontend builds URLs only with `convertFileSrc(route, 'puw')`.
  - `app/src/lib/api.ts`: one typed async function per command (camelCase names), the only module that imports `@tauri-apps/api`; `types.ts` mirrors the Rust DTOs.
  - `app/src/lib/copy.ts`: every user-visible string (functions for strings with numbers).
  - `wizard.svelte.ts`: `type Screen = 'welcome' | 'add' | 'check' | 'checkAll' | 'remove' | 'done' | 'saved' | 'settings'`; `wizard.go(screen)`, `wizard.back()`, `wizard.step` (1-5 or null).
  - Window: title "PhotoUnwatermark", 1200x800, min 960x640. CSP: `default-src 'self'; img-src 'self' puw: http://puw.localhost data: blob:; style-src 'self' 'unsafe-inline'; connect-src ipc: http://ipc.localhost`. Capabilities: core default, `dialog:allow-open`, `opener:allow-reveal-item-in-dir`, `opener:allow-open-url` limited to `https://github.com/007hacky007/PhotoUnwatermark/*`. Drag and drop on.
  - CI: before the cargo steps, set up Node (LTS) + pnpm, `pnpm -C app install --frozen-lockfile`, `pnpm -C app check`, `pnpm -C app test`, `pnpm -C app build` (the Tauri crate needs `app/dist`).

- [ ] **Step 1: Write failing tests:** `copy.test.ts` `noBannedWords` (every string and every function's output for sample numbers: no banned word from Global Constraints, no Unicode dash) and `exactSpecCopy` (the strings listed in Global Constraints match exactly); `wizard.test.ts` `backKeepsState`, `progressLineMarksDoneSteps`; Rust unit tests in `protocol.rs`: `route_parsing` (`thumb/7?v=3` -> Thumb(7); `edit/x` -> 404; `../etc` -> 404).
- [ ] **Step 2: Run** `pnpm -C app test` and `cargo test -p photounwatermark` - Expected: FAIL.
- [ ] **Step 3: Implement** the scaffold, commands, protocol, `App.svelte` routing by `wizard.screen`, `ProgressLine`, and `Waiting` (bar + sentence + optional Cancel, props `{ sentence: string; fraction: number | null; detail?: string; onCancel?: () => void }`; `fraction: null` shows an indeterminate bar).
- [ ] **Step 4: Run** `pnpm -C app check && pnpm -C app test && pnpm -C app build && cargo clippy --workspace --all-targets -- -D warnings && pnpm -C app tauri dev` - Expected: checks pass; the window opens showing an empty welcome screen without console errors.
- [ ] **Step 5: Push to CI only with the owner's OK; otherwise run the same commands locally.** Commit `git commit -m "Add the Tauri shell and the UI scaffold"`

### Task 11: First launch and the download screen

**Files:**
- Create: `app/src/screens/Welcome.svelte`, `app/src/test/welcome.test.ts`
- Modify: `app/src/App.svelte`, `app/src/lib/copy.ts`, `docs/superpowers/specs/2026-11-23-photounwatermark-design.md` ("~450 MB target" -> "about 690 MB")

**Interfaces:**
- Consumes: `appStatus`, `downloadModels`, `cancelJob` from `api.ts`.
- Produces: start-up routing: `modelsReady` false -> `welcome`, true -> `add`. Welcome text (copy.ts): one sentence on what the app does, "The app needs to download its watermark remover once (about 690 MB). Your photos never leave this computer." Button "Download". During download: bar, "X MB of 690 MB", Cancel ("Pause"); on error: the `UserError.message`, "Try again" (resumes). On success -> `add`.

- [ ] **Step 1: Write failing tests** (`welcome.test.ts`, `api.ts` mocked with `vi.mock`): `showsSizeAndPrivacy`, `progressShowsMegabytes`, `errorOffersTryAgain` (a rejected `downloadModels` shows the message and "Try again" calls it again), `successGoesToAddPhotos`, `noBannedWordsRendered` (render each state; `document.body.textContent` plus all `title`/`aria-label` attributes pass the banned-word check from `copy.test.ts`, exported as a helper `assertPlainText(el)`).
- [ ] **Step 2: Run** `pnpm -C app test welcome` - Expected: FAIL.
- [ ] **Step 3: Implement**; fix the spec size text.
- [ ] **Step 4: Run** `pnpm -C app test`; then with `models/` moved away from the data folder, `pnpm -C app tauri dev` downloads, and a second start goes straight to Add photos - Expected: PASS / observed.
- [ ] **Step 5: Commit** `git commit -m "Add the first-launch download screen"`

### Task 12: Step 1 Add photos and the detection wait

**Files:**
- Create: `app/src/screens/AddPhotos.svelte`, `app/src/screens/Finding.svelte`, `app/src/test/addPhotos.test.ts`
- Modify: `app/src/lib/copy.ts`, `app/src/App.svelte`

**Interfaces:**
- Consumes: `addPaths`, `removePhoto`, `listPhotos`, `suggestSaved`, `detect`, `useSaved`, `cancelJob`; Tauri `getCurrentWebview().onDragDropEvent`; dialog `open({ multiple: true, directory: false, filters: [{ name: 'Photos', extensions: [...] }] })` and a second button for a folder.
- Produces: drop zone ("Drop photos or a folder here"), "Choose photos..." and "Choose a folder..."; count + thumbnail grid (`thumb/<id>`, `loading="lazy"`), remove one photo (x on hover); skipped notice listing names and reasons; after each add, `suggestSaved` runs and, if it returns, a banner "These look like <name>. Use it?" with "Yes" / "No". Primary button "Find the watermark →" (disabled with 0 photos) -> `Finding` screen running `detect`; "Yes" -> `Finding` running `useSaved`. `Finding` shows `Waiting` with: `LoadingModels{Finder}` -> "Getting the watermark finder ready..." with its fraction; `Reading` -> "Reading your photos..."; `Looking` -> "Looking for the watermark... photo X of Y". Back cancels the job and returns to Add photos. On success -> `check` with the `DetectOutcome`.

- [ ] **Step 1: Write failing tests:** `primaryDisabledWithoutPhotos`, `showsSkippedReasons`, `offerSavedWatermark` (`suggestSaved` resolves `{ id, name: 'Studio' }` -> text "These look like Studio. Use it?"), `yesUsesSaved`, `findingSentencesFollowEvents` (feed the channel callback each `ProgressEvent` kind and assert the sentence), `backCancels`, `noBannedWordsRendered`.
- [ ] **Step 2: Run** `pnpm -C app test addPhotos` - Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** tests; in `tauri dev`, drop the synthetic `fixed` set folder: thumbnails show, Find the watermark shows the load bar and then the photo count - Expected: PASS / observed.
- [ ] **Step 5: Commit** `git commit -m "Add the Add photos step and the detection wait"`

### Task 13: Device chip, Settings, Saved watermarks, About

**Files:**
- Create: `crates/puw-app/src/device.rs`, `app/src/lib/components/DeviceChip.svelte`, `app/src/lib/components/WhyDialog.svelte`, `app/src/screens/Settings.svelte`, `app/src/screens/SavedWatermarks.svelte`, `app/src/screens/About.svelte`, `app/src/test/device.test.ts`, `app/src/test/settings.test.ts`
- Modify: `crates/puw-app/src/core.rs`, `app/src-tauri/src/commands.rs`, `app/src/lib/copy.ts`, spec (replace the sentence "On macOS the chip reflects whether the CoreML path is actually used..." with: on macOS every model runs on the processor, see `docs/notes/coreml.md`)

**Interfaces:**
- Produces:
  - `#[serde(rename_all = "camelCase")] pub struct DeviceStatus { pub state: DeviceState, pub reason: Option<String>, pub details: Option<String>, pub fixes: Vec<DeviceFix> }`, `pub enum DeviceState { Gpu, Cpu, NotStarted }`, `pub enum DeviceFix { TryAgain, UseProcessor, UseGraphicsCard, DownloadHelper { bytes: u64 } }`; `pub enum Platform { MacOs, Windows, Other }` with `Platform::current()`; `pub fn device_status(report: Option<&DeviceReport>, prefer_gpu: bool, platform: Platform) -> DeviceStatus` (pure): `Gpu` only when every loaded model is GPU (Core ML on macOS, DirectML on Windows); `Cpu` reason sentences per `CpuReason`: NotRequested "You chose the processor in Settings." [UseGraphicsCard]; ProviderUnavailable "This computer's graphics card could not be used." [TryAgain]; LoadFailed "The graphics card could not load the watermark remover." [TryAgain, UseProcessor]; PartialGpu "Your graphics card can run only part of the watermark remover, so the processor runs all of it." [UseProcessor]; Gpu with a `share` -> chip "Using graphics card (fast)", why text "Most of the work runs on the graphics hardware; a small part runs on the processor."; RunFailed "The graphics card stopped working, so the app switched to the processor." [TryAgain]. `details` = the raw reason text. `DownloadHelper` is never produced in Plan 2 (Plan 3 adds the helper asset).
  - `AppCore::device_status()`, `AppCore::device_fix(fix) -> Result<()>` (TryAgain: drop the engine; UseProcessor: `prefer_gpu = false` + drop; UseGraphicsCard: `prefer_gpu = true` + drop).
  - `DeviceChip` (shown on step 2 preview, step 4, Settings): "Using graphics card (fast)" or "Using processor (slower) - why?"; `NotStarted` shows nothing on steps, "Not started yet" in Settings. "why?" opens `WhyDialog` with reason, "Show details", fix buttons.
  - Settings screen: output rule (next to originals with folder name field, or "Always save to..." folder picker), "Detail vs speed" (Best detail / Balanced / Faster; one plain sentence each with the measured CPU time per 24 MP photo from `docs/notes/thresholds.md`), processor/graphics card choice (Windows only; on Macs Core ML is always used unless it fails), Advanced (collapsed): "Remove downloaded data" with an in-app confirmation (lists what goes; after it the app shows Welcome), About.
  - Saved watermarks screen (from Add photos and Settings): list with thumbnail (`saved/<id>`), name, date, rename inline, delete with confirmation.
  - About: version, licences (segmenter Apache-2.0 christophernavas/watermark-remover; big-LaMa Apache-2.0 advimman/lama with the weights-licence note; YuNet MIT, OpenCV Zoo), app MIT, responsible-use note "Intended for photos you own or have permission to edit.", link "Releases page" (opener, the allowed URL).

- [ ] **Step 1: Write failing tests:** Rust `device_status_cases` (one case per `CpuReason`, all-GPU, mixed GPU/CPU -> `Cpu`, no report -> `NotStarted`, mac reason has no fixes); TS `chipTexts`, `whyShowsFixes`, `removeDataNeedsConfirmation`, `renameSaved`, `deleteSavedNeedsConfirmation`, `noBannedWordsRendered` for Settings, Saved watermarks, About, WhyDialog.
- [ ] **Step 2: Run** `cargo test -p puw-app device` and `pnpm -C app test device settings` - Expected: FAIL.
- [ ] **Step 3: Implement**; update the spec sentence.
- [ ] **Step 4: Run** all tests - Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Add the device chip, settings and saved watermarks"`

### Task 14: Editor logic (pure TypeScript)

**Files:**
- Create: `app/src/lib/editor/paint.ts`, `history.ts`, `view.ts`, `loupe.ts`, `keys.ts`, and tests `app/src/lib/editor/*.test.ts`

**Interfaces:**
- Produces:
  - `paint.ts`: `type Area = { w: number; h: number; data: Uint8Array }` (0 or 255); `stampLine(a: Area, x0, y0, x1, y1, radius: number, value: 0 | 255): Rect` (filled circles along the segment at spacing <= radius / 2; returns the dirty rect, clipped); `fillRect(a, r: Rect, value): Rect`; `renderOverlay(a: Area, out: ImageData, r: Rect)` (RGBA 255,59,48,115 where 255, transparent elsewhere); `isEmpty(a)`.
  - `history.ts`: `class History` with `begin(a: Area)`, `commit(a: Area, dirty: Rect)` (stores the dirty patch before/after), `undo(a): Rect | null`, `redo(a): Rect | null`, `canUndo`, `canRedo`; capped at 100 steps or 256 MB of patches, oldest dropped.
  - `view.ts`: `type View = { scale: number; x: number; y: number }`; `fit(imgW, imgH, boxW, boxH): View`; `zoomAt(v: View, factor: number, px: number, py: number): View` (point under the cursor stays put; scale clamped to [0.05, 16]); `toImage(v, px, py): [number, number]`; `toScreen(v, ix, iy)`; `ZOOM_STEPS = [0.05, 0.1, 0.25, 0.5, 0.75, 1, 1.5, 2, 3, 4, 6, 8, 12, 16]` for -/+.
  - `loupe.ts`: `LOUPE_SIZE = 220` (css px, round), `LOUPE_STEPS = [2, 4, 8]`, default 4 (relative to the current view scale); `loupeSource(ix, iy, viewScale, mag): { sx, sy, sw, sh }` (image rect centred on the cursor); `loupePlacement(cx, cy, vw, vh): { x, y }` (up-right of the cursor by 24 px, flipped to stay fully inside the viewport).
  - `keys.ts`: `actionFor(e: { key: string; metaKey: boolean; ctrlKey: boolean; shiftKey: boolean }, isMac: boolean): Action | null` for B, E, R, `[`, `]`, M (magnifier on/off), Cmd/Ctrl+Z, Shift+Cmd/Ctrl+Z, Ctrl+Y (Windows redo), `0` (Fit), `1` (100 %), `+`/`-`.

- [ ] **Step 1: Write failing tests:** `stampLineCoversSegmentWithoutGaps` (a fast diagonal stroke of radius 3 leaves no gap), `stampClipsAtEdges`, `eraserClears`, `fillRectNormalisesCorners` (drag up-left works), `overlayOnlyInDirtyRect`, `undoRedoRestoresBytes`, `historyCapDropsOldest`, `newStrokeClearsRedo`, `fitCentres`, `zoomKeepsPointUnderCursor`, `zoomClamped`, `loupeSourceScalesWithMag`, `loupeStaysInViewport` (cursor in each corner), `keysMacAndWindows` (Cmd on mac, Ctrl elsewhere; plain `z` does nothing).
- [ ] **Step 2: Run** `pnpm -C app test editor` - Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** tests - Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Add the editor painting, history, view and loupe logic"`

### Task 15: Step 2 editor screen with magnifier and preview

**Files:**
- Create: `app/src/lib/editor/Editor.svelte` (canvas stack + input), `app/src/lib/editor/Toolbar.svelte`, `app/src/lib/editor/Loupe.svelte`, `app/src/lib/components/BeforeAfter.svelte`, `app/src/screens/CheckWatermark.svelte`, `app/src/test/editor.test.ts`
- Modify: `app/src/lib/copy.ts`, `app/src/App.svelte`

**Interfaces:**
- Consumes: Task 14 modules; `editSize`, `getArea`, `putDraft`, `preview`, `confirmSetArea`, `openPhoto`, `savePhotoArea` from `api.ts`; `DeviceChip`, `Waiting`.
- Produces:
  - `Editor.svelte` props `{ photoId: number; onChange?: () => void }` and methods `getArea(): Area`, `setArea(a: Area)`. Layers: photo `<img>` from `edit/<id>` drawn into a canvas, overlay canvas, cursor/brush-outline canvas; pointer events (pen/touch/mouse); wheel zoom (pinch = ctrlKey wheel on macOS trackpads), Space-drag or middle-button pan, hold Space without dragging hides the red layer; Rectangle draws on release with a live outline; Brush/Eraser outline follows the cursor.
  - Magnifier: toolbar toggle "Magnifier" (shortcut M) with 2x / 4x / 8x; the loupe shows photo, red layer and brush outline around the cursor, hidden while the pointer is outside the photo or while panning.
  - `CheckWatermark.svelte` (step 2): help line (exact copy), toolbar (Brush, Eraser, Rectangle, Size slider 2-200 px in image pixels, Undo, Redo, - / 100% / + / Fit, Magnifier, "Hold Space to hide red"), photo strip of the `DetectOutcome.sample` (switching keeps each photo's edits in memory), "Preview on this photo" -> `putDraft` + `preview` with `Waiting` (`LoadingModels{Remover}` -> "Getting the watermark remover ready...", `Preview` -> "Trying it on this photo...") then `BeforeAfter` (drag handle; "Back to editing"), `DeviceChip` next to the preview; primary "Looks right →" -> `putDraft` + `confirmSetArea` -> `checkAll` (empty area: inline message "Paint the watermark area first."). Back -> Add photos (session kept).
  - Single-photo mode for step 3: `CheckWatermark` with prop `mode: 'photo'` -> help line "Fix the watermark area on this photo only.", no strip, primary "Save this photo" -> `putDraft` + `savePhotoArea`, Back = discard; `openPhoto(id)` called on open.

- [ ] **Step 1: Write failing tests** (jsdom; canvas via a minimal 2D-context stub in `setup.ts`; Task 14 logic is already tested, so these test wiring): `toolShortcutsSwitchTool`, `magnifierToggleWithM`, `looksRightSendsAreaThenConfirms` (mock order: `putDraft` with the area bytes, then `confirmSetArea`), `emptyAreaShowsHint`, `previewShowsRemoverLoadingSentence`, `photoModeSavesOnlyThisPhoto`, `noBannedWordsRendered` (both modes, including tool titles and aria-labels).
- [ ] **Step 2: Run** `pnpm -C app test editor` - Expected: FAIL.
- [ ] **Step 3: Implement.** Overlay updates repaint only the dirty rect; strokes are committed to history on pointer up.
- [ ] **Step 4: Run** tests; in `tauri dev` with a synthetic set: paint, erase, rectangle, undo/redo, zoom/pan, loupe at 2/4/8x near all four edges, Space hides red, preview shows before/after - Expected: PASS / observed.
- [ ] **Step 5: Commit** `git commit -m "Add the watermark editor with magnifier and preview"`

### Task 16: Step 3 Check all photos

**Files:**
- Create: `app/src/screens/CheckAll.svelte`, `app/src/test/checkAll.test.ts`
- Modify: `app/src/lib/copy.ts`, `app/src/App.svelte`

**Interfaces:**
- Consumes: `matchAll`, `photoStates`, `setIncluded`, `removalBlockers`, the single-photo `CheckWatermark` mode.
- Produces: on entry, `matchAll` with `Waiting` ("Finding the watermark in all photos... X of Y") unless every photo already has a result; then a grid of `grid/<id>?v=<n>` tiles with checkbox, badge (Found: none; Unsure: yellow "?"; Not found: grey "Not found"; Edited: small "Checked"), summary line (exact copy; "Z need a look" counts blockers, sentence omitted when Z = 0), click opens single-photo mode and returns here with the tile refreshed; primary "Remove watermark from N photos" (N = included), disabled while `removalBlockers` is non-empty with the hint "Open or untick the photos marked ?". Back -> step 2 with the set area kept.

- [ ] **Step 1: Write failing tests:** `summaryLine` ("Found in 118 of 120 photos. 2 need a look."), `buttonCountsIncluded`, `disabledUntilUnsureHandled` (unticking the last unsure photo enables the button), `notFoundStartsUnticked`, `openingPhotoMarksReviewed`, `noBannedWordsRendered`.
- [ ] **Step 2: Run** `pnpm -C app test checkAll` - Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** tests; in `tauri dev` with the synthetic `random` + `negative` sets combined - Expected: PASS / observed.
- [ ] **Step 5: Commit** `git commit -m "Add the Check all photos step"`

### Task 17: Steps 4 and 5: Remove and Done

**Files:**
- Create: `app/src/screens/Remove.svelte`, `app/src/screens/Done.svelte`, `app/src/test/removeDone.test.ts`
- Modify: `app/src/lib/copy.ts`, `app/src/App.svelte`

**Interfaces:**
- Consumes: `checkOutput`, `setOutputOverride`, `remove`, `cancelJob`, `saveWatermark`, `defaultWatermarkName`, `clearPhotos`; dialog folder picker; opener `revealItemInDir`; `getCurrentWindow().onCloseRequested`.
- Produces:
  - Remove: first `checkOutput`; if not ok, an in-app panel "The app cannot save next to these photos: <folders>. Choose a folder for the results." with "Choose a folder..." (-> `setOutputOverride`) and Back. Then the run: overall bar, "Photo X of Y", "About M minutes left" / "Working out the time left..." from `secondsLeft`, current photo thumbnail turning from `thumb/<id>` to `result/<id>` when done, `DeviceChip`, Cancel ("Stop": an in-app confirmation "Stop after discarding the current photo? Finished photos are kept."); `LoadingModels{Remover}` shows the remover sentence; `FellBackToProcessor` shows a persistent notice "The graphics card stopped working, so the app continues on the processor." While running, closing the window asks in-app "Removal is still running. Stop and quit?".
  - Done: before/after slider over `original/<id>` and `result/<id>` with arrow buttons and Left/Right keys, "N photos saved in 'Without watermark'" (or the chosen folder), "Open folder" (reveal the first result), "Save this watermark for next time" (name field prefilled with `defaultWatermarkName`; after saving shows "Saved"), "Start a new set" (`clearPhotos`, models stay loaded, -> Add photos). Failures listed with name and reason; cancelled runs say "Stopped. N photos were finished."

- [ ] **Step 1: Write failing tests:** `asksForFolderWhenUnwritable`, `etaTexts` (null -> "Working out the time left...", 90 -> "About 2 minutes left", 30 -> "Less than a minute left"), `stopNeedsConfirmation`, `fallbackNoticeShown`, `doneListsFailures`, `saveWatermarkUsesDefaultName`, `newSetClearsAndGoesToAdd`, `noBannedWordsRendered`.
- [ ] **Step 2: Run** `pnpm -C app test removeDone` - Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** tests; in `tauri dev`, full run on the synthetic `fixed` set, cancel one run midway, check `Without watermark` contents - Expected: PASS / observed.
- [ ] **Step 5: Commit** `git commit -m "Add the Remove and Done steps"`

### Task 18: End-to-end check and docs

**Files:**
- Modify: `README.md` (run from source: prerequisites, `scripts/fetch-models.sh` optional, `pnpm -C app install`, `pnpm -C app tauri dev`; where data is stored; responsible-use note), `docs/notes/thresholds.md` (app timings), `.github/workflows/ci.yml` (Windows job also runs `pnpm -C app tauri build --no-bundle` and uploads the exe as an artifact on `workflow_dispatch`)
- Create: `docs/notes/plan-2-check.md` (checklist results, aggregates only)

- [ ] **Step 1: macOS walk-through** with `pnpm -C app tauri build --no-bundle` (release binary) and a fresh data folder: first-launch download from the real release, synthetic `fixed`, `random`, `styles` sets, then local real set-a and set-b (results written under `local/` only). Record per step: wait times (model load, detection, matching, preview, per-photo removal at Best and Faster), whether any frozen window occurred, every UI text that read as jargon. Pass criteria: no frozen window over 200 ms during interaction (watch with the browser devtools performance panel), preview of a 24 MP photo reported honestly (spec target ~15 s; record the measured value), all Review Focus behaviours observed.
- [ ] **Step 2: Windows smoke** in the winbench VM with the CI artifact exe (models copied into `%LOCALAPPDATA%\PhotoUnwatermark\models` to skip the download, or download once): app opens, synthetic `fixed` set goes through all 5 steps, the chip shows the real reason. Record results.
- [ ] **Step 3: Fix** anything found (each fix its own commit with a test where the code is testable).
- [ ] **Step 4: Run** `pnpm -C app check && pnpm -C app test && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace --release` - Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Document running the app and the Plan 2 check"`

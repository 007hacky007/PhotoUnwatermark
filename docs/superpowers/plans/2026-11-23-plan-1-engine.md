# PhotoUnwatermark Plan 1: Engine and Model Tooling

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A tested Rust library (`puw-core`) that loads photos, guesses the watermark area, finds a confirmed watermark in other photos, removes it with LaMa, and saves results safely; plus the Python tooling that produces and verifies the ONNX models and the synthetic test sets.

**Architecture:** Cargo workspace with `puw-core` (library used by the app in Plan 2) and `puw-dev` (developer-only CLI for benchmarks, never shipped). Python tooling under `tools/` (uv project) converts the models once, checks parity, and generates synthetic test data. Models are published as assets of the GitHub release `models-v1` and consumed by tests through `PUW_MODELS_DIR`.

**Tech Stack:** Rust edition 2024; `ort = "=2.0.0-rc.15"` (ONNX Runtime 1.31; features `coreml`, `directml`), `image = "0.25"`, `imageproc = "0.27"`, `rustfft = "6"`, `rayon = "1"`, `serde`/`serde_json`, `sha2 = "0.11"`, `thiserror = "2"`, `kamadak-exif = "0.6"`, `little_exif = "0.6"`, `img-parts = "0.4"`. Python 3.12 via uv: torch, segmentation-models-pytorch, onnx, onnxruntime, onnxconverter-common, huggingface_hub, numpy, pillow, scikit-image, pytest.

**Spec:** `docs/superpowers/specs/2026-11-23-photounwatermark-design.md` (read it first). Plan 2 (desktop app) and Plan 3 (packaging and release) follow and consume this plan's public API.

## Global Constraints

- Commits as `007hacky007 <007hacky007@users.noreply.github.com>`, `commit.gpgsign false`; short imperative subject, no body, no AI attribution.
- No real names, emails, local absolute paths, real photos or anything derived from them in the repo. Real-photo validation reads `PUW_REAL_DIR` only and writes outside the repo (`/local/` is gitignored).
- Test data in the repo: synthetic only (scikit-image sample photos + generated marks).
- Model conventions: segmenters take ImageNet-normalised float RGB (mean 0.485/0.456/0.406, std 0.229/0.224/0.225), 512x512, sigmoid output. LaMa: image float 0-1, mask float 1 = hole, output 0-255, fixed 512x512.
- Detection defaults: long sides 768/1280/2048, tile 512 stride 384, probability > 0.5, at least 2 of 3 scales agree, grow ~0.4 % of the long side (minimum 7 px).
- Batch layer: per-image 2-D Sobel, per-pixel median over images, hysteresis low 40 / high 80 after removing regions that survive a 7x7 opening (dilated by 2), dilate 7; discard if > 10 % of the image; needs >= 3 same-size photos.
- Inpainting: LaMa 512 tiles, each fills holes only in its central 384 px (image borders excepted) with all remaining holes in view; working long side default 4096, filled pixels pasted back with a 1.5 px soft edge.
- Matching statuses: found / unsure / not found; unsure and not found are never processed without the user (enforced in Plan 2, exposed here as status).
- Output: never overwrite or modify originals; same name and format; JPEG quality 95; EXIF and ICC preserved, orientation normalised to 1; " (2)" suffix on clash.
- `puw-core` makes no network requests (model download belongs to Plan 2). No telemetry.
- App code MIT; model licences recorded in `models/manifest.json` and kept in the release.

## Review Focus

1. Camera photos with EXIF orientation 6 or 8 (portrait): the mask the user paints on the displayed image must land on the same pixels in the saved result, and the result must open upright everywhere. Test: Task 6 `orientation_roundtrip`, Task 14 `remove_one_respects_orientation`.
2. Small images (a side under 512 px, e.g. a 400x300 web copy) and very large ones (50 MP): tiling must pad, not panic, and memory must stay bounded. Test: Task 8 `predict_prob_small_image`, Task 11 `inpaint_small_image`.
3. A folder mixing sizes or orientations: the batch layer must only combine same-size photos, never error the whole set. Test: Task 10 `median_gradient_rejects_mixed_sizes` and Task 14 `auto_guess_set_groups_by_size`.
4. Unusual pixel formats: grayscale JPEG, CMYK JPEG, 16-bit PNG, PNG with transparency. They must load; transparency must survive in the output. Test: Task 6 `loads_unusual_formats`, `png_alpha_preserved`.
5. Cancel or crash mid-save, and name clashes: no half-written file with the final name, no overwrite. Test: Task 6 `save_is_atomic_and_unique`, Task 11 `inpaint_cancel`.

---

### Task 1: Workspace, CI skeleton, tiling helper

**Files:**
- Create: `Cargo.toml` (workspace, members `crates/puw-core`, `crates/puw-dev`), `rust-toolchain.toml` (stable), `crates/puw-core/Cargo.toml`, `crates/puw-core/src/lib.rs`, `crates/puw-core/src/tiles.rs`, `crates/puw-dev/Cargo.toml`, `crates/puw-dev/src/main.rs` (prints usage), `.github/workflows/ci.yml`
- Test: `crates/puw-core/src/tiles.rs` (unit tests)

**Interfaces:**
- Produces: `pub fn tile_starts(length: u32, tile: u32, stride: u32) -> Vec<u32>`; `pub const TILE: u32 = 512; pub const STRIDE: u32 = 384;` in `puw_core::tiles`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn covers_exactly() { assert_eq!(tile_starts(1000, 512, 384), vec![0, 384, 488]); }
#[test] fn shorter_than_tile() { assert_eq!(tile_starts(300, 512, 384), vec![0]); }
#[test] fn exact_fit() { assert_eq!(tile_starts(512, 512, 384), vec![0]); }
```

- [ ] **Step 2: Run** `cargo test -p puw-core tiles` - Expected: FAIL (function missing).
- [ ] **Step 3: Implement `tile_starts`** (same rule as the prototype: steps of `stride`, last start clamped to `length - tile`, single 0 when `length <= tile`).
- [ ] **Step 4: Run** `cargo test -p puw-core` - Expected: PASS.
- [ ] **Step 5: CI** `ci.yml`: on push/PR, matrix `macos-latest`, `windows-latest`: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test --workspace`. Push and confirm both jobs green with `gh run watch`.
- [ ] **Step 6: Commit** `git commit -m "Add workspace and tiling helper"`

### Task 2: Synthetic test-set generator

**Files:**
- Create: `tools/pyproject.toml` (uv project, Python 3.12), `tools/synth/make_sets.py`, `tools/synth/test_make_sets.py`, `tools/README.md`

**Interfaces:**
- Produces: `uv run --project tools python tools/synth/make_sets.py --out <dir> [--src <clean photos dir>]` writing:
  - `<out>/<set>/clean/*.png`, `<out>/<set>/marked/*.png`, `<out>/<set>/mask/*.png` (255 = watermark), `<out>/<set>/manifest.json` = `{"set": str, "kind": "styles"|"fixed"|"random"|"negative", "items": [{"name": str, "has_mark": bool, "bbox": [x, y, w, h] | null}]}`.
  - Sets: `styles` (centered text, corner logo, tiled text, overlay lines, outline lettering), `fixed` (8 photos, same size 1536x1024, one faint outline mark at the same position, alpha ~0.25), `random` (12 photos, mixed portrait/landscape and 2 export sizes, one mark at a random position, scale proportional to the long side), `negative` (6 photos, no mark).

- [ ] **Step 1: Write failing tests** in `test_make_sets.py`: `test_deterministic` (two runs with seed 0 give byte-identical masks), `test_manifest_matches_files` (every item has 3 files of equal size), `test_random_bbox_inside_image`, `test_negative_has_empty_masks`.
- [ ] **Step 2: Run** `uv run --project tools pytest tools/synth -q` - Expected: FAIL.
- [ ] **Step 3: Implement `make_sets.py`.** Sources: scikit-image colour samples (`astronaut`, `chelsea`, `coffee`, `rocket`, `cat`, `immunohistochemistry`, `retina`, `hubble_deep_field`) unless `--src`. Font: `PIL.ImageFont.load_default(size=...)` (bundled, no font file). Outline lettering = text drawn with `stroke_width` and transparent fill. GT mask = alpha > 8, dilated 3x3. `random.Random(seed)` everywhere.
- [ ] **Step 4: Run** tests - Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Add synthetic test-set generator"`

### Task 3: Model export and parity check

**Files:**
- Create: `tools/models/export.py`, `tools/models/verify.py`, `tools/models/test_verify.py`, `tools/models/LICENSES.md` (texts/links for each model)
- Output (not committed, `models/` is gitignored): `models/segmenter.onnx`, `models/lama.onnx`, `models/face.onnx`, `models/manifest.json`

**Interfaces:**
- Produces `models/manifest.json`:
  `{"version": "models-v1", "files": [{"name": "segmenter.onnx", "sha256": str, "bytes": int, "dtype": "fp16"|"fp32", "licence": str, "source": str}, ...]}`
  - `segmenter.onnx`: input `image` float32 `[1,3,512,512]` (ImageNet-normalised), output `prob` float32 `[1,1,512,512]` = pixel max of the sigmoids of the 6 specialists (`centered_text`, `line_pattern`, `logo`, `overlay_text`, `repeated_text`, `tiny_corner`) from HF `christophernavas/watermark-remover`.
  - `lama.onnx`: from HF `ravetools/watermark-remover` `lama_fp32.onnx` (sha256 `1faef5301d78db7dda502fe59966957ec4b79dd64e16f03ed96913c7a4eb68d6`), inputs `image`, `mask`, output `output` as in Global Constraints.
  - `face.onnx`: YuNet face detector (OpenCV Zoo `face_detection_yunet_2023mar.onnx`, MIT).
  - Model graph I/O is float32 even when weights are fp16 (keep_io_types).

- [ ] **Step 1: Write failing test** `test_verify.py::test_manifest_hashes_match_files` (recomputes sha256 of every file listed) and `test_segmenter_io_names`.
- [ ] **Step 2: Implement `export.py`:** load each `.pth` with `torch.load(weights_only=True)` into `smp.UnetPlusPlus(encoder_name="efficientnet-b4", encoder_weights=None, in_channels=3, classes=1)`; call `encoder.set_swish(memory_efficient=False)`; wrap the six in one `nn.Module` returning `amax(stack(sigmoid(m(x))))`; `torch.onnx.export` opset 17, fixed shape. fp16 via `onnxconverter_common.float16.convert_float_to_float16(keep_io_types=True)`.
- [ ] **Step 3: Implement `verify.py`** on the `styles` set (Task 2): segmenter ONNX vs PyTorch: max abs prob diff < 0.02 and IoU of (prob > 0.5) > 0.99 per image; LaMa fp16 candidate vs fp32: PSNR inside the hole > 40 dB, else ship fp32. Writes `dtype` and hashes into the manifest; exits non-zero on any failure.
- [ ] **Step 4: Run** `uv run --project tools python tools/models/export.py && uv run --project tools python tools/models/verify.py && uv run --project tools pytest tools/models -q` - Expected: PASS, total size printed (target <= 500 MB).
- [ ] **Step 5: Commit** (tooling only) `git commit -m "Add model export and parity check"`

### Task 4: Publish models and wire them into tests

**Files:**
- Create: `scripts/fetch-models.sh`, `scripts/fetch-models.ps1` (download `models-v1` assets into `$PUW_MODELS_DIR`, verify sha256 against `manifest.json`), `crates/puw-core/tests/common/mod.rs`
- Modify: `.github/workflows/ci.yml` (cache `models/` keyed on `models-v1`, run fetch script, export `PUW_MODELS_DIR`)

**Interfaces:**
- Produces: `tests/common::models_dir() -> Option<PathBuf>` (reads `PUW_MODELS_DIR`; integration tests call it and return early with `eprintln!("skipped: PUW_MODELS_DIR not set")` when `None`), `tests/common::synth_dir() -> PathBuf` (generates sets into `target/synth` via Task 2 if missing).

- [ ] **Step 1:** `gh release create models-v1 models/*.onnx models/manifest.json tools/models/LICENSES.md --title "Models v1" --notes "Converted model files used by PhotoUnwatermark. Licences in LICENSES.md."` (prerelease; app code releases use separate tags).
- [ ] **Step 2:** Implement both fetch scripts; run `scripts/fetch-models.sh` into a fresh temp dir. Expected: all hashes OK.
- [ ] **Step 3:** CI change; push; `gh run watch` - Expected: green on both OSes with models cached.
- [ ] **Step 4: Commit** `git commit -m "Fetch release models in tests and CI"`

### Task 5: Engine, execution providers, device report

**Files:**
- Create: `crates/puw-core/src/engine.rs`, `crates/puw-core/tests/engine.rs`, `docs/notes/ort-linking.md`

**Interfaces:**
- Produces in `puw_core::engine`:
  - `pub struct ModelPaths { pub segmenter: PathBuf, pub lama: PathBuf, pub face: PathBuf }` with `pub fn from_dir(dir: &Path) -> Result<Self>` (checks files exist).
  - `pub enum Backend { Gpu { name: String }, Cpu { reason: CpuReason } }`, `pub enum CpuReason { NotRequested, ProviderUnavailable(String), LoadFailed(String), PartialGpu { gpu_nodes: usize, total_nodes: usize } }`.
  - `pub struct DeviceReport { pub segmenter: Backend, pub lama: Backend, pub face: Backend }`.
  - `pub struct Engine` with `pub fn new(paths: &ModelPaths, prefer_gpu: bool, gpu_helper_dir: Option<&Path>) -> Result<Engine>`, `pub fn report(&self) -> &DeviceReport`, and crate-internal `run_segmenter(&self, nchw: &[f32]) -> Result<Vec<f32>>`, `run_lama(&self, img: &[f32], mask: &[f32]) -> Result<Vec<f32>>`, `run_face(...)`.
  - `pub enum Error` (thiserror) in `puw_core::Error` with variants `Io(std::io::Error)`, `Image(String)`, `Model(String)`, `SizeMismatch`, `UnsupportedFormat(String)`, `Cancelled`; `pub type Result<T> = std::result::Result<T, Error>`.

- [ ] **Step 1: Write failing tests** `engine_loads_on_cpu` (prefer_gpu = false -> all `Cpu { NotRequested }`, segmenter output length 512*512, values in 0..=1), `engine_reports_gpu_honestly` (prefer_gpu = true: on macOS each model is `Gpu` or `Cpu` with a non-empty reason; never panics), `lama_output_range` (zeros image + empty mask -> output within 0..=255).
- [ ] **Step 2: Run** `cargo test -p puw-core --test engine` - Expected: FAIL.
- [ ] **Step 3: Implement.** macOS: register CoreML EP; decide `Gpu` vs `PartialGpu` from ORT's session partitioning info (verbose log or profiling); Windows: try DirectML EP after adding `gpu_helper_dir` to the DLL search path; any failure -> CPU with the error text as reason.
- [ ] **Step 4: Linking spike on Windows CI** and record the outcome in `docs/notes/ort-linking.md`: can a single `.exe` statically link ORT CPU while DirectML stays optional (DirectML.dll from the helper dir)? If not, list the fallback chosen (e.g. ORT `load-dynamic` with CPU runtime embedded vs. shipped beside the exe) for Plan 3. Expected: the note exists with the CI run link and the verdict.
- [ ] **Step 5: Run** tests - Expected: PASS on macOS and Windows CI.
- [ ] **Step 6: Commit** `git commit -m "Add inference engine with device report"`

### Task 6: Image loading and safe saving

**Files:**
- Create: `crates/puw-core/src/image_io.rs`, `crates/puw-core/tests/image_io.rs`, small fixtures generated in-test (no binary fixtures beyond a few KB).

**Interfaces:**
- Produces in `puw_core::image_io`:
  - `pub struct LoadedPhoto { pub path: PathBuf, pub rgb: RgbImage /* upright */, pub alpha: Option<GrayImage>, pub format: ImageFormat, pub exif: Option<Vec<u8>>, pub icc: Option<Vec<u8>> }`
  - `pub fn load(path: &Path) -> Result<LoadedPhoto>`
  - `pub fn save_result(src: &LoadedPhoto, rgb: &RgbImage, out_dir: &Path) -> Result<PathBuf>` (same stem and format; JPEG q95; EXIF copied with Orientation set to 1; ICC copied; alpha re-attached; unique name; writes `<name>.partial` then renames).
  - `pub fn output_dir_for(src: &Path) -> PathBuf` = `<src parent>/Without watermark`.

- [ ] **Step 1: Write failing tests:** `orientation_roundtrip` (JPEG with Orientation 6 loads upright; saved file has Orientation 1 and identical upright pixels within JPEG tolerance), `exif_and_icc_preserved` (Make tag and ICC bytes survive), `loads_unusual_formats` (grayscale JPEG, CMYK JPEG, 16-bit PNG load to RGB8), `png_alpha_preserved`, `save_is_atomic_and_unique` (existing `a.jpg` -> `a (2).jpg`; no `.partial` left; original untouched by hash), `output_dir_name`.
- [ ] **Step 2: Run** `cargo test -p puw-core --test image_io` - Expected: FAIL.
- [ ] **Step 3: Implement** (`image` for pixels, `kamadak-exif` to read orientation, `little_exif` to rewrite Orientation, `img-parts` to carry ICC/EXIF into JPEG/PNG/WebP). HEIC: return `Error::UnsupportedFormat("heic")`.
- [ ] **Step 4: Run** tests - Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Add photo loading and safe saving"`

### Task 7: Mask operations

**Files:**
- Create: `crates/puw-core/src/mask.rs` (unit tests inline)

**Interfaces:**
- Produces in `puw_core::mask`: `pub type Mask = GrayImage;` (255 = watermark area), `pub fn dilate(m: &Mask, radius: u32) -> Mask` (square kernel `2r+1`), `pub fn union(a: &Mask, b: &Mask) -> Mask`, `pub fn resize(m: &Mask, w: u32, h: u32) -> Mask` (bilinear then > 127), `pub fn coverage(m: &Mask) -> f32` (fraction 0..1), `pub fn bbox(m: &Mask) -> Option<Rect>` (`imageproc::rect::Rect`), `pub fn components(m: &Mask) -> Vec<Component>` with `pub struct Component { pub bbox: Rect, pub pixels: u32, pub mask: Mask /* cropped to bbox */ }`, `pub fn grow_for(w: u32, h: u32, frac: f32) -> u32` = `max(7, round(max(w,h) * frac))`.

- [ ] **Step 1: Write failing tests:** `dilate_single_pixel` (radius 2 -> 5x5 block), `components_two_blobs`, `grow_for_values` (`grow_for(7360, 4912, 0.004) == 29`, `grow_for(800, 600, 0.004) == 7`), `coverage_half`.
- [ ] **Step 2: Run** `cargo test -p puw-core mask` - FAIL. **Step 3: Implement** (imageproc morphology / region labelling). **Step 4: Run** - PASS.
- [ ] **Step 5: Commit** `git commit -m "Add mask operations"`

### Task 8: Automatic guess (detector)

**Files:**
- Create: `crates/puw-core/src/detect.rs`, `crates/puw-core/tests/detect.rs`

**Interfaces:**
- Consumes: `Engine::run_segmenter`, `tile_starts`, `mask::*`.
- Produces in `puw_core::detect`:
  - `pub fn predict_prob(engine: &Engine, img: &RgbImage, long_side: u32) -> Result<ImageBuffer<Luma<f32>, Vec<f32>>>` (image-size output; pads images smaller than 512 with edge replication).
  - `pub struct GuessOptions { pub long_sides: [u32; 3], pub threshold: f32, pub votes: usize, pub grow_frac: f32 }` with `Default` = `[768, 1280, 2048]`, `0.5`, `2`, `0.004`.
  - `pub struct Guess { pub mask: Mask, pub confidence: f32 /* mean of the per-pixel max probability over scales inside the mask; 0 when empty */ }`
  - `pub fn guess_single(engine: &Engine, img: &RgbImage, opts: &GuessOptions) -> Result<Guess>` (vote, no face guard, no growth; skips long sides > 1.5x the image's long side, requiring `min(votes, scales used)`).

- [ ] **Step 1: Write failing tests** (skip without models): `predict_prob_small_image` (300x200 -> output 300x200, no panic), `styles_recall` (on Task 2 `styles` set, mean recall of `guess_single(..).mask` dilated 3 px vs GT dilated 1 px >= 0.9), `negative_set_quiet` (mean coverage on `negative` set < 0.02).
- [ ] **Step 2: Run** `cargo test -p puw-core --test detect` - FAIL. **Step 3: Implement** (tiles in parallel batches is optional; correctness first). **Step 4: Run** - PASS.
- [ ] **Step 5: Commit** `git commit -m "Add watermark area guess"`

### Task 9: Face guard

**Files:**
- Create: `crates/puw-core/src/faces.rs`, `crates/puw-core/tests/faces.rs`

**Interfaces:**
- Consumes: `Engine::run_face`.
- Produces: `pub struct Face { pub bbox: Rect, pub eyes: [(f32, f32); 2], pub score: f32 }`, `pub fn detect_faces(engine: &Engine, img: &RgbImage) -> Result<Vec<Face>>` (score >= 0.7), `pub fn guard(mask: &Mask, faces: &[Face], keep: Option<&Mask>) -> Mask` (clears mask pixels inside an ellipse around each eye with radius 0.12 x face width, except where `keep` is set).

- [ ] **Step 1: Write failing tests:** `guard_clears_eye_region` (synthetic mask over a face's eye is cleared), `guard_respects_keep`, `detect_faces_on_astronaut` (scikit-image `astronaut`: exactly one face, both eyes inside its bbox).
- [ ] **Step 2: Run** - FAIL. **Step 3: Implement** YuNet decoding (strides 8/16/32, NMS 0.3). **Step 4: Run** - PASS.
- [ ] **Step 5: Commit** `git commit -m "Add face guard for the automatic guess"`

### Task 10: Batch layer for fixed-position marks

**Files:**
- Create: `crates/puw-core/src/batch.rs`, `crates/puw-core/tests/batch.rs`

**Interfaces:**
- Produces: `pub fn median_gradient(images: &[&RgbImage]) -> Result<ImageBuffer<Luma<f32>, Vec<f32>>>` (Error::SizeMismatch unless all equal; processes 512-row bands with 1-row overlap to bound memory), `pub fn batch_lines(mag: &ImageBuffer<Luma<f32>, Vec<f32>>, low: f32, high: f32) -> Option<Mask>` (constants from Global Constraints; None when no pixel > high or result coverage > 0.10).

- [ ] **Step 1: Write failing tests:** `median_gradient_rejects_mixed_sizes`, `median_is_per_image` (3 different random images: result equals median of per-image 2-D Sobel magnitudes computed separately; guards the 3-D filter bug from the prototype), `fixed_set_found` (Task 2 `fixed` set: recall vs GT >= 0.8, coverage < 0.05), `negative_set_none` (6 negatives resized to one size -> `None` or coverage < 0.01).
- [ ] **Step 2: Run** - FAIL. **Step 3: Implement.** **Step 4: Run** - PASS.
- [ ] **Step 5: Commit** `git commit -m "Add batch layer for fixed-position marks"`

### Task 11: Inpainting

**Files:**
- Create: `crates/puw-core/src/inpaint.rs`, `crates/puw-core/tests/inpaint.rs`

**Interfaces:**
- Produces: `pub struct Progress<'a> { pub on_fraction: &'a (dyn Fn(f32) + Sync), pub cancel: &'a AtomicBool }`; `pub fn inpaint(engine: &Engine, img: &RgbImage, mask: &Mask, work_long_side: Option<u32>, progress: Progress) -> Result<RgbImage>` (Error::Cancelled when cancelled; returns the input unchanged for an empty mask; default working side `pub const WORK_LONG_SIDE: u32 = 4096`).

- [ ] **Step 1: Write failing tests:** `pixels_outside_mask_unchanged` (exactly equal beyond the 2 px soft edge), `every_hole_filled` (diff vs input non-zero in > 95 % of mask pixels on a mark), `inpaint_small_image` (300x200 works), `inpaint_cancel` (cancel flag set before start -> `Err(Cancelled)`), `styles_quality` (on `styles` with GT masks: mean PSNR inside the mark vs clean >= 22 dB, the prototype's oracle level).
- [ ] **Step 2: Run** - FAIL. **Step 3: Implement** per Global Constraints; tiles visited in order, progress = filled tiles / tiles with holes. **Step 4: Run** - PASS.
- [ ] **Step 5: Commit** `git commit -m "Add tiled LaMa inpainting"`

### Task 12: Watermark template and matching

**Files:**
- Create: `crates/puw-core/src/template.rs`, `crates/puw-core/src/ncc.rs`, `crates/puw-core/tests/template.rs`

**Interfaces:**
- Produces in `puw_core::template` (all `Serialize`/`Deserialize`):
  - `pub struct Piece { pub rel_bbox: [f32; 4] /* x,y,w,h as fractions of the reference image */, pub shape: Vec<u8> /* PNG of the cropped mask */, pub edges: Vec<f32>, pub edges_w: u32, pub edges_h: u32 }`
  - `pub struct WatermarkTemplate { pub pieces: Vec<Piece>, pub ref_w: u32, pub ref_h: u32 }`
  - `pub fn build_template(img: &RgbImage, mask: &Mask) -> WatermarkTemplate` (pieces = mask components with >= 0.1 % of the mask's pixels, merged when bboxes are closer than 2 % of the long side; edges = Sobel magnitude of `img` inside each piece's bbox at a reference long side of 1536).
  - `pub enum MatchStatus { Found, Unsure, NotFound }`; `pub struct MatchResult { pub status: MatchStatus, pub confidence: f32, pub mask: Mask /* image-size */ }`
  - `pub fn find(t: &WatermarkTemplate, img: &RgbImage) -> MatchResult`
  - `pub const FOUND: f32 = 0.6; pub const UNSURE: f32 = 0.35;` (initial values; Step 5 tunes them).
- In `ncc.rs`: `pub fn masked_ncc(image: &[f32], iw: usize, ih: usize, tmpl: &[f32], tmask: &[f32], tw: usize, th: usize) -> Vec<f32>` (score map, FFT-based masked normalised cross-correlation after Padfield 2010, via `rustfft`).

Algorithm (not determined by the signatures):

```
scale0 = target_long_side / ref_long_side
1. Fixed fast path: place all pieces (as a rigid group) at the same relative position and scale0;
   score = mean masked NCC of the pieces. If score >= FOUND -> Found.
2. Group search: edge maps at long side 384 (coarse) for scales scale0 * {0.7,0.8,0.9,1.0,1.1,1.25,1.4};
   best (scale, offset) for the group; refine at long side 1536 within +-8 px and +-5 % scale.
3. If group score < UNSURE and there are >= 2 pieces: search each piece independently the same way;
   score = min over pieces.
4. Status from FOUND/UNSURE; mask = piece shapes pasted at the found placement(s), image size.
```

- [ ] **Step 1: Write failing tests:** `masked_ncc_finds_shift` (template cut from a random image found at its true offset, score > 0.99), `fixed_set_all_found` (Task 2 `fixed`: build from item 0 with its GT mask; all others `Found`, mask IoU vs GT >= 0.8), `random_set` (Task 2 `random`: >= 95 % `Found` with placement IoU >= 0.6; the rest `Unsure`, never a wrong `Found`), `negatives_never_found` (no `Found` on `negative`), `roundtrip_serde`.
- [ ] **Step 2: Run** - FAIL. **Step 3: Implement.** **Step 4: Run** - PASS.
- [ ] **Step 5: Tune** `FOUND`/`UNSURE` with `puw-dev match-sweep` (Task 14) on synthetic sets and, locally, on `PUW_REAL_DIR`; keep 0 `Found` on negatives as a hard limit. Record chosen values and the synthetic numbers (no real file names) in `docs/notes/thresholds.md`.
- [ ] **Step 6: Commit** `git commit -m "Add watermark template matching"`

### Task 13: Saved watermarks library

**Files:**
- Create: `crates/puw-core/src/library.rs`, `crates/puw-core/tests/library.rs`

**Interfaces:**
- Produces: `pub struct SavedWatermark { pub id: String /* uuid v4 */, pub name: String, pub created: String /* RFC 3339 */, pub template: WatermarkTemplate, pub thumbnail_png: Vec<u8> }`; `pub struct Library` with `pub fn open(dir: &Path) -> Result<Library>`, `pub fn list(&self) -> Result<Vec<SavedWatermark>>` (newest first), `pub fn save(&self, w: &SavedWatermark) -> Result<()>` (one JSON file per id, written atomically), `pub fn rename(&self, id: &str, name: &str) -> Result<()>`, `pub fn delete(&self, id: &str) -> Result<()>`, `pub fn suggest(&self, sample: &[&RgbImage]) -> Result<Option<(String, f32)>>` (runs `find` on up to 5 photos; returns the watermark with the best mean confidence if at least half are `Found`).

- [ ] **Step 1: Write failing tests:** `save_list_rename_delete`, `corrupt_file_skipped` (a garbage JSON file in the dir does not break `list`), `suggest_matches_fixed_set`, `suggest_none_for_negatives`.
- [ ] **Step 2: Run** - FAIL. **Step 3: Implement.** **Step 4: Run** - PASS.
- [ ] **Step 5: Commit** `git commit -m "Add saved watermarks library"`

### Task 14: Pipeline facade and developer CLI

**Files:**
- Create: `crates/puw-core/src/pipeline.rs`, `crates/puw-core/tests/pipeline.rs`, `crates/puw-dev/src/main.rs` (replace stub)
- Modify: `crates/puw-core/src/lib.rs` (public re-exports)

**Interfaces:**
- Consumes: everything above.
- Produces (the API Plan 2 builds on):
  - `pub fn auto_guess_set(engine: &Engine, photos: &[LoadedPhoto], opts: &GuessOptions) -> Result<Vec<Guess>>` (per photo: `guess_single` -> face guard (keep = batch layer) -> union with the batch layer of its same-size group if the group has >= 3 photos -> dilate by `grow_for`; confidence kept from `guess_single`, or 1.0 where only the batch layer contributed).
  - `pub fn pick_representative(guesses: &[Guess]) -> usize` (highest confidence among non-empty guesses; ties and all-empty -> 0).
  - `pub fn remove_one(engine: &Engine, photo: &LoadedPhoto, mask: &Mask, out_dir: &Path, work_long_side: Option<u32>, progress: Progress) -> Result<PathBuf>`.
- `puw-dev` subcommands (developer only, not shipped): `bench --set <dir>` (recall, coverage, PSNR in mark, time per photo), `match-sweep --set <dir>`, `run <photos dir> --mask <png> --out <dir>`; `--real` reads `PUW_REAL_DIR` and prints aggregate numbers only.

- [ ] **Step 1: Write failing tests:** `auto_guess_set_groups_by_size` (mixed-size folder: no error, batch layer applied only to the >= 3 same-size group), `remove_one_respects_orientation` (orientation-6 JPEG: mask painted on the upright image removes the mark at the right place; output upright with Orientation 1), `remove_one_never_touches_original` (hash unchanged).
- [ ] **Step 2: Run** - FAIL. **Step 3: Implement.** **Step 4: Run** `cargo test --workspace` - PASS.
- [ ] **Step 5: Quality bar:** `cargo run -p puw-dev --release -- bench --set target/synth/styles` - Expected: mark-region PSNR >= prototype `seg-lama` (21.1 dB) and printed per-photo time. Locally: `PUW_REAL_DIR=... cargo run -p puw-dev --release -- bench --real` and note aggregate results in `docs/notes/thresholds.md` (no file names, no images).
- [ ] **Step 6: Commit** `git commit -m "Add pipeline facade and developer CLI"`

# Desktop watermark remover: design

Date: 2026-11-23. Status: approved. App name: **PhotoUnwatermark** (repo `007hacky007/PhotoUnwatermark`).
Items marked *(builder's call, check)* were decided by the designer, not the owner.

## Intent

A free, open-source desktop app for macOS and Windows that removes a visible watermark (text, logo, outline lettering)
from a set of photos. It is for people who have never edited an image beyond cropping and have never heard of a
"mask". They add a folder of photos, confirm what the watermark is, and get clean copies.

It grows out of a research prototype (Python, in the author's working folder, not part of this repo) that compared
open watermark-removal models. The prototype established what works:

- Inpainting (LaMa) is good when it is told exactly where the watermark is. Detection is the weak link.
- Learned watermark detectors find bold text but miss faint marks and large semi-transparent logos, and produce
  false positives on texture (paving, foliage, fabric), jewellery and, critically, eyes. A fully automatic run
  replaced a child's eye with a dark blob. Therefore a person must confirm the area before anything is removed.
- A watermark stamped at a fixed position across a set can be recovered from the set itself (per-pixel median of
  gradients across images), even when it is too faint for any detector.

Typical use: one photo set (e.g. 20-200 proofs from one event) carries one watermark; the next set usually carries
a different one. Worst case: the same watermark placed at a different spot on each photo.

## Goals

1. A 5-step guided flow (wizard) that a first-time user completes without reading documentation.
2. Automatic first guess of the watermark area, editable with Paint-style tools.
3. Confirm the watermark once per set; the app finds it in every other photo of the set, whether it is at the same
   position or moved (and possibly scaled), and flags the photos it is unsure about.
4. Saved watermarks: reuse a confirmed watermark on future sets; the app suggests a saved watermark when a new set
   matches one.
5. High-quality removal at full resolution (tested up to 36 MP), with progress, time left, and cancel.
6. Never modify originals.
7. Runs fully offline after a one-time model download. No account, no telemetry, no network use except the model
   download (and an optional, user-triggered "check for updates" link that just opens the releases page).

## Non-goals

- Invisible / steganographic watermarks (SynthID, StegaStamp, etc.).
- Video.
- General photo editing or object removal beyond the watermark flow (the brush could do it, but it is not marketed
  or designed for it).
- A command-line interface or server/cloud mode (v1).
- Linux, Windows on ARM, mobile (v1).

## Anti-goals (would make it a failure even if it works technically)

- Silently damaging photos: removing something that is not the watermark (faces, eyes, jewellery, text on documents)
  without the user having seen the area first.
- Jargon in the UI ("mask", "inpainting", "segmentation", "threshold", "dilation").
- Overwriting or moving original files.
- Phoning home, analytics, crash uploads.
- Anything in the repo, app, releases or metadata that identifies the author beyond the GitHub handle
  (see Constraints).

## Constraints

- **Platforms:** macOS 12+ as a universal binary (Apple Silicon + Intel); Windows 10/11 x64.
- **Distribution, macOS:** `.app` ad-hoc signed (`codesign -s -`), not notarized, no Apple Developer ID. Shipped in
  a DMG built with `dmgbuild`. The DMG window background is an HTML page rendered to 1x and 2x PNGs (combined to a
  HiDPI TIFF) that shows the app icon and hand-drawn red pencil steps: "1. Run me", "2. Click Done",
  "3. Open System Settings > Privacy & Security (scroll down)", "4. Open Anyway", plus the macOS 14-and-older
  right-click > Open tip and the `xattr -d com.apple.quarantine <dmg>` one-liner. Release notes repeat the steps.
- **Distribution, Windows:** a single portable `.exe`, unsigned, no installer *(GPU handling: builder's call, check)*.
  The exe runs on CPU out of the box (ONNX Runtime CPU linked in). The DirectML GPU runtime files are downloaded with
  the models on first launch (SHA-256 verified, stored in the app data folder) and loaded dynamically; if they fail to
  load, the app falls back to CPU and says so (see "Processing device" below). No self-extracting of DLLs to temp
  folders (antivirus heuristics). If WebView2 is missing, show a native message box with the download
  link. README explains SmartScreen "More info > Run anyway" with a screenshot in the same pencil style.
- **Data location:** models, saved watermarks and settings in the per-user app data folder
  (macOS `~/Library/Application Support/<app>`, Windows `%LOCALAPPDATA%\<app>`). Settings screen has
  "Remove downloaded data" (models + saved watermarks, with confirmation).
- **Identity:** public repo under the `007hacky007` GitHub account. Commits as `007hacky007`
  `<007hacky007@users.noreply.github.com>`, `commit.gpgsign false`. No real names, emails, Apple team IDs, local
  paths, or real photos in code, docs, tests, commit metadata or binaries. No signing with any personal developer
  certificate.
- **Test data in the repo:** synthetic only (natural sample images from scikit-image or public-domain sources, with
  generated watermarks). Never real people or real studios' marks.
- **Local real-photo validation (allowed):** the owner's real photo sets may be used locally for tuning thresholds,
  checking quality and the usability test. They live outside the repo, are referenced only through a local path
  (e.g. an environment variable or an untracked config file listed in `.gitignore`), and nothing derived from them
  (images, crops, masks, metrics that name files) is ever committed. Thresholds must be checked on both synthetic and
  real sets before release.
- **Fresh repo:** nothing is copied from the prototype folder unreviewed (it contains real photos and a real
  studio's name). Conversion tooling and the synthetic generator are rewritten clean in the repo.
- **Licences:** app code MIT. Models keep their own licences, credited in the app's About screen and README:
  watermark segmenters (christophernavas/watermark-remover, Apache-2.0), big-LaMa (advimman/lama, Apache-2.0;
  note the upstream weights-licence caveat), MI-GAN if used (Picsart, MIT).
- **Responsible-use note** in README and About: intended for photos you own or have permission to edit.

## Approaches considered

1. **Tauri 2: Rust core + web UI (TypeScript, Svelte) - chosen.** Rust does image I/O, detection, matching and
   inpainting via ONNX Runtime (`ort` crate; CoreML EP on macOS, DirectML EP on Windows, CPU fallback). The UI,
   including the brush canvas, is web tech in the system web view. Small app (~20 MB + models), polished editor is
   straightforward. Cost: two languages; WebView2 dependency on Windows.
2. Pure Rust native GUI (egui / Slint). Single language and binary, no web view. Rejected: a friendly, polished
   brush canvas, before/after slider and step screens take much more custom work, and egui reads as a developer tool.
3. Python + Qt reusing the prototype. Fastest to a first version. Rejected: 0.5-1 GB download, slow start, PyInstaller
   executables are often flagged by Windows antivirus (bad for an unsigned portable exe for strangers), not Rust.

(Electron dropped early: a heavier Tauri with no benefit here.)

## User flow (decided)

Wizard, one task per screen, progress line on top (`1 Add photos > 2 Check the watermark > 3 Check all photos >
4 Remove > 5 Done`), one obvious primary button per screen, one or two plain sentences of explanation per step.
Back is always possible without losing work.

**First launch:** welcome screen, one sentence on what the app does, then the model download (~450 MB target) with
progress, resume after interruption, retry on failure, SHA-256 verification. Explain that it is one-time and that
photos never leave the computer.

**Step 1 - Add photos:** drop zone accepting files or folders (also "Choose photos..." button). JPEG, PNG, TIFF, WebP
(HEIC if feasible on both platforms; otherwise a clear "not supported yet" message). EXIF orientation applied.
Shows count and thumbnails. If photos match a saved watermark, offer it: "These look like *<saved name>*. Use it?"
(Yes skips the auto-guess; the user still lands in step 2 to confirm.)

**Step 2 - Check the watermark (editor):** shows one representative photo (auto-picked: the one where detection is
most confident; user can switch via a small strip). The red semi-transparent layer is the area that will be removed,
pre-filled by the automatic guess. Help line: "We found the watermark and painted it red. Use the Brush to paint over
anything we missed and the Eraser to clean up anything that is not watermark."

- Tools: **Brush**, **Eraser**, **Rectangle**; **Size** slider; **Undo / Redo**; zoom **- / 100% / + / Fit**.
- Shortcuts: B / E / R, `[` `]` size, Cmd/Ctrl+Z / Shift+Cmd/Ctrl+Z, wheel/pinch zoom, drag with Space or middle
  button to pan, hold Space (without dragging) to hide the red layer.
- **Preview on this photo:** runs removal on this photo only, shows a before/after slider; the user can go back to
  editing. Primary button: **Looks right ->**.
- The terms "mask", "mark", "unmark" are never shown. "Watermark area" is the UI noun.

**Step 3 - Check all photos:** grid of all photos with the watermark area drawn where the app found it, each with a
status: found (no badge) / unsure (yellow "?") / not found (grey).

Safety rule *(builder's call, check)*: "found" photos (strong match of the confirmed shape) are processed after the
grid check. Every "unsure" photo must be either opened in the editor or unticked before the Remove button is enabled.
"Not found" photos are unticked by default; the user can open one and paint the area by hand, which ticks it. Summary line: "Found in 118 of 120 photos. 2 need
a look." Clicking a photo opens the same editor for that photo only (its own area, starting from the found
position); fixes there do not change the set's watermark. Photos can be excluded with a checkbox. Primary button:
**Remove watermark from N photos**.

**Step 4 - Remove:** overall progress bar, "photo 37 of 118", time left, current photo thumbnail turning from before
to after, Cancel (finishes or discards the current photo, keeps completed ones). The machine must stay usable
(processing off the UI thread, bounded parallelism).

**Step 5 - Done:** before/after slider over the results (browse with arrows), **Open folder**, **Save this watermark
for next time** (name it, default "Watermark from <date>"), **Start a new set**. If some photos failed, list them
with a plain-language reason.

**Output:** copies in a new subfolder next to the originals named "Without watermark" (configurable in Settings;
if the source folder is not writable, ask for a folder). Same file name and format as the source; JPEG saved at high
quality (~95) with EXIF and ICC profile preserved, orientation normalised. Never overwrite: add " (2)" on clash.

**Saved watermarks screen** (from the welcome/start screen and Settings): list with thumbnail, name, date, rename,
delete.

**Processing device (owner's decision):** the app always shows clearly whether it is using the GPU or the CPU, and
why. A small status chip is visible on the Remove step (step 4), on the step-2 preview, and in Settings, e.g.
"Using graphics card (fast)" or "Using processor (slower) - why?". The "why?" opens a plain-language explanation of
the actual reason (GPU helper not downloaded, download failed, helper failed to load with the error, graphics card
not supported, GPU run failed and fell back mid-way) and, when a fix exists, a button for it: "Download GPU helper
(N MB)", "Try again", or "Use processor anyway". A fallback from GPU to CPU during a run is announced in the progress
screen, never silent. On macOS the chip reflects whether the CoreML path is actually used for the models (not only
whether it was requested).

**Settings / Advanced (collapsed):** output folder rule, "detail vs speed" (maps to inpainting working resolution),
"Remove downloaded data", About (licences, credits, responsible-use note, version, link to releases).

## Processing (decided, from the prototype)

**Automatic first guess (step 2 pre-fill):**
- Six specialist UNet++ segmenters (EfficientNet-B4) run on 512 px tiles (stride 384) of the photo resized so its
  long side is 768, 1280 and 2048 px; a pixel is watermark when at least 2 of the 3 scales agree (probability > 0.5
  in each). Inputs ImageNet-normalised RGB.
- If the set has >= 3 photos of the same size, also compute the per-pixel median of 2-D Sobel gradients across them
  (per image, never a 3-D filter over the stack), keep thin consistent lines by hysteresis (low 40, high 80, thick
  regions removed by a 7x7 opening first), and add them. Discard this layer if it covers > 10 % of the image
  (static scene, not a watermark).
- Grow the result by ~0.4 % of the long side.
- Because this guess can hit eyes, jewellery and document text, it is only ever a starting point shown to the user.
  The builder should make it conservative: remove components that overlap detected faces/eyes unless the set-level
  median layer supports them, or at least never pre-fill eye regions (face detection model choice left to the
  builder; it must run offline and be licence-compatible).

**Finding the confirmed watermark in the other photos (step 3):**
- Build a watermark template from the confirmed photo: the area's shape (binary, at its native size) plus the
  appearance inside its bounding box (gradient-magnitude / edge map, and the colour patch).
- For each other photo: first test the same relative position (fast path for fixed-position marks); otherwise search
  the whole photo with normalised cross-correlation on edge maps at a few scales (relative to image size, e.g.
  0.7x-1.4x, to cover portrait/landscape and different export sizes). Optionally refine with that photo's own
  detector output inside the matched box.
- Confidence = match score; thresholds for found / unsure / not found tuned on the synthetic test set (including a
  "random position" set). Unsure and not found are never processed without the user looking.
- Rotation search is not required for v1 (left to builder if cheap).
- Multi-piece watermarks *(builder's call, check)*: the confirmed area is split into connected pieces; the pieces are
  searched for as one rigid group (they normally move together); only if the group score is low is each piece
  searched for independently. Watermarks repeated across the whole photo (tiled text) are treated as fixed-position;
  matching a repeating pattern at a moving offset is out of scope for v1.

**Removal:** big-LaMa on 512 px tiles (each tile fills only holes in its central 384 px; all remaining holes in view
are given as context), sequential across tiles. Photos larger than the "detail vs speed" working size (default long
side 4096) are inpainted at that size and only the filled pixels are pasted back into the full-resolution original
with a soft (1.5 px) edge. MI-GAN is not used for final output (it leaves ghost letters at high resolution); the
builder may use it for the step-2 quick preview if LaMa is too slow there.

**Model conventions (verified in the prototype):** segmenters: ImageNet-normalised float RGB, sigmoid output.
LaMa ONNX (Carve export, 512x512): image float 0-1, mask float 1 = hole, output 0-255. MI-GAN pipeline v2: uint8
image, uint8 mask 255 = keep, 0 = hole.

**Models in the app:** converted once to ONNX (segmenters from PyTorch; the six specialists may be merged into one
graph that outputs the pixel max), fp16 where accuracy holds, verified by parity tests against the PyTorch/prototype
outputs on the synthetic set. Hosted as assets on this repo's GitHub Releases with SHA-256 in the app. The
conversion tooling (Python) lives in the repo under a separate tools folder and is not shipped.

## Quality bar

- Synthetic benchmark (generated in-repo: centered text, corner logo, tiled text, overlay lines, outline lettering,
  fixed-position faint mark across a set, same mark at random positions) with ground-truth areas; CI runs a reduced
  version.
- Targets, relative to the prototype on the same synthetic set: watermark-area PSNR at least as good as prototype
  `seg-lama`; random-position set: >= 95 % of photos found, 0 false "found" on photos without the mark.
- Performance target on an M1 / mid-range Windows laptop: step-2 preview of a 24 MP photo under ~15 s;
  batch throughput shown with an honest ETA. (Prototype on CPU: ~100-140 s per 24-36 MP photo; GPU EPs should beat it.)
- Usability check before release: a person who has not seen the app removes a watermark from a 10-photo set without
  help.

## Release

GitHub Actions: on tag, build macOS universal `.app` (ad-hoc signed) -> DMG via `dmgbuild` with the rendered
background -> `.sha256`; build Windows portable `.exe` -> `.sha256`; create/update the release with notes that
include the opening steps for both systems. Model assets attached to a separate, rarely changing release tag that
the app references by URL + hash.

## Left to the builder

Repo layout, Svelte vs other framework details, state management, tile/thread scheduling, exact NCC implementation
and thresholds, face/eye guard approach, thumbnail caching, icon design (keep it generic; no brand imitation),
DMG background artwork details, error-message wording (plain language), HEIC support decision.

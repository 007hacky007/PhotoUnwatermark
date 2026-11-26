# PhotoUnwatermark Plan 2c: Groups Board

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The app sorts every photo into watermark groups first (one shape per group, a spot per photo), the person fixes the groups on a board with drag and drop, picks the photos to clean, and one run cleans them. Finding 200 photos of 24 MP takes about 5 minutes at most on the owner's Mac and on a Windows PC without a usable graphics card; the groups screen opens as soon as the first group exists (about 40 s on the Mac, about 2 minutes on the PC) and finding goes on in the background.

**Architecture:** Speed work in `puw-core` comes first: a photo is prepared for matching once (`MatchImage`) and reused by every drawing and round, a search that cannot find the watermark gives up after its coarse stage, and a lighter finding profile serves machines whose finder runs on the processor. `puw-app` then gets a board model next to the old peel-off session (`board.rs`: groups with a shape at the frame's edit size, members with a spot each, "?" state, picks, board undo), the finding rounds (`core/find.rs`) running as a background job that plain board calls work alongside, cleaning with turned spots and per-group saving. The Tauri shell and `api.ts` expose it, a UI store (`finding.svelte.ts`) keeps the background finding's progress across screens, new screens replace steps 2 and 3, and the last code task deletes the peel-off flow. The old flow keeps compiling and passing its tests until Task 14, so every task ends green.

**Tech Stack:** as Plan 2 and 2b (Rust 2024, Tauri 2, Svelte 5 runes, TypeScript, Vitest, `@testing-library/svelte`). No new dependencies.

**Spec:** `docs/superpowers/specs/2026-11-26-groups-board-design.md` (binding). It replaces steps 2 and 3 of `docs/superpowers/specs/2026-11-25-watermark-groups-design.md`; that document and `docs/superpowers/specs/2026-11-23-photounwatermark-design.md` hold wherever the new spec is silent. Previous plan: `docs/superpowers/plans/2026-11-26-plan-2b-watermark-groups.md`.

**When it runs:** after the engine fix for wrong placements (owner-test-fixes, `.superpowers/sdd/2026-11-26-owner-test-fixes/`) has landed on master. The plan was written against HEAD 9ef0d7e. The fix keeps the found / unsure / not-found contract, `MatchResult`, `Transform` and `PositionPrior`; if it renamed a function this plan names (`match_with`, `find_until`, `saved_score`), the implementer uses the landed name and keeps the contract described here.

## Global Constraints

- Work on `master`, one task at a time. The git identity is already set. Do not push. Commit messages: one short imperative subject line under about 60 chars, no body, no trailers, no AI attribution, plain `git commit -m "..."` with no command substitution.
- Never commit real photos, real photo file names or paths, or descriptions of real photo content. Real runs read `PUW_REAL_DIR` (set-a: 8 photos of 24 MP, one fixed watermark; set-b: 1 photo of another size) and write only under the gitignored `local/`. Docs get aggregate numbers only.
- UI copy only in `app/src/lib/copy.ts`. `app/src/lib/api.ts` is the only module that imports Tauri. The UI never says mask, mark, unmark or their inflections (`assertPlainText` in `app/src/test/plainText.ts`); every new or changed screen test file has a `noBannedWordsRendered` case covering its new text. UI nouns: "watermark", "drawing", "group".
- No Unicode dashes anywhere, in code, copy, tests or docs: hyphen-minus only.
- One job at a time with per-call tickets: every long call goes through `job()` in `api.ts`, is stopped with `stopJob(call)`, and every new `AppCore` call that reads photos, runs a model or reads the library holds the job slot and runs in the shell through `self::job(...)`. A screen never freezes or looks done while work runs; every long job has Stop or Back.
- The one exception (spec, owner's choice): finding runs in the background. While it runs, plain board calls (board, moves, shapes, spots, picks, current group, showing a photo, board and drawing pictures) work alongside it, and every other job is refused with `AppError::FindingRuns` ("Waiting for the search to finish."). Finding never moves a photo the person placed or settled and never changes a shape. Every background job shows visible progress and updates the screen that shows its results live; nothing changes silently.
- Time budget (spec): finding for about 200 photos of 24 MP within about 5 minutes (300 s), on a mix of 2-3 watermarks, on the owner's Apple-silicon Mac (Core ML) and on a Windows PC without a usable graphics card (the Windows test VM is the reference). First group within about 40 s on the Mac and about 120 s on the PC. The first guess runs only on samples. A budget that cannot be met after the levers stops the task as BLOCKED; it is never relaxed.
- Real photos never leave the Mac: they are never copied to the Windows VM, CI or GitHub. Windows costs come from synthetic 24 MP photos made in memory, and the controller times the real app on the Windows VM with photos that are not the owner's (Task 15).
- Spec rules every task keeps: a group is one shape (stored in its frame, the reference photo's coordinates) with a spot (move, size, turn) per photo; a photo is in at most one group or in "Not grouped"; group numbers 1, 2, 3 in creation order, never reused within a set; editing never re-sorts photos; "?" photos are never cleaned before the person looked (shown in the editor or dragged); matching keeps turn 0; every count on screen adds up (group photos + Not grouped + still looking + unreadable = all photos).
- Terms: **frame** = the native upright size of the photo (or saved watermark) a group's shape was drawn in, plus that photo if there is one. **Spot** = affine map from frame native pixels to the photo's native pixels, `(x, y) -> (a x + b y + c, d x + e y + f)`, (0, 0) at the top-left corner of the first pixel. An **edit spot** maps the frame's edit image (`images::edit_size`) onto the photo's edit image. **New photos** = readable photos the finding has not looked at yet.
- Verification commands: `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `PUW_MODELS_DIR=$PWD/models cargo test --workspace --release`; `pnpm -C app check`; `pnpm -C app test`; `pnpm -C app build`. Real acceptance: `PUW_MODELS_DIR=$PWD/models PUW_REAL_DIR=$PWD/local/real cargo test -p puw-app --release --test timings <name> -- --ignored --nocapture`. Each task runs the commands of the layers it touched; Task 15 runs all of them.
- Implementers cannot click the app. Manual checks go into Task 15's checklist.

## Review Focus

1. Photos of other sizes and orientations in one group (portrait next to landscape, another camera), with a turned spot: the drawing must land on the watermark and a stroke on a portrait photo must come back to the right place in the shape. Tests: Task 3 `edit_spot_round_trips_across_sizes`, Task 4 `drop_scales_the_reference_spot`, Task 9 `carryBackThroughATurn`.
2. Stop or Back at any moment of finding (model load, a sample's first guess, mid matching pass): every photo ends in a group or in Not grouped, counts add up, no half-made group without its reference, and the next Find picks up the photos not reached. Tests: Task 5 `stop_keeps_what_was_found`, `stop_during_a_first_guess_leaves_no_half_group`.
3. The person works on the board while finding runs in the background (drags, settles a "?", paints, undoes a move just as a result arrives): the person's change wins, finding never moves that photo again or changes a shape, undo never takes back finding's work, and the screen keeps the selection, the drag and the editor view. Tests: Task 5 `a_drop_during_finding_wins`, `settling_during_finding_wins`, `undo_never_undoes_finding`, `look_for_more_never_moves_placed_photos`; Task 10 `liveUpdatesKeepSelectionAndDrag`; Task 11 `liveUpdatesKeepTheEditorView`.
4. A large set (200 photos of 24 MP): prepared copies stay within the cache budget; an evicted photo is prepared again, never dropped from the finding. Tests: Task 5 `match_cache_stays_in_budget`, `evicted_photos_are_still_matched`.
5. A moved, resized or turned drawing partly outside the photo: cleaning clips it, never fails, and an unchanged photo is not cleaned again on the next run. Tests: Task 3 `warp_clips_to_target`, Task 6 `clean_with_the_drawing_partly_outside`.

---

### Task 1: Prepare each photo once for matching

**Files:**
- Modify: `crates/puw-core/src/template.rs`, `crates/puw-core/src/pipeline.rs`, `crates/puw-core/src/lib.rs`
- Test: `crates/puw-core/tests/template.rs`, `crates/puw-core/tests/pipeline.rs`

**Interfaces:**
- Produces (puw-core, re-exported from `lib.rs`):
  - `pub struct MatchImage` (Debug, Clone): a photo as matching sees it, the native upright size plus the luma at the reference space (long side 1536, `ref_dims`) kept as `Vec<u16>` (luma x 128, rounded). `MatchImage::new(img: &RgbImage) -> MatchImage`, `native_size(&self) -> (u32, u32)`, `heap_bytes(&self) -> usize`. Edges are worked out from the luma when a search starts (`Target` is built from a `MatchImage`).
  - `pub fn find_in(t: &WatermarkTemplate, img: &MatchImage, stop: &AtomicBool) -> MatchResult`; `find_until(t, img, stop)` becomes `find_in(t, &MatchImage::new(img), stop)`.
  - `pub fn saved_score_in(t: &WatermarkTemplate, img: &MatchImage) -> Option<f32>`; `saved_score` delegates to it.
  - `pub fn match_in(t: &WatermarkTemplate, prior: Option<&PositionPrior>, img: &MatchImage, stop: &AtomicBool) -> MatchResult`: the set-position score and the search use the same prepared image (today `match_with` builds the reference-space image twice). `match_with` delegates to it.
  - `pub fn prepare(path: &Path) -> Result<MatchImage>` in `pipeline.rs`: `load` then `MatchImage::new`; the full-size pixels are dropped before it returns.
  - The reduction to the reference space (today `working_rgb`, Triangle over the whole photo) becomes: box-average by the integer factor `f = max(1, floor(min(nw / w, nh / h) / 2))`, then Triangle to `(w, h)`. `MatchImage::new` and `build_template` both use it, so templates and photos are reduced the same way. A photo already at the reference size is not resized (synthetic sets are unchanged). `batch.rs` keeps its own reduction.
  - `MatchBasis::PiecesApart`: the result came from the per-piece search (`search_each`), so the pieces did not match together. `transform` is still the largest piece's.
  - `template::saved_placement` becomes `pub` and is re-exported.
- Consumes: nothing from earlier tasks.

- [ ] **Step 1: Write failing tests.**
  - `tests/template.rs`:
    - `prepared_luma_is_the_working_luma`: for `random/marked/00.png`, `MatchImage::new` luma / 128 is within 1/256 of the old `working_luma` at every pixel (expose a `#[doc(hidden)] pub fn luma_at(&self, x, y) -> f32` for the test).
    - `reduction_is_close_to_triangle`: `fixed/marked/01.png` upscaled 4x in memory (6144 x 4096, Triangle); the new reduction against a plain Triangle resize to 1536 x 1024: mean absolute difference < 1.0 grey level, max < 8.
    - `big_photo_matches_like_small`: `fixed` 00 and 01 both upscaled 4x in memory; template from 00 with its truth upscaled; `find_in` on 01 is `Found` with `|scale - 1| <= 0.01` and `|dx|, |dy| <= 0.01 x 6144`.
    - `pieces_apart_basis`: two-piece template (the text truth of `fixed/00` plus the ring of `two_piece_layer` at (60, 60)); on `fixed/01` with the ring blended at (880, 560) the result is not `NotFound`, `basis == MatchBasis::PiecesApart`, and `transform` is within 3 px of identity (the text is the largest piece).
    - `matching_is_unchanged_on_synthetic_sets`: golden values recorded from the code before this task (run it once, paste the numbers as constants): per photo the status, basis, transform (to 0.5 px) and confidence (to 1e-3) of `random` with the template of 00, `styles` with each photo's own truth and `negative` with the template of `fixed` 00. `find_in` on a `MatchImage` must give the same.
  - `tests/pipeline.rs`: `prior_results_are_unchanged` (the same golden check for `match_in` over the `fixed` set with its layers and the template of 00), and `prepare_drops_the_pixels`: `heap_bytes()` of a prepared 1536 x 1024 photo is at most 1536 x 1024 x 2 + 4096.
- [ ] **Step 2: Run** `cargo test -p puw-core --release --test template --test pipeline`. Expected: FAIL (missing items).
- [ ] **Step 3: Implement** the items above. Every existing test of `puw-core` must still pass unchanged.
- [ ] **Step 4: Run** `PUW_MODELS_DIR=$PWD/models cargo test -p puw-core --release`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check`. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Prepare each photo once for matching"`

### Task 2: Cheap misses, the light profile and the speed acceptance

**Files:**
- Modify: `crates/puw-core/src/template.rs`, `crates/puw-core/src/detect.rs` (`GuessOptions` scales), `crates/puw-core/src/pipeline.rs` (`FindProfile`), `crates/puw-core/src/lib.rs`, `crates/puw-app/tests/timings.rs`, `.github/workflows/ci.yml` (models job), `docs/notes/thresholds.md` (new section "Finding speed (Plan 2c)")
- Test: `crates/puw-core/tests/template.rs`, `crates/puw-core/tests/detect.rs`

**Interfaces:**
- Consumes: Task 1 `MatchImage`, `find_in`, `match_in`, `prepare`.
- Produces:
  - `pub const COARSE_REJECT: f32` in `template.rs`: in `search`, when the best coarse score over all scales is below it, the search returns `None` with no fine look (the result is `NotFound`, confidence = that coarse score). `#[doc(hidden)] pub fn coarse_best(t: &WatermarkTemplate, img: &MatchImage) -> Option<f32>` returns that score for calibration.
  - `GuessOptions.long_sides` becomes `Vec<u32>` (default `[768, 1280, 2048]`, `votes: 2`, unchanged results). `GuessOptions::light()` = `long_sides: [1280]`, `votes: 1`, the rest as default.
  - `pub struct FindProfile { pub sample: usize, pub guess: GuessOptions, pub match_long: u32 }` in `pipeline.rs`, re-exported: `FindProfile::full()` = `{ sample: REPRESENTATIVE_CAP (3), guess: default, match_long: 1536 }`; `FindProfile::light()` = `{ sample: 1, guess: GuessOptions::light(), match_long: 1536 }`. Task 5 picks `light()` when the segmenter runs on the processor (`DeviceReport`), else `full()`.
  - Lever "smaller copies", used only when the light profile misses a budget below: `MatchImage::new_at(img: &RgbImage, long_side: u32)` and matching in that space (template edges pooled to it when a search starts; thresholds unchanged unless the recall rows below show a loss, which is then recorded); `FindProfile::light().match_long = 768`.
  - Lever "half-size fine looks", used only when a Mac target fails after smaller copies: the fine look of each coarse peak runs at half the reference space and only the best `CANDIDATES` are scored and refined at full size.
  - Lever "cheaper set check", used only when the `hit` row misses a budget: the set-position checks the owner-test engine fix added to matching (per-part layer scoring and the set-explains check in `pipeline.rs`) are worked out once per group and pass instead of per photo, or run per photo only when the photo's score is within 0.1 of a cut. It is accepted only if the 56-triple real test, the synthetic false-Found tests in `tests/pipeline.rs` and `session.rs` still pass with no Found at a wrong place.
  - Lever "fewer segmenters" needs a new model export (`tools/`) and a new download asset; it is not in this plan. If the budgets fail without it, the task stops BLOCKED and names it.
  - Ignored tests `real_finding_costs` (Mac, real photos) and `synth_finding_costs` (any machine, synthetic 24 MP photos) in `timings.rs`, plus a CI step that runs the synthetic one on `windows-latest`.

- [ ] **Step 1: Calibrate `COARSE_REJECT`.** Add the ignored test `coarse_score_sweep` to `tests/template.rs`: it prints the minimum `coarse_best` at the true placement over `random` (template from 00), `styles` (each photo's own truth on itself shifted by 40 px in memory) and `fixed`, and the maximum over `negative` with each of those templates. With `PUW_REAL_DIR` it also prints the minimum over set-a with set-a's drawing built as in Step 5 (aggregates only). Set `COARSE_REJECT` to the largest multiple of 0.01 that is at most 0.8 x the smallest true-placement score; write the numbers and the choice in `thresholds.md`.
- [ ] **Step 2: Write failing tests.**
  - `tests/template.rs`: `cheap_miss_keeps_every_find` (every `random` photo still `Found` or `Unsure` with the template from 00, no count of the existing random-set test drops, every `styles` photo keeps its status) and `cheap_miss_on_photos_without_it` (every `negative` photo with the `fixed` template is `NotFound`, `coarse_best` below `COARSE_REJECT` on at least 4 of 6).
  - `tests/detect.rs`: `default_guess_is_unchanged` (golden coverage and confidence of `guess_single` on `fixed/00` from before the change) and `light_guess_runs_one_scale` (models; a non-empty guess on `fixed/00` with IoU >= 0.3 against the truth).
- [ ] **Step 3: Implement** `COARSE_REJECT`, `coarse_best`, `GuessOptions` scales and `FindProfile`. Run `PUW_MODELS_DIR=$PWD/models cargo test -p puw-core --release`. Expected: PASS.
- [ ] **Step 4: Measure the recall of each lever** with the ignored test `profile_recall` in `tests/detect.rs` (models): for `full()` and `light()` (and `new_at(768)` when used), the first guess's IoU with the truth on `fixed`, `random`, `styles` and the logo photos (mean and minimum), the share of `random` and `styles` photos matched `Found` or `Unsure` at the true placement, and false `Found` on `negative`. Write the table in `thresholds.md`. A light profile with any false `Found` on `negative` is not accepted.
- [ ] **Step 5: Write the cost tests.** Both measure on the matching pool (half the cores, as `match_photos`), models warm, per profile, and print every row and the models below; aggregates only.
  - `real_finding_costs` (Mac, `PUW_REAL_DIR`): `prep` = wall time to `prepare` the 9 real photos / 9; `hit` = set-a's drawing (`cleaned_guess` of the finder's guess on the representative, as in `real_small_sets_keep_the_set_position`) over the 7 other set-a photos with set-a's layers / 7, each `Found` or `Unsure` within 1 % of identity; `moved` = the same drawing without layers over the 7 set-a photos shifted in memory by (+7 % of width, +5 % of height) / 7, each `Found` or `Unsure` within 1 % of the shift; `miss` = set-b's drawing over the 8 set-a photos and set-a's drawing on set-b / 9, none `Found`; `guess` = median of 3 (decode + `guess_photo`) per profile; `layers8` = `build_set_layers` over set-a; `load` = finder load, warm. It runs the light profile also with `prefer_gpu = false` for a processor-only row on the Mac.
  - `synth_finding_costs` (any machine): 24 MP photos made in memory by upscaling synthetic photos to 6000 x 4000 (Triangle) and written as JPEG (quality 92) into a temporary folder that is deleted afterwards: 8 from `fixed/marked` (fixed watermark), 4 logo photos (`logo_photos`, moving), 6 from `negative/marked`. The same rows as above with these roles (hit: fixed drawing over fixed photos with layers; moved: logo drawing over the other logo photos; miss: fixed drawing over negative photos). `PUW_BUDGET=mac` asserts the Mac targets with `full()`, `PUW_BUDGET=windows` asserts the Windows targets with `light()`; without it, it only prints. With `PUW_SYNTH_SET=<folder>` it also writes the 200-photo scenario B set as 24 MP JPEGs for timing the app by hand (W1: the `fixed` text at its place on 100 photos made from `clean` photos of all synthetic sets, W2: `ring_logo` at one place on 60, W3: a new `bars_logo()` in `tests/common/mod.rs` (three filled bars, 90 x 40, alpha 0.3) at a different place on each of 30, and 10 with none); nothing derived from real photos.
  - The models (every round counted as a full sample of `k = profile.sample` first guesses and one matching pass over every photo left: the worst case; set-a has 8 photos of one watermark and set-b 1, so the counts are modelled, not measured):

    ```
    layers24 = 3 x layers8        (build_set_layers decodes at most 24 photos per size)
    F   = load + 5 prep + 15 miss + layers24 + k guess
          (first group: 3 saved watermarks tried on 5 sample photos, then the first round's sample)
    T_A = load + 200 prep + 2 (layers24 + k guess) + 198 hit + (80 + 15) miss
          (W1 on 120 photos, W2 on 80, both fixed)
    T_B = load + 200 prep + 3 layers24 + (10/8) layers8 + 4 k guess + 158 hit + 29 moved + 165 miss
          (W1 100 fixed, W2 60 fixed, W3 30 moving, 10 photos with none; misses 100 + 40 + 10 + 15;
           the last round finds nothing)
    Mac (full profile):     F <= 40,  T_A <= 300, T_B <= 300
    Windows (light profile): F <= 120, T_A <= 300, T_B <= 300
    ```
- [ ] **Step 6: Add the CI step** to the `models` job of `.github/workflows/ci.yml` (windows-latest, no graphics card, the stand-in for the Windows PC since real photos never leave the Mac): `cargo test -p puw-app --release --test timings synth_finding_costs -- --ignored --nocapture` with `PUW_BUDGET: windows`. The controller checks the printed rows on the next CI run; the Windows test VM is measured by hand in Task 15.
- [ ] **Step 7: Run** `real_finding_costs` and `synth_finding_costs` with `PUW_BUDGET=mac` on the Mac, and `synth_finding_costs` with `PUW_BUDGET=windows` and `prefer_gpu = false` on the Mac as a first look at the processor-only numbers. Measure with the engine as it stands at this task's base (after the owner-test engine fix), never with speeds from earlier notes. If a Mac target fails, use the levers in this order: the cheaper set check when the `hit` row is the largest term, smaller copies for the miss path, then half-size fine looks. If a Windows target fails on the CI run, use the cheaper set check when the `hit` row is the largest term, then smaller copies in the light profile, then `sample: 1` is already the floor. Record the recall of every lever used (Step 4). If a target still fails, stop with status BLOCKED and report the tables; do not loosen a budget.
- [ ] **Step 8: Record** every measured table, `F`, `T_A`, `T_B` per machine and profile, and the formulas in `thresholds.md` "Finding speed (Plan 2c)".
- [ ] **Step 9: Run** `PUW_MODELS_DIR=$PWD/models cargo test --workspace --release`, clippy, fmt. Expected: PASS.
- [ ] **Step 10: Commit** `git commit -m "Speed up finding and add a light profile"`

### Task 3: Spots and drawing a shape at a spot

**Files:**
- Create: `crates/puw-app/src/spot.rs`, `crates/puw-app/src/area.rs` (`Area` moves here from `session.rs`, unchanged; `session.rs` keeps `pub(crate) use crate::area::Area;`)
- Modify: `crates/puw-app/src/session.rs`, `crates/puw-app/src/lib.rs`
- Test: unit tests in `spot.rs` and `area.rs`

**Interfaces:**
- Produces:
  - `#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")] pub struct Spot { pub a: f32, pub b: f32, pub c: f32, pub d: f32, pub e: f32, pub f: f32 }`, exported as `puw_app::Spot`:
    - `Spot::IDENTITY`, `apply(&self, x: f32, y: f32) -> (f32, f32)`, `then(&self, next: &Spot) -> Spot` (self first, then `next`), `inverse(&self) -> Option<Spot>` (`None` when not finite or `|ad - bc| < 1e-9`), `from_transform(t: Transform) -> Spot`, `scale(sx: f32, sy: f32) -> Spot`, `shift(dx: f32, dy: f32) -> Spot`, `turn_about(radians: f32, cx: f32, cy: f32) -> Spot`.
    - `to_edit(&self, frame: (u32, u32), photo: (u32, u32)) -> Spot` = `Ef^-1 then self then Ep`, with `E` the scale from native to `images::edit_size`; `from_edit(edit: &Spot, frame, photo) -> Spot` is its inverse.
    - `close_to(&self, other: &Spot, box_wh: (u32, u32), tol_px: f32) -> bool`: the four corners of a `box_wh` box land within `tol_px` under both.
  - `Area::warp(&self, spot: &Spot, w: u32, h: u32) -> Area`: the area drawn on a `w` x `h` target, `spot` mapping this area's pixels onto the target's. Only target pixels inside the mapped box of this area's bounds are visited. For target pixel centre `(u + 0.5, v + 0.5)` the inverse gives `(x, y)`; the target pixel is set when any source pixel within `r` of `(floor(x), floor(y))` (a square) is set, with `r = max(0, ceil(0.5 / s_min - 0.5))` and `s_min` the smaller singular value of the linear part. So scale >= 1 is nearest-pixel, and a downscale never drops a 1 px stroke. A non-invertible spot gives an empty area.
  - `Area::bounds(&self) -> Option<[u32; 4]>` (x, y, w, h) and `Area::from_edit_bytes(bytes: &[u8], w: u32, h: u32) -> Result<Area>` (non-zero = set; wrong length is `InvalidInput`).
- Consumes: puw-core `Transform`.

- [ ] **Step 1: Write failing tests.**
  - `spot.rs`: `then_and_inverse_round_trip` (random affine with a turn of 0.3 rad: `s.then(&s.inverse())` maps 5 points back within 1e-3); `turn_about_keeps_the_centre`; `from_transform_matches_apply`; `edit_spot_round_trips_across_sizes` (frame 6000 x 4000, photo 4000 x 6000 and 900 x 1200: `from_edit(to_edit(s))` equals `s` within 1e-3, and a frame point mapped natively then scaled to edit equals the frame's edit point mapped by the edit spot); `close_to_uses_the_corners`; `spot_is_camel_case` (`{"a":1.0,"b":0.0,...}`).
  - `area.rs`: `warp_identity_is_the_area`; `warp_shift_moves_the_bounds`; `warp_half_keeps_thin_lines` (a 1 px diagonal line at scale 0.5 has a set pixel in every target row it crosses); `warp_double_fills_blocks` (one pixel at scale 2 gives exactly a 2 x 2 block); `warp_quarter_turn` (an L of 5 x 3 pixels turned 90 degrees about its centre equals the L rotated by hand); `warp_clips_to_target` (shift so half lies outside: only the inside half, no panic); `warp_empty_is_empty`; `warp_degenerate_spot_is_empty`; `from_edit_bytes_rejects_a_wrong_length`. The existing `Area` tests move with it.
- [ ] **Step 2: Run** `cargo test -p puw-app --release --lib`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p puw-app --release`, clippy, fmt. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Add spots and draw a shape at a spot"`

### Task 4: The board in the session

**Files:**
- Create: `crates/puw-app/src/board.rs` (plain data and its rules, no file reads), `crates/puw-app/src/core/board.rs` (an `impl AppCore` block)
- Modify: `crates/puw-app/src/session.rs` (`Session.board: Board`; `clear()` and `remove(id)` reach it), `crates/puw-app/src/core.rs` (`add_paths` keeps the board, only the old groups are cleared; `remove_photo` removes the photo from the board), `crates/puw-app/src/lib.rs`
- Test: `crates/puw-app/tests/board.rs`, unit tests in `board.rs`

**Interfaces:**
- Consumes: Task 3 `Spot`, `Area::warp`, `Area::from_edit_bytes`.
- Produces (crate-private, `board.rs`):
  - `struct Frame { size: (u32, u32), photo: Option<(PhotoId, PathBuf)> }`; `struct SavedFrom { id: String, name: String }`; `struct Learned { on: Option<PhotoId>, spot: Spot, template: Arc<WatermarkTemplate> }` (the template was built on photo `on` with the shape at `spot`; a match transform `T` gives the spot `Spot::from_transform(T)` after `spot`, i.e. `learned.spot.then(&from_transform(T))`; for a saved watermark used as it is `on` is `None` and `spot` is the identity).
  - `struct Group { number: u32, frame: Frame, shape: Arc<Area> /* at the frame's edit size, as the editor sends it */, saved: Option<SavedFrom>, edited: bool, learned: Option<Learned> }`. `learned` is dropped whenever the shape changes, and only then.
  - `struct Member { group: u32, spot: Spot, start: Spot, unsure: bool, score: f32 }` (`start` = where matching found it, or where it was dropped; `score` = match score, 1.0 for a frame photo or a hand drop).
  - `struct Board { groups: Vec<Group>, members: HashMap<PhotoId, Member>, current: Option<u32>, next_number: u32, looked: HashSet<PhotoId>, owned: HashSet<PhotoId>, unpicked: HashSet<PhotoId>, found: HashMap<(PhotoId, u32), (Spot, f32)>, undo: Vec<UndoStep>, finding: bool, revision: u64 }` with `const UNDO_DEPTH: usize = 50`. An `UndoStep` holds only what one action of the person changed: each touched photo's membership before it (`Option<Member>`), `current` before it, and the groups it created. Undo puts back exactly those photos, so it never takes back what finding did in the meantime. Shapes are not in it.
  - `owned`: photos the person placed or settled (moved by `move_photos`, settled by `show_photo` or a drop, spot changed by `set_spot`, `move_all` or `reset_spot`, put back by `undo_move`). Finding never moves an owned photo (Task 5).
- Produces (public, exported from `puw_app`; all `#[serde(rename_all = "camelCase")]`):
  - `pub enum MoveTo { Group(u32), NotGrouped, NewGroup }` (Deserialize; JSON `{"group":2}`, `"notGrouped"`, `"newGroup"`).
  - `pub struct FrameSize { pub width: u32, pub height: u32 }`.
  - `pub struct BoardPhoto { pub id: PhotoId, pub unsure: bool, pub moved: bool, pub picked: bool, pub spot: Spot /* edit spot */ }`.
  - `pub struct BoardGroup { pub number: u32, pub saved_name: Option<String>, pub edited: bool, pub has_drawing: bool, pub can_save: bool, pub frame: FrameSize /* edit size */, pub photos: Vec<BoardPhoto> }`. `can_save = has_drawing && !(saved && !edited)`.
  - `pub struct BoardState { pub groups: Vec<BoardGroup>, pub not_grouped: Vec<PhotoId>, pub still_looking: Vec<PhotoId>, pub unreadable: Vec<PhotoId>, pub current: Option<u32>, pub can_undo: bool, pub unsure: u32, pub no_drawing: Vec<u32>, pub finding: bool, pub revision: u64 }`. `still_looking` = new photos in no group while finding runs (empty otherwise; they show in `not_grouped` when no finding runs). Groups in creation order, photos in list order; a group is listed when it has photos or is current.
  - `AppCore` (plain calls, no job): `board(&self) -> BoardState`; `set_current(&self, group: Option<u32>) -> Result<()>`; `show_photo(&self, id: PhotoId) -> Result<()>` (settles its "?"); `move_photos(&self, ids: &[PhotoId], to: MoveTo) -> Result<Option<u32>>` (the new group's number for `NewGroup`); `undo_move(&self) -> Result<bool>`; `group_shape(&self, group: u32) -> Result<Vec<u8>>` (frame edit size, 0/255); `put_shape(&self, group: u32, w: u32, h: u32, bytes: Vec<u8>) -> Result<()>`; `set_spot(&self, group: u32, id: PhotoId, edit: Spot) -> Result<()>`; `move_all(&self, group: u32, delta: Spot /* frame edit coordinates */) -> Result<()>`; `reset_spot(&self, group: u32, id: PhotoId) -> Result<()>`; `set_picked(&self, ids: &[PhotoId], picked: bool) -> Result<()>`.
  - `AppCore::clear_board(&self) -> Result<()>` (job): no groups, nothing looked at, picks and undo cleared, group numbers start at 1 again.
  - `#[doc(hidden)] pub fn place_for_tests(&self, id: PhotoId, group: u32, spot: Spot /* native */, unsure: bool)`: joins as matching would (sets `found` too).

Rules (spec calls 3, 5, 8, 10, 11 made exact):
- `move_photos` to `Group(n)`: a photo already in `n` keeps its spot and loses its "?" (dropping settles it). Otherwise it leaves its group and gets `found[(id, n)]` when matching found it there before (no "?"), else the dropped spot below with "?". The frame photo of `n` gets its own spot (identity when it was never moved) and no "?".
- Dropped spot (spec "another size" rule): base = the frame photo's spot when it is a member of `n`, else identity. With `R` = frame size, `P` = photo size, `c` = centre of the shape's bounds mapped by base (the frame centre for an empty shape), `s = min(Pw / Rw, Ph / Rh)`: spot = base, then shift by `-c`, then scale `s`, then shift to `(cx Pw / Rw, cy Ph / Rh)`. For `P == R` this is base.
- `NotGrouped`: the photo leaves its group (its `found` entries stay). `NewGroup`: group `next_number` with an empty shape and the first id's photo and size as frame, all ids join (first: identity, no "?"; others: dropped spot with "?"), it becomes current, and its number is returned.
- `undo_move` puts back the last `UndoStep` (pushed by `move_photos`, `set_spot`, `move_all`, `reset_spot` and, in Task 5, `look_for_more`), returns false when there is none. Cleared by `clear_board`.
- `put_shape` binarises, checks the size against the frame's edit size (`InvalidInput`), replaces the shape, drops `learned`, sets `edited` for a saved group when the bytes differ, and never touches members, spots or "?".
- `set_spot` stores `Spot::from_edit`. `reset_spot` sets `spot = start`. `move_all` turns `delta` into frame native coordinates (`Ef then delta then Ef^-1`) and sets every member's `spot = delta_native.then(&spot)`. `moved = !spot.close_to(start, frame size, 0.5)`.
- `set_picked` changes only members; `picked = !unpicked.contains(id)`, so photos are picked until the person unpicks them, and the choice follows the photo.
- `unsure` counts members with "?"; `no_drawing` lists listed groups with photos and an empty shape. Unreadable photos are never members; reading failures in later tasks remove them from their group.
- `add_paths` keeps the board (new photos are not looked at and show in `not_grouped`, or in `still_looking` while finding runs); `remove_photo` removes the photo from its group, picks and `found`; a frame photo removed from the list keeps its path in the frame.
- Every change bumps `revision`.

- [ ] **Step 1: Write failing tests** in `tests/board.rs` (no models). Every test calls `assert_counts(&core)`: the sum of group photos, `not_grouped`, `still_looking` and `unreadable` equals `photos().len()`, and no id appears twice.
  - `new_group_from_a_drop`: `fixed` 00 and 01 to `NewGroup` gives `Some(1)`, `current == Some(1)`, 00 with the identity spot and no "?", 01 with "?", `no_drawing == [1]`.
  - `numbers_are_never_reused`: empty group 1 by dragging out, `set_current(None)`: group 1 is not listed; the next `NewGroup` is 2.
  - `empty_current_group_stays_listed`.
  - `drop_onto_its_own_group_settles_it`.
  - `drop_back_uses_the_found_spot`: `place_for_tests(01, 1, shift(30, 0), true)`, to `NotGrouped`, back to group 1: spot equals `shift(30, 0)` and no "?".
  - `drop_scales_the_reference_spot`: frame `random` 00 (1536 x 1024) with a painted shape; `random` 01 (1024 x 1536) dropped in: shape centre lands at the same relative position (within 1 px) and the spot's scale is 2/3 on both axes; a 1536 x 1024 photo gets the frame photo's spot exactly.
  - `painting_never_resorts`: `put_shape` leaves members, spots, "?" and `not_grouped` equal.
  - `shape_reads_back_as_sent`: `group_shape` after `put_shape` returns the same bytes (binarised); a wrong size is an error.
  - `move_on_this_photo_and_put_back`: `set_spot` gives `moved`; `reset_spot` gives `!moved` and `spot == start`.
  - `move_on_all_photos_is_relative`: members with spots identity and `scale(2, 2)`; `move_all(shift(10, 0))` in frame edit coordinates (frame of edit size equal to native) gives edit spots `shift(10, 0)` and `shift(10, 0).then(scale(2, 2))` within 1e-3.
  - `undo_restores_moves`: a drop, a `set_spot` and a `move_all`, three `undo_move` calls give the first state; a fourth returns false; `can_undo` follows.
  - `undo_touches_only_the_persons_photos`: a drop of photo A, then `place_for_tests` joins photo B (as finding would), then `undo_move`: A is back, B stays.
  - `person_changes_own_the_photo`: after `move_photos`, `show_photo` on a "?", `set_spot` or `undo_move`, the photo is in `owned` (checked through a `#[doc(hidden)] pub fn owned_for_tests(&self) -> Vec<PhotoId>`); `place_for_tests` does not own.
  - `picks_follow_the_photo`: all picked at first; unpick one, move it to another group, it stays unpicked.
  - `blockers_list_unsure_and_no_drawing`.
  - `adding_photos_keeps_the_board` and `removing_a_photo_leaves_its_group`.
  - `unreadable_photos_are_listed_apart` (a broken JPEG added, `board().unreadable == [id]` after a failed `edit_image`).
  - Unit test in `board.rs`: `board_state_is_camel_case` (keys `notGrouped`, `stillLooking`, `canUndo`, `noDrawing`, `savedName`, `hasDrawing`, `canSave`, `finding`; `MoveTo` parses all three JSON forms).
- [ ] **Step 2: Run** `cargo test -p puw-app --release --test board`. Expected: FAIL.
- [ ] **Step 3: Implement** the types and rules. The old session fields and calls stay as they are.
- [ ] **Step 4: Run** `cargo test -p puw-app --release`, clippy, fmt. Expected: PASS (old tests included).
- [ ] **Step 5: Commit** `git commit -m "Keep the groups board in the session"`

### Task 5: Finding the groups in the background

**Files:**
- Create: `crates/puw-app/src/core/find.rs`, `crates/puw-app/src/match_cache.rs`
- Modify: `crates/puw-app/src/jobs.rs` (`ProgressEvent::Finding`; the background lane of `JobSlot`), `crates/puw-app/src/messages.rs` (`AppError::FindingRuns`), `crates/puw-app/src/core.rs` (`match_cache` field, locking notes), `crates/puw-app/src/board.rs` (joins from finding, `still_looking`, `finding`), `crates/puw-app/src/lib.rs`
- Test: `crates/puw-app/tests/find.rs`, `crates/puw-app/tests/background.rs`, unit tests in `match_cache.rs`, `find.rs` and `jobs.rs`, `crates/puw-app/tests/timings.rs` (`real_finding_rounds`)

**Interfaces:**
- Consumes: Task 1 `MatchImage`, `prepare`, `match_in`, `MatchBasis::PiecesApart`, `saved_placement`; Task 2 `FindProfile`, `GuessOptions`; Task 4 board, `Learned`, `found`, `owned`, `UndoStep`.
- Produces:
  - `ProgressEvent::Finding { fraction: f32, groups: Vec<GroupCount>, left: u32, revision: u64 }` and `#[serde(rename_all = "camelCase")] pub struct GroupCount { pub number: u32, pub photos: u32 }`. `left` = new photos not in a group yet; `revision` = the board's revision after the change the event reports.
  - `#[serde(rename_all = "camelCase")] pub struct FindReport { pub stopped: bool, pub capped: bool, pub rounds: u32, pub new_groups: u32 }`; `pub const ROUND_CAP: u32 = 6;` (a round = one new sample guessed).
  - `AppCore::find_groups(&self, ev: Events) -> Result<FindReport>`: the background job. `#[doc(hidden)] pub fn find_groups_with(&self, ev: Events, guess: &(dyn Fn(PhotoId, &LoadedPhoto, &SetLayers) -> puw_core::Result<Guess> + Sync)) -> Result<FindReport>`: the same, with `guess` instead of the finder (no model is loaded). `find_groups` loads the finder models (`LoadingModels`, `LoadPurpose::Finder`) only when a round needs a first guess, then takes `FindProfile::light()` when the report puts the segmenter on the processor, else `full()`. `#[doc(hidden)] pub fn set_find_profile(&self, p: Option<FindProfile>)` overrides it for tests and timings.
  - `JobSlot::start_background(&self) -> Result<Job>`: the finding's slot. While it is held, `JobSlot::start` returns `AppError::FindingRuns` (user message `Waiting for the search to finish.`), not `Busy`. Plain calls never touch the slot. Stop by ticket works as for any job.
  - `AppCore::look_for_more(&self, group: u32, ev: Events) -> Result<u32>` (job): returns how many photos joined.
  - `MatchCache` (`match_cache.rs`): `get_or_prepare(&self, id, path) -> Result<Arc<MatchImage>>` style API behind `AppCore.match_cache: Mutex<MatchCache>` (held only to look up or insert), budget `MATCH_CACHE_BYTES: usize = 800_000_000`, oldest-first eviction, `forget(id)`, `clear()`. Cleared by `clear_photos`, `forget` on `remove_photo`. `#[doc(hidden)] pub fn photos_prepared(&self) -> usize` counts preparations; `#[doc(hidden)] pub fn set_match_cache_budget(&self, bytes: usize)` lets tests shrink the budget.

The finding, in this order (spec "How the groups are made"; calls 9 and 11):
1. New photos (readable, not looked at, list order). None: return `FindReport` with zeros at once. Otherwise `board.finding = true` until the call returns (on every path).
2. Existing groups with a drawing (adding photos later): one `read_layers` over all readable photos, then each group's learned template (built when missing: on the frame photo at its member spot, or for a group without a frame photo on its best-scoring member) is matched over the new photos still without a group.
3. Saved watermarks: each is matched first on `spread(remaining, 5)` (the existing `spread` and `SUGGEST_PHOTOS`); only one found (`Found` or `Unsure`) on at least one of them is matched over all remaining new photos. Photos found form one group per saved watermark (not one per spot), named after it: frame = the template's reference size with no photo, shape = `saved_placement` at its edit size, `learned` = the saved template.
4. Rounds over the remaining new photos:
   - Queued guesses first (FIFO, photos still without a group): each queued photo's own guess is the drawing; the photo is the group's frame. Queued guesses do not count as rounds.
   - Then, unless `rounds == ROUND_CAP` (then `capped = true`), a new round: `read_layers` over the remaining photos, `round_sample(&remaining, &guessed, profile.sample)`, the first guess with `profile.guess` on each sampled photo (each photo is decoded once; its `MatchImage` is made from the same pixels and cached). If every guess of the sample is empty, the search ends. Else the best (`pick_representative`) is the drawing, the others with non-empty guesses are queued.
   - A drawing makes a group: frame = its photo, shape = the guess at the frame's edit size, `learned` built from the decoded photo and the guess. The group is created together with its frame photo and a `Finding` event is sent at once, before its matching pass (the UI opens the groups screen on the first one). The drawing is matched over the remaining photos (except its own) on the matching pool with `PositionPrior` of the round's layers. `Found` joins without "?"; `Unsure` or `PiecesApart` joins with "?". The frame photo gets the identity spot; "?" when no other photo joined (a one-photo group).
   - The loop goes on while photos remain and the last drawing made a group.
5. Every new photo the finding reached is marked looked at; photos not reached (Stop) stay new. Once `finding` is false, both show in `not_grouped`.
- Working alongside the person (spec "Finding in the background"):
  - The session lock is taken only to copy a pass's inputs and to store one result; no photo read, model run, match or encode happens under it, and storing a result is a few map updates and a revision bump. So `board()` and every plain call wait at most for one such store.
  - A result is stored only when the photo is still new, in no group and not `owned`; otherwise it is dropped. `owned` photos leave `remaining` before the next pass. A photo the person dragged out of a group finding made stays where the person put it.
  - Finding writes a group's shape and `learned` only when it creates the group. A pass already running keeps the template it started with; a shape the person paints is never overwritten.
  - Finding pushes no `UndoStep` and never clears undo. A group the person emptied (every photo dragged out while finding runs) is closed to finding: its pass drops further results, so the person's choice is not undone behind their back. Test in `tests/background.rs`: `an_emptied_group_gets_no_more_photos`.
- `round_sample(remaining: &[(PhotoId, (u32, u32))], skip: &HashSet<PhotoId>, cap: usize) -> Vec<PhotoId>` (pure, in `find.rs`): photos in `skip` (already guessed) are left out; sizes are ordered by count, largest first, ties by first appearance; each size's candidates are evenly spaced as `representative_sample`; picks go round-robin over the sizes until `cap`.
- A photo that cannot be read gets its `Problem`, joins nothing and is not guessed again.
- Stop (the job's ticket): finding returns `Ok` with `stopped: true`; groups and joins made so far stay. Any other error is returned; the board keeps what was made.
- `Finding` events: the first has `fraction` 0.0, `fraction` never decreases, at least one event per new group and per photo matched in a pass, the last one has 1.0, and `groups`, `left` and `revision` equal the board when the event was made (suggested estimate: max so far of `(in a group + 0.5 x matched in the current pass) / new photos`).
- `look_for_more`: `NothingToRemove` when the group has no drawing. Pushes an `UndoStep` (filled with the photos it joins), reads layers over the group's members and the readable Not grouped photos, matches the learned template over the readable Not grouped photos (`Matching` events), and joins each found photo only if it is still in Not grouped and not owned when its result arrives. Stop keeps the photos joined so far and returns `Ok(count)`.

- [ ] **Step 1: Write failing tests.** In `tests/find.rs` and `tests/background.rs` (no models), the guess closure returns each photo's truth (`Guess { mask, confidence: 0.9 }`; empty for photos without a watermark) and records every call. Fixtures from `tests/common/mod.rs` (`mixed_set`, `logo_photos`, `synth_truth`, `ring_logo`). Background tests run `find_groups_with` on a thread and act from the test thread; a closure that waits on a channel holds a round where needed.
  - `tests/find.rs`:
    - `mixed_set_makes_two_groups`: one group holds exactly the 8 `fixed` photos, one exactly the 4 logo photos, the 2 plain photos are in `not_grouped`; no photo is guessed twice; calls <= 3 x `rounds`.
    - `moving_watermark_is_one_group`: `fixed` + `random` (20 photos, three sizes): the first group holds at least 19 photos; any other group has one photo with "?".
    - `saved_watermarks_go_first`: the library holds the logo template (built from logos[0] and its truth); group 1 has `saved_name`, holds the 4 logos, `has_drawing`, `!can_save`; the closure is never called with a logo id.
    - `saved_watermark_off_the_sample_is_not_matched_everywhere`: the same library on `fixed` + plain photos: no saved group, and `photos_prepared() <= photos().len()`.
    - `each_photo_is_prepared_once`: after the mixed-set finding, `photos_prepared() <= photos().len()`.
    - `a_guess_found_nowhere_else_is_a_one_photo_group`: a box guess on plain[0] gives a one-photo group with "?".
    - `empty_guesses_end_the_search`: `rounds == 1`, no group, every photo in `not_grouped`.
    - `round_cap_stops_the_search`: 26 photos without a watermark (`fixed/clean`, `random/clean`, `negative/clean`), each guess a box at a place that depends on the id: `capped && rounds == ROUND_CAP`, at most 18 closure calls, counts add up.
    - `light_profile_samples_one_photo_per_round`: with `set_find_profile(Some(FindProfile::light()))`, closure calls <= `rounds` + queued (0 here), and the mixed set still makes the same two groups.
    - `stop_keeps_what_was_found`: cancel from the event callback at the first `Finding` event with a group: `stopped`, that group listed with its frame photo, the rest in `not_grouped`; a second `find_groups_with` sorts the rest and ends with the same groups as an unstopped run.
    - `stop_during_a_first_guess_leaves_no_half_group`: cancel inside the closure call of a round: no group without its frame photo, counts add up.
    - `progress_never_goes_back`: the `Finding` rules above, checked on a Recorder.
    - `new_photos_join_existing_groups`: find on `fixed` 00-03 and logos 0-1; `set_spot` on `fixed` 01; add `fixed` 04-07 and logos 2-3, find again: no closure call for the new photos, groups of 8 and 4, `fixed` 01's spot unchanged.
    - `pieces_apart_join_unsure`: a frame of `fixed/00` with the ring blended at (60, 60) and a drawing of text plus ring; `fixed/01` with the ring at (880, 560) joins with "?" and its spot within 3 px of identity.
    - `look_for_more_takes_from_not_grouped`: after the mixed finding, drag 2 logo photos to Not grouped; `look_for_more(logos group)` returns 2, they are back with "?" only where unsure, and `undo_move` puts them back in Not grouped.
    - `look_for_more_never_moves_placed_photos`: during the pass (from the `Matching` event callback) move one Not grouped photo into group 1; it stays in group 1.
    - `unsure_spots_keep_turn_zero`: every member spot from matching has `b == 0` and `d == 0`.
  - `tests/background.rs`:
    - `first_group_event_arrives_before_the_end`: the first `Finding` event with a group comes while `left > 0`, before `find_groups_with` returns, and `board()` called from that callback lists the group with its frame photo and `finding == true`.
    - `board_calls_run_alongside_finding`: while the closure is held, `board`, `move_photos`, `put_shape`, `set_spot`, `show_photo`, `set_picked` and `set_current` succeed from the test thread.
    - `board_calls_do_not_wait_for_a_matching_pass`: during a matching pass over 40 photos, a test thread calls `board()` in a loop; every call returns within 50 ms, and at least one call sees `left` between its first and last value.
    - `a_drop_during_finding_wins`: while round 1's matching runs, move a still-looking logo photo into group 1 and an unsure member of group 1 to Not grouped; at the end both are where the person put them, though later drawings match them.
    - `settling_during_finding_wins`: `show_photo` on a "?" member during the run; it keeps its group, spot and settled state to the end.
    - `finding_never_changes_a_shape`: `put_shape` on group 1 during its pass; at the end `group_shape(1)` is what the person sent.
    - `undo_never_undoes_finding`: a person's drop during the run, finding joins other photos after it, `undo_move` puts back only the dropped photo.
    - `other_jobs_wait_while_finding`: `look_for_more`, `preview`, `clean`, `add_paths`, `remove_photo`, `clear_board` and `saved_shape` return `Err(AppError::FindingRuns)` while finding runs, and `user_message` gives `Waiting for the search to finish.`; after the end they run.
    - `still_looking_until_the_end`: during the run new photos in no group are in `still_looking`, not `not_grouped`; after it they are in `not_grouped`; counts add up at every event.
  - `match_cache.rs`: `match_cache_stays_in_budget` (budget of 3 images: after 5 inserts, `heap_bytes` sum <= budget, the oldest are gone) and `evicted_photos_are_still_matched` (a core with a test budget of 2 images, mixed-set finding: same groups as with the full budget).
  - `find.rs` unit: `round_sample_one_size` (10 photos of one size, cap 3: the indexes of `representative_sample(10)`), `round_sample_largest_size_first` (sizes A x 6, B x 2, cap 3: `[A, B, A]` picks), `round_sample_skips_guessed`, `round_sample_cap_one`, `profile_follows_the_device` (segmenter on the processor: light; Core ML or DirectML: full).
  - `jobs.rs` unit: `background_lane_refuses_other_jobs_with_finding_runs`.
  - Model tests in `tests/find.rs`: `real_guesses_find_the_fixed_group` (`PUW_MODELS_DIR`): `find_groups` on the mixed set puts the 8 `fixed` photos in one group with no plain photo in it; prints the time. Ignored `profile_recall_on_the_mixed_set`: `find_groups` with `full()` and `light()` on the mixed set plus `random`; prints the share of watermarked photos in the right group and of plain photos in no group, per profile (recorded in Step 5).
- [ ] **Step 2: Run** `cargo test -p puw-app --release --test find --test background`. Expected: FAIL.
- [ ] **Step 3: Implement.** The matching pass follows `match_photos` (half the cores, each result stored as soon as it is ready, one `Finding` or `Matching` event at a time under the counter lock, never under the session lock).
- [ ] **Step 4: Add the real acceptance** `real_finding_rounds` to `timings.rs` (ignored): set-a + set-b, `find_groups` with models warm. Asserts: one group holds all 8 set-a photos, each `Found` or "?", spot within 1 % of identity; the set-b photo is not in it; counts add up; the first group's event came within 40 s. Prints the total time, the time to the first group, rounds, `photos_prepared`, and `F` and `T_B` of Task 2 recomputed with this run's measured round cost (layers + guesses) in place of the modelled one.
- [ ] **Step 5: Run** `PUW_MODELS_DIR=$PWD/models cargo test -p puw-app --release`, the real acceptance and `profile_recall_on_the_mixed_set`, clippy, fmt. Expected: PASS. Add the real rows and the recall per profile to `thresholds.md` "Finding speed (Plan 2c)".
- [ ] **Step 6: Commit** `git commit -m "Find the watermark groups in the background"`

### Task 6: Cleaning picked photos, saving groups, board pictures

**Files:**
- Modify: `crates/puw-app/src/core/remove.rs` (`clean`), `crates/puw-app/src/core/saved.rs` (`save_group`, `saved_shape`, `SavedInfo` size), `crates/puw-app/src/core/board.rs` (`board_image`, `shape_picture`), `crates/puw-app/src/session.rs` (`Made` compares areas by value for the board path)
- Test: `crates/puw-app/tests/remove.rs`, `crates/puw-app/tests/board.rs`

**Interfaces:**
- Consumes: Task 3 `Area::warp`; Task 4 board; Task 5 learned templates and `find_groups_with`.
- Produces:
  - `AppCore::clean(&self, ev: Events) -> Result<RemoveReport>` (job): the picked members of every group, in list order, each with the area `shape.warp(&(Ef^-1 then spot), w, h)` at native size. Same output, progress, Stop, fallback and re-run rules as `remove`, except: a photo whose earlier result was made from an equal area (by value), the same output rule and detail is not cleaned again. `FindingRuns` while finding runs (it is a job), `NotReady` while `unsure > 0` or `no_drawing` is not empty; `NothingToRemove` when nothing is picked. `left_out` is empty and `failed` lists only photos that failed during the run (not picked, not grouped and unreadable photos are not failures).
  - `AppCore::save_group(&self, group: u32, name: &str) -> Result<String>`: saves the learned template (built if missing) with a picture made as `save_watermark` makes it (the template photo around the watermark, tinted). `NothingToRemove` for an unknown group, an empty shape, or a saved group not edited.
  - `AppCore::board_image(&self, id: PhotoId) -> Result<Vec<u8>>`: the grid-size JPEG (`GRID_LONG_SIDE`) with the group's shape tinted at the photo's spot; a photo in no group is plain.
  - `AppCore::shape_picture(&self, group: u32) -> Result<Vec<u8>>`: PNG of the shape's bounds, tinted as `overlay` tints, on transparent, long side at most 96.
  - `AppCore::saved_shape(&self, id: &str) -> Result<Vec<u8>>`: the saved template's `saved_placement` at its edit size, 0/255. `SavedInfo` gains `width` and `height` (that edit size).

- [ ] **Step 1: Write failing tests.**
  - No models (`tests/board.rs`): `clean_waits_for_unsure_and_drawings`; `save_group_rules` (unknown, empty and unedited saved groups give `NothingToRemove`; a painted group saves, and `find` with the saved template on another `fixed` photo is `Found`; an edited saved group saves under a new id and the old one stays); `board_image_shows_the_drawing` (a member's picture differs from a Not grouped photo's inside the shape by mean > 20, and equals the plain grid picture outside it within 3); `shape_picture_is_small_png`; `saved_shape_is_the_saved_drawing` (bytes equal `saved_placement` resized to `width x height`).
  - Models (`tests/remove.rs`): `clean_removes_a_turned_drawing` (`fixed` 00 and 01 grouped with truth guesses; `set_spot` turns 01's drawing 90 degrees about its centre: the result differs from the original inside the turned area by mean > 10 and is unchanged (mean < 2) where only the unturned drawing was); `clean_skips_unpicked_and_not_grouped` (`done` = the picked members, in list order); `clean_again_skips_unchanged` (a second run sends no `Removing` start for those photos; after `put_shape` the group's photos run again; after `set_spot` on one photo only that photo runs again); `clean_with_the_drawing_partly_outside` (a spot shifting half the shape past the right edge: cleaned, no failure, and a second run skips it).
  - Port the Plan 2 removal tests that guard output folders, failures during a run, cancel and the processor fallback so they also run through `clean` (same assertions, board built with `find_groups_with`).
- [ ] **Step 2: Run** the tests. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `PUW_MODELS_DIR=$PWD/models cargo test -p puw-app --release`, clippy, fmt. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Clean picked photos and save each group"`

### Task 7: Commands, API, types, copy and wizard

**Files:**
- Create: `app/src/lib/finding.svelte.ts`, `app/src/lib/finding.test.ts`
- Modify: `app/src-tauri/src/commands.rs`, `app/src-tauri/src/main.rs`, `app/src-tauri/src/protocol.rs`, `crates/puw-app/src/messages.rs` (`UserError.finding_runs`), `app/src/lib/api.ts`, `app/src/lib/types.ts`, `app/src/lib/copy.ts`, `app/src/lib/wizard.svelte.ts`, `app/src/App.svelte` (Settings hidden while finding runs)
- Test: `app/src/test/api.test.ts`, `app/src/test/copy.test.ts`, `app/src/test/wizard.test.ts`, unit tests in `commands.rs`, `protocol.rs` and `messages.rs`

**Interfaces:**
- Consumes: Tasks 4-6.
- Produces:
  - Commands (registered in `main.rs`): jobs `find_groups(on_event, job)` (the background job; its channel keeps sending after the screen that started it is gone), `look_for_more(group, on_event, job)`, `clear_board(job)`, `clean(on_event, job)`; plain `board()`, `set_current(group)`, `show_photo(id)`, `move_photos(ids, to)`, `undo_move()`, `set_spot(group, id, spot)`, `move_all(group, delta)`, `reset_spot(group, id)`, `set_picked(ids, picked)`, `save_group(group, name)`; binary `group_shape(group)` and `saved_shape(id)` (raw `Response` like `get_area`); `put_shape` with a raw body and headers `x-group`, `x-width`, `x-height` (checked like `put_draft`). Plain commands run on a blocking worker without the job slot, so they answer while finding runs.
  - `AppError::FindingRuns` becomes a `UserError` with `finding_runs: true` (`findingRuns` in JSON) and the message `Waiting for the search to finish.`, so a screen can tell a refused job from a failure.
  - Routes: `board/<id>` (`Route::Board`, `core.board_image`) and `shape/<group>` (`Route::Shape`, `core.shape_picture`, PNG); both in `changes()` (no-store).
  - `api.ts`:
    ```ts
    export const findGroups = (onEvent: (e: ProgressEvent) => void) =>
      job((t) => invoke<FindReport>('find_groups', { onEvent: channel(onEvent), job: t }));
    export const lookForMore = (group: number, onEvent: (e: ProgressEvent) => void) =>
      job((t) => invoke<number>('look_for_more', { group, onEvent: channel(onEvent), job: t }));
    export const clearBoard = () => job((t) => invoke<void>('clear_board', { job: t }));
    export const clean = (onEvent: (e: ProgressEvent) => void) =>
      job((t) => invoke<RemoveReport>('clean', { onEvent: channel(onEvent), job: t }));
    export const boardState = () => invoke<BoardState>('board');
    export const setCurrent = (group: number | null) => invoke<void>('set_current', { group });
    export const showPhoto = (id: PhotoId) => invoke<void>('show_photo', { id });
    export const movePhotos = (ids: PhotoId[], to: MoveTo) => invoke<number | null>('move_photos', { ids, to });
    export const undoMove = () => invoke<boolean>('undo_move');
    export async function getShape(group: number): Promise<Uint8Array>; // like getArea
    export const putShape = (group: number, width: number, height: number, area: Uint8Array) => ...; // like putDraft
    export const setSpot = (group: number, id: PhotoId, spot: Spot) => invoke<void>('set_spot', { group, id, spot });
    export const moveAll = (group: number, delta: Spot) => invoke<void>('move_all', { group, delta });
    export const resetSpot = (group: number, id: PhotoId) => invoke<void>('reset_spot', { group, id });
    export const setPicked = (ids: PhotoId[], picked: boolean) => invoke<void>('set_picked', { ids, picked });
    export const saveGroup = (group: number, name: string) => invoke<string>('save_group', { group, name });
    export async function getSavedShape(id: string): Promise<Uint8Array>;
    ```
  - `types.ts`: `Spot { a; b; c; d; e; f: number }`; `MoveTo = { group: number } | 'notGrouped' | 'newGroup'`; `FrameSize { width; height }`; `BoardPhoto`, `BoardGroup`, `BoardState`, `FindReport`, `GroupCount` mirroring Task 4 and 5 field by field; `ProgressEvent` gains `{ phase: 'finding'; fraction: number; groups: GroupCount[]; left: number; revision: number }`; `BoardState` has `stillLooking` and `finding`; `UserError` gains `findingRuns?: boolean`; `SavedInfo` gains `width`, `height`; `ImageRoute` gains `'board' | 'shape'`.
  - `wizard.svelte.ts`: screens `'finding' | 'board' | 'choose'` added; steps `add 1, finding 2, board 3, choose 4, remove 5, done 6` (the old `check` and `checkAll` map to 3 until Task 14); field `shown: { group: number | null; photo: PhotoId | null }`, reset by `restart()`.
  - `finding.svelte.ts` (the background finding outlives every screen; no Tauri import): `export const finding = $state({ running: false, fraction: 0, groups: [] as GroupCount[], left: 0, revision: 0, report: null as FindReport | null, error: null as string | null, note: null as 'done' | 'stopped' | 'capped' | null })`; `startFinding(): Promise<FindReport>` (calls `findGroups`, copies every `finding` event into the store, sets `report` and `note` when the call ends, `error` when it fails; refuses a second start while running); `stopFinding(): Promise<void>` (`stopJob` on the running call, resolves once the call has ended); `closeNote(): void`; `resetFinding(): void` (for `restart()` and tests). `App.svelte` hides Settings while `finding.running`, as it does while `wizard.busy`.
  - `copy.ts` (exact strings; `photos(n)` is the existing helper):
    - `steps`: `['Add photos', 'Finding watermarks', 'Check the groups', 'Choose photos to clean', 'Remove', 'Done']`
    - `addPhotos.heading` `Add every photo you have with this watermark`; `addPhotos.intro` `Also the ones you do not want cleaned. The app learns what the watermark looks like by comparing photos: the more photos with the same watermark, the better it finds it. You pick which photos to clean in step 4.`; `addPhotos.sizeNote` `Photos are compared only with photos of the same size.`; `addPhotos.sizeLabel(w, h, n)` `${w} x ${h}: ${photos(n)}`; `addPhotos.verdict.tooFew(n)` `Too few to compare. Add ${n} more.`; `.start(n)` `Enough to start. Add ${n} more for good results.`; `.good` `Good`; `.veryGood` `Very good`; `addPhotos.findWatermarks` `Find watermarks`; `addPhotos.backToGroups` `Back to the groups`
    - `finding.title` `Finding watermarks`; `finding.line(groups, left)`: `No watermark found yet.` or `Found ${n} watermark(s) so far.`, then `Group ${number}: ${photos(p)}.` per group, then `Still looking at ${photos(left)}.` when left > 0. Spec example: `Found 2 watermarks so far. Group 1: 140 photos. Group 2: 52 photos. Still looking at 38 photos.`; `finding.findFailed` `The watermarks could not be found.`
    - The strip (spec example): `finding.strip(groups, left)` `Finding watermarks: found ${n} so far.` (n = 0: `Finding watermarks: none found yet.`), then the same group and still-looking sentences: `Finding watermarks: found 2 so far. Group 1: 140 photos. Group 2: 52 photos. Still looking at 38 photos.`; `finding.stopLooking` `Stop looking`; `finding.done(g, n)` `Finding done: ${g} group(s), ${photos(n)} not grouped.` (spec example `Finding done: 3 groups, 12 photos not grouped.`; n = 0: `Finding done: ${g} group(s).`); `finding.waiting` `Waiting for the search to finish`
    - `board.banner(n, k)` `Group ${n}: one drawing for all ${k} photos in this row. Painting on any of them changes it on every one of them.` (k = 1: `Group ${n}: one drawing for the photo in this row.`); `board.groupLabel(n, saved)` `Group ${n}` or `Group ${n}: ${saved}`; `board.hasDrawing` `Has a drawing`; `board.noDrawing` `No drawing yet`; `board.notGrouped` `Not grouped`; `board.newGroupRow` `Drop photos here to start a new group`; `board.unsure` `?`; `board.unsureLabel` `Not sure the watermark is here. Click to look.`; `board.moved` `moved by you`; `board.putBack` `Put it back where it was found`
    - `board.moveThis` `Move drawing on this photo`; `board.moveAll` `Move drawing on all photos`; `board.boxHint` `Drag inside the box to move it, a corner to resize (hold Shift to stretch), the round handle to turn.`; `board.copyFrom` `Copy drawing from...`; `board.copyAdd` `Add to this group's drawing`; `board.copyCancel` `Cancel`; `board.copySaved(name)` `Saved: ${name}`; `board.preview` `Preview on this photo`
    - `board.lookFor(n)` `Look for this watermark in the ${n} not grouped photos` (n = 1: `Look for this watermark in the 1 not grouped photo`); `board.looking(done, total)` `Looking in the not grouped photos... ${done} of ${total}`; `board.lookFound(k)` `${photos(k)} added to this group.` (k = 0: `No more photos found.`)
    - `board.moveTo` `Move to group...`; `board.moveToNew` `A new group`; `board.selected(n)` `${photos(n)} selected`; `board.undo` `Undo move`; `board.addMore` `Add more photos`; `board.next` `Next: choose photos to clean`
    - `board.needLook(n)` `${n} photos with a ? need a look: click them or drag them out.` (n = 1: `1 photo with a ? needs a look: click it or drag it out.`); `board.needDrawing(list)` `Group ${n} has no drawing yet: paint the watermark on one of its photos.` (several: `Groups 2, 3 have no drawing yet: paint the watermark on one of their photos.`)
    - `board.capped(r)` `The search stopped after ${r} rounds. Photos it did not reach are in Not grouped.`; `board.stopped` `Finding was stopped. Photos it did not reach are in Not grouped.`; `board.notGroupedPhoto` `This photo is in no group. Drag it into a group, or onto the new group row, to paint its watermark.`; `board.emptyGroup` `This group has no photos yet. Drag photos here.`; `board.loadFailed` `The groups could not be shown.`; `board.moveFailed` `The photos could not be moved.`; `board.shapeFailed` `The drawing could not be kept.`; `board.lookFailed` `The watermark could not be looked for.`
    - `choose.heading` `Which photos should be cleaned?`; `choose.intro` `Photos you do not pick were still used to find the watermark; they are left as they are.`; `choose.picked(n, k)` `${n} of ${k} picked`; `choose.pickAll` `Pick all`; `choose.pickNone` `Pick none`; `choose.pickOne(name)` `Pick ${name}`; `choose.notGrouped(n)` `Not grouped: ${photos(n)}, not cleaned. Back to put them in a group.`; `choose.clean(n)` `Clean ${photos(n)}`
    - `tools.hand` `Hand` and `editor.keys.hand` `H` are added in Task 9.

- [ ] **Step 1: Write failing tests.** `api.test.ts` `boardJobsHaveTheirOwnTickets` (`findGroups`, `lookForMore`, `clearBoard`, `clean` each get a distinct ticket, passed as `job`) and `putShapeSendsRawBytesWithHeaders`; `copy.test.ts` `exactSpecCopy` gains every string above with the example values (the spec's finding line, the strip example, `done(3, 12)`, `needLook(3)`, `needLook(1)`, `lookFor(1)`, `steps`); `wizard.test.ts` `sixSteps` and `restartClearsTheBoardFields`; `finding.test.ts` (api mocked) `eventsFillTheStore`, `noteAtTheEnd` (`done`, `stopped`, `capped` from the report), `secondStartIsRefused`, `stopWaitsForTheEnd`; `messages.rs` `finding_runs_is_its_own_error`; `commands.rs` `shape_headers_are_read`; `protocol.rs` `parses_board_and_shape_routes` and `board_pictures_are_not_cached`.
- [ ] **Step 2: Run** `pnpm -C app test` and `cargo test -p photounwatermark` (the shell crate). Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `pnpm -C app check`, `pnpm -C app test`, `cargo clippy --workspace --all-targets -- -D warnings`. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Add board calls, copy and wizard steps"`

### Task 8: Add photos and Finding watermarks screens

**Files:**
- Create: `app/src/lib/sizes.ts`, `app/src/lib/sizes.test.ts`, `app/src/screens/FindingWatermarks.svelte`, `app/src/test/findingWatermarks.test.ts`, `app/src/test/findingRealApi.test.ts`
- Modify: `app/src/screens/AddPhotos.svelte`, `app/src/App.svelte`
- Test: `app/src/test/addPhotos.test.ts`, `app/src/test/addPhotosRealApi.test.ts`

**Interfaces:**
- Consumes: Task 7 `finding` store (`startFinding`, `stopFinding`), `clearBoard`, `boardState`, copy `addPhotos.*`, `finding.*`.
- Produces:
  - `sizes.ts`: `export interface SizeLine { width: number; height: number; count: number; verdict: 'tooFew' | 'start' | 'good' | 'veryGood'; more: number; fraction: number }` and `export function sizeMeter(photos: PhotoInfo[]): SizeLine[]`: photos with a `problem` left out; sizes as given (upright); sorted by count, largest first, ties by first appearance; `tooFew` below 3 (`more = 3 - count`), `start` for 3-7 (`more = 8 - count`), `good` for 8-11, `veryGood` from 12 (`more = 0`); `fraction = min(1, count / 12)`.
  - AddPhotos: the heading, intro, one line per size (label, a `role="meter"` bar with `fraction`, the verdict text) and the size note. The saved-watermark offer, `suggestSaved`, `startFromSaved` and the Saved watermarks link are gone from this screen. "Groups exist" comes from `boardState()` on mount. "Find watermarks" (disabled with no photo or while adding): with no group, `go('finding')`; with groups (Add more photos), `startFinding()` without waiting for it and `go('board')` at once (the strip shows the work there; with no new photo it ends at once). With groups, "Back to the groups" goes to `'board'`.
  - FindingWatermarks (step 2, only reached with an empty board): calls `startFinding()` once on mount and shows the store: `copy.loading.finder` with the load fraction while models load, then one bar with `finding.fraction` and `finding.line(groups, left)`. As soon as the store has a group, or the call has ended (also with no group, or stopped), it goes to `'board'` without stopping anything; finding goes on in the background. Back (before that): `await stopFinding()`, then `clearBoard()`, then `go('add')`. A failure before the hand-over shows the message with Back (and "Download again" for missing data).
- [ ] **Step 1: Write failing tests.**
  - `sizes.test.ts`: `verdictsAtTheThresholds` (counts 2, 3, 7, 8, 11, 12 give tooFew/1, start/5, start/1, good, good, veryGood), `largestSizeFirst`, `photosThatCannotBeOpenedDoNotCount`, `uprightSizesAsGiven`.
  - `addPhotos.test.ts`: `headingIntroAndMeter` (two sizes with exact lines), `findGoesToFinding` (empty board), `addMoreFindsInTheBackground` (with groups: `findGroups` called once, screen `'board'` while the call is pending, Back to the groups shown), `noSavedOfferAnyMore` (`suggestSaved` never called), `findDisabledWithoutPhotos`, `noBannedWordsRendered`.
  - `findingWatermarks.test.ts`: `liveLineAndBar` (`No watermark found yet. Still looking at 38 photos.` and `aria-valuenow`), `loadingSentenceFirst`, `handsOverAtTheFirstGroup` (a `finding` event with one group: screen `'board'` while the call is still pending, `cancelJob` never called), `findingGoesOnAfterTheHandOver` (later events still reach the store after the screen is gone), `endWithoutAGroupGoesToTheBoard`, `backStopsAndDrops` (`cancelJob` with the call's ticket, then `clearBoard`, screen `'add'`), `failureShowsBack`, `noBannedWordsRendered`.
  - `findingRealApi.test.ts` (real `api.ts`, mocked `invoke`): `backClearsOnlyAfterTheStopIsTaken` (`clear_board` is invoked after `cancel_job` resolved and the job ended).
  - `addPhotosRealApi.test.ts`: replace the saved-offer ticket tests with `findStartsTheFindingJobOnce`.
- [ ] **Step 2: Run** `pnpm -C app test`. Expected: FAIL.
- [ ] **Step 3: Implement.** `App.svelte` renders `FindingWatermarks` for `'finding'`.
- [ ] **Step 4: Run** `pnpm -C app check` and `pnpm -C app test`. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Ask for every photo and find the watermarks"`

### Task 9: Editor pieces: Hand, spots, move box

**Files:**
- Create: `app/src/lib/editor/affine.ts`, `affine.test.ts`, `moveBox.ts`, `moveBox.test.ts`, `MoveBox.svelte`
- Modify: `app/src/lib/editor/shared.ts`, `shared.test.ts`, `tools.ts`, `keys.ts`, `keys.test.ts`, `Toolbar.svelte`, `Editor.svelte`, `app/src/lib/copy.ts`, `app/src/screens/CheckWatermark.svelte` (adapter only)
- Test: `app/src/test/editor.test.ts`

**Interfaces:**
- Consumes: Task 7 `Spot`.
- Produces:
  - Hand: `Tool` becomes `'hand' | 'brush' | 'eraser' | 'rect'`, key action `'hand'` (H), `copy.tools.hand = 'Hand'` and `copy.editor.keys.hand = 'H'` replace `tools.move` and `keys.move`.
  - `affine.ts`: `IDENTITY_SPOT`, `apply(m: Spot, x: number, y: number): [number, number]`, `then(m: Spot, n: Spot): Spot` (m first), `invert(m: Spot): Spot | null`, `shift(dx, dy)`, `scaleAbout(sx, sy, cx, cy)`, `turnAbout(radians, cx, cy)`, `corners(r: Rect, m: Spot): [Pt, Pt, Pt, Pt]` (`type Pt = [number, number]`; order nw, ne, se, sw).
  - `shared.ts`: `SharedDrawing.viewOn(m: Spot | null, w, h)`, `carryBack(m: Spot, view, dirty)` (any invertible spot: the corners of `dirty` mapped back, their box clipped and grown by 1 px; each drawing pixel whose centre maps inside `dirty` takes the view's value), `paste(source: Area, m: Spot /* source pixels to drawing pixels */): boolean` (union, one undo step, false when nothing new), `bounds(): Rect | null`. `fromPlacement(p: StripPlacement): Spot | null` keeps the old step 2 working until Task 14 (null for `notFound`; unsure placements stay unpaintable there as today).
  - `moveBox.ts`: `type Handle = 'inside' | 'nw' | 'ne' | 'se' | 'sw' | 'turn'`; `interface Box { corners: [Pt, Pt, Pt, Pt] }`; `boxFor(bounds: Rect, m: Spot): Box`; `hit(box: Box, p: Pt, tolerance: number): Handle | null` (the turn handle sits `tolerance x 3` outside the middle of the top edge, along the box's own up direction); `gesture(handle: Handle, box: Box, from: Pt, to: Pt, free: boolean): Spot`, the change in photo edit pixels: inside = shift; a corner scales about the opposite corner along the box's own axes, by one ratio (projection on the diagonal) unless `free`, each ratio at least 0.05 (no flip); turn rotates about the box centre by the angle between `from` and `to`.
  - `MoveBox.svelte`: props `box: Box`, `view: View`, `onChange(d: Spot)` (while dragging), `onEnd(d: Spot)`; draws the box, four corner handles and a round turn handle in SVG over the photo; Shift read from the pointer event.
  - `Editor.svelte`: prop `overlay?: Snippet<[View]>` drawn over the photo with the current view; while it is given, brush, eraser and rectangle do nothing (Hand, zoom, Space and the magnifier still work). Export `getView(): View`.
- [ ] **Step 1: Write failing tests.** `affine.test.ts`: `thenAppliesLeftFirst`, `invertRoundTrips`, `turnAboutKeepsTheCentre`, `cornersOfATurnedBox`. `shared.test.ts`: the existing cases rewritten with spots, plus `viewOnTurned` (an L turned 90 degrees), `carryBackThroughATurn` (a 20 px disc painted in a view at 30 degrees and scale 0.75, carried back and shown again covers >= 90 % of the disc and adds <= 10 % outside it), `pasteIsAUnionAndOneUndoStep`, `pasteThroughScaleAndTurn`. `moveBox.test.ts`: `insideDragShifts`, `cornerKeepsProportions` (se corner dragged to twice the diagonal: scale 2 about nw, both axes), `shiftCornerStretches`, `turnHandleTurnsAboutTheCentre` (a quarter turn keeps the centre and moves nw to ne), `scaleNeverFlips`, `hitFindsHandlesOfATurnedBox`. `keys.test.ts`: `hIsHand`. `editor.test.ts`: `handToolIsCalledHand` (button "Hand (H)"), `overlayGetsTheView`, `noPaintingUnderAnOverlay`.
- [ ] **Step 2: Run** `pnpm -C app test`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `pnpm -C app check` and `pnpm -C app test`. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Rename the pan tool and add the move box"`

### Task 10: Check the groups: rows, drag and drop, live finding, Next

**Files:**
- Create: `app/src/screens/Board.svelte`, `app/src/lib/board/Rows.svelte`, `app/src/lib/board/MoveMenu.svelte`, `app/src/lib/board/FindingStrip.svelte`, `app/src/lib/board/selection.ts`, `selection.test.ts`, `app/src/lib/board/drag.ts`, `drag.test.ts`, `app/src/lib/board/live.ts`, `live.test.ts`, `app/src/test/board.test.ts`, `app/src/test/boardLive.test.ts`
- Modify: `app/src/App.svelte`

**Interfaces:**
- Consumes: Task 7 `boardState`, `setCurrent`, `showPhoto`, `movePhotos`, `undoMove`, `imageUrl('board' | 'shape', ...)`, copy `board.*`, `finding.*`, the `finding` store (`stopFinding`, `closeNote`), wizard `shown`.
- Produces:
  - `selection.ts`: `interface Selection { ids: PhotoId[]; anchor: PhotoId | null }`; `select(sel: Selection, id: PhotoId, order: PhotoId[], mods: { toggle: boolean; range: boolean }): Selection` (plain click: only `id`; toggle (Cmd on Mac, Ctrl elsewhere): add or remove; range (Shift): from anchor to `id` in `order`, which is every thumbnail of every row in screen order; no anchor: only `id`).
  - `drag.ts`: `DRAG_START_PX = 4`; `class DragTracker { down(id: PhotoId, at: Pt, selection: Selection): void; move(at: Pt): boolean /* true while dragging */; up(): PhotoId[] | null /* null for a click */ }` (a drag of a selected thumbnail carries the selection, else only that photo); `targetAt(rows: { to: MoveTo; rect: { left; top; right; bottom } }[], at: Pt): MoveTo | null`.
  - `live.ts`: `REFRESH_MS = 250`, `ARRIVAL_MS = 2000`; `arrivals(prev: BoardState | null, next: BoardState): { groups: number[]; photos: PhotoId[] }` (groups listed in `next` but not in `prev`, photos in a group in `next` that were in none in `prev`; nothing for the first state); `class Refresher { constructor(load: () => Promise<BoardState>, apply: (s: BoardState) => void, wait = REFRESH_MS); want(revision: number): void /* loads at most once per wait, and once more after the last want; drops answers older than one already applied */ }`.
  - Board screen (rows part): `Board.svelte` holds the editor area (filled by Task 11) above `Rows.svelte`. Rows: each listed group (label: shape picture when it has a drawing, `groupLabel`, `photos(k)`, has or no drawing), then Not grouped (plain thumbnails; unreadable photos at its end with their problem sentence), then the dashed `newGroupRow`. Thumbnails are buttons with the photo name; "?" badge with `unsureLabel`; `moved` note. A row click makes its group current (`setCurrent`). A thumbnail click selects (with modifiers), makes its group current and shows it (`wizard.shown`, `showPhoto`, board refreshed). Names and problems come from `listPhotos()`. Drags use pointer events and `DragTracker`; drop targets get `data-drop-target`; a drop calls `movePhotos`; a `NewGroup` drop makes that group current and shows the dropped photo. "Move to group..." (enabled with a selection) opens `MoveMenu` (`role="menu"`: every listed group, Not grouped, A new group; Escape closes, focus returns). "Undo move" calls `undoMove`. Footer: "Add more photos" (`go('add')`), Next (`go('choose')`) disabled while `unsure > 0` or `noDrawing` is not empty, with `needLook` and `needDrawing` shown next to it. Back goes to `'add'`.
  - Live finding (spec "Finding in the background"): while `finding.running`, `FindingStrip` sits above the rows: a bar (`finding.fraction`), `finding.strip(groups, left)` and "Stop looking" (`stopFinding`). The Not grouped row also says `photos(left)` are still being looked at. When the call ends the strip shows `finding.done(g, n)` (or `board.stopped`) for 6 s and goes away; `board.capped(r)` stays until "Close" (`closeNote`). A failure shows `finding.findFailed` with Close. Every `finding` event with a newer `revision` calls `refresher.want(revision)`; the applied state keeps the selection (ids still on the board), the current group, the shown photo, a drag in progress (the tracker and its drop targets survive the update) and the open menu. Arrivals get `data-arrived` for `ARRIVAL_MS`. While finding runs, "Add more photos", Back and Next are disabled with `finding.waiting` next to them (Undo move, moves and the menu work). A job refused with `findingRuns` shows its message next to the button that started it, never as a failure.
- [ ] **Step 1: Write failing tests.** `selection.test.ts`: `plainClickSelectsOne`, `toggleAddsAndRemoves`, `rangeFromTheAnchorAcrossRows`, `rangeWithoutAnchorSelectsOne`. `drag.test.ts`: `aSmallMoveIsAClick`, `dragCarriesTheSelection`, `dragOfAnUnselectedThumbCarriesOnlyIt`, `targetAtFindsTheRow`. `board.test.ts` (api mocked): `rowsInOrderThenNotGroupedThenNew`, `labelShowsCountDrawingAndSavedName`, `unsureBadgeAndMovedNote`, `clickSelectsAndShows`, `modifierClicksSelect`, `dragToAGroupMovesTheSelection` (pointer down, move over the row past 4 px, up: `movePhotos([2, 3], { group: 2 })`), `dropOnTheDashedRowMakesAGroup`, `targetsHighlightWhileDragging`, `moveToMenuWithTheKeyboard` (Tab to a thumbnail, Space selects, open the menu, choose Not grouped: `movePhotos([id], 'notGrouped')`), `undoMove`, `nextWaitsForUnsureAndDrawings` (exact reasons), `nextGoesToChoose`, `addMorePhotos`, `noBannedWordsRendered` (menu open, both reasons shown). `live.test.ts`: `arrivalsFindNewGroupsAndPhotos`, `firstStateHasNoArrivals`, `refresherLoadsAtMostOncePerWait` (fake timers: 10 wants in 250 ms give 2 loads), `refresherDropsOldAnswers`. `boardLive.test.ts` (api and store driven by hand): `stripShowsProgressAndStopLooking` (the spec's strip example; Stop looking calls `cancelJob` with the finding call's ticket), `newRowsAndCountsArriveLive` (a later event adds group 2 and moves photos out of the still-looking count), `arrivalsAreHighlightedBriefly`, `liveUpdatesKeepSelectionAndDrag` (three photos selected and a drag started before an update; the update arrives; the drop still sends `movePhotos` with the three ids), `liveUpdatesKeepTheCurrentGroupAndMenu`, `doneMessageGoesAway` (6 s), `cappedMessageStaysUntilClosed`, `waitingActionsSayWhy` (Add more photos, Back and Next disabled with `Waiting for the search to finish`), `refusedJobShowsTheWaitingReason`, `noBannedWordsRendered` (strip, done and capped messages).
- [ ] **Step 2: Run** `pnpm -C app test`. Expected: FAIL.
- [ ] **Step 3: Implement.** `App.svelte` renders `Board` for `'board'`.
- [ ] **Step 4: Run** `pnpm -C app check` and `pnpm -C app test`. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Show the groups board with drag and drop"`

### Task 11: Check the groups: the editor, painting, look for more

**Files:**
- Modify: `app/src/screens/Board.svelte`
- Create: `app/src/test/boardEditor.test.ts`

**Interfaces:**
- Consumes: Task 7 `getShape`, `putShape`, `editSize`, `putDraft`, `preview`, `lookForMore`, `showPhoto`; Task 9 `SharedDrawing`, `Editor` (`outer`, `onCommit`, `paintable`, `showArea`), `BeforeAfter`, `Waiting`, `DeviceChip`; Task 10 `Refresher` and the `finding` store.
- Behaviour:
  - The editor shows `wizard.shown.photo` of `wizard.shown.group` (the group's first photo when none is shown). A module-level `Map<number, SharedDrawing>` keeps each group's drawing (from `getShape`, at the group's `frame` size) and its undo history for the session; finding never changes a shape, so a live update never reloads it; only `clearBoard` drops the map.
  - The view on a photo is `shared.viewOn(photo.spot, editWidth, editHeight)`. Strokes: `onCommit(dirty)` carries back through the photo's spot, then `putShape(group, frame.width, frame.height, shared.area)`, then the board is read again (thumbnails get the new `revision`). Undo and redo act on the shared drawing (`outer`).
  - Painting is on for a member photo with an invertible spot. A Not grouped photo shows no drawing, painting off, `board.notGroupedPhoto`. An empty current group shows `board.emptyGroup`.
  - A live update keeps the editor as it is: same photo, zoom and pan, no reload of the drawing. When the shown photo's own entry changed (it joined a group, or its spot came from finding), the new view is shown with `showArea` (zoom and pan kept). The banner's count follows the update.
  - "Preview on this photo" and `board.lookFor(n)` are disabled while `finding.running`, with `finding.waiting` next to them.
  - The banner `board.banner(n, k)` sits above the editor. Showing a photo calls `showPhoto` (settles its "?").
  - "Preview on this photo": `putDraft(photo, view)`, then `preview(photo)` with the remover loading sentence, Stop, and `BeforeAfter`, as step 2 does today.
  - `board.lookFor(n)` (current group with a drawing and n > 0 readable Not grouped photos): runs `lookForMore(group, onEvent)` in a `Waiting` with `board.looking(done, total)` and Stop (`stopJob`); afterwards `board.lookFound(k)` and the board is read again.
- [ ] **Step 1: Write failing tests** in `boardEditor.test.ts`: `editorShowsTheGroupOnTheShownPhoto` (area equals `viewOn(spot)`), `paintingChangesTheDrawingForEveryPhoto` (a stroke on a photo with spot `scale(2, 2)` sends `putShape` with the stroke halved), `bannerSaysOneDrawingForAll`, `undoCoversStrokesOnAnyPhoto`, `notGroupedPhotoCannotBePainted`, `showingAPhotoSettlesIt`, `paintingNeverMovesPhotos` (no `movePhotos`, `setSpot` or `moveAll` after strokes), `previewSendsTheView`, `lookForMoreRunsAndCanStop` (Stop calls `cancelJob` with its ticket; the found note), `liveUpdatesKeepTheEditorView` (zoom 200 % and a pan, then an update that changes other photos: same photo, same view, `getShape` not called again), `shownPhotoJoiningAGroupShowsItsDrawing` (a Not grouped photo on screen joins group 1 in an update: its drawing appears, zoom kept), `previewAndLookForWaitForTheSearch`, `noBannedWordsRendered`. Move here the editor behaviour tests of `editor.test.ts` that are not about the strip or photo mode (`toolShortcutsSwitchTool`, `magnifierToggleWithM`, `holdSpaceHidesRedAndReleaseShowsIt`, `spaceDragPansWithoutPainting`, `plainWheelMovesThePhotoAndCtrlWheelZooms`, `safariPinchGesturesZoomOnceAndAreRemoved`, Hand drag, `noStrokesWhileTheNextPhotoLoads`, `shortcutLabelsComeFromCopy`), rendered on the board.
- [ ] **Step 2: Run** `pnpm -C app test`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `pnpm -C app check` and `pnpm -C app test`. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Paint one drawing per group on the board"`

### Task 12: Check the groups: Move drawing tools and Copy drawing from

**Files:**
- Create: `app/src/lib/board/CopyFrom.svelte`, `app/src/test/boardMove.test.ts`
- Modify: `app/src/screens/Board.svelte`

**Interfaces:**
- Consumes: Task 7 `setSpot`, `moveAll`, `resetSpot`, `getShape`, `getSavedShape`, `listSaved` (`width`, `height`); Task 9 `affine.ts`, `moveBox.ts`, `MoveBox`, `SharedDrawing.paste`, `Editor.overlay`.
- Behaviour:
  - "Move drawing on this photo" and "Move drawing on all photos" are toggles (`aria-pressed`) for a member photo of a group with a drawing. While one is on, the editor's overlay shows `MoveBox` for `boxFor(shared.bounds(), spot)`; `onChange(d)` shows `viewOn(then(spot, d))`; `onEnd(d)`: this photo calls `setSpot(group, id, then(spot, d))`; all photos call `moveAll(group, then(then(spot, d), invert(spot)))` (the change in drawing terms). The board is read again after each. `board.boxHint` is shown while a tool is on.
  - "Put it back where it was found" shows for a `moved` photo and calls `resetSpot`.
  - "Copy drawing from..." lists the other groups with a drawing (shape picture, `groupLabel`) and the saved watermarks (`imageUrl('saved', id)`, `copySaved(name)`). While `finding.running` the saved watermarks are listed but disabled with `finding.waiting` (reading the library is a job); groups can be copied. Picking one loads its drawing (`getShape(n)` at its frame size, or `getSavedShape(id)` at `width x height`) and places it with `m0 = then(scaleAbout(s, s, 0, 0), shift(cx, cy))`, `s = min(photoW / srcW, photoH / srcH)`, centred (`cx = (photoW - s srcW) / 2`, same for y). The move box works on it as above (proportions kept unless Shift). The red layer shows the view with the copy added. "Add to this group's drawing" calls `shared.paste(source, then(then(m0, d), invert(spot)))` and `putShape`; "Cancel" restores the view. The source group's drawing is never sent.
- [ ] **Step 1: Write failing tests** in `boardMove.test.ts`: `moveOnThisPhotoSetsItsSpot` (drag inside the box by 10 screen px at zoom 100 %: `setSpot` with `c` + 10), `moveOnAllPhotosSendsTheChangeInDrawingTerms` (photo spot `scale(2, 2)`: the same drag sends `moveAll` with a shift of 5), `turnHandleTurns` (`setSpot` with `b != 0`), `putBackResetsTheSpot`, `paintingIsOffWhileMoving`, `copyFromListsGroupsWithADrawingAndSavedWatermarks` (not the current group, not groups without a drawing), `copyFromAddsAUnion` (`putShape` bytes = old OR pasted), `copyFromCancelChangesNothing`, `copyKeepsProportionsUnlessShift`, `copyIsACopy` (no `putShape` for the source group), `copyFromSavedWaitsForTheSearch`, `moveToolsWorkWhileFinding` (`setSpot` and `moveAll` sent while `finding.running`), `noBannedWordsRendered` (tool on, list open).
- [ ] **Step 2: Run** `pnpm -C app test`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `pnpm -C app check` and `pnpm -C app test`. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Move, resize, turn and copy drawings"`

### Task 13: Choose photos to clean, Remove, Done, Saved watermarks

**Files:**
- Create: `app/src/screens/Choose.svelte`, `app/src/test/choose.test.ts`
- Modify: `app/src/screens/Remove.svelte`, `app/src/screens/Done.svelte`, `app/src/screens/SavedWatermarks.svelte`, `app/src/App.svelte`
- Test: `app/src/test/removeDone.test.ts`, `app/src/test/settings.test.ts`

**Interfaces:**
- Consumes: Task 7 `boardState`, `setPicked`, `clean`, `saveGroup`, copy `choose.*`.
- Behaviour:
  - Choose: heading and intro; one block per listed group with photos: label, `choose.picked(n, k)`, Pick all and Pick none (`setPicked` with the block's ids), thumbnails as toggle buttons (`aria-pressed`, `pickOne(name)`, unpicked faded), each click `setPicked([id], !picked)`. Then `choose.notGrouped(n)` when n > 0. "Clean N photos" (disabled at 0) goes to `'remove'`; Back goes to `'board'`. Picks live in the app, so later visits show them as left.
  - Remove: runs `clean` instead of `remove`; everything else as today.
  - Done: save rows from `boardState().groups` with `canSave` and photos (names as today: `editedName(savedName)` for an edited saved group, `groupDefault(base, n)` with several groups, else `base`); saving calls `saveGroup`. No "Left out" list.
  - Saved watermarks: no choose mode and no "Use for these photos" button (rename and delete stay; opened from Settings).
- [ ] **Step 1: Write failing tests.** `choose.test.ts`: `everyPhotoPickedAtFirst`, `pickNoneAndAll`, `toggleOne`, `notGroupedLine`, `cleanButtonCountsAndDisablesAtZero`, `backKeepsTheChoices`, `noBannedWordsRendered`. `removeDone.test.ts`: `removeRunsClean`, `doneRowsFromTheBoard` (edited saved: "Studio (edited)"; unedited saved: no row; two groups: "(group 1)" and "(group 2)"), `savingCallsSaveGroup`, `noLeftOutList`. `settings.test.ts`: `savedWatermarksHasNoUseButton`.
- [ ] **Step 2: Run** `pnpm -C app test`. Expected: FAIL.
- [ ] **Step 3: Implement.** `App.svelte` renders `Choose` for `'choose'`.
- [ ] **Step 4: Run** `pnpm -C app check` and `pnpm -C app test`. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Choose the photos to clean and run on them"`

### Task 14: Remove the peel-off flow

**Files:**
- Delete: `app/src/screens/CheckWatermark.svelte`, `app/src/screens/CheckAll.svelte`, `app/src/screens/Finding.svelte`, `app/src/lib/groups.ts`, `app/src/test/CheckWatermarkStub.svelte`, `app/src/test/checkAll.test.ts`, `app/src/test/checkWatermarkRealApi.test.ts`, the strip and photo-mode cases of `app/src/test/editor.test.ts`, `crates/puw-app/src/core/step3.rs`, `crates/puw-app/src/core/strip.rs`, `crates/puw-app/tests/groups.rs`
- Modify: `app/src/App.svelte`, `app/src/lib/api.ts`, `types.ts`, `copy.ts`, `wizard.svelte.ts`, `editor/shared.ts` (no `fromPlacement`), `app/src/test/*` that still use removed names, `app/src-tauri/src/commands.rs`, `main.rs`, `protocol.rs` (no `Route::Grid`), `crates/puw-app/src/core.rs`, `session.rs`, `core/remove.rs`, `core/saved.rs`, `lib.rs`, `crates/puw-app/tests/session.rs`, `remove.rs`, `common/mod.rs`, `timings.rs`

**What goes:**
- Rust: `detect`, `use_saved`, `suggest_saved`, `group_sample`, `look_at_sample`, `confirm_set_area`, `confirm_new_group`, `confirm_area`, `confirmed_template`, `last_detect`, `layer_photos`, `area`, `start_area`, `match_all`, `photo_states`, `groups`, `clear_groups`, `open_photo`, `save_photo_area`, `set_included`, `removal_blockers`, `to_remove`, `grid_image`, `place_on`, `remove` (`clean` stays), `save_watermark` (`save_group` stays); session `groups`, `last_detect`, `strip*`, `Group`, `Confirmed`, `SavedOrigin`, `Matched`, and `PhotoData` fields `guess`, `matched`, `own`, `group`, `included`, `reviewed` (`draft`, `preview`, `result`, `result_made` stay); types `AreaStatus`, `PhotoState`, `GroupInfo`, `GroupsState`, `MatchSummary`, `DetectOutcome`, `Suggestion`, `StripPlacement`, `Placed`, `LeftOut` and `RemoveReport.left_out`; their commands.
- UI: the `check`/`checkAll` screens and their steps, wizard `current`, `draft`, `outcomePhotos`, `savedMode`, `sameSet`, `startDraft`, `confirmDraft`, `useGroup`, `clearGroups`, `backFromCheck`; api functions and types for the removed commands; copy `checkWatermark.*` (the preview strings the board uses move to `board.*` first), `groups.*`, `checkAll.*`, `addPhotos.offer/yes/no/checkingSaved/find/savedWatermarks`, `saved.useForThese/useOne/useFailed`, `done.leftOut`, `tools.move`, `editor.keys.move`.
- Tests and timings: Plan 2 session tests that still guard live behaviour (photo list, unreadable photos, edit images, drafts, preview, output checks) stay, rewritten without removed calls; `common/mod.rs` keeps `mixed_set`, `logo_photos`, `ring_logo`, `synth_truth` and gains `truth_guess(core) -> impl Fn(...)` for `find_groups_with`. In `timings.rs`: `app_timings` uses `find_groups` instead of `detect` + `match_all` and drops the `place_on` rows; `real_group_from_the_first_guess` is deleted (`real_finding_rounds` replaces it); `real_small_sets_keep_the_set_position` runs `find_groups` on each small set and asserts every photo ends in one group, `Found` or "?", spot within 1 % of identity, none placed elsewhere.

- [ ] **Step 1: Delete and rewrite** as listed. Each removed test that guarded a rule still in force gets its counterpart in the board, find, clean or screen tests first (check against Tasks 4-13; add any missing one).
- [ ] **Step 2: Check nothing is left:** `git grep -nE 'placeOn|matchAll|confirmNewGroup|confirmSetArea|useSaved|groupSample|suggestSaved|photoStates|setIncluded|removalBlockers|savePhotoArea|CheckAll|CheckWatermark|StripPlacement|place_on|match_all|confirm_new_group|confirm_set_area|use_saved|group_sample|suggest_saved|photo_states|set_included|removal_blockers|save_photo_area|grid_image' -- app/src app/src-tauri/src crates` prints nothing.
- [ ] **Step 3: Run** every verification command. Expected: PASS. Run `real_small_sets_keep_the_set_position` and `real_finding_rounds` with `PUW_REAL_DIR`. Expected: PASS.
- [ ] **Step 4: Commit** `git commit -m "Remove the peel-off group flow"`

### Task 15: Timings, checklist and full verification

**Files:**
- Modify: `crates/puw-app/tests/timings.rs` (only if a row is missing), `docs/notes/thresholds.md` ("App timings (Plan 2c, groups board)"), `docs/notes/plan-2-check.md` (new section "12. Groups board")

- [ ] **Step 1: Run** `app_timings`, `real_finding_costs`, `real_finding_rounds` and `synth_finding_costs` (Mac), and read the latest CI `synth_finding_costs` output of the Windows runner; write the rows (aggregates only): finding on the synthetic mixed set, finding on set-a + set-b, time to the first group, `photos_prepared`, `F`, `T_A` and `T_B` per machine and profile with the formulas, the recall per profile, clean of one 24 MP photo with a turned spot, and one line per machine against the targets (first group 40 s Mac / 120 s PC, total 300 s).
- [ ] **Step 2: Add the checklist** to `plan-2-check.md` in the existing `[ ]` style: Add photos heading and meter (2, 3, 8, 12 photos of one size, plus a second size); the Finding screen's live line and Back (result gone); rows, labels, "?" and "moved by you"; drag and drop with Cmd/Ctrl and Shift, drop targets, new group row; keyboard "Move to group..."; Undo move; banner; painting changes every thumbnail; Hand (H); both Move drawing tools with move, resize (Shift) and turn; Put it back; Copy drawing from a group and from a saved watermark; Look for this watermark in the not grouped photos with Stop; Next reasons; Add more photos from the board; Choose step (all picked, Pick none, Back keeps choices); Remove and Done (rows per group, edited saved name, no row for an unedited saved group); Saved watermarks without Use; background finding (the groups screen opens at the first group; strip with bar, live line and Stop looking; rows and counts update live with a short highlight; a drag, a selection, the current group and the editor's zoom and pan survive updates; a photo dragged or settled during finding stays put; painting during finding is kept; Add more photos, Next, Preview, Look for this watermark and Copy from a saved watermark wait with "Waiting for the search to finish"; the done message goes away, the cap message stays until closed); the synthetic 200-photo set written by `synth_finding_costs` with `PUW_SYNTH_SET` timed in the app on the Mac (targets: first group 40 s, all 300 s): ____ s / ____ s; the same synthetic set copied to the Windows test VM and timed there by the controller (targets: first group 120 s, all 300 s): ____ s / ____ s. Real photos are never copied to the VM.
- [ ] **Step 3: Full verification:** `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `PUW_MODELS_DIR=$PWD/models cargo test --workspace --release`, `pnpm -C app check`, `pnpm -C app test`, `pnpm -C app build`. Expected: all PASS. `git grep -nP '[\x{2010}-\x{2015}\x{2212}]' -- docs app/src crates` finds nothing new.
- [ ] **Step 4: Commit** `git commit -m "Record board timings and checks"`

## Calls made in this plan (recorded for review)

1. One group per saved watermark (spots per photo), following the spec rule "same watermark in another spot is the same group" over the phrase "one group per saved watermark and spot".
2. A dropped photo gets "?" whenever its spot is a guess (no match spot for that group is known), even when dropped by hand; dropping a photo onto its own group, or onto a group whose matching found it, settles its "?".
3. The Next reason says "photos with a ?" instead of the spec's example "photos marked ?", because "marked" is a banned word.
4. Finding started from the board looks only at the new photos; photos already in Not grouped are not sampled again (the person uses "Look for this watermark...").
5. Saved watermarks are matched on 5 sample photos first; a saved watermark on only a few photos outside the sample is left to the rounds.
6. Round cap 6 (a round = one new sample of first guesses; queued guesses are free); the cap message is in the board's strip and stays until closed.
7. A photo whose first guess was empty is never sampled again.
8. Board undo covers moves of photos and spots (drops, Move to group, both Move drawing tools, Put it back, Look for this watermark); painting has the editor's own undo per group. No board redo.
9. The shape is stored at the frame's edit size (the bytes the editor sends), so a drawing read back is the drawing sent; spots are native and converted at the boundary.
10. Done drops the "Left out" list and does not list unreadable or unpicked photos as failures; unreadable photos show at the end of the Not grouped row.
11. Group numbers start at 1 again after Back from a finding that started on Add photos (that set's groups are gone).
12. The Saved watermarks link leaves Add photos; the screen opens from Settings only.
13. The Finding screen is shown only when the board is empty. "Find watermarks" reached through Add more photos starts finding in the background and goes straight to the board.
14. While finding runs, new photos not yet sorted are a count ("still looking", in the strip and the Not grouped row), not thumbnails; they become Not grouped photos when finding ends or is stopped.
15. A photo is the person's (finding leaves it alone) once the person moved it, settled its "?", changed its spot or undid a move of it; only looking at a photo without a "?" does not count. A group the person emptied while finding runs gets no more photos from finding (controller ruling: re-filling it would fight the person).
16. Board undo keeps only the photos each action of the person touched, so it never takes back finding's work; finding no longer clears undo.
17. Back from the board, Settings and the Saved watermarks library wait while finding runs (they lead to jobs: adding or removing photos, model settings, reading the library), with the same reason.
18. The light profile (1 sample photo per round, first guess at one scale of 1280 with 1 vote, matching at 1536 unless the budget needs smaller copies) is used whenever the finder runs on the processor, also on an Intel Mac or a Mac whose Core ML failed.
19. The Windows budget is checked on the CI `windows-latest` runner with synthetic 24 MP photos (a stand-in: real photos never leave the Mac), and confirmed by hand on the Windows test VM with the synthetic 200-photo set. "Fewer segmenters" needs a new model export and download asset, so it is outside this plan; if the targets need it, the task stops as BLOCKED.
20. The targets are modelled with the models already loaded once on that machine (warm); the first-ever load (Core ML compile on the Mac) is printed separately and not counted.

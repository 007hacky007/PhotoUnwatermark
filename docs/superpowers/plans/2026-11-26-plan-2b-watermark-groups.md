# PhotoUnwatermark Plan 2b: Watermark Groups

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One shared drawing per watermark in step 2, watermark groups taken one at a time from the photos that have no group yet, "Use for these photos" on saved watermarks, and one removal run that handles every group.

**Architecture:** `puw-core` gains one thing: a match also says how the reference photo maps onto the photo it was found on (`Transform`). `puw-app` keeps the groups in the session (each with its template, where it came from and whether its matching finished; each photo records which group its result came from) and adds the strip placement, the leftovers' first guess, saved-watermark groups and per-group removal and saving. In the UI, a pure TypeScript module (`editor/shared.ts`) holds the shared drawing at the reference photo's edit size and shows it on each strip photo through that photo's placement. The wizard keeps track of the current group and of a draft (a new group still being drawn).

**Tech Stack:** as Plan 2 (Rust 2024, Tauri 2, Svelte 5 runes, TypeScript, Vitest, `@testing-library/svelte`). No new dependencies.

**Spec:** `docs/superpowers/specs/2026-11-25-watermark-groups-design.md`. Its calls 1-14 are decided. It sits on top of `docs/superpowers/specs/2026-11-23-photounwatermark-design.md`, which still applies wherever the new spec says nothing. Plan 2: `docs/superpowers/plans/2026-11-25-plan-2-desktop-app.md`. Plan 2 rulings R1-R49: `.superpowers/sdd/2026-11-25-plan-2-desktop-app/progress.md`.

## Global Constraints

- Work on `master`. The git identity is already set. Do not push. Commit messages: one short imperative line under ~60 chars, no body, no trailers, no AI attribution, no command substitution.
- Never commit real photos or real photo names/paths/descriptions. Real-photo runs read `PUW_REAL_DIR` and write only under the gitignored `local/`. Docs get aggregate numbers only.
- Copy only in `app/src/lib/copy.ts`. `api.ts` is the only Tauri importer. Banned UI words: mask/mark/unmark and their inflections, plus no Unicode dashes (`assertPlainText` from `app/src/test/plainText.ts`). Every new or changed screen test file keeps a `noBannedWordsRendered` case that covers the new text. UI nouns: "watermark area", "group".
- Per-call job tickets: every long call goes through `job()` in `api.ts` and is stopped with `stopJob(call)` (`jobs.svelte.ts`), never with a global cancel. Every new `AppCore` call that reads photos, runs a model or reads the library holds the job slot (`self.jobs.start()`) and runs in the shell through `self::job(job, ...)`.
- Timing budget: a new group costs one sample first guess (the `representative_sample` of the photos with no group, at most 3 photos; none for a saved watermark) plus one matching pass over the photos with no group, and nothing more. No first guess and no matching ever runs over grouped or unreadable photos.
- Plan 2 rules still hold: processing never runs on the UI thread, every area is binarised at the API boundary (R29), one job runs at a time, Core ML on Apple silicon with processor fallback. Rulings R1-R49 stand except for these overrides: R19 (`area()` no longer falls back to the step-2 guess; `start_area()` does), R18c (batch layers are checked and read over the photos being matched), R31 (a saved watermark goes straight to step 3 with no wait screen), R41(3) ("Saved" is tracked per group), R49 (unreadable photos are left out of Y in "Found in X of Y").
- Terms used in every task. A photo is a **member** of group N when it is ticked, readable, its non-empty area came from group N, and it is not an unsure photo that nobody opened. A **leftover** (no group yet) is a readable photo that is not a member. Group numbers are 1, 2, 3 in creation order and are never reused within a set. A draft gets no number until it is confirmed or a saved watermark is used. The **current group** is the last one created.
- Verification commands: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace --release` (model tests with `PUW_MODELS_DIR=$PWD/models`), `pnpm -C app check`, `pnpm -C app test`, `pnpm -C app build`. Each task runs the commands for the layers it touched. Task 9 runs all of them.
- Implementers cannot click the app (R29). Manual checks go into Task 9's checklist.

## Review Focus

1. A strip photo with a different size or orientation from the reference (portrait next to landscape, another camera): the shared drawing must land on the watermark, and edits must come back to the right place even when the x and y scales differ. Tests: Task 2 `placement_crosses_sizes`, Task 6 `roundTripAtUnevenScale`.
2. An edit near the border of a strip photo while part of the shared drawing lies outside that photo: the outside part must survive. Test: Task 6 `carryBackKeepsPartsOutsideTheView`.
3. "Looks right" pressed while a strip photo other than the reference is on screen: the group must be learned from the reference photo and the whole shared drawing, never from the view on screen. Test: Task 6 `looksRightFromAStripPhotoUsesTheSharedDrawing`.
4. Unticking, opening, saving and making new groups in any order must never lose a photo. At every point, the members of all groups plus the leftovers plus the unreadable photos add up to the photo count. Test: Task 1 `counts_always_add_up`.
5. Back or Cancel anywhere on the leftovers' path (while saved watermarks are checked, during the first guess, from a draft in step 2) must leave groups, ticks and step-3 results as they were. Tests: Task 3 `stopped_leftover_guess_changes_nothing`, Task 7 `backDuringTheNextGuessKeepsTheGroups`.

---

### Task 1: Groups in the session

**Files:**
- Modify: `crates/puw-app/src/session.rs`, `crates/puw-app/src/core.rs`, `crates/puw-app/src/core/step3.rs`, `crates/puw-app/src/core/saved.rs` (reads the current group instead of `Session.template`), `crates/puw-app/src/lib.rs`
- Modify tests: `crates/puw-app/tests/common/mod.rs` (shared helpers), `crates/puw-app/tests/session.rs` (helpers moved out; detect tests read `start_area`)
- Create: `crates/puw-app/tests/groups.rs`

**Interfaces:**
- Produces (crate-private, `session.rs`):
  - `struct SavedOrigin { id: String, name: String }`
  - `struct Group { number: u32, template: Arc<WatermarkTemplate>, confirmed: Option<Confirmed>, saved: Option<SavedOrigin>, edited: bool, outcome: Option<DetectOutcome>, matched: bool }`. `confirmed` is `None` only for an unedited saved watermark. `matched` means this group's matching over the photos with no group has finished.
  - `Session.groups: Vec<Group>` (the last one is current) and `Session.last_detect: Option<DetectOutcome>` replace `template` and `confirmed`. `PhotoData.group: Option<u32>` is the group the photo's result or own area came from. `Matched.confidence: f32` is the match score.
  - `Session::current() -> Option<&Group>`, `is_member(id) -> bool`, `ungrouped() -> Vec<Unmatched>` (readable leftovers, in list order), `clear_groups()`.
  - `Session::area(id)` = own area, else matched area (no guess). `Session::start_area(id)` = the step-2 guess when the photo has one and is not a member, else `area(id)`.
- Produces (public, `puw_app`):
  - `PhotoState` gains `group: Option<u32>` and `member: bool`.
  - `#[serde(rename_all = "camelCase")] pub struct GroupInfo { pub number: u32, pub photos: u32, pub saved_name: Option<String>, pub edited: bool, pub can_save: bool }`. `photos` counts members. `can_save` = `confirmed.is_some()`.
  - `#[serde(rename_all = "camelCase")] pub struct GroupsState { pub groups: Vec<GroupInfo>, pub current: Option<u32>, pub matched: bool, pub leftovers: u32, pub unreadable: u32 }`.
  - `AppCore::groups(&self) -> GroupsState`. `AppCore::start_area(&self, id: PhotoId) -> Result<Vec<u8>>` (edit size, 0/255). `AppCore::area` becomes own-else-matched.
  - `AppCore::confirm_set_area(&self, id: PhotoId) -> Result<()>`: with no group, makes group 1; otherwise changes the current group's drawing.
  - `AppCore::confirm_new_group(&self, id: PhotoId) -> Result<()>`: makes the next group (group 1 when there is none).
  - `AppCore::clear_groups(&self) -> Result<()>` (job): removes every group and every photo's result, tick, opened state and own area. Guesses, previews and removal results stay.
  - Private `fn ready_for_next_group(&self) -> Result<()>`: `Err(AppError::NotReady)` while the current group's matching is unfinished or `removal_blockers()` is not empty; `Ok` when there is no group. `confirm_new_group` uses it, and so does Task 3.

Rules (the spec's calls 4, 9, 10 and 11, made exact):
- Either confirm call drops every draft and every step-2 guess. It sets the group's `outcome` from `last_detect` when that sample contains `id`, else to `{ sample: [id], representative: id }`. Photo `id` becomes Found in that group.
- A new group first resets every leftover: `matched`, `own` and `group` are cleared, and the photo is unticked and marked as not opened. Members of earlier groups are not touched.
- Changing the current group k resets every photo whose result came from k by matching. Own areas saved in k stay, earlier groups stay as they are, and `matched` becomes false.
- No-op rule (Plan 2 fix A2, per group): confirming on a photo with no own area, whose result came from the current group as Found, with a drawing equal to that area, changes nothing.
- `match_all` matches only readable photos with no result (status Pending), using the current group. It sets `matched = true` when it finishes, and matches nothing when `matched` is already true. A photo that fails to read gets no group. `MatchSummary` counts only photos whose result came from the current group.
- `set_included(id, false)` on a member of an earlier group drops its area and its group at once (it becomes Pending). `set_included(id, true)` needs `group.is_some()` and a non-empty area, else it returns `NothingToRemove`. Unticking a current-group photo keeps its result (it can be ticked again while the group is current) and makes it a leftover.
- `save_photo_area(id)`: the photo keeps an earlier group if it has one, else it joins the current group. Saving an empty area on an earlier-group photo drops it the same way unticking does. Returns `NotReady` when there is no group.
- `removal_blockers()` returns ticked unsure photos nobody opened, plus the readable Pending photos while the current group's matching is unfinished. Once matching has finished, a Pending photo is a leftover and does not block.
- `add_paths` and `remove_photo` call `clear_groups` when a group exists. `Session::clear` ("Start a new set") also clears groups and `last_detect`.
- `layers_stale(size, todo: &[PhotoId])` and `read_layers(job, ev, ids, paths)` look only at the photos being matched, not the whole list.

- [ ] **Step 1: Move the shared helpers and add a two-watermark fixture** to `tests/common/mod.rs`. Move these from `session.rs`: `add_synth`, `synth_truth`, `confirm_truth`, `rect_area`, `state`, `matching`, `Recorder`, `noop`, `iou`. Add:
  - `put_truth(core, id, &GrayImage)`.
  - `settle(core)`: unticks every ticked unsure photo nobody opened.
  - `ring_logo() -> GrayImage`: ring with outer radius 40, inner radius 27, and a 5 px bar through the middle, as in `puw-core/tests/template.rs::two_piece_layer`.
  - `logo_photos(dir, n) -> Vec<(PathBuf, GrayImage)>`: takes `negative/clean/00..n`, blends the logo in white at alpha 0.3 at `[(60, 60), (900, 40), (80, 560), (880, 560)]`, and writes each photo as PNG together with its truth.
  - `mixed_set(core, dir) -> Mixed { fixed: Vec<u32>, logos: Vec<u32>, plain: Vec<u32> }`: fixed/marked (8 photos), 4 logo photos, and negative/marked 04 and 05.

  The synthetic `random` set has the same text as `fixed`, so it is never used as a second watermark.
- [ ] **Step 2: Write failing tests** in `tests/groups.rs` (no models). Every test calls `assert_counts(core)` after each step: the sum of `GroupInfo.photos`, plus `leftovers`, plus `unreadable`, must equal `photos().len()`. That check is `counts_always_add_up` (Review Focus 4), run as its own test over the flow of `next_group_matches_only_photos_without_a_group`.
  - `first_confirm_makes_group_one`: `confirm_truth` on fixed[0]. Then `groups().groups == [GroupInfo { number: 1, photos: 1, saved_name: None, edited: false, can_save: true }]`, `current == Some(1)` and `!matched`. After `match_all`: `matched`, every fixed photo has `member` and `group == Some(1)`, and `leftovers == 6`.
  - `next_group_matches_only_photos_without_a_group`: group 1 from fixed[0], then `match_all` and `settle`; keep a copy of the fixed photos' states. Then `put_truth(logos[0])` and `confirm_new_group(logos[0])`: `current == Some(2)`. `match_all` with a Recorder: the `Matching` total is 5. The fixed photos' states equal the saved copy. logos[1..] are Found and members of group 2. Both plain photos are `NotFound` with `group == Some(2)`, not members, and `leftovers == 2`.
  - `unticking_an_earlier_group_photo_drops_it`: while group 2 is current, `set_included(fixed[3], false)` gives `Pending`, `group == None` and `!has_area`. `set_included(fixed[3], true)` returns `Err(NothingToRemove)`. The photo is not in `removal_blockers()`, and `leftovers` grew by 1.
  - `unticking_a_current_group_photo_can_be_undone`: untick logos[1]: `group == Some(2)`, `!member`, `leftovers` grows by 1. Tick it again: `member`.
  - `next_group_rematches_unticked_photos`: group 1 from fixed[0], then untick fixed[2]. Making group 2 from logos[0] gives a `Matching` total of 6 (5 + fixed[2]). fixed[2] ends with `group == Some(2)` and `NotFound`.
  - `new_group_waits`: `confirm_new_group` returns `Err(NotReady)` before group 1's `match_all`. For the unsure case, use a core with `random` 05 and 10 plus the logo photos, and make group 1 from 05's truth (10 then scores unsure, as in `unsure_blocks_until_opened_or_unticked`). `confirm_new_group(logos[0])` returns `Err(NotReady)` while 10 is ticked and not opened. After `open_photo(10)` it succeeds.
  - `unchanged_confirm_keeps_each_group`: two matched groups; untick logos[2]. `confirm_set_area(logos[0])` with the same area (draft = `area()`) leaves `photo_states()` unchanged, and `confirmed_template()` is the same `Arc` (`Arc::ptr_eq`). A changed area makes logos[1] Pending and leaves every fixed photo unchanged.
  - `saving_a_photo_joins_the_right_group`: `save_photo_area` on plain[0] with a painted area gives `Edited`, `group == Some(2)` and member. On fixed[1] the photo keeps `Some(1)`.
  - `unreadable_photos_never_join_a_group`: add a broken JPEG (as in `unreadable_photos_get_their_own_state`). After `match_all` it has `group == None` and `!member`. `groups().unreadable == 1`, and it is not counted in `leftovers`.
  - `groups_clear_when_the_list_changes`: with two groups, `add_paths` of one more photo empties `groups().groups` and leaves every state Pending and unticked. `clear_groups()` on its own does the same.
- [ ] **Step 3: Run** `cargo test -p puw-app --release --test groups`. Expected: FAIL (compile errors on the new API).
- [ ] **Step 4: Implement** the types and rules above. Membership is computed in one place (`Session::is_member`) and used by `states()`, `groups()` and `ungrouped()`. Update the Plan 2 tests that read the first guess through `area()` so they use `start_area()`. `reconfirm_clears_matches_keeps_edits` keeps expecting `removal_blockers() == [ids[0]]` (Pending before matching).
- [ ] **Step 5: Run** `cargo test -p puw-app --release` and `cargo clippy --workspace --all-targets -- -D warnings`. Expected: PASS.
- [ ] **Step 6: Commit** `git commit -m "Keep watermark groups in the session"`

### Task 2: Place the shared drawing on a strip photo

**Files:**
- Modify: `crates/puw-core/src/template.rs`, `crates/puw-core/src/pipeline.rs`, `crates/puw-core/src/lib.rs`, `crates/puw-app/src/session.rs` (`MatchResult` in tests, strip cache), `crates/puw-app/src/lib.rs`
- Create: `crates/puw-app/src/core/strip.rs` (an `impl AppCore` block, as `step3.rs`)
- Test: `crates/puw-core/tests/template.rs`, `crates/puw-core/tests/pipeline.rs`, `crates/puw-app/tests/groups.rs`

**Interfaces:**
- Consumes: Task 1 `Session::start_area`, the drafts, `Session.layers`.
- Produces:
  - `pub struct Transform { pub scale_x: f32, pub scale_y: f32, pub dx: f32, pub dy: f32 }` in `puw_core::template` (Debug, Clone, Copy, PartialEq), re-exported as `puw_core::Transform`. Also `Transform::IDENTITY` and `Transform::apply(&self, x: f32, y: f32) -> (f32, f32)` = `(x * scale_x + dx, y * scale_y + dy)`. It maps the reference photo's native pixels onto the searched photo's native pixels, with (0, 0) at the top-left corner of the first pixel.
  - `MatchResult.transform: Option<Transform>`. `None` for `NotFound`, `IDENTITY` for `MatchBasis::SetPosition`. Otherwise it comes from the anchor, the largest searched piece (the one carried pieces follow): per axis, `scale = p.s_ref * f` and `d = (pl.x - p.ox) * f`, with `f = pl.k * n / t` as in `paste`.
  - `#[serde(rename_all = "camelCase")] pub enum Placed { Found, Unsure, NotFound }` and `#[serde(rename_all = "camelCase")] pub struct StripPlacement { pub status: Placed, pub scale_x: f32, pub scale_y: f32, pub dx: f32, pub dy: f32 }`. It maps the reference photo's edit-image pixels onto the edit image of photo `id`. Scale and shift are 0 for `NotFound`.
  - `AppCore::place_on(&self, reference: PhotoId, id: PhotoId) -> Result<StripPlacement>` (job). It returns where the shared drawing (the reference's draft, else `start_area(reference)`) lands on `id`, found the same way step 3 finds it: `build_template` on the reference photo with that drawing, then `match_photo` with the session's layers (`SetLayers::none()` when there are none).
    - `id == reference`: Found with the identity, no photo read.
    - An empty drawing, or a template without pieces: `NotFound` without reading `id`.
    - Native to edit coordinates: `scale_x_e = scale_x * (w_r / ew_r) * (ew_p / w_p)` and `dx_e = dx * ew_p / w_p` (y the same way).
    - A photo that cannot be read gets its `Problem` and returns the error.
  - The last template is kept in `Session.strip: Option<(PhotoId, Arc<Area>, Arc<WatermarkTemplate>)>` and rebuilt only when the reference or the drawing changes. `#[doc(hidden)] pub fn strip_templates_built(&self) -> usize` counts the rebuilds.

- [ ] **Step 1: Write failing tests.**
  - puw-core `tests/template.rs`:
    - `transform_maps_the_reference_onto_the_found_mark`: random set, template from 00's truth. For every other photo `find` returns Found, and the bbox of 00's truth mapped through `transform` (its two corners) has IoU >= 0.8 with that photo's truth bbox. Portrait photos included.
    - `fixed_set_transform_is_identity`: for every photo, `|scale_x - 1|` and `|scale_y - 1|` <= 0.01, and `|dx|`, `|dy|` <= 3.
    - `not_found_has_no_transform`: negative set.
  - puw-core `tests/pipeline.rs`: `prior_finds_faint_fixed_mark` also asserts `transform == Some(Transform::IDENTITY)` for its `SetPosition` results.
  - puw-app `tests/groups.rs` (no models):
    - `placement_on_the_reference_is_identity`.
    - `placement_follows_the_drawing`: `put_truth(logos[0])`, then `place_on(logos[0], logos[1])` is Found, and logos[0]'s truth bbox mapped through it has IoU >= 0.8 with logos[1]'s.
    - `placement_crosses_sizes`: reference `random` 00 (landscape 1536), targets 01 (portrait 1536) and 03 (portrait 1024). Both Found, mapped bbox IoU >= 0.8 (Review Focus 1).
    - `empty_drawing_is_not_found_anywhere`.
    - `placement_on_a_photo_without_it`: plain[0] gives `NotFound`.
    - `template_is_kept_until_the_drawing_changes`: two `place_on` calls with the same draft give `strip_templates_built() == 1`. After a new draft it is 2.
- [ ] **Step 2: Run** `cargo test -p puw-core --release --test template --test pipeline` and `cargo test -p puw-app --release --test groups`. Expected: FAIL.
- [ ] **Step 3: Implement** `Transform`, the field (set at every `MatchResult` construction site), `Placed`, `StripPlacement`, `place_on` and the cache.
- [ ] **Step 4: Run** `cargo test --workspace --release` and clippy. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Place the shared drawing on each strip photo"`

### Task 3: Saved watermark groups and the leftovers' first guess

**Files:**
- Modify: `crates/puw-app/src/core.rs` (`detect`, `use_saved`, `suggest_saved`, `group_sample`), `crates/puw-app/src/session.rs`, `app/src-tauri/src/commands.rs` (`use_saved` wrapper only, so the workspace still compiles)
- Test: `crates/puw-app/tests/groups.rs`, `crates/puw-app/tests/session.rs` (replace `use_saved_skips_models`)

**Interfaces:**
- Consumes: Task 1 `ready_for_next_group`, `Session::ungrouped`, `Group`, `last_detect`, the reset of leftovers for a new group.
- Produces:
  - `AppCore::detect(&self, ev: Events) -> Result<DetectOutcome>` works on the photos with no group (all photos when there is no group). It runs `ready_for_next_group()?`, loads the finder models, reads the batch layers from those photos only (stored with their ids), and makes the first guess on their `representative_sample`. The guesses replace all earlier guesses and the outcome is stored as `last_detect`. Groups, ticks and results do not change.
  - `AppCore::use_saved(&self, id: &str) -> Result<u32>`. It runs `ready_for_next_group()?`, resets the leftovers as for a new group, then makes the next group from saved watermark `id` with `confirmed: None`, `saved: Some`, `outcome: None` and `matched: false`. It drops guesses and drafts, reads no photo and loads no model, and returns the new group's number. Step 3's `match_all` then does the matching.
  - `AppCore::suggest_saved` tries up to 5 photos spread over the photos with no group.
  - `AppCore::group_sample(&self) -> Result<DetectOutcome>` returns the current group's step-2 photos, which is its `outcome`. A saved group without an outcome gets one, computed once and stored: up to 3 members found by matching, highest `confidence` first, with the first as representative. If no member was found, it takes the first (up to 3) photos whose result came from the group. Returns `NothingToRemove` when there is no group.
  - Confirming a saved group with a changed drawing sets `confirmed` and `edited = true` (so `can_save` is true). Confirming it unchanged hits Task 1's no-op rule, so `edited` stays false.
  - `commands.rs`: `use_saved(id, job) -> u32` with no channel. Its `api.ts` side is done in Task 5.

- [ ] **Step 1: Write failing tests.** Library entries are built with `build_template` on a photo and its truth, then `SavedWatermark::new` and `Library::save` (see `save_fixed_watermark` in `session.rs`).
  - No models:
    - `use_saved_starts_group_one`: with no group, `use_saved` returns 1, and `device_status().state` is still `NotStarted`.
    - `use_saved_makes_the_next_group`: save a watermark from logos[0]'s truth. Make group 1 from fixed, then `match_all` and `settle`. `use_saved` returns 2. Group 2 has `saved_name == Some(name)`, `can_save == false`, `matched == false`. `match_all` gives a `Matching` total of 6 with no `Looking` and no `LoadingModels` events, and every logo photo ends up a member of group 2.
    - `use_saved_waits`: `Err(NotReady)` before group 1's matching has finished.
    - `saved_group_sample_is_its_best_photos`: the sample is a subset of group 2's Found members, has at most 3 photos, and contains the representative. A second call returns the same value.
    - `unchanged_saved_group_stays_saved`: `put_draft(rep, area(rep))` and then `confirm_set_area(rep)` leave `edited == false` and `can_save == false`. A changed drawing gives `edited == true` and `can_save == true`, and `saved_name` is kept.
    - `suggest_looks_only_at_photos_without_a_group`: the library holds the fixed template and the logo template. With group 1 made from fixed and settled, `suggest_saved` returns the logo one. With only the fixed template in the library, it returns `None`.
  - Models (`PUW_MODELS_DIR`):
    - `next_group_first_guess_costs_one_sample`: group 1 from fixed, matched and settled. `detect` with a Recorder gives a `Looking` total of 3, every sample id in logos plus plain, and at least one `Reading` event. `groups()` and `photo_states()` are unchanged. Every sample photo has an empty `area`, and at least one has a non-empty `start_area` (its first guess). Then `put_truth` on a logo photo of the sample (logos[0] if none is in it), followed by `confirm_new_group` on that photo and `match_all`: the `Matching` total is 5. Print both durations with `eprintln!`.
    - `stopped_leftover_guess_changes_nothing`: stop `detect` by its ticket after the first `Looking` event (same pattern as `cancel_detect`). The call returns `Err` with `Cancelled`, and `photo_states()` and `groups()` equal what they were before (Review Focus 5).
- [ ] **Step 2: Run** `cargo test -p puw-app --release --test groups --test session`. Expected: FAIL.
- [ ] **Step 3: Implement**, and rewrite `use_saved_skips_models` for the new return value.
- [ ] **Step 4: Run** `PUW_MODELS_DIR=$PWD/models cargo test -p puw-app --release` and clippy. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Start groups from saved watermarks and leftovers"`

### Task 4: Removal and saving per group

**Files:**
- Modify: `crates/puw-app/src/core/remove.rs`, `crates/puw-app/src/core/saved.rs`, `crates/puw-app/src/photos.rs`, `crates/puw-app/src/lib.rs`, `app/src-tauri/src/commands.rs` (`save_watermark` wrapper only)
- Test: `crates/puw-app/tests/remove.rs`, `crates/puw-app/tests/groups.rs`

**Interfaces:**
- Consumes: Task 1 membership, `Group.confirmed`, `ready_for_next_group` rules.
- Produces:
  - `RemoveReport { done, failed, left_out: Vec<LeftOut>, cancelled }` and `#[serde(rename_all = "camelCase")] pub struct LeftOut { pub id: PhotoId, pub name: String }`, exported from `puw_app`.
  - `remove(ev)` runs over the members of every group in list order, each photo with its own area. It returns `NotReady` while `removal_blockers()` is not empty or the current group's matching is unfinished. `left_out` lists the leftovers at the start, in list order. `failed` lists the photos already known to be unreadable at the start first, each with `Problem::message()`, then the photos that fail during the run as before.
  - `Problem::message(self) -> String`: the `user_message` sentence for a missing file (Gone) or a damaged one (Damaged).
  - `AppCore::save_watermark(&self, group: u32, name: &str) -> Result<String>` saves that group's confirmed photo and area. It returns `NothingToRemove` for an unknown group or one without `confirmed` (an unedited saved watermark is already saved).
  - `commands.rs`: `save_watermark(group, name)`.

- [ ] **Step 1: Write failing tests.**
  - No models (`groups.rs`):
    - `removal_waits_for_the_current_matching`: group 2 confirmed but not matched, so `remove` returns `Err(NotReady)` and no event is sent.
    - `each_group_saves_its_own_watermark`: with two groups, `save_watermark(1, "A")` and `save_watermark(2, "B")` both appear in the library. `find` with A's template on fixed/marked/01 is Found, and with B's on logos[1]. A group from `use_saved` returns `Err(NothingToRemove)`.
    - `report_is_camel_case` (unit test in `remove.rs`) now includes `"leftOut":[]`.
  - Models (`remove.rs`):
    - `one_run_removes_every_group`: fixed 00 and 01, logos 0 and 1, one plain photo and one broken JPEG. Group 1 from fixed, group 2 from logos, each matched and settled. `remove` then gives:
      - `done` ids = the 4 members, in list order;
      - each result differs from its original inside that photo's truth area (mean absolute difference > 10);
      - `left_out == [plain]`;
      - `failed` = the broken photo, with the "could not be opened" sentence;
      - `done + failed + left_out` = the photo count.
- [ ] **Step 2: Run** the tests. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `PUW_MODELS_DIR=$PWD/models cargo test -p puw-app --release` and clippy. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Remove and save watermarks per group"`

### Task 5: Commands, API, wizard state and copy

**Files:**
- Modify: `app/src-tauri/src/commands.rs`, `app/src-tauri/src/main.rs`, `app/src/lib/api.ts`, `app/src/lib/types.ts`, `app/src/lib/wizard.svelte.ts`, `app/src/lib/copy.ts`
- Modify call sites (renamed wizard fields only, plus the step-1 "Yes"): `app/src/screens/AddPhotos.svelte`, `CheckWatermark.svelte`, `Done.svelte`
- Test: `app/src/test/api.test.ts`, `wizard.test.ts`, `copy.test.ts`, `addPhotos.test.ts`, and `editor.test.ts` and `removeDone.test.ts` adapted to the new fields

**Interfaces:**
- Consumes: Tasks 1-4 (`confirm_new_group`, `clear_groups`, `groups`, `start_area`, `place_on`, `group_sample`, `use_saved`, `save_watermark`).
- Produces:
  - Commands registered in `main.rs`: `confirm_new_group(id, job)`, `clear_groups(job)`, `place_on(reference, id, job)` as jobs; `groups()` and `group_sample()` as plain calls; `start_area(id)`, which returns a binary body like `get_area`.
  - `api.ts`:
    ```ts
    export const confirmNewGroup = (id: PhotoId) => job((t) => invoke<void>('confirm_new_group', { id, job: t }));
    export const clearGroups = () => job((t) => invoke<void>('clear_groups', { job: t }));
    export const groupsState = () => invoke<GroupsState>('groups');
    export async function getStartArea(id: PhotoId): Promise<Uint8Array>; // like getArea
    export const placeOn = (reference: PhotoId, id: PhotoId) =>
      job((t) => invoke<StripPlacement>('place_on', { reference, id, job: t }));
    export const groupSample = () => invoke<DetectOutcome>('group_sample');
    export const useSaved = (id: string) => job((t) => invoke<number>('use_saved', { id, job: t }));
    export const saveWatermark = (group: number, name: string) => invoke<string>('save_watermark', { group, name });
    ```
  - `types.ts`: `PhotoState.group: number | null` and `PhotoState.member: boolean`; `type Placed = 'found' | 'unsure' | 'notFound'`; `StripPlacement { status: Placed; scaleX; scaleY; dx; dy: number }`; `GroupInfo { number; photos: number; savedName: string | null; edited: boolean; canSave: boolean }`; `GroupsState { groups: GroupInfo[]; current: number | null; matched: boolean; leftovers: number; unreadable: number }`; `LeftOut { id: PhotoId; name: string }`; `RemoveReport.leftOut: LeftOut[]`.
  - `wizard.svelte.ts` (no api imports):
    - `export interface Drawing { group: number; outcome: DetectOutcome | null }`.
    - Fields `groups` (confirmed groups), `current` and `draft` (both `Drawing | null`), `outcomePhotos`, `savedMode: 'manage' | 'choose'`, `savedGroups: number[]`. These replace `outcome` and `watermarkSaved`.
    - `get drawing(): Drawing | null` returns `draft ?? current`, which is what step 2 shows.
    - `startDraft(outcome, photos?)` sets `draft = { group: groups + 1, outcome }`. `photos` sets `outcomePhotos` for group 1.
    - `confirmDraft()` moves the draft to `current` and sets `groups = draft.group`. It does nothing when there is no draft.
    - `useGroup(group, photos?)` sets `current = { group, outcome: null }`, `groups = group` and `draft = null`.
    - `clearGroups()` sets `groups = 0`, `current = null` and `savedGroups = []`, but keeps group 1's drawing as the draft so an unchanged list returns to it.
    - `backFromCheck()`: for group 1 it goes to `'add'` (a group-1 draft is kept); for a draft of group 2 or later it drops the draft and goes to `'checkAll'`; for a reopened current group k > 1 it goes to `'checkAll'`. Step screens move with `go()`. `back()` stays for Settings, Saved and About.
    - `restart()` also clears groups, draft, `savedMode` and `savedGroups`.
    - `sameSet(wizard, photos)` is true when group 1's drawing (draft or current) has an outcome and `outcomePhotos` equals `photos`.
  - `copy.ts` (exact strings; `photos(n)` is the existing helper):
    - `groups.checkHeading(n)` → `Group ${n}: check the watermark`; `groups.checkAllHeading(n)` → `Group ${n}: check all photos`; `groups.label(n)` → `Group ${n}`
    - `groups.noGroup` → `No group yet`; `groups.notInGroup` → `Not in a group yet`
    - `groups.line(list: { number: number; photos: number }[], leftovers: number)` → `Group 1: 140 photos. Group 2: 52 photos. 38 photos have no group yet.` (`1 photo has no group yet.`; the last sentence only when leftovers > 0)
    - `groups.findNext(n)` → `Find the next watermark in these ${n}` (n = 1: `Find the next watermark in this photo`); `groups.useSaved` → `Use a saved watermark...`; `groups.leaveOut` → `Leave them out`
    - `groups.confirmLeftovers(left, grouped)` → `38 photos have no group yet. Remove from the 192 photos in groups and leave the 38 out?` (left = 1: `1 photo has no group yet. ... and leave it out?`); `groups.confirmRemove` → `Remove`
    - `groups.nextFailed` → `The next watermark could not be looked for.`
    - `checkWatermark.notFoundHere` → `The watermark was not found on this photo.`; `checkWatermark.unsureHere` → `Not sure the watermark is here on this photo.`; `checkWatermark.placing` → `Finding the watermark on this photo...`; `checkWatermark.placeFailed` → `The watermark area could not be shown on this photo.`
    - `saved.useForThese` → `Use for these photos`; `saved.useOne(name)` → `Use ${name} for these photos` (accessible name); `saved.useFailed` → `The saved watermark could not be used.`
    - `done.leftOut(n)` → `Left out: ${photos(n)}`; `done.groupName(n)` → `Name for the saved watermark of group ${n}`; `done.groupDefault(base, n)` → `${base} (group ${n})`; `done.editedName(name)` → `${name} (edited)`

- [ ] **Step 1: Write failing tests.**
  - `wizard.test.ts`: `draftBecomesTheCurrentGroup`, `backFromADraftReturnsToStepThree`, `backFromGroupOneGoesToAddPhotos`, `backFromAReopenedGroupReturnsToStepThree`, `clearGroupsKeepsGroupOnesDrawing` (`draft.outcome` is the same object), `sameSetFollowsGroupOne`, `restartClearsGroups`.
  - `copy.test.ts` `exactSpecCopy`: every string above with the example values.
  - `api.test.ts` `groupCallsHaveTheirOwnTickets`: `placeOn`, `confirmNewGroup`, `clearGroups` and `useSaved` each get a distinct ticket (`ticketOf`), and `invoke` receives it as `job`.
  - `addPhotos.test.ts` `yesUsesSavedAndGoesToStepThree` replaces `yesUsesSaved`: `useSaved('s1')` is called, then `wizard.current` equals `{ group: 1, outcome: null }`, the screen is `'checkAll'`, and `detect` is never called.
- [ ] **Step 2: Run** `pnpm -C app test`. Expected: FAIL.
- [ ] **Step 3: Implement.** Update the call sites mechanically:
  - AddPhotos: `done` calls `wizard.startDraft(outcome, ids)`; "Yes" calls `stopCheck()`, then `useSaved(id)`, then `wizard.useGroup(n, ids)` and `go('checkAll')`, with no Finding wait.
  - CheckWatermark reads `wizard.drawing?.outcome`.
  - Done keeps its single save row, calling `saveWatermark(wizard.current?.group ?? 1, name)` and using `wizard.savedGroups` (Task 8 replaces this).
- [ ] **Step 4: Run** `pnpm -C app check`, `pnpm -C app test`, `cargo clippy --workspace --all-targets -- -D warnings`. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Add group calls, wizard state and copy"`

### Task 6: One shared drawing in step 2

**Files:**
- Create: `app/src/lib/editor/shared.ts`, `app/src/lib/editor/shared.test.ts`
- Modify: `app/src/lib/editor/Editor.svelte`, `app/src/screens/CheckWatermark.svelte`
- Test: `app/src/test/editor.test.ts`

**Interfaces:**
- Consumes: Task 5 `placeOn`, `getStartArea`, `putDraft`, `preview`, `confirmSetArea`, `confirmNewGroup`, `clearGroups`, `StripPlacement`; the wizard's `drawing`, `draft`, `groups`, `confirmDraft()`, `clearGroups()` and `backFromCheck()`; copy `checkWatermark.notFoundHere / unsureHere / placing / placeFailed` and `groups.checkHeading`.
- Produces:
  - `shared.ts`:
    ```ts
    export const IDENTITY: StripPlacement; // found, scale 1, shift 0
    /** Undo and redo of a drawing kept outside the editor. */
    export interface OuterHistory { canUndo(): boolean; canRedo(): boolean; undo(): void; redo(): void }
    /** The group's one drawing, at the reference photo's edit size, with its own undo history. */
    export class SharedDrawing {
      constructor(area: Area);
      readonly area: Area;
      get revision(): number;            // grows with every change, undo and redo
      canUndo(): boolean; canRedo(): boolean;
      viewOn(p: StripPlacement, w: number, h: number): Area;          // empty for notFound
      carryBack(p: StripPlacement, view: Area, dirty: Rect): boolean; // one undo step; false for no change or a non-found p
      undo(): boolean; redo(): boolean;
    }
    ```
    `viewOn`: every view pixel centre `(u + 0.5, v + 0.5)` is mapped back with `x = (u + 0.5 - dx) / scaleX` (y the same way), then `floor` picks the drawing pixel; a point outside the drawing gives 0. `carryBack`: map the corners of `dirty` back, clip to the drawing and grow by 1 px. For each drawing pixel in that box, map its centre forward; when it lands inside `dirty`, the drawing pixel takes the view's value there. It commits one `History` step (`history.ts`) on the drawing.
  - `Editor.svelte` gets these props: `outer?: OuterHistory` (when given, the undo and redo buttons and keys use it), `onCommit?: (dirty: Rect) => void` (called after each stroke or rectangle with the changed part of the shown area), and `paintable?: boolean` (default true; when false, brush, eraser and rectangle do nothing, while zoom, pan, Space and the magnifier still work). New method `showArea(a: Area)`: like `setArea`, but keeps zoom and pan.
  - Set mode in `CheckWatermark.svelte`:
    - It works on `wizard.drawing`. A module-level `WeakMap<DetectOutcome, { shared: SharedDrawing; placed: Map<PhotoId, { p: StripPlacement; revision: number }> }>` replaces `editsBySet`, so the drawing and its undo history live as long as the group's outcome does. The drawing starts from `getStartArea(representative)`.
    - Showing a strip photo uses its cached placement when the revision matches. Otherwise it calls `putDraft(reference, shared.area)` and then `placeOn(reference, id)`, showing `copy.checkWatermark.placing` meanwhile with painting off.
    - `found`: the view is shown and painting is on. `unsure`: the view is shown, painting is off, and `unsureHere` is shown. `notFound`: an empty view, painting off, `notFoundHere`, and Preview disabled.
    - A first guess that found nothing opens step 2 with an empty drawing. The reference photo can still be painted, the other strip photos are `notFound`, and "Looks right" shows the existing `emptyArea` hint.
    - `onCommit` calls `carryBack` and then `showArea(viewOn(...))`. A photo's own edits keep its placement (its cache entry takes the new revision).
    - Preview runs `putDraft(current, view)` and then `preview(current)`.
    - "Looks right" runs `putDraft(reference, shared.area)`, then `confirmNewGroup(reference)` for a draft or `confirmSetArea(reference)` otherwise, then `wizard.confirmDraft()` and `go('checkAll')`.
    - Back for group 1 calls `await clearGroups()` and `wizard.clearGroups()` when `wizard.groups > 0`; then, for any group, `wizard.backFromCheck()`.
    - The heading `groups.checkHeading(n)` appears when `n >= 2`.
    - Photo mode does not change.

- [ ] **Step 1: Write failing tests.**
  - `shared.test.ts`:
    - `identityViewIsACopy`.
    - `viewOnMapsThroughScaleAndShift`: 10x10 drawing with (2, 3) set, p = { scale 2, dx 5, dy 1 }, view 30x30: exactly pixels x 9-10, y 7-8 are set.
    - `carryBackAtIdentityIsTheStroke`.
    - `carryBackKeepsPartsOutsideTheView` (Review Focus 2): a drawing with a block that maps outside a view with dx = -40; paint and erase in the view; the outside block is unchanged.
    - `roundTripAtUnevenScale` (Review Focus 1): scaleX 0.5, scaleY 0.75. Paint a 20 px disc in the view and carry it back. `viewOn` again covers >= 90 % of the disc and adds <= 10 % outside it.
    - `undoRedoAcrossPhotos`: one stroke through p1, one through p2. Two undos give the original drawing, one redo brings back the first stroke, and `revision` changes every time.
    - `notFoundPlacementCarriesNothing`.
  - `editor.test.ts`:
    - `stripShowsTheSharedDrawingOnEachPhoto`: `placeOn` is mocked as found with a shift; the editor's area on photo 2 equals `viewOn`.
    - `paintingOnAStripPhotoChangesTheSharedDrawing`: paint on photo 2, then "Looks right" sends `putDraft(reference, ...)` with the stroke mapped back.
    - `looksRightFromAStripPhotoUsesTheSharedDrawing` (Review Focus 3): `putDraft` and confirm both use the reference id and the shared area.
    - `unsureStripPhotoShowsTheDrawingWithPaintingOff`: the note is shown and a pointer stroke changes nothing.
    - `notFoundStripPhotoShowsTheNote`: no red pixels, Preview disabled.
    - `undoCoversEditsOnAnyStripPhoto`.
    - `previewOnAStripPhotoSendsItsView`: `putDraft(2, view)`, then `preview(2)`.
    - `placementIsAskedAgainOnlyAfterTheDrawingChanged`: count the `placeOn` calls.
    - `draftLooksRightMakesTheNextGroup`: `confirmNewGroup`, `wizard.groups == 2`, screen `'checkAll'`.
    - `backFromADraftReturnsToStepThree`.
    - `backFromGroupOneClearsGroupsAndKeepsTheDrawing`: `clearGroups` is called, and showing step 2 again gives the same area and the same undo history.
    - `headingFromGroupTwo`.
    - `emptyFirstGuessOpensEmpty`: `placeOn` is never called for an empty drawing, the reference can be painted, and "Looks right" on an empty drawing shows the hint.
    - `noBannedWordsRendered` also renders the two notes.
    - Replace `stripKeepsEachPhotosEdits`.
- [ ] **Step 2: Run** `pnpm -C app test`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `pnpm -C app check` and `pnpm -C app test`. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Share one drawing across the strip photos"`

### Task 7: Groups and leftovers in step 3, saved watermark Use

**Files:**
- Modify: `app/src/screens/CheckAll.svelte`, `app/src/screens/SavedWatermarks.svelte`, `app/src/screens/AddPhotos.svelte` (Saved watermarks link), `app/src/screens/Settings.svelte` (link sets `savedMode = 'manage'`)
- Test: `app/src/test/checkAll.test.ts`, `app/src/test/settings.test.ts` (Saved watermarks part), `app/src/test/addPhotos.test.ts`

**Interfaces:**
- Consumes: Task 5 `groupsState`, `photoStates` (`group`, `member`), `matchAll`, `suggestSaved`, `detect`, `useSaved`, `groupSample`, `listPhotos`; the wizard's `current`, `groups`, `startDraft`, `useGroup` and `savedMode`; copy `groups.*`, `saved.useForThese / useOne / useFailed`, `addPhotos.checkingSaved / offer / yes / no`; the existing `Finding`, `Waiting` and `Dialog` components.
- Behaviour:
  - Mount: read `groupsState()` and the photo states. Run `matchAll` only when `!matched`.
  - The summary "Found in X of Y photos. Z need a look." counts only states with `group === current` (found = `found`, or `edited` with an area).
  - The heading `groups.checkAllHeading(n)` appears when `n >= 2`.
  - Tiles:
    - A member of an earlier group shows `groups.label(n)` and keeps its tick (unticking it is allowed).
    - Current-group tiles show their badges as today.
    - `group === null` tiles show `groups.noGroup`, a disabled checkbox, and the visible reason `groups.notInGroup`.
    - Unreadable tiles stay as today.
  - With `leftovers > 0`, below the grid:
    - the line `groups.line(...)`;
    - primary `groups.findNext(n)`;
    - `groups.useSaved`, which sets `savedMode = 'choose'` and goes to `'saved'`;
    - `groups.leaveOut`, which goes straight to `'remove'`;
    - `removeFrom(N)` as a secondary button that opens a `Dialog` with `groups.confirmLeftovers` and the buttons `groups.confirmRemove` (to `'remove'`) and Back (closes the dialog).
    - All of these are disabled while blockers exist or a tick is still on its way. With 0 leftovers the footer is as today.
  - "Find the next watermark":
    - First a `Waiting` with `addPhotos.checkingSaved` while `suggestSaved()` runs. Its Back stops the call and returns to the grid.
    - If a watermark is suggested: the offer `addPhotos.offer(name)`. Yes calls `useSaved(id)` and `wizard.useGroup(n)`, then matching runs again in place. No goes on to the first guess.
    - Without a suggestion, `Finding` with `run={detect}`: `onDone` calls `wizard.startDraft(outcome)` and `go('check')`; `onBack` goes back to the grid with nothing changed.
  - Back from step 3: when `wizard.current.outcome` is null, set it from `groupSample()` first, then `go('check')`.
  - Saved watermarks screen, `savedMode === 'choose'`: each row gets a `saved.useForThese` button (accessible name `saved.useOne(name)`). It calls `useSaved(id)`, then `wizard.useGroup(n, n === 1 ? ids : undefined)` with `ids` from `listPhotos()`, then `go('checkAll')`. On failure it shows `saved.useFailed`. In `'manage'` mode there is no Use button.
  - AddPhotos' Saved watermarks link: `await stopCheck()`, then `savedMode = photos.length > 0 ? 'choose' : 'manage'`, then `go('saved')`.

- [ ] **Step 1: Write failing tests.**
  - `checkAll.test.ts`:
    - `earlierGroupTilesShowTheirGroup`.
    - `noGroupTilesCannotBeTicked`.
    - `groupsLineWithLeftovers`: the spec's example sentence.
    - `summaryCountsTheCurrentGroup`.
    - `findNextChecksSavedFirstThenFinds`: `suggestSaved` gives null, then `detect` runs, then `wizard.draft.group == 2` and the screen is `'check'`.
    - `findNextOfferYesUsesSaved`: `useSaved('s1')` is called, `matchAll` is called a second time, and `wizard.groups == 2`.
    - `backDuringTheNextGuessKeepsTheGroups` (Review Focus 5): Back during `suggestSaved` and Back during `detect` each call `cancelJob` with that call's ticket, the grid is shown again, `wizard.draft` is null, and no confirm or `useSaved` call was made.
    - `leaveThemOutGoesToRemove`: no dialog.
    - `removeWithLeftoversAsksOnce`: exact dialog text; Back closes it; Remove goes to `'remove'`.
    - `nextStepsWaitForUnsure`.
    - `useSavedFromStepThreeOpensChooseMode`.
    - `matchesOnlyWhenTheGroupIsNotMatched`.
    - `backFetchesTheSampleOfASavedGroup`.
    - `headingFromGroupTwo`.
    - `noBannedWordsRendered` with leftovers and the dialog open.
  - `settings.test.ts`: `chooseModeOffersUse` and `manageModeHasNoUse`.
  - `addPhotos.test.ts`: `savedLinkStopsTheCheckAndOpensChooseMode` and `savedLinkWithoutPhotosManages`.
- [ ] **Step 2: Run** `pnpm -C app test`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `pnpm -C app check` and `pnpm -C app test`. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Take watermark groups one at a time in step 3"`

### Task 8: Done per group

**Files:**
- Modify: `app/src/screens/Done.svelte`
- Test: `app/src/test/removeDone.test.ts`

**Interfaces:**
- Consumes: Task 5 `groupsState`, `saveWatermark(group, name)`, `defaultWatermarkName`, `RemoveReport.leftOut`, `wizard.savedGroups`; copy `done.leftOut / groupName / groupDefault / editedName`.
- Behaviour:
  - Below the failure list: `done.leftOut(n)` and the names of the photos that were left out.
  - One save row for each group with `canSave`; groups without it (unedited saved watermarks) get no row.
  - Default name: `editedName(savedName)` for an edited saved watermark. Otherwise `groupDefault(base, n)` when there are several groups, else `base` (from `defaultWatermarkName()`).
  - The field label is `done.groupName(n)` when there are several rows, else `done.name`.
  - After a save, that row shows "Saved", and `n` is added to `wizard.savedGroups`, so Settings and Back cannot save it twice.

- [ ] **Step 1: Write failing tests:** `leftOutListed` ("Left out: 2 photos" and both names), `oneSaveRowPerGroup` (defaults "Watermark from 2026-11-26 (group 1)" and "... (group 2)"), `savedGroupWithoutEditsHasNoRow`, `editedSavedGroupDefaultsToTheEditedName` ("Studio (edited)"), `savedShowsPerGroup` (saving group 2 calls `saveWatermark(2, ...)`, only that row shows "Saved", and the state survives Settings and Back), `singleGroupKeepsTheOldDefault`, `newSetClearsGroups`. Keep `noBannedWordsRendered` with two rows and the left-out list.
- [ ] **Step 2: Run** `pnpm -C app test`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `pnpm -C app check` and `pnpm -C app test`. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Show left out photos and a save row per group"`

### Task 9: Timings, checklist and full verification

**Files:**
- Modify: `crates/puw-app/tests/timings.rs`, `docs/notes/thresholds.md` (new section "App timings (Plan 2b, watermark groups)"), `docs/notes/plan-2-check.md` (new section "11. Watermark groups")

- [ ] **Step 1: Extend `app_timings`** with a second-group pass:
  - Synthetic `mixed_set`: `detect` of the leftovers (6 photos, 3 looked at), `match_all` of group 2, `place_on` per strip photo (first call, then cached), and `use_saved` + `match_all` for the logo watermark.
  - Real (when `PUW_REAL_DIR` is set): set-a + set-b as one set gives two groups (group 2 = the set-b photo). Record the leftover `detect`, group 2's `match_all`, and `place_on` on a 24 MP strip photo.

  Run `PUW_MODELS_DIR=$PWD/models cargo test -p puw-app --release --test timings -- --ignored --nocapture`. Write the rows as aggregates only, and add one line checking them against the spec's budget (one sample guess plus one matching pass over the photos with no group).
- [ ] **Step 2: Add the checklist section** to `plan-2-check.md`, in the existing `[ ]` style:
  - mixed set `target/synth/fixed/marked` + the logo photos written by the timings run under `local/timing`;
  - one drawing shown on every strip photo, and painting on one changes all;
  - unsure and not-found strip notes, with painting off;
  - undo across strip photos;
  - the group heading from group 2;
  - earlier-group labels, "No group yet" tiles and unticking an earlier-group photo;
  - the summary line;
  - "Find the next watermark" (saved check first, then the wait; Back keeps everything);
  - "Use a saved watermark..." and "Use for these photos" from Add photos and from step 3;
  - "Leave them out" and the Remove dialog;
  - Done: left-out list and one save row per group;
  - Back from step 2 of group 1 to Add photos clears the groups and keeps the drawing;
  - timings ____ s for the second group.
- [ ] **Step 3: Full verification:** `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `PUW_MODELS_DIR=$PWD/models cargo test --workspace --release`, `pnpm -C app check`, `pnpm -C app test`, `pnpm -C app build`. Expected: all PASS. `git grep -nP '[\x{2010}-\x{2015}\x{2212}]' -- docs app/src crates` finds nothing new.
- [ ] **Step 4: Commit** `git commit -m "Record group timings and checks"`

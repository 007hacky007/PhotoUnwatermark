# Matching thresholds and detection scales (measured)

`puw_core::template` sorts every match into Found / Unsure / NotFound by its score (masked NCC of
edge maps, 0..1):

| constant | value | was |
|---|---|---|
| `FOUND` | 0.48 | 0.45 (Task 12), 0.6 (plan) |
| `UNSURE` | 0.35 | 0.35 |

A wrong Found is the worst outcome: that photo is processed with a misplaced area and nobody looks
at it. Unsure and NotFound are always shown to the user. Both thresholds therefore lean towards
the negatives.

Real photos were read from `PUW_REAL_DIR` with `puw-dev ... --real`, which prints aggregate
numbers only. Nothing from those photos is in the repository.

- Real set 1: 8 photos, 24 MP, all the same upright size (portrait after EXIF orientation), one
  faint fixed-position outline mark.
- Real set 2: 1 photo, 36 MP, large text plus a big semi-transparent logo.

## Synthetic score distributions (Task 12, `score_sweep` test)

Positives are `find` scores on photos that hold the mark, with these templates:
- fixed/01-07 (template fixed/00)
- random/01-11 (template random/00)
- 9 scaled marks
- 4 two-piece marks moved together and 4 moved apart
- 4 twin-piece cases

Results:
- Visible marks: 0.515-0.987.
  - Lowest: 0.515 fixed/05, 0.553 random/10, 0.566 fixed/03, 0.588 fixed/01.
  - The twin-piece cases score 0.610-0.790.
- random/02 scores 0.076. Its mark is near-invisible: white at alpha 0.2 on a bright background,
  a mean luma change of 6.9.
- Negatives: 120 runs, 5 templates x 24 unmarked photos (the negative set plus the clean
  fixed/random photos). Scores 0.064-0.251, with 0 Found and 0 Unsure.

## Synthetic `match-sweep`

`puw-dev match-sweep --set target/synth/<set> --negatives target/synth/negative [--all-refs]`.
The ground-truth mask of the reference photo is the confirmed mask.

| set, references | positives | positive scores | negatives | negative scores | negatives >= 0.45 |
|---|---|---|---|---|---|
| fixed, first | 7 | 0.515-0.640, 7 Found, IoU 1.0 | 6 | 0.174-0.201 | 0 |
| fixed, all 8 | 56 | 0.515-0.948, 56 Found, IoU 1.0 | 48 | 0.075-0.311 | 0 |
| random, first | 11 | 0.076-0.987, 10 Found (IoU >= 0.83), 1 NotFound (random/02) | 6 | 0.064-0.124 | 0 |
| random, all 12 | 132 | 0.074-0.987, 103 Found (IoU >= 0.82), 1 Unsure, 28 NotFound | 72 | 0.050-0.601 | 1 |

Positives at each candidate `FOUND` (all-references runs):
- fixed: 56/56 at 0.45, 0.48 and 0.50.
- random: 103/132 at 0.45, 0.48 and 0.50.

Of the 28 random NotFound cases:
- 22 involve random/02, as the reference or as the target.
- The other 6 pair weaker marks with each other (references 06, 09 and 10).

The one Unsure is reference 05 on random/10: 0.432, IoU 0.85.

The one high negative (0.601, Found under either threshold) happens only with random/02 as the
reference, against negative/04. Its template is essentially background, because the mark is
invisible. The generator crops both photos from the same scikit-image source (index 5), so the
template finds the same scene. This is an artefact of the synthetic data, not of the threshold:
no `FOUND` value that keeps the visible positives (>= 0.515) would block it.

## Real photos

Set 1 has no ground truth. Its batch layer (median-gradient layer, 2.91 % of the photo, 1083
components) served as the confirmed mask. It holds the mark plus static scene edges, which make
6 template pieces: one 0.225 x 0.951 of the photo, and five small ones. `find` ran on the other
7 photos of set 1 (positives), on set 2, and on the 6 synthetic negatives.

Two variants:
- the whole batch layer;
- its largest piece only, standing in for a mask the user cleaned up.

A placement counts as right when its IoU with the template's placement on its own reference is
>= 0.5. The mark is fixed, so that is where it sits.

| variant, references | positive runs | right placements | right-placement scores | wrong-placement scores | statuses |
|---|---|---|---|---|---|
| batch layer, 1 | 7 | 1 | 0.400 | 0.220-0.455 | 3 Unsure, 4 NotFound |
| batch layer, all 8 | 56 | 2 | 0.399-0.400 | 0.077-0.455 | 4 Unsure, 52 NotFound |
| largest piece, 1 | 7 | 2 | 0.399-0.400 | 0.234-0.455 | 1 Found (wrong place), 3 Unsure, 3 NotFound |
| largest piece, all 8 | 56 | 9 | 0.360-0.435 | 0.190-0.455 | 1 Found (wrong place, IoU 0.137), 13 Unsure, 42 NotFound |

Statuses in the table were counted under FOUND 0.45.

Negatives, all runs:
- Set 2 against the set 1 templates: 0.082-0.123.
- Synthetic negatives against the set 1 templates: 0.047-0.206.
- 0 Found and 0 Unsure.

What this shows:
- No threshold works for this mark. A faint outline over a busy background makes the template
  mostly scene edges. Right placements score 0.36-0.44, and wrong ones up to 0.455, so the two
  ranges overlap. Lowering `FOUND` to catch the right placements would also mark wrong
  placements as Found. The real negatives stay low (<= 0.21), but they are not the problem.
- The only real Found in all runs (0.455) was in the wrong place. `FOUND` therefore moves from
  0.45 to 0.48:
  - The real misplacement becomes Unsure, so the user looks at it.
  - The lowest synthetic visible positive (0.515) stays Found, with a margin of 0.035.
  - Every negative stays below 0.35, except the same-scene artefact above. The highest are 0.323
    (synthetic, random all-references sweep), 0.311 (synthetic, fixed all-references sweep) and
    0.206 (synthetic negatives against the real set 1 templates); the highest real negative is
    0.123.
- `UNSURE` stays 0.35. The right real placements (0.36-0.44) land in Unsure, which is the
  wanted outcome for a mark this faint.
- With these thresholds, set 1 is not processed automatically; the user is asked to look at each
  photo. Making matching work for faint fixed marks needs a change to the method, not to the
  thresholds. One candidate: when a set's batch layer holds the confirmed mark, use the fixed
  position. The template and library tests pass with the new value.

## Real-set bench: guess and removal (`puw-dev bench --real`)

Release build on an Apple silicon Mac, all models on the processor.

- Engine creation: 32.4 s, a cold load in a fresh process. Other bench runs measured
  30.9-34.7 s.
- `auto_guess_set` (since replaced by `build_set_layers` + `guess_photo`):

| set | time per photo | coverage % | confidence | empty guesses |
|---|---|---|---|---|
| 1 (8 photos, 24 MP) | 6.4 s | 14.63-15.22 (mean 14.99) | 0.917-0.930 | 0 |
| 2 (1 photo, 36 MP) | 7.5 s | 6.44 | 0.950 | 0 |

Set 1's guess holds all of its batch layer, which is 2.91 % of the photo in 1083 components: the
mark plus static scene edges. Grown by 24 px, that is most of the 15 %.

- `remove_one` with the auto-guess mask, by working long side:

| set | work side | seconds per photo | LaMa tiles per photo |
|---|---|---|---|
| 1 | 4096 | 97.8-113.7 (median 102.1) | 47-55 (median 50) |
| 1 | 2048 | 27.7-38.1 (median 31.4) | 13-19 (median 16) |
| 2 | 4096 | 111.4 | 52 |
| 2 | 2048 | 40.0 | 19 |

That is about 2.0-2.1 s per LaMa tile. At these mask sizes neither working side meets the spec's
~15 s step-2 preview target on the processor. A smaller preview working side, a tighter mask, or a
faster preview model is needed (Plan 2).

## Detection scales (ruling R10 revisit)

`puw-dev bench --real` measured coverage of `prob > 0.5` per scale and per vote. For set 1 it
also measured agreement with the batch layer:
- recall: share of the batch layer found;
- precision: share of detected pixels within the grow radius of the batch layer.

The batch layer includes scene edges, so the absolute values understate both. Use them to
compare scales only.

Set 1 (8 photos, means):

| detector | coverage % | recall % | precision % |
|---|---|---|---|
| 512 | 1.28 | 3.9 | 37.7 |
| 768 | 1.46 | 5.1 | 26.5 |
| 1280 | 1.35 | 12.5 | 53.3 |
| 2048 | 1.31 | 17.0 | 54.3 |
| 2 of 768/1280/2048 (default) | 0.54 | 10.0 | 73.6 |
| 2 of 512/768/1280 | 0.49 | 4.0 | 46.8 |
| 2 of 512/768/1280/2048 | 0.78 | 10.5 | 60.3 |

Set 2 (1 photo), coverage %:

| 512 | 768 | 1280 | 2048 | default vote | 512 instead of 2048 | 2 of 4 |
|---|---|---|---|---|---|---|
| 0.20 | 1.51 | 3.31 | 2.99 | 1.53 | 0.61 | 1.56 |

Recommendation: do not add 512.
- On set 1 it is the weakest scale by recall, and swapping it in for 2048 halves the vote's
  recall and lowers its precision from 74 % to 47 %.
- A fourth scale with a 2-of-4 vote adds 0.5 points of recall for 13 points less precision.
- On set 2, 512 sees only a seventh of what 768 sees.
- The only case for 512 remains the synthetic full-frame line overlay (styles/03, recall 0.175
  with the default scales). The editor covers that case.

## Batch layer parity

Set 1, 8 photos of one upright size: batch layer coverage 2.91 %. The research prototype
measured about 2.9 % on the same set with the same constants.

Since the final fix wave the batch layer comes from `build_set_layers`, which reads photos one
at a time and keeps an evenly spaced sample of each size group as 2-byte luma (`puw-dev layers
--real`): set 1 still gives 2.91 % (1084 components, was 1083), in 4.3 s for the 8 photos.

Peak memory of `build_set_layers` on synthetic 36 MP JPEGs (7360x4912, release build, macOS
peak memory footprint): 0.76 GB for 3 photos, 0.95 GB for 8, 1.53 GB for 40 and 1.53 GB for
200 (the sample stops at 16 photos at this size, `BATCH_LUMA_BUDGET`). 200 photos took 8.3 s.

## Batch-layer filter (Plan 2)

`build_set_layers` now passes every layer through `batch::filter_layer` and drops a layer that
filters to empty. The filter works on the layer's 8-connected components:

| step | constant | value |
|---|---|---|
| drop a component wider or taller than this share of the image (scene lines) | `LINE_SPAN` | 0.6 |
| drop components below this share of the remaining pixels (specks) | `MIN_PIECE_FRAC` | 0.001 |
| group components whose boxes are closer than this share of the long side | `MERGE_FRAC` | 0.02 |
| keep the largest group and every group with at least half its pixels | - | 0.5 |

The last two match the template pieces of `template.rs`. The plan dropped *groups* spanning
more than 60 % of the image. On real set 1 that removes the mark itself: a mark spanning 0.95
of the photo's height, whose separate components group into one piece of 0.225 x 0.951 of the
photo. Each of those components is small, so the span test runs on components instead. A scene
line broken into short pieces would not be caught by it. Set 1 has no component spanning 60 % either way.

What the batch layer of set 1 holds besides the mark: specks of static subject texture
across the set (a dilated single pixel is 49 px; median component 98 px). Grown by the 24 px of
the guess, each becomes a 55 x 55 px square, which is most of the old 15 %. Layer-only sweep of
`MIN_PIECE_FRAC` on set 1 (components spanning 60 % dropped, groups as above; coverage of the
layer grown by 24 px; "mark kept" is the share of the mark's components, picked by hand, that
survives):

| `MIN_PIECE_FRAC` | components left | grown layer % | mark kept |
|---|---|---|---|
| none (unfiltered layer) | 1084 | 13.81 | 100 % |
| 0 (grouping only) | 1084 | 11.86 | 100 % |
| 0.0002 | 361 | 9.24 | 100 % |
| 0.0005 | 132 | 8.41 | 100 % |
| 0.001 | 80 | 8.13 | 100 % |
| 0.002 | 49 | 7.91 | 100 % |
| 0.005 | 31 | 7.16 | 96.8 % |

The smallest of those mark components holds 0.38 % of the layer's pixels, so 0.001 keeps a
margin of about 4x.

Set 1 with `puw-dev guess --real` (Core ML, release build):

| | batch layer | components | guesses | guess coverage % (grown) | guess time |
|---|---|---|---|---|---|
| before (6 guesses) | 2.91 % | 1084 | 6 | 14.46-14.99 (mean 14.79) | 41.1 s |
| after (3 guesses) | 1.80 % | 57 | 3 | 8.89-9.46 (mean 9.18) | 20.2 s |

The representative photo of the old sample went from 14.71 % to 9.18 %. The overlay of the new
representative photo shows every component of the mark inside the layer. What is left besides
the mark is two clusters of subject-texture specks that touch mark components, and the
detector's own guess.

The plan's target of at most 3 % is out of reach while the layer grows with the guess: the
mark's components (with the two speck clusters joined to them), grown by 24 px, cover about
7.2 % of the photo, because the mark spans 0.95 of the photo's height. Adding the layer after
the growth instead of before would give about 3.2 % (estimate from the layer and the overlay).
That is a change to `guess_photo` and needs a removal check on a 3 px border around the mark's
edges first.

Set 2 (1 photo) has no batch layer; its guess stays at 6.40 %.

Synthetic: the stored layer of the fixed set covers 99.8 % of the mark (coverage 1.90 %), and a
layer made of the fixed mark plus a full-width and a full-height 1 px line keeps all of the mark
and none of the lines.

## Position prior (`match_photo`)

`match_photo` calls `find`; when that is not `Found`, the photo has the template's size and the
set has a batch layer for it, and at least 50 % of the saved placement lies in the layer grown
by `grow_for(w, h, 0.004)`, the photo must also show the mark at the saved position. The score
is `find`'s fast path alone (masked NCC, +-2 px, scale 1, minimum over the pieces), exposed as
`template::saved_score`. `PRIOR_FOUND` 0.28 gives `Found`, `PRIOR_UNSURE` 0.16 gives `Unsure`
(both with basis `SetPosition`, confidence = that score); below it `find`'s result stays.
The first version of the rule (layer support alone) marked every same-size photo, with or
without the mark, as `Found`; the score gate is the fix.

Calibration, synthetic fixed set (template of photo 0, the set's own layer):

| photos | n | saved-position score |
|---|---|---|
| faint marked (alpha 0.12), photos 1-7 | 7 | 0.175-0.331 (4 Found, 3 Unsure) |
| clean/ of the same scenes, photos 1-7 | 7 | 0.000-0.155 |
| negative set resized to 1536x1024 | 6 | 0.000-0.102 |

The reference's own clean photo scores 0.836 (the template holds that scene's edges), so it is
not a negative. The margin between the highest negative (0.155) and `PRIOR_FOUND` is 0.125;
between it and `PRIOR_UNSURE` only 0.005, so a clean photo can land just under the Unsure bar
and a faint one (0.175) just over it. Synthetic only; the faint positives and the negatives
overlap closely at the low end.

Real set-a (7 photos besides the reference, `puw-dev match-sweep --real`, template built from
the batch layer, so the layer supports its own placement by construction): `find` Found 0 / 7
(4 Unsure, 3 NotFound). Saved-position score min 0.234, median 0.329, max 0.435. `match_photo`:
Found 6 / Unsure 1 / NotFound 0. The one photo of the other real set (different size) stays not
`Found` (the prior does not apply).

## Detector specks in the first guess (Plan 2 fix wave, C1)

The first guess used to keep every component the detector found, union it with the batch layer
and grow the lot. On the synthetic `fixed` set the batch layer is clean (the mark alone), but the
detector also finds dozens of small specks on fur, vessels and stars, so a guess covered 4-11 %
of the photo for a mark of 1.22 %. Those specks went into the step-3 template (matching found the
mark in 1 of 8 photos) and into the saved watermark (its box spanned most of the photo, and the
offer took minutes on one core).

`pipeline::guess_photo` now passes the detector's mask through `keep_mark_parts` before the face
guard, the union with the layer and the growth:

| step | constant | value |
|---|---|---|
| "near" the layer, and the grouping distance of detector components | `LAYER_NEAR_FRAC` | 0.02 of the long side |
| the layer is the mark when the detector comes near at least this share of its pixels | `LAYER_SEEN` | 0.9 |
| without that, a group with less than this share of the largest group's pixels is a speck | `SPECK_FRAC` | 0.1 |

- Layer seen by the detector (the fixed set: 1.0 on every photo): only components near the layer
  stay.
- Otherwise (no layer, an empty one, or a layer of static scene edges the detector does not see,
  as in the random and styles sets, 0.0-0.95 per photo, mostly below 0.6): components near the
  layer stay, and so does every group of at least 10 % of the largest group. Tiled text is many
  groups of about the same size, so it stays.
- 0.02 is the distance at which components join one piece in `filter_layer` and the template
  (30 px at 1536). The rule "keep only components near any non-empty layer" (as first planned)
  cut the random set's recall to 0.974 and the styles set's to 0.642, because their layers hold
  scene edges while the marks move; the 0.9 gate keeps it to layers that are the mark.

`puw-dev guess --all --set target/synth/<set>` (new `--all`: every photo, with recall against
mask/), Core ML, before -> after:

| set | recall mean (min) | guess coverage % mean (min-max) | mark coverage % mean |
|---|---|---|---|
| fixed (8) | 1.000 (1.000) -> 1.000 (1.000) | 7.60 (3.84-11.03) -> 2.61 (2.48-2.76) | 1.22 |
| random (12) | 0.975 (0.704) -> 0.975 (0.704) | 6.73 (2.44-16.09) -> 4.64 (1.97-11.18) | 1.31 |
| styles (5) | 0.809 (0.049) -> 0.805 (0.047) | 13.34 (7.33-27.21) -> 11.65 (6.58-26.00) | 5.20 |

- fixed: 2.61 % is about the batch layer (1.90 %, the mark grown by 3 px) grown by the guess's
  7 px; the guess holds the whole mark and nothing else.
- styles: the 0.4 point drop is styles/02 (tiled text, 0.996 -> 0.981): the cut-off tiles at the
  photo's edges are small groups. styles/03 (full-frame thin lines) stays at 0.05, as before.
- Real sets (`guess --real`, 3 + 1 photos, coverage only, no ground truth): set 1 9.18 (8.89-9.46)
  -> 8.79 (8.44-9.11) %, set 2 6.40 -> 4.67 %. Set 1's layer is not seen by the detector (the
  faint outline), so it takes the size rule; its coverage is mostly the grown layer (R12). The
  overlay of set 2 was checked by eye: the whole mark is still in the guess.

`puw-dev bench --set ...` (guess and removal at 4096, Core ML), before -> after:

| set | recall mean | mark PSNR after, mean dB | time per photo, mean s |
|---|---|---|---|
| fixed | 1.000 -> 1.000 | 23.62 -> 23.58 | 34.9 -> 13.5 |
| random | 0.975 -> 0.975 | 27.67 -> 27.63 | 20.5 -> 12.5 |
| styles | 0.809 -> 0.805 | 19.98 -> 19.98 | 19.8 -> 14.8 |

The PSNR in the mark is unchanged (within 0.05 dB) and removal is faster, as less is painted.
Times are from one run each on a machine doing other work, so read them as a trend only.

Matching with the first guess as it is (no editing): `first_guess_matches_fixed_set_and_is_offered`
(puw-app session test, needs `PUW_MODELS_DIR`) confirms the representative's guess as the set area,
then `match_all` finds the mark on 8 of 8 fixed photos (was 1 of 8 in the Windows walkthrough).
Saved and offered again for the same photos, `suggest_saved` answers in 0.1 s with that watermark.

### Stray parts far from the mark (`template.rs`)

A piece of a template (components grouped at 2 % of the long side) with less than `CARRY_FRAC`
(0.1) of the largest piece's pixels is not searched for: `find` places the other pieces and
carries it along at its saved offset from the largest piece (same scale). `saved_score` scores
only the searched pieces, `main_bbox` gives their box (the saved watermark's picture uses it).
The smallest real part of a multi-piece mark in the tests (the ring logo beside the fixed
text) holds about 18 % of the text's pixels; a dot of the default 24 px brush about 2 %. Test
`stray_dot_is_carried_along`: random/00's mark plus a 25 px dot 300 px away gives the same
status and confidence as the mark alone on all 11 other photos, the dot lands at its offset, and
the main box is the mark's. Saved templates of earlier versions get this too (it is decided when
matching).

## App timings (Plan 2)

Measured with `cargo test -p puw-app --release --test timings -- --ignored --nocapture` (needs
`PUW_MODELS_DIR`, and `PUW_REAL_DIR` for the real rows), which drives `AppCore` the way the UI
does. Machine: Apple silicon Mac, Core ML (Neural Engine / GPU), nothing else heavy running. Aggregates only; the real set-a has 8 photos of 24.0 MP.
Two full runs were made; the table shows the second (the first differed by a few seconds at most).

| Step | Time |
|---|---|
| Finder load (segmenter + face), cold Core ML cache | 17.0 s |
| Remover load (LaMa), cold Core ML cache | 22.3 s |
| Finder load, warm cache (new engine, same cache folder) | 0.9 s |
| Remover load, warm cache | 1.9 s |
| detect, synthetic `fixed` (8 photos of 1536x1024, models loaded, 3 looked at) | 13.8 s |
| detect, real set-a (8 photos, 3 looked at, finder load and reading the set included) | 25.0 s |
| match_all, synthetic `random` (12 photos; 11 found, 1 not found) | 1.5 s |
| match_all, real set-a (8 photos; 1 found, 3 unsure, 4 not found) | 33.8 s |
| Preview of one 24 MP photo, remover loaded (edit image at 1024) | 13.4 s (was 25.8 s at 1536) |
| Preview of the same photo, first use (includes about 2 s remover load) | 17.4 s (was 29.1 s at 1536) |
| Removal of one 24 MP photo at Best (4096) | 78.0 s and 86.5 s (2 photos) |
| Removal of one 24 MP photo at Faster (2048) | 35.1 s and 33.1 s (2 photos) |

The real match_all row used the finder's own guess as the set area (no hand editing), so its
found/unsure split says nothing about matching quality; only the time is of interest.

Honest comparison with the spec: the preview target is about 15 s. At long side 1024 it measured 13.4 s with a warm
remover and 17.4 s the first time (single run, preview rows only; at 1536 it was 25.8 s and 29.1 s), so a warm preview meets the target and the first one is about 2 s over. Removal at Best takes 78-87 s per 24 MP photo, so a 100-photo set at Best is about
2.3 hours; at Faster it is about 34 s per photo (about 55 minutes for 100). The cold Core ML
compile (17 s + 22 s) happens once per machine and models folder; every later start takes
about 3 s. Where the preview time goes was not profiled here (the photo is decoded at full
size, the area is applied at the working size, then LaMa runs); that is the place to look if
the target matters.

## App timings (Plan 2b, watermark groups)

Same harness and machine as "App timings (Plan 2)" (Core ML, models and cache warm). Aggregates
only. The synthetic mixed set is 8 `fixed` photos, 4 logo photos and 2 photos with no watermark;
the real set is set-a (8 photos of 24.0 MP) plus set-b as one set of 15 photos, with group 1 from
set-a. One run.

| Step | Time |
|---|---|
| detect of the leftovers, synthetic mixed set (6 photos left, 3 looked at) | 4.2 s |
| place_on, one synthetic strip photo, first call (learns the drawing) | 0.34 s |
| place_on, the next 2 synthetic strip photos | 0.57 s in all |
| match_all of group 2, synthetic mixed set (6 photos; 4 found, 2 not found) | 0.3 s |
| use_saved, then match_all for a saved logo watermark (6 photos; 4 found, 2 not found) | 0.00 s + 0.4 s |
| detect of the leftovers, real (7 photos left, 3 looked at) | 23.4 s |
| match_all of group 2, real (7 photos; 1 found, 6 not found) | 6.1 s |
| place_on, one 24 MP strip photo, first call and again | 5.5 s and 5.1 s |

Against the spec's budget (one sample first guess plus one matching pass over the photos with no
group, and nothing more): a new group on the real set costs 23.4 + 6.1 = 29.5 s, which is those
two parts and nothing else; a saved watermark skips the first guess and costs only the matching
pass (0.4 s synthetic). Both rows fit. The real place_on is on a photo that does not carry the
second watermark (not found), and it does not get faster on a repeat: it reads and matches the 24 MP photo each time, so painting in step 2 on a big photo
is the thing to watch by hand (checklist section 11).

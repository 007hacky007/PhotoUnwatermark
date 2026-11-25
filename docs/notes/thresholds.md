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
set has a batch layer for it, the saved placement counts as `Found` if at least 50 % of its
pixels lie inside the layer grown by `grow_for(w, h, 0.004)`. The confidence stays `find`'s.

Synthetic: the fixed set re-blended at alpha 0.12 (stored photos carry 0.25) leaves free search
below `Found` on at least one photo; with the prior all 8 are `Found` at IoU >= 0.8. Negative
photos resized to the template size with no layer never reach `Found`.

Real set-a (7 photos besides the reference, `puw-dev match-sweep --real`, template built from
the batch layer, so the layer supports its own placement by construction): `find` Found 0 / 7
(4 Unsure, 3 NotFound); `match_photo` Found 7 / 7, all on the set position. The one photo of
the other real set (different size) stays not `Found` (the prior does not apply).

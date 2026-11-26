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
Since round 2 of the small-set fix (`batch::agree_count`: 6 of 8 photos must agree) set 1 gives
0.99 % (53 components, the mark only), in 5.6 s (the photos are also kept at long side 1536
for `SetLayers::edges`).

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

This section describes the prior as Plan 2 made it; Plan 2b's final fix changed what is scored
and the order of the checks, see "Position prior on the layer inside the drawing" below.

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

## Position prior on the layer inside the drawing (Plan 2b final fix, C1)

The template of a group is learnt from the drawing, and the drawing is usually the first guess:
the detector's area and the batch layer, grown by `grow_for(0.004)`. On real set 1 that guess
covers 9.3 % of the photo, its main piece 2.1 M px against 0.43 M px of layer. Scored at the
saved position, such a thick band is mostly scene: the other photos of the set
scored 0.09-0.16, under `PRIOR_UNSURE`, so the prior never fired; the search alone found 1 photo
(at the wrong place, IoU 0.42 with the saved placement), 2 unsure (also misplaced) and 4 not
found. In step 2 two of the three strip photos said "not found".

`pipeline::PositionPrior` (worked out once per matching pass, and per drawing in the strip) now
has two support rules, with `g` = `grow_for(0.004)` and "placement" = the template's pieces at
their saved position:

| rule | test | scored with |
|---|---|---|
| 1, the layer is the mark under the drawing | at least `PRIOR_LAYER_INSIDE` 0.9 of the layer lies in the placement grown by `g`, and the layer inside the placement is at least `PRIOR_LAYER_SHARE` 0.1 of it | the template cut down to the raw layer (`template::within`: same boxes and edges, shapes cut) |
| 2, the drawing follows the layer (as before) | at least 0.5 of the placement lies in the layer grown by `g` | the template itself |

A set-position result still pastes the whole template, so removal covers the whole drawing.
`match_with` then decides, for a photo of the template's size:
- set-position score >= `PRIOR_FOUND` 0.28: `Found` at the saved position, no search;
- else the search runs; below `PRIOR_UNSURE` 0.16 its result stands; a search `Found` whose area
  overlaps the saved placement by IoU >= 0.8 stands; anything else is `Unsure` at the saved
  position (a search `Found` elsewhere on a set that shows the mark here is not trusted).
  (Since the small-set fix the search no longer runs between the two bars, see "Small sets of
  one shoot" below.)

Before, `find` ran first and its `Found` was kept wherever it was; that is how the real set got a
`Found` at the wrong place.

Support measured on the synthetic sets (true area of each photo, grown by 0, 8, 24 and 48 px):

| set | share of the layer in the grown placement | layer share of the placement | rule 2 share |
|---|---|---|---|
| fixed (8 photos x 4 growths) | 1.00 every time | 1.00 / 0.81 / 0.55 / 0.36 | 1.00 / 1.00 / 0.71 / 0.46 |
| random (12) and styles (5) | 0.00-1.00, most below 0.8 | 0.00-0.18 | 0.00-0.48 |

On random and styles the layer holds static scene edges and the marks move. Of their 56
drawings on photos whose size has a layer, rule 1 passes for 4 (the true area of two photos whose
size group has a small layer next to the mark, grown by 0 or 8 px); the cut template there scores
0.00-0.13 on the other photos of that size, under `PRIOR_UNSURE`. A cut template of a few scene
pixels can score high on photos without the mark (up to 0.62 on random, for a drawing holding
2 % of its layer), so rule 1 needs the layer to lie inside the drawing, and to be a real part of
it: styles' tiled and full-frame drawings hold all of their small layer, but it is 1 % of them.
Test `prior_never_places_a_moving_mark` pins that no set-position result lands where a photo of
those sets has no mark.

Grown drawing on the fixed set (the true area of photo 0 grown by 40 and 60 px, 4.7 % and 6.4 %
of the photo for a mark of 1.22 %):

| photos | the drawing at the saved position | cut to the layer | `find` alone | `match_photo` now |
|---|---|---|---|---|
| stored (alpha 0.25), +40 px | 0.30-0.40 | 0.46-0.56 | 0 Found of 7 | 7 Found |
| stored, +60 px | 0.28-0.38 (rule 2 fails, so no prior before) | 0.46-0.56 | 0 Found | 7 Found |
| faint (alpha 0.12), +60 px | 0.00-0.17 | 0.13-0.27 | 0 Found | 5 Unsure, 2 NotFound |

The faint photos with the true area alone score 0.18-0.33 (4 Found, 3 Unsure), so the cut
template loses a little there (it is the layer, the mark grown by 3 px, not the mark itself) but
far less than the grown drawing. Tests: `prior_matches_a_drawing_grown_beyond_the_mark` (puw-core,
stored photos, +60 px, all 7 `Found` at the saved position) and
`grown_first_guess_still_matches_fixed_set` (puw-app, models: the finder's own guess on the fixed
set at alpha 0.14 grown by 60 px more; the search alone misses 5 of 7 and only 0.34 of the
drawing follows the layer, yet matching finds 8 of 8 at the set position).

Real set 1 + real set 2 as one set (9 photos, 8 of one size with a fixed mark, 1 of another
size), group 1 from the finder's own guess as it is (`timings.rs`
`real_group_from_the_first_guess`, Core ML):

| | before | after |
|---|---|---|
| step-2 strip, the 2 other sample photos | both "not found" | 2 of 2 `Found`, placed at the saved position |
| group 1 over the 8 photos of the mark's size (the reference counts as found) | 2 Found (the reference and 1 misplaced), 2 Unsure (misplaced), 4 NotFound; 7 photos left over | 6 Found, 2 Unsure, all at the saved position (IoU 1.00) |
| the photo of the other size | not found | not found |
| set-position scores of the 7 other photos | 0.09-0.16 (the drawing) | 0.233-0.381 (cut to the layer) |

The two Unsure photos (0.233, 0.254) are opened by the person before removal, as for any unsure
photo. No `Found` lands anywhere but the saved position.

## Small sets of one shoot (owner test fix, 2026-11-26)

The owner added 3 photos of one shoot (set-a: same size, one faint watermark at the same place).
The reference's first guess looked right; the other two strip photos said "not sure" with the
drawing moved a little or far to the side.

Root cause, measured on the triples 0,1,2 / 3,4,5 / 5,6,7 / 0,3,7 of set-a before the fix:

| | triples |
|---|---|
| batch layer of the size | none for 3 of 4: the median of 3 photos kept 1.53-1.58 % of the photo above `BATCH_HIGH` (8 photos: 0.36 %), and `batch_lines` gave up at more than 10 % coverage; for 5,6,7 a layer of 1.32 M px (8 photos: 0.43 M px), only 59 % of it within 0.4 % of the long side of the mark |
| position prior | none for those 3; for 5,6,7 rule 1 held, but the drawing cut to that half-scene layer scored 0.09 and 0.12 at the saved position, under `PRIOR_UNSURE` |
| the drawing (first guess) | 14.6 % of the photo in 2-9 pieces, 10-13 % of its pixels on the mark; at the saved position (minimum over the pieces) 0.00-0.11 |
| the search | the scene where it moved between shots: Unsure 0.36-0.45 or Found 0.68 at 50-570 px off and scale 0.73-1.03, else NotFound 0.17-0.32; 5-10 s per 24 MP photo |

Photos of one shoot share most of the scene: two shots nearly line up, so the median of three
keeps every edge two of them share. The faint mark is a small part of a large drawing, the scene
in it moved, and the search follows the scene. With all 8 photos the median needs 4 photos per
edge, the layer is the mark, and the prior placed both strip photos.

Fix (`batch::AGREE`, `batch::agreed`): with 3 or 4 photos the layer's gradient takes, per pixel
and per axis, the value 3 photos reach on the same side of zero (all 3, or 3 of 4) instead of
the median. From 5 photos on the median of an odd count is that value already, so larger sets
keep their numbers (set-a's 8-photo layer, the synthetic fixed, styles and negative sets are
unchanged). (Round 2 below replaced this by `batch::agree_count` for 3 to 11 photos.)

| set-a triple (6 measured) | median of 3 | 3 agree |
|---|---|---|
| photo above `BATCH_HIGH` | 0.95-1.64 % | 0.23-0.28 % |
| layer | none for 4 of 6; 1.3-1.5 M px | 0.25-0.32 M px, 55-60 % of the 8-photo layer |
| share of the layer within 10 px of the 8-photo layer | 0.43-0.47 | 0.79-0.99 |

With that layer the first guess is smaller (7.8-9.1 % of the photo, layer share of it
0.13-0.15), rule 1 of the prior holds for every triple, and the drawing cut to the layer scores
0.24-0.44 at the saved position on the other photos.

Second change (`pipeline::match_with`): between `PRIOR_UNSURE` and `PRIOR_FOUND` the result is
`Unsure` at the saved position with no search. Before, the search ran there only to turn the
result `Found` when it agreed with the saved place (IoU >= 0.8); it never did: 0 of the 4 faint
synthetic fixed photos in that band, 0 of the 2 real 8-photo ones, 0 of the 1 in the triples. It cost
2-9 s on a 24 MP photo (9.2 s for one strip photo of triple 1,3,6). A search `Found` elsewhere was
already not trusted there, and the real triples show why: the drawing at the saved position
scored 0.13 and the scene where it moved 0.52 (Found), so "prefer the search when it scores
clearly better than the saved position" would bring the owner's bug back. Parked N1 (an
`Unsure` from the prior discards a correct search placement elsewhere) therefore stays as a
rule, not a bug; on the moving-mark checks below the prior never reaches the `Unsure` band.

Moving marks (`prior_never_places_a_moving_mark`, tightened from "IoU > 0"):
- random and styles: a set-position result must cover at least half of the photo's own mark.
  With the new layers no drawing of either set gets a prior any more (before: 4 of 56, all on
  the random set, whose size groups have 3 photos);
- the fixed set's 8-photo layer (the prior holds) with photos whose mark moved 400 px up: the
  saved position scores 0.00-0.10, no set-position result, and the 5 photos the search alone
  finds at their moved mark (3 Found, 2 Unsure) come out the same.

Random set first guesses (`puw-dev guess --all`, Core ML): recall unchanged (mean 0.975, min
0.704); guess coverage mean 4.64 % -> 2.75 % (max 11.18 % -> 3.58 %), as the 3-photo layers no
longer hold scene edges (layers of 0.30-2.82 % -> two sizes with 0.03 % and 0.13 %, two none).

Synthetic test of the owner's case (`three_photos_of_one_shoot_keep_the_set_position`, models;
`three_photos_of_one_shoot_give_the_mark`, no models): one scene moved by (0, 0), (1, 1) and
(24, -16) px, the fixed set's mark at alpha 0.14 at one place. Median of 3: 3.36 % above
`BATCH_HIGH`, no layer, the strip placed a photo `Found` 79 px off. Now: 0.36 %, the layer holds
95 % of the mark and lies 91 % within 7 px of it; strip and step 3 place all
photos at the saved position, `Found`, as guessed and with the guess cut down to the mark.

Real acceptance (`timings.rs` `real_small_sets_keep_the_set_position`, Core ML): 8 triples of
set-a (0,1,2 / 3,4,5 / 5,6,7 / 0,3,7 / 1,4,6 / 2,5,7 / 0,2,4 / 1,3,6), each with the first guess
as it is and cleaned up (groups of less than a quarter of the largest group dropped, a band of
1 % of the height erased at the top: 2.2-7.7 % of the guess removed, 0.4-7.7 % before the fix), a fresh session each:

| | before | after |
|---|---|---|
| strip photos at the same place (scale and shift within 1 %) | 0 of 32 (19 Unsure elsewhere, 13 NotFound) | 32 of 32 (30 Found, 2 Unsure) |
| strip placement, 24 MP | 6.62 s median, 16.92 s max | 0.94 s median, 2.06 s max (the first call also learns the drawing) |
| step 3, photos at the saved place | 16 of 48 (the 16 references only); the other 32 Unsure elsewhere or NotFound, none Found elsewhere | 48 of 48 (46 Found, 2 Unsure), none Found elsewhere |

The 8 + 1 acceptance (`real_group_from_the_first_guess`) still passes: 2 of 2 strip photos
`Found` at the same place (1.14 s at most), group 1 6 Found and 2 Unsure, all at the saved place,
none elsewhere, the photo of the other size not found. Its match_all takes 1.3 s (was 6.0-6.7 s:
the 2 Unsure photos no longer run the search).

Not covered: a person who erases a whole part of the mark from the drawing (in the first
measurement the "cleanup" kept only the largest group of the guess, which on triple 1,3,6 cut
away half of the mark): then less than 90 % of the layer lies in the drawing, rule 1
fails, rule 2 scores the drawing itself (0.07-0.10), and the search decides (Unsure, moved 128 and 105 px, and
NotFound). The drawing then does not cover the mark anyway.

### Sets of 4-7 photos and the set's own edges (round 2)

After the first fix, subsets of 4-7 photos of set-a (the eight below, as is and cleaned) placed
68 of 72 strip photos and 84 of 88 step-3 photos at the true place, but 48 of the 84 as Unsure,
and set 1,3,5,6,7 gave NotFound for 2 strip photos.

Root cause, measured with the finder's own first guess on one session per subset (code before
this round):

| photos | layer (% of the photo) | share of it off the mark | saved-position scores (rule 1) |
|---|---|---|---|
| 4 (2 sets) | 1.96-3.34 | 0.15-0.41 | 0.22-0.40 |
| 5 (2 sets) | 3.42-4.87 | 0.33-0.47 | 0.13-0.33 |
| 6 (2 sets) | 2.93-3.51 | 0.27-0.37 | 0.19-0.29 |
| 7 (2 sets) | 2.29-3.17 | 0.10-0.33 | 0.17-0.33 |
| 8 | 1.80 | 0.00 | 0.23-0.38 |

"Off the mark" is the share of the layer more than 10 px from the 8-photo median layer. Rule 1 of
the prior held every time (all of the layer inside the grown guess, layer share of the guess
0.19-0.37). Over the 43 photos the scores ran 0.13-0.40, median 0.25: 16 at or over
`PRIOR_FOUND` 0.28, 2 under `PRIOR_UNSURE` 0.16 (those went to the search, which found the
scene: the NotFound of set 1,3,5,6,7).

Two causes:
1. The median keeps an edge that half of the photos share, and in a shoot two or three shots
   nearly line up: 10-47 % of the 4-7 photo layers was scene. The cut template then holds scene
   pixels that the other photos do not have.
2. Even on the clean 8-photo layer the scores sat at 0.23-0.38: the cut template carries the
   reference photo's edges, the faint mark plus the reference's own scene under it, and every
   other photo has another scene there. The synthetic calibration behind `PRIOR_FOUND` had the
   same spread (faint marked photos 0.175-0.331, photos without the mark up to 0.155).

Changes:
- `batch::agree_count`: an edge stays in the layer when `n - (n - 1) / 4` of the `n` photos
  show it with the same sign, at most `AGREE_MAX` 6, and the median once that is half of the
  photos or fewer (from 12 photos). 3 -> 3, 4 -> 4, 5 -> 4, 6 -> 5, 7 -> 6, 8 to 11 -> 6. Layers
  of set-a subsets (the finder's own guess): 0.87-1.09 % of the photo, 0.00-0.01 off the mark
  for 4-8 photos; the 8-photo layer itself 0.99 % (was 1.80 %).
- `SetLayers::edges`: `build_set_layers` also keeps each sampled photo at long side 1536 and
  takes the same agreed median of their gradients there. Rule 1 of the prior scores the cut
  template on those edges (`template::with_edges`), the mark without any one photo's scene.
- `PRIOR_LAYER_SHARE` 0.1 -> 0.05: the agreed layers hold the mark's lines only, so the real first
  guess holds 0.12-0.14 of its layer, and a drawing 1.6 times as large fell under 0.1 (no set
  position, the search found the scene). At 0.05 the layer grown by 120 px (16-18 % of the photo)
  still passes and scores 0.49-0.58. No drawing of the random and styles sets, up to 96 px
  grown, passes rule 1 on the new layers.
- `keep_mark_parts`: a layer counts as the mark only when it also holds at least `SPECK_FRAC` of
  the pixels of the detector's largest group. The styles set's layer shrank to 210 px of scene
  that the detector happens to cover, and only the parts near it were kept (styles/05 recall
  1.000 -> 0.475); that layer is 0.004-0.011 of the styles marks' groups, the fixed set's about
  1, the real set's 3-10.

Scores at the saved position after the change (rule 1, the set's edges):

| photos | before | after |
|---|---|---|
| real set-a subsets of 4-8 photos, finder's guess (43 photos) | 0.13-0.40, median 0.25 | 0.41-0.57, median 0.52 |
| synthetic fixed, faint (alpha 0.12), true area / grown 60 px | 0.18-0.33 / 0.13-0.27 | 0.51-0.95 / 0.45-0.95 |
| synthetic photos without the mark (7 clean scenes, 6 negatives), true area / grown 60 px | 0.00-0.153 / 0.00-0.128 | 0.00-0.07 / 0.01-0.05 |
| a clean photo inside the layer's own set (sets of 5 and 8) | 0.00-0.13 | 0.01-0.07 |
| photos whose mark moved 400 px (fixed layer) | 0.00-0.10 | no set-position result (`prior_never_places_a_moving_mark`) |

The thresholds stay (`PRIOR_FOUND` 0.28, `PRIOR_UNSURE` 0.16): the ceiling of photos without the
mark fell from 0.155 to 0.07, and the real photos rose from a median of 0.25 to 0.52.
`prior_unsure_on_weak_evidence` now uses the set's photo 5 faded to alpha 0.06 (0.23), since at
0.12 it scores 0.49.

Real acceptance (`real_small_sets_keep_the_set_position`, now 16 sets: the 8 triples and the 8
subsets of 4-7 photos, each as is and cleaned, a fresh session each; strip placement on every
other photo):

| | before (round 1) | after |
|---|---|---|
| 3 photos: strip / step 3 | 32 of 32 (30 Found, 2 Unsure) / 48 of 48 (46 Found, 2 Unsure) | 32 of 32 Found / 48 of 48 Found |
| 4-7 photos: strip / step 3 | 68 of 72 / 84 of 88 (36 Found, 48 Unsure), 4 NotFound | 72 of 72 Found / 88 of 88 Found |
| strip placement, 24 MP | 0.94 s median, 2.06 s max (triples); 0.53 s median, 7.81 s max (4-7) | 0.45 s median, 1.26 s max (all 16 sets) |
| Found elsewhere | 0 | 0 |

8 + 1 acceptance (`real_group_from_the_first_guess`): group 1 8 Found, 0 Unsure, all at the saved
place (was 6 Found, 2 Unsure); strip 2 of 2 Found, 1.11 s at most; match_all 1.1 s.

Synthetic first guesses (`puw-dev guess --all`, Core ML): fixed unchanged (recall 1.000,
coverage 2.61 %); random unchanged from round 1 (recall mean 0.975, min 0.704); styles recall mean
0.805 -> 0.796 (styles/04, 0.047 -> 0.002: the old median layer, 20 components of scene, happened
to cover 5 % of that photo's mark; the detector finds none of it either way), coverage mean 11.55
-> 10.01 %.

Memory and time: `build_set_layers` keeps each sampled photo at long side 1536 as well, 2 bytes
per pixel (3 MB for a 3:2 photo, 72 MB for 24), on top of `BATCH_LUMA_BUDGET`; set 1's 8 photos
take 5.6 s (was 4.3 s).

### Review fixes: static scene, clean photos, edited drawings (fix round 1)

The review of rounds 1-2 built synthetic sets (fixed set scenes, 1536x1024) where a set
position gave a false `Found`. The same sets are now committed tests (`patch_set` in
`tests/pipeline.rs`): a 340 x 250 patch of static scene on every photo, and the fixed mark
either moving between photos ("A") or fixed but missing from the last photo ("B"). The patch is
far from the mark or ends 8 px above it. The drawing is the first guess, emulated as
`dilate(mark | layer, grow_for(0.004))`.

| case (3, 4, 5, 8 photos) | 2170b2b | now |
|---|---|---|
| A far: moving mark | every other photo `Found` at the saved place (0.36-0.87), its mark not covered | cautious prior: the search places the moved marks where they are, as `Unsure` (0.37-0.58); one is not found |
| A next to the patch | `Found` at the saved place (0.38-0.94), mark not covered | cautious prior: `Unsure` where the mark is (0.51-0.57), NotFound (0.26-0.34) or once `Unsure` (0.37) where it is not; never `Found` |
| B far: clean last photo | clean photo `Found` (0.61-0.70) at 3-4 photos | clean photo NotFound (0.17-0.23); the marked photos `Found` at their mark |
| B next to the patch | clean photo `Found` (0.45-0.80) at 3-5 photos | clean photo NotFound (0.28) at 3-4 photos, `Unsure` 0.45 / 0.17 at 5 / 8 (set position, a part of the layer missing) |

Three causes, three changes (`pipeline::PositionPrior`, `match_with`):
1. The first guess always holds the layer, so rule 1 held for every untouched guess. Now the
   set must also explain the drawing (`explained_by_set`): no connected stretch of the drawing
   farther than `PRIOR_REACH` (2.4 % of the long side) from the layer may hold more than
   `PRIOR_UNEXPLAINED` 0.1 of it, unless the set's own edges over the far part stand out from
   the photo's edges by at least `PRIOR_FAR_EDGES` 0.05 of how much they stand out over the
   layer.
   - Far part: real set-a triples' first guesses 0.09-0.25 in all (a layer of 3 photos misses
     stretches of the faint mark); the 8-photo guess 0.107 in three blobs of scene of 0.04
     each; the synthetic static-patch cases 0.24-0.44.
   - Edge ratio of the far part: real triples 0.13-0.47 (the mark is there, too faint for the
     layer); synthetic -0.09 to -0.03 (nothing is stamped there).
   - A first version counted the whole far part against 0.1. It refused the 8-photo guess
     (0.107), and the search then put a photo `Found` elsewhere. That led to the next point.
   When the drawing holds part of the layer but the set does not support its position, the
   prior now stays "cautious" (`PositionPrior::supports` false). Nothing is scored at the
   saved position, and a search `Found` that does not overlap the saved placement by
   `PRIOR_AGREE` 0.8 is `Unsure`. A drawing that holds no layer at all gets no prior, and the
   search decides as before.
2. Static scene in the same stencil as the mark lifted a clean photo. Rule 1 now also scores each
   part of the layer under the drawing on its own: components grouped closer than
   `PRIOR_PART_NEAR` 1 % of the long side, parts of at least `PRIOR_PART_MIN` 0.3 of the largest.
   A photo whose weakest part scores under `PRIOR_UNSURE` is at most `Unsure`.
   - Measured as the minimum over parts: synthetic marked photos 0.50-0.69, the clean photo
     beside the patch 0.11-0.15; real set-a subsets of 3-8 photos 0.23-0.51. Counting every
     small part as well drops real photos to 0.15, which is why small parts are left out.
   - A plain minimum over parts as the score would have made most real photos Unsure, so the
     parts only veto `Found`.
3. With a set position, too (score under `PRIOR_UNSURE`), a search `Found` that does not
   overlap the saved placement by `PRIOR_AGREE` is `Unsure`. On the real burst sets the search
   found the moved scene at 0.68. A mark that really moved inside a fixed set keeps its place
   and score, but is opened by the person.

The owner's failing triple (a random 3 of set-a's 8, step 3: the reference Found, one photo
Unsure with the drawing misplaced, one NotFound) came from an edited drawing: the owner erased
the red blobs of scene from the first guess. Those blobs are parts of the 3-photo layer (scene the
three photos share). Rule 1 asked 90 % of the whole layer to lie in the drawing, so erasing them
left rule 2, which scores the reference's own edges. The faint mark scored low there, and the
search found the scene. Rule 1 now looks only at the layer components that reach into the
drawing: a component the person erased whole is scene, not the mark.

All 56 triples of set-a, each with seven drawings (`timings.rs`
`real_triples_with_edited_drawings`: the first guess as is, cleaned, everything farther than
1 % of the long side from the mark erased, a band across the middle erased, four brush dots on
the scene, grown and shrunk by 8 px; 112 photos per drawing):

| drawing | 2170b2b | now |
|---|---|---|
| as is, cleaned, middle erased, brush dots, grown 8 px, shrunk 8 px | 112 of 112 `Found` at the saved place, each | 97 `Found`, 15 `Unsure` at the saved place, each; none elsewhere, none not found |
| scene parts erased | 82 `Found` and 12 `Unsure` at the saved place, 10 not found, 5 `Unsure` and 3 `Found` elsewhere (15 of 56 triples wrong) | 112 of 112 `Found` at the saved place |

The 15 `Unsure` photos come from 9 triples whose 3-photo layer holds a part (at least 0.3 of the
largest) that scores under `PRIOR_UNSURE` on the other photos. On one of them the whole layer scores
0.51, so only the part veto holds it back. Erasing the scene parts of the guess removes those parts
from the drawing, and then all 112 photos are `Found`. This is the price of the I1 fix:
`Unsure` there is opened by the person and not trusted blindly.

Synthetic tests added: `static_scene_never_places_a_moving_mark`,
`clean_photo_of_the_set_is_not_found` (B far and next, 3/4/5/8 photos),
`erased_scene_part_keeps_the_set_position`, `thin_mark_in_a_generous_drawing_keeps_the_set_position`
(pins `PRIOR_LAYER_SHARE` 0.05: the layer share 0.05-0.1, refused at 0.1),
`photos_of_one_shoot_give_the_mark` and `photos_of_one_shoot_keep_the_set_position` (6 shots
with 3 lined up). `prior_never_places_a_moving_mark` part 1 now uses the first guess, with a
premise that the set has a layer. `prior_matches_a_drawing_grown_beyond_the_mark` grows the
drawing by 36 px, as far as `PRIOR_REACH` lets it reach at 1536 (60 px is now refused: 30 % of
it far from the layer, where the set shows nothing).

Real acceptance with this code (Core ML):
- `real_group_from_the_first_guess` (now asserts all 8 photos of the fixed set at the saved
  place): 2 of 2 strip photos `Found` at the same place, 2.01 s at most (was 1.1-1.3 s: the
  set's explanation of the drawing reads the layer at full size); group 1 8 `Found`, 0 elsewhere;
  match_all 2.5 s (was 1.1-1.3 s).
- `real_small_sets_keep_the_set_position` (16 sets, now asserts at most one set per size left
  `Unsure`): 104 of 104 strip photos and 136 of 136 step-3 photos at the true place, 0
  elsewhere. 3 photos: strip 28 `Found` and 4 `Unsure`, step 3 44 `Found` and 4 `Unsure` (one
  triple, as is and cleaned). 4-7 photos: all `Found`. Strip 0.89 s median, 2.29 s max.

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
the real set is set-a (8 photos of 24.0 MP) plus set-b (1 photo) as one set of 9 photos, with group 1 from
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

Plan 2b final fix, same harness (`real_group_from_the_first_guess`), real set as above, group 1
from the finder's own guess:

| Step | Before | After |
|---|---|---|
| place_on, 24 MP strip photo, first look | 5.5-5.8 s | 1.17 s at most (reads both photos, scores the set position, no search) |
| place_on after a stroke on the drawing, per strip photo | 5-6 s (matched again) | 0.70 s for the first (learns the drawing again), 0.01 s for the rest (placement kept) |
| place_on with the drawing sent back unchanged | 5.9 s (the draft was upsampled and never equal) | 0.01 s |
| full search on a 24 MP photo (a drawing with no set position) | | 7.8 s |
| stop half a second into that search: the call ends after | the rest of the search (about 7 s) | 0.04 s |
| match_all, group 1 (8 photos to match) | 12.2 s (scratch run of the review) | 6.0-6.7 s (three runs) |
| "Looks right" unchanged on a 24 MP photo | group marked edited, matching reset | nothing changes (0.0 s) |

A placement is kept per strip photo while the drawing's main box moves by at most 1 % of the
reference's sides (`KEEP_PLACEMENT`), for `Found` and `Unsure`; `NotFound` is looked for again
after every change of the drawing, as the change may be what finds it.

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
  - Every negative (synthetic and real, except the same-scene artefact above) stays below 0.35.
- `UNSURE` stays 0.35. The right real placements (0.36-0.44) land in Unsure, which is the
  wanted outcome for a mark this faint.
- With these thresholds, set 1 is not processed automatically; the user is asked to look at each
  photo. Making matching work for faint fixed marks needs a change to the method, not to the
  thresholds. One candidate: when a set's batch layer holds the confirmed mark, use the fixed
  position. The template and library tests pass with the new value.

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

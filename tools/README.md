# tools

Python developer tooling, managed by [uv](https://docs.astral.sh/uv/) (Python 3.12). Not a package.

## Synthetic test sets

Generates deterministic benchmark sets (clean / marked / mask PNGs plus `manifest.json`) from the
scikit-image sample photos, or from your own clean photos with `--src`:

    uv run --project tools python tools/synth/make_sets.py --out <dir> [--src <clean photos dir>] [--seed 0]

Sets: `styles` (one photo per mark style), `fixed` (8 photos at 1536x1024, same mark, same place),
`random` (12 mixed portrait/landscape photos, mark at a random place), `negative` (6 photos, no mark).
Masks are 255 where the watermark is. `bbox` in the manifest is `[x, y, w, h]` of the mask in pixels.

Tests:

    uv run --project tools pytest tools/synth -q

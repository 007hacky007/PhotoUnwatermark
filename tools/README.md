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

## Models

Builds the engine's ONNX models into `models/` (gitignored, never committed), checks parity and
writes `models/manifest.json` (dtype, sha256, bytes, licence, source per file):

    uv run --project tools python tools/models/export.py [--out <dir>]
    uv run --project tools python tools/models/verify.py [--out <dir>] [--styles <styles set>]
    uv run --project tools pytest tools/models -q

`export.py` downloads the upstream weights (Hugging Face cache, OpenCV Zoo) and writes fp32 and
fp16 candidates to `models/build/`. The shipped precision per model is the explicit
`SHIPPED_DTYPE` in `export.py` (currently fp32 for all three, chosen by CPU speed; the reasons and
numbers are next to it). `verify.py` checks every candidate on the synthetic `styles` set, copies
the shipped dtype per model to `models/` and exits non-zero on any failure, including a shipped
candidate that misses its parity threshold. The tests check
the built directory (`PUW_MODELS_DIR` to point elsewhere), so they need the two steps above first.
Licences: [models/LICENSES.md](models/LICENSES.md).

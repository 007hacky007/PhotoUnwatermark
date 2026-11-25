# Core ML on macOS

Owner's decision (2026-11-25): Macs use Core ML on purpose, and the app says so ("Using graphics
card (fast)"). Since Plan 2 Task 1b the engine works like this on macOS:

- With `prefer_gpu`, each model is loaded with the Core ML provider registered explicitly:
  compute units `CPUAndNeuralEngine`, model format NeuralNetwork (the default), and
  `with_model_cache_dir` when `EngineOptions.cache_dir` is set.
- The report says `Gpu { name: "Core ML", share }` only when the session log shows Core ML runs
  the model. `share` is `None` for a whole model and `Some((gpu_nodes, total_nodes))` for a
  partial placement that was measured to be faster than the processor alone (see "Partial
  placement" below). Any other partial placement would be loaded again for the processor and
  reported as `Cpu { PartialGpu }`.
- Every other session (`prefer_gpu = false`, and every fallback) sets
  `AutoDevicePolicy::PreferCPU`, so `Cpu` in the report really is the processor. Before Task 1b
  those sessions ran on Core ML without saying so (see `model-loading.md`).

## Why that configuration

Plan 1 tuned every threshold on a Mac while Core ML was in use implicitly: `ort` sets ONNX
Runtime's automatic provider selection to `MaxEfficiency`, which on macOS registers Core ML by
itself. The explicit configuration therefore had to reproduce that automatic placement's
outputs: segmenter max probability difference <= 0.01 and LaMa PSNR in the hole >= 40 dB.

The automatic placement turned out to be Core ML with compute units `CPUAndNeuralEngine`
("efficiency" prefers the Neural Engine). That explicit configuration gives bit-identical outputs
for all three models. Core ML's own defaults (compute units All) match for the segmenter and the
face model, but they give a different LaMa result, and on one of the four test images a broken one
(4.7 dB).

## Measurements

Setup: Apple M1 Pro (10 cores), macOS 27.2, ONNX Runtime 1.31.0 through `ort` 2.0.0-rc.15
(pyke prebuilt `aarch64-apple-darwin+coreml`), release build, models-v2 (all fp32). Throwaway
example, not committed.

Inputs:
- segmenter: synthetic `styles` images 00-03, resized to 512x512 and ImageNet-normalised;
- LaMa: the same images 00-03 (0..1 RGB) with a centred 128x128 hole;
- face: images 00-01 resized to 640x640, BGR 0..255.

Columns:
- load: a fresh session with no cache, files in the page cache;
- cache cold / warm: the first and the second load with `with_model_cache_dir` pointing at an
  empty folder;
- run: per tile, the fastest of 2 runs after one warm-up (LaMa medians are further down);
- vs (a), vs (d): the output difference against the automatic placement (a) and against the
  processor (d). For the segmenter it is the max probability difference per image, for LaMa the
  PSNR in the hole per image, for the face model the max output difference per image.

Configurations: (a) the automatic placement used until Task 1b, (b) explicit Core ML with default
options (All, NeuralNetwork), (c) explicit Core ML with CPUAndNeuralEngine and NeuralNetwork
(chosen), (d) `PreferCPU`. Also tried: MLProgram with All or CPUAndGPU, NeuralNetwork with
CPUAndGPU or CPUOnly, and the default options plus `RequireStaticInputShapes`, `FastPrediction`
or `EnableOnSubgraphs`.

### Segmenter

| config | load | cache cold / warm | run | Core ML nodes | vs (a) | vs (d) |
|---|---|---|---|---|---|---|
| (a) automatic | 16.1-16.9 s | - | 0.150 s | 3223 of 3231, 1 piece | 0 | 0.094 / 0.209 / 0.077 / 0.094 |
| (b) All, NeuralNetwork | 16.3 s | 16.1 / 0.83 s | 0.150 s | 3223 of 3231, 1 piece | 0 | as (a) |
| (c) CPUAndNeuralEngine, NeuralNetwork | 17.0 s | 16.9 / 0.89 s | 0.152 s | 3223 of 3231, 1 piece | 0 | as (a) |
| All, MLProgram | 6.0 s | 6.7 / 0.55 s | 0.205 s | 3231 of 3231 | as (d) | < 0.0001 |
| CPUAndGPU, MLProgram | 4.9 s | 4.8 / 0.30 s | 0.203 s | 3231 of 3231 | as (d) | < 0.0001 |
| CPUAndGPU, NeuralNetwork | 4.4 s | 4.3 / 1.97 s | 0.208 s | 3223 of 3231 | 0.061-0.253 | 0.077-0.196 |
| (d) `PreferCPU` | 0.26 s | - | 2.129 s | none (2181 nodes on the processor) | 0.094 / 0.209 / 0.077 / 0.094 | 0 |

Pixels on the other side of 0.5 between (a) and (d): 73 / 347 / 129 / 122 of 262144.

The 8 segmenter nodes Core ML leaves to the processor are small nodes at the end of the graph:
6 `Unsqueeze`, 1 `Concat` and 1 `ReduceMax`.

### LaMa

| config | load | cache cold / warm | run | Core ML nodes | vs (a) | vs (d) |
|---|---|---|---|---|---|---|
| (a) automatic | 15.6-18.3 s | - | 1.93-2.17 s | 1680 of 2841, 189 pieces | inf | 39.4 / 45.3 / 48.8 / 57.6 dB |
| (b) All, NeuralNetwork | 15.8-16.9 s | 16.1 / 2.40 s | 4.15-4.65 s | 1680 of 2841, 189 pieces | 4.7 / 70.2 / 71.9 / 74.7 dB | 4.7 / 45.3 / 48.9 / 57.3 dB |
| (c) CPUAndNeuralEngine, NeuralNetwork | 16.5-18.2 s | 16.5 / 2.00 s | 1.89-2.04 s | 1680 of 2841, 189 pieces | inf | as (a) |
| All or CPUAndGPU, MLProgram | 6.8-9.1 s | 6.6-6.8 / 0.50-1.42 s | 3.51-3.59 s | 2370 of 2841, 112 pieces | as (d) | 124-138 dB |
| CPUAndGPU, NeuralNetwork | 2.9 s | 2.6 / 1.22 s | 5.16 s | 1680 of 2841, 189 pieces | 4.7 / 57.4 dB (00-01) | 4.7 / 46.5 dB (00-01) |
| CPUOnly, NeuralNetwork | 2.2 s | - | 1.48 s | 1680 of 2841, 189 pieces | as (d) | 106-124 dB |
| (d) `PreferCPU` | 0.17-0.22 s | - | 2.15-2.20 s | none (2727 nodes on the processor) | 39.4 / 45.3 / 48.8 / 57.6 dB | inf |

The default options with `RequireStaticInputShapes`, `FastPrediction` or `EnableOnSubgraphs`
behave like (b): 4.7 dB on image 00 and 4.2-4.4 s per tile. (a), (b) and (c) were each loaded
two or three times and gave the same outputs every time, so the differences are not noise between
loads.

LaMa nodes Core ML rejects (measured in Plan 1 with MLProgram): `Einsum` (the Fourier units),
`Gather`, `Neg`, and `ConvTranspose` ("output_padding is not supported").

### Face (YuNet)

| config | load | cache cold / warm | run | Core ML nodes | vs (a) | vs (d) |
|---|---|---|---|---|---|---|
| (a) automatic | 0.44-0.46 s | - | 0.003-0.004 s | 106 of 106 | 0 | 0.023 / 0.032 |
| (b) All, NeuralNetwork | 0.40 s | 0.39 / 0.04 s | 0.003 s | 106 of 106 | 0 | as (a) |
| (c) CPUAndNeuralEngine, NeuralNetwork | 0.39 s | 0.39 / 0.04 s | 0.004 s | 106 of 106 | 0 | as (a) |
| All or CPUAndGPU, MLProgram | 0.12-0.19 s | 0.12-0.16 / 0.01-0.02 s | 0.005 s | 106 of 106 | as (d) | 0 |
| CPUAndGPU, NeuralNetwork | 0.11 s | 0.02 / 0.01 s | 0.003 s | 106 of 106 | 0.035 / 0.034 | 0.036 / 0.032 |
| (d) `PreferCPU` | < 0.01 s | - | 0.013 s | none (91 nodes on the processor) | 0.023 / 0.032 | 0 |

### Observations

- MLProgram computes in full precision: its outputs equal the processor's (segmenter difference
  below 0.0001, LaMa above 120 dB), and it loads 2-3 times faster. It was not chosen because
  Plan 1 was tuned on (a), which differs from the processor by up to 0.209 (segmenter) and down
  to 39.4 dB (LaMa). It is the candidate if the thresholds are ever tuned again on processor
  output.
- A session's verbose log adds nothing during runs (0 lines per run), so the engine logs every
  session verbosely while it is created and reads the placement from that log.

## Partial placement

Both partial placements are kept on Core ML, because neither is slower than the processor alone:

- Segmenter: Core ML runs 3223 of 3231 nodes; the 8 left are the small nodes listed above. It
  takes 0.15 s per tile against 2.13 s on the processor, 14 times faster. Reported as
  `Gpu { name: "Core ML", share: Some((3223, 3231)) }`.
- LaMa: Core ML runs 1680 of the 2841 nodes it is offered, in 189 pieces. Eight runs per session
  (inputs cycling through images 00-03), two sessions each, alternating:

  | | median per tile | range | mean |
  |---|---|---|---|
  | (c) Core ML | 2.05 / 2.02 s | 1.89-2.63 s | 2.20 / 2.13 s |
  | (d) processor | 2.19 / 2.29 s | 2.15-2.37 s | 2.19 / 2.27 s |

  The median tile is 6-12 % faster on Core ML, but the occasional slow Core ML run makes the mean
  about the same. LaMa is kept on Core ML because it is not slower, its outputs are exactly the
  ones Plan 1 was tuned on (on the processor, image 00 is 39.4 dB from them, just under the
  40 dB bar), and the app can then say "graphics card" for every model. The cost: the first LaMa
  load compiles for about 16 s; with `cache_dir`, later loads take about 2 s. Reported as
  `Gpu { name: "Core ML", share: Some((1680, 2877)) }`: the engine's total counts the compute
  nodes on both sides after partitioning, which is a few more than the 2841 Core ML was offered.
- Face: the whole model runs on Core ML (`share: None`).

On Windows the policy is unchanged: DirectML is used only for a model it runs completely.

## Core ML vs processor (the GPU parity test)

`gpu_matches_cpu` in `tests/engine.rs` compares the GPU engine with a `PreferCPU` engine on styles
image 00. On macOS it uses Core ML limits: segmenter max difference below 0.25 (measured 0.094;
0.077-0.209 over images 00-03) and LaMa hole PSNR above 35 dB (measured 39.4; 39.4-57.6 dB over
images 00-03). On Windows it keeps Plan 1's limits (0.02 and 40 dB).

## Loading delay

Load times and the optimized-model cache are in `model-loading.md`. In short: Core ML compiles
each model when its session is created (about 16 s each for the segmenter and LaMa), and
`EngineOptions.cache_dir` keeps the compiled models so that later loads take about 1-2 s. Each
model gets its own cache subfolder named `<label>-<file bytes>-<file mtime secs>`, because Core ML
reuses a compiled model as long as the graph's structure is unchanged, even when the weights
changed. Folders left by older files of the same model are deleted.

## History

Plan 1 measured Core ML with the fp16 models-v1 and MLProgram, and decided not to use it. The
report then said "processor" on Macs, but every session in fact ran on Core ML through the
automatic placement, so some of the "processor" figures in the old version of this note (for
example 0.17 s per tile for the fp32 segmenter) were Core ML figures. The tables above replace
them.

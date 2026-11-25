# Model loading and the optimized-model cache

Plan 2 loads each model on first use (`Engine::load`, or the first run of that model). This note
records how long a load takes and whether ONNX Runtime's optimized-model cache
(`SessionBuilder::with_optimized_model_path`) is worth adding.

Setup: ONNX Runtime 1.31.0 through `ort` 2.0.0-rc.15, Apple M1 Pro (10 cores), macOS, release
build, models-v2 (all fp32: segmenter 480 MB, LaMa 209 MB, face 0.2 MB). Each load time is the
median of 3 loads after one warm-up load, so the files are in the page cache and disk reads are
not part of the numbers. Run time is the faster of 2 runs on one 512x512 tile.

## Finding: sessions without an explicit provider use Core ML on macOS

`ort` 2.0.0-rc.15 calls `SessionOptionsSetEpSelectionPolicy(MaxEfficiency)` in every
`SessionBuilder::new` (`src/session/builder/mod.rs`). With that policy ONNX Runtime picks a
provider on its own, and on macOS it always registers Core ML. Its verbose log for the face model
built with no provider registered:

```
CoreMLExecutionProvider::GetCapability, number of partitions supported by CoreML: 1 number of nodes in the graph: 106 number of nodes supported by CoreML: 106
 All nodes placed on [CoreMLExecutionProvider]. Number of nodes: 1
```

With `.with_auto_device(AutoDevicePolicy::PreferCPU)` the same load logs
`All nodes placed on [CPUExecutionProvider]. Number of nodes: 106`.

Until Task 1b `engine.rs` never set the policy, so every session the engine reported as
`Cpu { NotRequested | GpuNotBeneficial(..) }` was in fact a Core ML session on macOS. Manually
registered providers take precedence over the policy, so the DirectML sessions on Windows were not
affected. Task 1 left this unchanged and reported it for a decision; the decision is below.

## Measurements

(a) is a normal load as the engine does it now. (b) is the second load of a file written by
`with_optimized_model_path`, loaded with `GraphOptimizationLevel::Disable`.

Normal loads, current engine (Core ML picked by the default policy):

| model | (a) load | run per tile |
|---|---|---|
| segmenter | 16.56 s | 0.15 s |
| LaMa | 16.79 s | 2.41 s |

The optimized file cannot be written in this mode. At every optimisation level (basic, extended,
layout, all) the load fails with "Unable to serialize model as it contains compiled nodes. Please
disable any execution providers which generate compiled nodes." Core ML compiles the model on
every load, and that compilation is the 16 s.

Processor only (`AutoDevicePolicy::PreferCPU`), cache written at each level:

| model | (a) load | written at | write | (b) load | (b) / (a) | run per tile (a) / (b) | max output diff |
|---|---|---|---|---|---|---|---|
| segmenter | 0.28 s | all | 0.82 s | 0.12 s | 42 % | 2.12 / 2.16 s | 0 |
| segmenter | | layout | 0.65 s | 0.11 s | 40 % | 2.12 / 2.14 s | 0 |
| segmenter | | extended | 0.62 s | 0.13 s | 44 % | 2.12 / 2.13 s | 2.7e-7 |
| segmenter | | basic | 0.69 s | 0.15 s | 54 % | 2.12 / 2.14 s | 2.7e-7 |
| LaMa | 0.14 s | all | 0.31 s | 0.08 s | 58 % | 2.14 / 2.14 s | 0 |
| LaMa | | layout, extended, basic | 0.27-0.32 s | 0.08 s | 58 % | 2.13-2.16 s | 0 |

The cached files are as large as the originals (479 MB and 209 MB).

## Decision

The cache is not adopted.

- With the engine as it is, the cache cannot be written at all (Core ML compiled nodes).
- On the processor alone the segmenter meets the 60 % bar (42 %). But the whole load is 0.28 s,
  so the cache would save about 0.16 s. In exchange it would add 690 MB of cache files, a
  staleness scheme and a fallback path. That is not worth it.

The "about 33 s to load" that motivated the cache is Core ML compiling the models, not ONNX
Runtime's graph optimisation. On the processor alone all three models load in well under a second
once the files are in the page cache.

## Decision on the policy (owner, 2026-11-25; Task 1b)

Macs use Core ML on purpose and the report says so. Since Task 1b:

- every session the engine creates sets `AutoDevicePolicy::PreferCPU`, so a session reported as
  `Cpu` runs on the processor on every platform (the tests check the session log);
- with `prefer_gpu` on macOS each model is loaded with Core ML registered explicitly, with the
  options that reproduce the automatic placement Plan 1 was tuned on, and reported as
  `Gpu { name: "Core ML" }` only when the session log shows Core ML runs it;
- `EngineOptions.cache_dir` gives Core ML a folder for its compiled models, which takes the
  16 s compile out of every load after the first.

Measurements and the per-model outcome are in `docs/notes/coreml.md`.

Before the decision, the whole test suite (`cargo test --workspace --release`, same machine) was
run with the policy as it was and with `PreferCPU` set in the engine's session builder; every
test passed both ways:

| test binary | Core ML (implicit) | `PreferCPU` |
|---|---|---|
| engine | 132 s | 3 s |
| detect | 31 s | 194 s |
| inpaint | 74 s | 58 s |
| pipeline | 60 s | 458 s |
| whole run (wall clock, with build) | 5 min 49 s | 12 min 39 s |

The speed figures in `docs/notes/coreml.md` were measured again in Task 1b.

How the cache measurement was made: a throwaway example (not committed) that loads each model with
`Session::builder`, then with `with_optimized_model_path(<file>)` at each `GraphOptimizationLevel`,
then loads the written file with `GraphOptimizationLevel::Disable`, and compares one run's output
on a constant 0.1 input.

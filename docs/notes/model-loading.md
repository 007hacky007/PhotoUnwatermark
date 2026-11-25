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

`engine.rs` never sets the policy, so every session the engine reports as
`Cpu { NotRequested | GpuNotBeneficial(..) }` is in fact a Core ML session on macOS. That
contradicts `MAC_GPU_REASON` and `docs/notes/coreml.md`. Manually registered providers take
precedence over the policy, so the DirectML sessions on Windows are not affected. The processor
fallback sessions on Windows may be, but that was not checked here. Task 1 leaves this behaviour
unchanged and reports it for a decision (see "Open question" below).

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

## Open question

Should the engine set `AutoDevicePolicy::PreferCPU` so that "processor" really means the processor?

- For: the device report would be honest again, and loads would drop from about 16 s per model to
  under 1 s. Results would match the processor numbers the GPU parity checks assume.
- Against: Core ML currently runs the fp32 segmenter at 0.15 s per tile, against 2.12 s on the
  processor (14 times slower), and coreml.md measured Core ML segmenter results 0.04-0.21 away
  from the processor's. Thresholds tuned in Plan 1 may have been tuned on Core ML output.

Whole test suite (`cargo test --workspace --release`, same machine) with the policy as now and
with `PreferCPU` set in the engine's session builder; every test passes both ways:

| test binary | as now | `PreferCPU` |
|---|---|---|
| engine | 132 s | 3 s |
| detect | 31 s | 194 s |
| inpaint | 74 s | 58 s |
| pipeline | 60 s | 458 s |
| whole run (wall clock, with build) | 5 min 49 s | 12 min 39 s |

The speed figures in `docs/notes/coreml.md` may need to be measured again. Its "processor"
segmenter fp32 time (0.17 s) matches the Core ML time measured here, not the processor time.

How the measurement was made: a throwaway example (not committed) that loads each model with
`Session::builder`, then with `with_optimized_model_path(<file>)` at each `GraphOptimizationLevel`,
then loads the written file with `GraphOptimizationLevel::Disable`, and compares one run's output
on a constant 0.1 input.

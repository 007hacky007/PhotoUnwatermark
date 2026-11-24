# Core ML on macOS (measured, not used in v1)

PhotoUnwatermark v1 never registers the Core ML execution provider. On macOS every model runs on
the processor and the device report says `Cpu { GpuNotBeneficial(..) }`. This note records the
measurements behind that.

Setup: ONNX Runtime 1.31.0 through `ort` 2.0.0-rc.15 (pyke prebuilt `aarch64-apple-darwin+coreml`),
Core ML provider with `ModelFormat::MLProgram`, on an Apple silicon Mac and on the GitHub
`macos-latest` runner. Release builds. Inputs: 512x512 tiles. Models are the models-v1 files
(segmenter and LaMa fp16) unless the row says fp32 (the fp32 export, shipped from models-v2).

## How much of each model Core ML takes

ONNX Runtime's verbose session log (`CoreMLExecutionProvider::GetCapability`):

| model | nodes taken / nodes in graph | partitions |
|---|---|---|
| segmenter | 4014 / 4014 | 1 |
| LaMa | 2517 / 2988 | 112 |
| face (YuNet) | 106 / 106 | 1 |

LaMa nodes Core ML rejects: 216 `Einsum` (the Fourier units), 72 `Gather`, 36 `Neg`, 3
`ConvTranspose` ("output_padding is not supported").

## Accuracy: segmenter, Core ML vs processor

Synthetic `styles` images 00-03, resized to 512x512, same model on both sides.

| compute units | max prob diff per image | mean diff | pixels on the other side of 0.5 |
|---|---|---|---|
| All | 0.185 / 0.102 / 0.085 / 0.183 | 0.0010-0.0014 | 140 / 129 / 166 / 225 |
| CPUAndGPU | 0.056 / 0.068 / 0.039 / 0.150 | 0.0004-0.0010 | 47 / 108 / 46 / 179 |
| CPUAndNeuralEngine | 0.114 / 0.108 / 0.073 / 0.142 | 0.0008-0.0013 | 94 / 200 / 124 / 181 |
| CPUAndGPU, fp32 model | 0.094 / 0.209 / 0.077 / 0.094 | 0.0007-0.0017 | 73 / 347 / 129 / 122 |

The project's GPU parity limit is a max diff below 0.02. No setting meets it, not even with fp32
weights, so the difference comes from Core ML's reduced-precision arithmetic rather than from the
fp16 weights. On `macos-latest` (Core ML active, compute units All) the first styles image gave
a max diff of 0.089.

## Speed per 512x512 tile (after warm-up)

| | processor | Core ML All | Core ML CPUAndGPU |
|---|---|---|---|
| segmenter fp16 | 2.4-2.6 s | 0.90-0.96 s | 0.21 s |
| segmenter fp32 | 0.17 s | - | 0.23 s |
| LaMa fp16 | 2.3-2.4 s | 3.5-3.8 s (112 partitions) | - |
| face | 8 ms | 25-37 ms | - |

With the fp32 segmenter the processor is faster than Core ML. A partly supported LaMa is slower
on Core ML than on the processor alone, because data crosses between the two at every
partition boundary.

## Loading delay (`Engine::new`, all three models)

| | time |
|---|---|
| processor only (models-v1) | 4.8-5.1 s |
| Core ML All | 41-55 s |
| Core ML CPUAndGPU | 12.5-14.7 s |
| Core ML CPUAndNeuralEngine | 226 s |

Core ML compiles the model every time a session is created; no model cache was configured.

## Conclusion

On macOS the processor is used. Core ML gave different results (diffs 2-10x the limit) and was
not faster than the processor for the fp32 models, and it adds seconds to minutes of load time.
Revisit if a later ONNX Runtime or Core ML gives full-precision GPU arithmetic.

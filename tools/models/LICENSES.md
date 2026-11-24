# Model licences

The engine ships three ONNX files, built by `export.py` and listed in `models/manifest.json`.
None of the model weights are part of this repository. This file goes into every release
next to the models.

## segmenter.onnx - watermark segmenter

- What: six UNet++ (efficientnet-b4) watermark segmenters, one per mark style (`centered_text`,
  `line_pattern`, `logo`, `overlay_text`, `repeated_text`, `tiny_corner`), merged into one graph
  that returns the pixel max of their sigmoid outputs. Shipped with fp32 weights (see the
  manifest's `dtype` and `note`).
- Upstream: https://huggingface.co/christophernavas/watermark-remover
  (files `segmenter_<specialty>.pth`, revision `509fb6833823f4e9c519f8e4f87a9053567c779c`),
  by DevynLabs.
- Licence: Apache License 2.0, as declared on the model card.
  Full text: https://www.apache.org/licenses/LICENSE-2.0.txt

## lama.onnx - LaMa (big-lama) inpainter

- What: LaMa, "Resolution-robust Large Mask Inpainting with Fourier Convolutions" (WACV 2022),
  Samsung Research. ONNX export (fixed 512x512, opset 17) by Carve
  (https://huggingface.co/Carve/LaMa-ONNX), taken from the byte-for-byte mirror below. Shipped
  with fp32 weights, batch fixed to 1 and constants folded (see the manifest's `dtype` and
  `note`).
- Upstream: https://github.com/advimman/lama; file `lama_fp32.onnx` from
  https://huggingface.co/ravetools/watermark-remover
  (revision `0bb7e443da9abf0813dcb0458d9fe5d20cb7445d`,
  sha256 `1faef5301d78db7dda502fe59966957ec4b79dd64e16f03ed96913c7a4eb68d6`).
- Licence: Apache License 2.0, Copyright 2021 Samsung Research.
  Full text: https://www.apache.org/licenses/LICENSE-2.0.txt
- Caveat on the weights: the upstream repository is licensed Apache 2.0 but has no separate
  licence for the pre-trained weights. Its maintainers have answered licence questions by
  pointing to Apache 2.0 (https://github.com/advimman/lama/issues/132,
  https://github.com/advimman/lama/issues/230), and Carve publishes the ONNX export under
  Apache 2.0. If the authors publish different terms for the weights, those terms apply.

## face.onnx - YuNet face detector

- What: YuNet face detector, `face_detection_yunet_2023mar.onnx`, shipped unchanged (fp32).
- Upstream: https://github.com/opencv/opencv_zoo/tree/main/models/face_detection_yunet
  (sha256 `8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4`).
- Licence: MIT. Full text
  (from https://github.com/opencv/opencv_zoo/blob/main/models/face_detection_yunet/LICENSE):

```
MIT License

Copyright (c) 2020 Shiqi Yu <shiqi.yu@gmail.com>

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

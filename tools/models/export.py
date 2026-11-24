"""Build the ONNX model candidates for the engine.

Writes into <out>/build/:
  segmenter.fp32.onnx  six UNet++ specialists merged by pixel max
  segmenter.fp16.onnx  same, fp16 except the small convs
  lama.upstream.onnx   LaMa (big-lama) inpainter, upstream file unchanged (parity reference)
  lama.fp32.onnx       same with batch fixed to 1 and constants folded
  lama.fp16.onnx       same as lama.fp32.onnx, fp16 except BatchNorm
  face.onnx            YuNet face detector, unchanged
Graph inputs and outputs stay float32 in every file.
verify.py then checks parity, copies the SHIPPED_DTYPE file of each model and writes
<out>/manifest.json.
"""
import argparse
import hashlib
import math
import shutil
import urllib.request
import warnings
from pathlib import Path

import onnx
import torch
from torch import nn

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_OUT = REPO_ROOT / "models"
SIZE = 512
# Segmenter convs with fewer weights than this stay fp32 in the fp16 candidate.
SMALL_CONV_WEIGHTS = 10_000

SEG_REPO = "christophernavas/watermark-remover"
SEG_REVISION = "509fb6833823f4e9c519f8e4f87a9053567c779c"
SPECIALTIES = ("centered_text", "line_pattern", "logo",
               "overlay_text", "repeated_text", "tiny_corner")

LAMA_REPO = "ravetools/watermark-remover"
LAMA_REVISION = "0bb7e443da9abf0813dcb0458d9fe5d20cb7445d"
LAMA_FILE = "lama_fp32.onnx"
LAMA_SHA256 = "1faef5301d78db7dda502fe59966957ec4b79dd64e16f03ed96913c7a4eb68d6"

FACE_URL = ("https://media.githubusercontent.com/media/opencv/opencv_zoo/main/models/"
            "face_detection_yunet/face_detection_yunet_2023mar.onnx")
FACE_SHA256 = "8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4"

# Release tag of the shipped files; also written into the manifest.
MODELS_VERSION = "models-v2"

# Shipped precision per model, an explicit choice (verify.py still checks parity of every
# candidate and fails if the shipped one does not pass). Chosen by CPU time per 512x512 tile
# with ONNX Runtime 1.31 (ort 2.0.0-rc.15) on Apple silicon, models-v1 -> models-v2:
#   segmenter  fp16 2.4-2.6 s, fp32 0.17 s  -> fp32 (the fp16 graph is about 15x slower)
#   lama       fp16 2.35 s, fp32 2.0 s (median of 7 after warm-up) -> fp32 (15 % faster)
#   face       upstream fp32 file
# fp32 also gives the closest match to PyTorch / upstream. Rule for a re-measure: ship the
# faster dtype; within 10 %, ship fp16.
SHIPPED_DTYPE = {"segmenter.onnx": "fp32", "lama.onnx": "fp32", "face.onnx": "fp32"}

# Per shipped file: licence and source, copied into the manifest by verify.py.
MODEL_INFO = {
    "segmenter.onnx": {
        "licence": "Apache-2.0",
        "source": f"https://huggingface.co/{SEG_REPO}/tree/{SEG_REVISION} "
                  f"segmenter_<specialty>.pth for {', '.join(SPECIALTIES)}, merged by pixel max",
    },
    "lama.onnx": {
        "licence": "Apache-2.0",
        "source": f"https://huggingface.co/{LAMA_REPO}/blob/{LAMA_REVISION}/{LAMA_FILE}",
    },
    "face.onnx": {
        "licence": "MIT",
        "source": FACE_URL,
    },
}


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def check_sha256(path, expected):
    got = sha256_file(path)
    if got != expected:
        raise SystemExit(f"sha256 mismatch for {path}: got {got}, expected {expected}")


def build_specialist(state_dict):
    import segmentation_models_pytorch as smp
    model = smp.UnetPlusPlus(encoder_name="efficientnet-b4", encoder_weights=None,
                             in_channels=3, classes=1)
    model.load_state_dict(state_dict)
    # Older efficientnet encoders use a memory-efficient swish that does not trace to ONNX.
    if hasattr(model.encoder, "set_swish"):
        model.encoder.set_swish(memory_efficient=False)
    return model.eval()


class MergedSegmenter(nn.Module):
    """ImageNet-normalised RGB in, pixel max of the specialists' sigmoids out."""

    def __init__(self, specialists):
        super().__init__()
        self.specialists = nn.ModuleList(specialists)

    def forward(self, image):
        probs = [torch.sigmoid(m(image)) for m in self.specialists]
        return torch.amax(torch.stack(probs), dim=0)


def load_segmenter():
    """The merged PyTorch segmenter (downloads the checkpoints into the HF cache)."""
    from huggingface_hub import hf_hub_download
    specialists = []
    for name in SPECIALTIES:
        path = hf_hub_download(SEG_REPO, f"segmenter_{name}.pth", revision=SEG_REVISION)
        specialists.append(build_specialist(
            torch.load(path, map_location="cpu", weights_only=True)))
    return MergedSegmenter(specialists).eval()


def to_fp16(model, dst, op_block_list=(), node_block_list=()):
    """fp16 weights and compute, float32 graph inputs and outputs.

    Ops in op_block_list (on top of the converter's defaults) and nodes in node_block_list
    stay fp32, with casts around them.
    """
    from onnxconverter_common import float16
    with warnings.catch_warnings():
        # One warning per tiny weight clamped to the fp16 range; parity is checked in verify.py.
        warnings.filterwarnings("ignore", message=".*will be truncated to")
        model = float16.convert_float_to_float16(
            model, keep_io_types=True,
            op_block_list=float16.DEFAULT_OP_BLOCK_LIST + list(op_block_list),
            node_block_list=list(node_block_list))
    onnx.save(model, str(dst))


def small_convs(model, max_weights=SMALL_CONV_WEIGHTS):
    """Names of the Conv nodes with fewer than max_weights weights."""
    sizes = {t.name: math.prod(t.dims) for t in model.graph.initializer}
    return [n.name for n in model.graph.node
            if n.op_type == "Conv" and sizes.get(n.input[1], max_weights) < max_weights]


def export_segmenter(build):
    fp32 = build / "segmenter.fp32.onnx"
    model = load_segmenter()
    with torch.no_grad(), warnings.catch_warnings():
        # The TorchScript exporter is deliberate (the dynamo one needs onnxscript); its
        # deprecation notice and the strided-Slice folding notes are not actionable.
        warnings.filterwarnings("ignore", category=DeprecationWarning)
        warnings.filterwarnings("ignore", message="Constant folding")
        torch.onnx.export(model, (torch.zeros(1, 3, SIZE, SIZE),), str(fp32),
                          input_names=["image"], output_names=["prob"],
                          opset_version=17, dynamo=False)
    # The exporter leaves the output dims symbolic; the shape is fixed, so record it.
    graph = onnx.load(str(fp32))
    for dim, size in zip(graph.graph.output[0].type.tensor_type.shape.dim, (1, 1, SIZE, SIZE)):
        dim.ClearField("dim_param")
        dim.dim_value = size
    onnx.checker.check_model(graph, full_check=True)
    onnx.save(graph, str(fp32))
    # With every conv in fp16 the probabilities drift by up to 0.13 on the styles set; the
    # small convs (squeeze-excitation, depthwise: about 2 % of the weights) are the sensitive ones.
    to_fp16(graph, build / "segmenter.fp16.onnx", node_block_list=small_convs(graph))


def fold_constants(src, dst):
    """Fix the batch dimension to 1 and fold constants (ONNX Runtime basic optimisations).

    LaMa builds its Fourier-unit DFT matrices from tensor shapes at run time; the fp16
    converter mistypes that subgraph, so it is folded into initializers first.
    """
    import onnxruntime as ort
    model = onnx.load(str(src))
    for value in list(model.graph.input) + list(model.graph.output):
        dim = value.type.tensor_type.shape.dim[0]
        dim.ClearField("dim_param")
        dim.dim_value = 1
    fixed = dst.with_suffix(".batch1.onnx")
    onnx.save(model, str(fixed))
    opts = ort.SessionOptions()
    opts.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_BASIC
    opts.optimized_model_filepath = str(dst)
    ort.InferenceSession(str(fixed), opts, providers=["CPUExecutionProvider"])
    fixed.unlink()
    strip_unused_opsets(dst)


def strip_unused_opsets(path):
    """Drop opset_import entries no node uses (ORT adds com.microsoft.nchwc and similar)."""
    model = onnx.load(str(path))
    used = {n.domain for n in model.graph.node} | {""}
    kept = [o for o in model.opset_import if o.domain in used or o.domain == "ai.onnx"]
    del model.opset_import[:]
    model.opset_import.extend(kept)
    onnx.checker.check_model(model, full_check=True)
    onnx.save(model, str(path))


def export_lama(build):
    from huggingface_hub import hf_hub_download
    path = hf_hub_download(LAMA_REPO, LAMA_FILE, revision=LAMA_REVISION)
    check_sha256(path, LAMA_SHA256)
    upstream = build / "lama.upstream.onnx"
    shutil.copyfile(path, upstream)
    folded = build / "lama.fp32.onnx"
    fold_constants(upstream, folded)
    # With BatchNormalization in fp16 the PSNR inside the hole drops to 21-32 dB on the
    # styles set (59-78 dB with it in fp32), so it stays fp32.
    to_fp16(onnx.load(str(folded)), build / "lama.fp16.onnx",
            op_block_list=["BatchNormalization"])


def export_face(build):
    dst = build / "face.onnx"
    tmp = dst.with_suffix(".part")
    with urllib.request.urlopen(FACE_URL) as resp, open(tmp, "wb") as f:
        shutil.copyfileobj(resp, f)
    check_sha256(tmp, FACE_SHA256)
    tmp.replace(dst)


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", type=Path, default=DEFAULT_OUT,
                    help="models directory (default: <repo>/models)")
    args = ap.parse_args()
    build = args.out / "build"
    build.mkdir(parents=True, exist_ok=True)
    for step in (export_segmenter, export_lama, export_face):
        print(f"{step.__name__} ...", flush=True)
        step(build)
    for p in sorted(build.glob("*.onnx")):
        print(f"{p.name}: {p.stat().st_size / 1e6:.1f} MB")


if __name__ == "__main__":
    main()

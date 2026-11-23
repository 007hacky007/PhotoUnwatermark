"""Check parity of the exported models and write <models>/manifest.json.

On the synthetic `styles` set (tools/synth/make_sets.py), resized to 512x512:
  segmenter  ONNX vs PyTorch, per image: max abs prob diff < 0.02 and IoU(prob > 0.5) > 0.99.
             fp16 ships if it passes, else fp32 (which must pass).
  lama       fp16 vs fp32, per image: PSNR inside the hole > 40 dB, else fp32 ships.
  face       shipped fp32 unchanged.
The chosen files are copied to <models>/ and listed with dtype, sha256, bytes, licence
and source. Exits non-zero on any failure (and then writes no manifest).
"""
import argparse
import json
import random
import shutil
import sys
import tempfile
from pathlib import Path

import numpy as np
import onnxruntime as ort
from PIL import Image

import export

SYNTH_DIR = Path(__file__).resolve().parents[1] / "synth"
MEAN = np.array([0.485, 0.456, 0.406], np.float32)
STD = np.array([0.229, 0.224, 0.225], np.float32)
PROB_MAX_DIFF = 0.02
PROB_MIN_IOU = 0.99
LAMA_MIN_PSNR = 40.0


def iou(a, b):
    union = np.logical_or(a, b).sum()
    return 1.0 if union == 0 else float(np.logical_and(a, b).sum() / union)


def psnr_in_hole(ref, out, hole):
    """PSNR (0-255 scale) of out vs ref over the hole pixels; arrays are CHW, hole HW."""
    diff = (np.asarray(ref, np.float64) - np.asarray(out, np.float64))[:, hole]
    mse = float(np.mean(diff ** 2))
    return float("inf") if mse == 0 else 10 * np.log10(255.0 ** 2 / mse)


def load_styles(styles_dir):
    """[(name, rgb float 0-1 HWC 512x512, hole bool HW)] from a styles set folder."""
    manifest = json.loads((styles_dir / "manifest.json").read_text())
    size = (export.SIZE, export.SIZE)
    items = []
    for item in manifest["items"]:
        name = item["name"]
        img = Image.open(styles_dir / "marked" / f"{name}.png").convert("RGB")
        mask = Image.open(styles_dir / "mask" / f"{name}.png").convert("L")
        rgb = np.asarray(img.resize(size, Image.BILINEAR), np.float32) / 255.0
        hole = np.asarray(mask.resize(size, Image.NEAREST)) > 127
        items.append((name, rgb, hole))
    return items


def generate_styles(root):
    sys.path.insert(0, str(SYNTH_DIR))
    import make_sets
    # Same seeding as `make_sets.py --seed 0`.
    make_sets.build_styles(root, make_sets.load_sources(None), random.Random("0:styles"))
    return root / "styles"


def session(path):
    return ort.InferenceSession(str(path), providers=["CPUExecutionProvider"])


def segmenter_input(rgb):
    return ((rgb - MEAN) / STD).transpose(2, 0, 1)[None].astype(np.float32)


def check_segmenter(build, items):
    """Returns (dtype to ship, failures)."""
    import torch
    model = export.load_segmenter()
    inputs = [(name, segmenter_input(rgb)) for name, rgb, _ in items]
    with torch.no_grad():
        refs = [model(torch.from_numpy(x)).numpy() for _, x in inputs]
    results = {}
    for dtype in ("fp32", "fp16"):
        try:
            sess = session(build / f"segmenter.{dtype}.onnx")
        except Exception as e:  # e.g. kernels missing for fp16 on this runtime
            print(f"segmenter {dtype}: cannot load: {e}")
            results[dtype] = False
            continue
        ok = True
        for (name, x), ref in zip(inputs, refs):
            prob = sess.run(["prob"], {"image": x})[0]
            diff = float(np.abs(prob - ref).max())
            score = iou(prob > 0.5, ref > 0.5)
            passed = diff < PROB_MAX_DIFF and score > PROB_MIN_IOU
            ok &= passed
            print(f"segmenter {dtype} {name}: max|dprob| {diff:.5f}  IoU {score:.5f}  "
                  f"mask px {int((ref > 0.5).sum())}  {'ok' if passed else 'FAIL'}")
        results[dtype] = ok
    if results["fp16"]:
        return "fp16", []
    if results["fp32"]:
        print("segmenter: fp16 failed parity, shipping fp32")
        return "fp32", []
    return None, ["segmenter fp32 failed parity with PyTorch"]


def check_lama(build, items):
    """Returns (dtype to ship, failures)."""
    ref_sess = session(build / "lama.fp32.onnx")
    try:
        cand = session(build / "lama.fp16.onnx")
    except Exception as e:
        print(f"lama fp16: cannot load: {e}\nlama: shipping fp32")
        return "fp32", []
    ok = True
    for name, rgb, hole in items:
        feed = {"image": rgb.transpose(2, 0, 1)[None].copy(),
                "mask": hole[None, None].astype(np.float32)}
        ref = ref_sess.run(["output"], feed)[0][0]
        out = cand.run(["output"], feed)[0][0]
        score = psnr_in_hole(ref, out, hole)
        passed = score > LAMA_MIN_PSNR
        ok &= passed
        print(f"lama fp16 {name}: PSNR in hole {score:.2f} dB  hole px {int(hole.sum())}  "
              f"{'ok' if passed else 'FAIL'}")
    if ok:
        return "fp16", []
    print("lama: fp16 below the PSNR threshold, shipping fp32")
    return "fp32", []


def check_face(build):
    path = build / "face.onnx"
    try:
        export.check_sha256(path, export.FACE_SHA256)
        session(path)
    except (SystemExit, Exception) as e:
        return [f"face: {e}"]
    return []


PRECISION_NOTES = {
    ("segmenter.onnx", "fp16"): "fp16 weights, small convolutions kept fp32",
    ("segmenter.onnx", "fp32"): "fp32 weights, unchanged precision",
    ("lama.onnx", "fp16"): ("fp16 weights, BatchNormalization kept fp32, batch fixed to 1, "
                            "constant-folded"),
    ("lama.onnx", "fp32"): "fp32 weights, batch fixed to 1, constant-folded",
    ("face.onnx", "fp32"): "unchanged upstream file",
}


def manifest_entry(path, dtype):
    return {"name": path.name, "sha256": export.sha256_file(path),
            "bytes": path.stat().st_size, "dtype": dtype,
            "note": PRECISION_NOTES[(path.name, dtype)], **export.MODEL_INFO[path.name]}


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", type=Path, default=export.DEFAULT_OUT,
                    help="models directory (default: <repo>/models)")
    ap.add_argument("--styles", type=Path,
                    help="existing styles set folder (default: generate one in a temp dir)")
    args = ap.parse_args()
    build = args.out / "build"
    manifest_path = args.out / "manifest.json"
    manifest_path.unlink(missing_ok=True)

    with tempfile.TemporaryDirectory() as tmp:
        styles = args.styles or generate_styles(Path(tmp))
        items = load_styles(styles)
    seg_dtype, failures = check_segmenter(build, items)
    lama_dtype, lama_failures = check_lama(build, items)
    failures += lama_failures + check_face(build)
    if failures:
        for f in failures:
            print(f"FAILED: {f}")
        return 1

    chosen = [(build / f"segmenter.{seg_dtype}.onnx", "segmenter.onnx", seg_dtype),
              (build / f"lama.{lama_dtype}.onnx", "lama.onnx", lama_dtype),
              (build / "face.onnx", "face.onnx", "fp32")]
    entries = []
    for src, name, dtype in chosen:
        dst = args.out / name
        shutil.copyfile(src, dst)
        entries.append(manifest_entry(dst, dtype))
    manifest_path.write_text(json.dumps({"version": "models-v1", "files": entries},
                                        indent=2) + "\n")
    total = sum(e["bytes"] for e in entries)
    for e in entries:
        print(f"{e['name']}: {e['dtype']}  {e['bytes'] / 1e6:.1f} MB")
    print(f"total: {total / 1e6:.1f} MB (target <= 500 MB)")
    return 0


if __name__ == "__main__":
    sys.exit(main())

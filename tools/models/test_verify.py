"""Checks on the built models directory (run export.py and verify.py first).

The directory defaults to <repo>/models; set PUW_MODELS_DIR to check another one.
"""
import hashlib
import json
import os
from pathlib import Path

import numpy as np
import onnx
import pytest

import export
import verify

MODELS = Path(os.environ.get("PUW_MODELS_DIR",
                             Path(__file__).resolve().parents[2] / "models"))
EXPECTED = {"segmenter.onnx", "lama.onnx", "face.onnx"}


@pytest.fixture(scope="module")
def manifest():
    path = MODELS / "manifest.json"
    assert path.is_file(), f"{path} missing: run tools/models/export.py and verify.py"
    return json.loads(path.read_text())


def io_signature(path):
    graph = onnx.load(str(path), load_external_data=False).graph
    init = {i.name for i in graph.initializer}

    def sig(values):
        return {v.name: (v.type.tensor_type.elem_type,
                         [d.dim_value for d in v.type.tensor_type.shape.dim])
                for v in values if v.name not in init}
    return sig(graph.input), sig(graph.output)


FLOAT = onnx.TensorProto.FLOAT


def test_manifest_hashes_match_files(manifest):
    assert manifest["version"] == export.MODELS_VERSION
    assert {f["name"] for f in manifest["files"]} == EXPECTED
    for entry in manifest["files"]:
        data = (MODELS / entry["name"]).read_bytes()
        assert entry["bytes"] == len(data), entry["name"]
        assert entry["sha256"] == hashlib.sha256(data).hexdigest(), entry["name"]
        assert entry["dtype"] in ("fp16", "fp32")
        assert entry["licence"] and entry["source"]


def test_shipped_dtypes_are_the_explicit_choice(manifest):
    assert {f["name"]: f["dtype"] for f in manifest["files"]} == export.SHIPPED_DTYPE


def test_segmenter_io_names(manifest):
    inputs, outputs = io_signature(MODELS / "segmenter.onnx")
    assert inputs == {"image": (FLOAT, [1, 3, 512, 512])}
    assert outputs == {"prob": (FLOAT, [1, 1, 512, 512])}


def test_lama_io_names(manifest):
    inputs, outputs = io_signature(MODELS / "lama.onnx")
    # Fixed shapes, batch 1: the engine requires fixed-size inputs.
    assert inputs == {"image": (FLOAT, [1, 3, 512, 512]), "mask": (FLOAT, [1, 1, 512, 512])}
    assert outputs == {"output": (FLOAT, [1, 3, 512, 512])}


def test_iou_and_psnr():
    a = np.zeros((4, 4), bool)
    b = a.copy()
    assert verify.iou(a, b) == 1.0
    a[:2] = True
    b[:1] = True
    assert verify.iou(a, b) == pytest.approx(0.5)
    ref = np.full((3, 4, 4), 100.0)
    hole = np.zeros((4, 4), bool)
    hole[0, 0] = True
    out = ref.copy()
    out[:, 1, 1] = 0  # outside the hole: ignored
    assert verify.psnr_in_hole(ref, out, hole) == float("inf")
    out[:, 0, 0] = 99
    assert verify.psnr_in_hole(ref, out, hole) == pytest.approx(20 * np.log10(255), rel=1e-6)

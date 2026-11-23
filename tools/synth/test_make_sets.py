import json
import sys
from pathlib import Path

import pytest
from PIL import Image

sys.path.insert(0, str(Path(__file__).parent))
import make_sets  # noqa: E402


@pytest.fixture(scope="module")
def built(tmp_path_factory):
    out = tmp_path_factory.mktemp("sets")
    make_sets.build_all(out, seed=0)
    return out


def load_manifest(root, name):
    return json.loads((root / name / "manifest.json").read_text())


def test_deterministic(built, tmp_path):
    make_sets.build_all(tmp_path, seed=0)
    for name in make_sets.SET_NAMES:
        for item in load_manifest(built, name)["items"]:
            a = (built / name / "mask" / f"{item['name']}.png").read_bytes()
            b = (tmp_path / name / "mask" / f"{item['name']}.png").read_bytes()
            assert a == b
        assert (built / name / "manifest.json").read_bytes() == (
            tmp_path / name / "manifest.json"
        ).read_bytes()


def test_manifest_matches_files(built):
    for name in make_sets.SET_NAMES:
        m = load_manifest(built, name)
        assert m["set"] == name
        assert m["kind"] in {"styles", "fixed", "random", "negative"}
        assert m["items"]
        for item in m["items"]:
            sizes = set()
            for sub in ("clean", "marked", "mask"):
                p = built / name / sub / f"{item['name']}.png"
                assert p.is_file()
                sizes.add(Image.open(p).size)
            assert len(sizes) == 1


def test_fixed_set_shape(built):
    m = load_manifest(built, "fixed")
    assert m["kind"] == "fixed"
    assert len(m["items"]) == 8
    assert len({tuple(i["bbox"]) for i in m["items"]}) == 1
    for i in m["items"]:
        assert Image.open(built / "fixed" / "marked" / f"{i['name']}.png").size == (1536, 1024)


def test_random_bbox_inside_image(built):
    m = load_manifest(built, "random")
    assert len(m["items"]) == 12
    orients, longs = set(), set()
    for item in m["items"]:
        w, h = Image.open(built / "random" / "marked" / f"{item['name']}.png").size
        x, y, bw, bh = item["bbox"]
        assert x >= 0 and y >= 0 and bw > 0 and bh > 0
        assert x + bw <= w and y + bh <= h
        orients.add(w > h)
        longs.add(max(w, h))
    assert orients == {True, False}
    assert longs == {1536, 1024}


def test_negative_has_empty_masks(built):
    m = load_manifest(built, "negative")
    assert len(m["items"]) == 6
    for item in m["items"]:
        assert item["has_mark"] is False and item["bbox"] is None
        mask = Image.open(built / "negative" / "mask" / f"{item['name']}.png")
        assert mask.getextrema() == (0, 0)
        assert (
            Image.open(built / "negative" / "clean" / f"{item['name']}.png").tobytes()
            == Image.open(built / "negative" / "marked" / f"{item['name']}.png").tobytes()
        )


def test_marks_change_pixels_and_bbox_matches_mask(built):
    for name in ("styles", "fixed", "random"):
        for item in load_manifest(built, name)["items"]:
            assert item["has_mark"] is True
            mask = Image.open(built / name / "mask" / f"{item['name']}.png")
            assert mask.getbbox() is not None
            x, y, w, h = item["bbox"]
            assert mask.getbbox() == (x, y, x + w, y + h)

"""Deterministic generator of synthetic watermark test sets.

Writes, per set, clean/marked/mask PNG folders plus manifest.json.
Photos come from scikit-image sample data (or --src); marks are generated.
"""
import argparse
import json
import random
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

SET_NAMES = ("styles", "fixed", "random", "negative")
SAMPLE_NAMES = (
    "astronaut", "chelsea", "coffee", "rocket",
    "cat", "immunohistochemistry", "retina", "hubble_deep_field",
)
MARK_TEXTS = ("SAMPLE", "PREVIEW", "(c) EXAMPLE STUDIO", "PROOF ONLY")
FIXED_SIZE = (1536, 1024)
FIXED_ALPHA = 0.25
MASK_THRESHOLD = 8


def load_sources(src_dir):
    if src_dir:
        paths = sorted(p for p in Path(src_dir).iterdir()
                       if p.suffix.lower() in {".png", ".jpg", ".jpeg"})
        if not paths:
            raise SystemExit(f"no images in {src_dir}")
        return [Image.open(p).convert("RGB") for p in paths]
    from skimage import data
    return [Image.fromarray(getattr(data, n)()).convert("RGB") for n in SAMPLE_NAMES]


def make_photo(sources, index, size, rng):
    """Random crop of a source at the target aspect, resized to size."""
    src = sources[index % len(sources)]
    w, h = size
    zoom = rng.uniform(0.6, 1.0)
    cw = min(src.width, max(1, round(min(src.width, src.height * w / h) * zoom)))
    ch = min(src.height, max(1, round(cw * h / w)))
    cw = min(cw, round(ch * w / h))
    x = rng.randint(0, src.width - cw)
    y = rng.randint(0, src.height - ch)
    img = src.crop((x, y, x + cw, y + ch))
    if rng.random() < 0.5:
        img = img.transpose(Image.FLIP_LEFT_RIGHT)
    return img.resize(size, Image.LANCZOS)


def font(size):
    return ImageFont.load_default(size=max(8, round(size)))


def text_shape(size, xy, text, fnt, outline_px=0):
    """L layer, 255 where the text is drawn. outline_px > 0: outline lettering."""
    layer = Image.new("L", size, 0)
    d = ImageDraw.Draw(layer)
    if outline_px:
        d.text(xy, text, font=fnt, fill=0, stroke_width=outline_px, stroke_fill=255)
    else:
        d.text(xy, text, font=fnt, fill=255)
    return layer


def text_extent(text, fnt, outline_px=0):
    d = ImageDraw.Draw(Image.new("L", (1, 1)))
    return d.textbbox((0, 0), text, font=fnt, stroke_width=outline_px)


def apply_mark(photo, shape, alpha, color=255):
    """Blend a flat colour through shape*alpha; return (marked, mask)."""
    a = np.asarray(shape, dtype=np.float32) / 255.0 * alpha
    base = np.asarray(photo, dtype=np.float32)
    out = base * (1 - a[..., None]) + color * a[..., None]
    marked = Image.fromarray(np.clip(np.rint(out), 0, 255).astype(np.uint8))
    mask = Image.fromarray(((a * 255) > MASK_THRESHOLD).astype(np.uint8) * 255)
    return marked, mask.filter(ImageFilter.MaxFilter(3))


def centered_text(size, rng):
    fnt = font(size[1] / 7)
    l, t, r, b = text_extent("SAMPLE", fnt)
    xy = ((size[0] - (r - l)) // 2 - l, (size[1] - (b - t)) // 2 - t)
    return text_shape(size, xy, "SAMPLE", fnt), 0.35


def corner_logo(size, rng):
    layer = Image.new("L", size, 0)
    d = ImageDraw.Draw(layer)
    s = size[1] // 6
    x0, y0 = size[0] - s * 3 - 24, size[1] - s - 24
    d.rounded_rectangle((x0, y0, x0 + s * 3, y0 + s), radius=s // 4, fill=255)
    d.ellipse((x0 + s // 6, y0 + s // 6, x0 + s - s // 6, y0 + s - s // 6), fill=0)
    fnt = font(s * 0.4)
    d.text((x0 + s * 1.15, y0 + s * 0.28), "STUDIO", font=fnt, fill=0)
    return layer, 0.5


def tiled_text(size, rng):
    diag = int(np.hypot(*size)) + 8
    big = Image.new("L", (diag, diag), 0)
    d = ImageDraw.Draw(big)
    fnt = font(size[1] / 14)
    step_x, step_y = int(size[0] / 3.2), int(size[1] / 4.5)
    for row, y in enumerate(range(0, diag, step_y)):
        for x in range(-step_x * (row % 2) // 2, diag, step_x):
            d.text((x, y), "PREVIEW", font=fnt, fill=255)
    big = big.rotate(30, resample=Image.BICUBIC)
    ox, oy = (diag - size[0]) // 2, (diag - size[1]) // 2
    return big.crop((ox, oy, ox + size[0], oy + size[1])), 0.25


def overlay_lines(size, rng):
    layer = Image.new("L", size, 0)
    d = ImageDraw.Draw(layer)
    gap = size[0] // 8
    for x in range(-size[1], size[0], gap):
        d.line((x, size[1], x + size[1], 0), fill=255, width=3)
    d.line((0, size[1] // 2, size[0], size[1] // 2), fill=255, width=3)
    return layer, 0.3


def outline_lettering(size, rng):
    fnt = font(size[1] / 11)
    sw = max(2, size[1] // 200)
    l, t, r, b = text_extent("(c) EXAMPLE STUDIO", fnt, sw)
    xy = ((size[0] - (r - l)) // 2 - l, size[1] // 5 - t)
    return text_shape(size, xy, "(c) EXAMPLE STUDIO", fnt, sw), 0.5


STYLES = (
    ("centered_text", centered_text),
    ("corner_logo", corner_logo),
    ("tiled_text", tiled_text),
    ("overlay_lines", overlay_lines),
    ("outline_lettering", outline_lettering),
)


def outline_mark(size, long_side, rng, xy=None, text="PREVIEW"):
    """Outline text mark sized by the long side; random position unless given."""
    fnt = font(long_side * 0.06)
    sw = max(2, round(long_side * 0.004))
    l, t, r, b = text_extent(text, fnt, sw)
    tw, th = r - l, b - t
    margin = 4
    if xy is None:
        xy = (rng.randint(margin, size[0] - tw - margin),
              rng.randint(margin, size[1] - th - margin))
    return text_shape(size, (xy[0] - l, xy[1] - t), text, fnt, sw), xy


def save_item(root, set_name, name, clean, marked, mask):
    for sub, img in (("clean", clean), ("marked", marked), ("mask", mask)):
        folder = root / set_name / sub
        folder.mkdir(parents=True, exist_ok=True)
        img.save(folder / f"{name}.png")
    box = mask.getbbox()
    return {
        "name": name,
        "has_mark": box is not None,
        "bbox": None if box is None else [box[0], box[1], box[2] - box[0], box[3] - box[1]],
    }


def write_manifest(root, set_name, kind, items):
    path = root / set_name / "manifest.json"
    path.write_text(json.dumps({"set": set_name, "kind": kind, "items": items},
                               indent=2) + "\n")


def build_styles(root, sources, rng):
    items = []
    for i, (label, fn) in enumerate(STYLES):
        clean = make_photo(sources, i, (1024, 683), rng)
        shape, alpha = fn(clean.size, rng)
        marked, mask = apply_mark(clean, shape, alpha)
        items.append(save_item(root, "styles", f"{i:02d}_{label}", clean, marked, mask))
    write_manifest(root, "styles", "styles", items)


def build_fixed(root, sources, rng):
    shape_rng = random.Random(rng.random())
    shape, _ = outline_mark(FIXED_SIZE, FIXED_SIZE[0], shape_rng)
    items = []
    for i in range(8):
        clean = make_photo(sources, i, FIXED_SIZE, rng)
        marked, mask = apply_mark(clean, shape, FIXED_ALPHA)
        items.append(save_item(root, "fixed", f"{i:02d}", clean, marked, mask))
    write_manifest(root, "fixed", "fixed", items)


def build_random(root, sources, rng):
    items = []
    for i in range(12):
        long_side = (1536, 1024)[(i // 2) % 2]
        short_side = long_side * 2 // 3
        size = (short_side, long_side) if i % 2 else (long_side, short_side)
        clean = make_photo(sources, i + 3, size, rng)
        shape, _ = outline_mark(size, long_side, rng, text=rng.choice(MARK_TEXTS))
        marked, mask = apply_mark(clean, shape, rng.uniform(0.2, 0.35))
        items.append(save_item(root, "random", f"{i:02d}", clean, marked, mask))
    write_manifest(root, "random", "random", items)


def build_negative(root, sources, rng):
    items = []
    for i in range(6):
        clean = make_photo(sources, i + 1, (1024, 683), rng)
        mask = Image.new("L", clean.size, 0)
        items.append(save_item(root, "negative", f"{i:02d}", clean, clean.copy(), mask))
    write_manifest(root, "negative", "negative", items)


BUILDERS = {"styles": build_styles, "fixed": build_fixed,
            "random": build_random, "negative": build_negative}


def build_all(out, seed=0, src=None):
    out = Path(out)
    sources = load_sources(src)
    for name in SET_NAMES:
        BUILDERS[name](out, sources, random.Random(f"{seed}:{name}"))


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--out", required=True, type=Path, help="output directory")
    ap.add_argument("--src", type=Path, help="folder of clean photos (default: scikit-image samples)")
    ap.add_argument("--seed", type=int, default=0)
    args = ap.parse_args()
    build_all(args.out, args.seed, args.src)
    print(f"wrote sets {', '.join(SET_NAMES)} to {args.out}")


if __name__ == "__main__":
    main()

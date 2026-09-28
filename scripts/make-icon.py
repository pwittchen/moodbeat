#!/usr/bin/env python3
"""Turns logo.png (a rounded-square icon on a flat white background) into the app icon source.

Detects the rounded square, fits its corner radius, cuts it out with an anti-aliased mask
(so no white fringe survives), and centers it on a transparent 1024x1024 canvas at Apple's
824px icon-body size. Then regenerate all platform icons with:

    python3 scripts/make-icon.py && npx tauri icon core/icons/app-icon.png -o core/icons

Requires Pillow.
"""

import math
import sys
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "logo.png"
OUT = ROOT / "core" / "icons" / "app-icon.png"

CANVAS, BODY = 1024, 824  # Apple icon grid: 824px body, 100px transparent margin
SUPERSAMPLE = 4
FRINGE_INSET = 2  # source px trimmed off the edge to drop anti-aliased white


def is_background(px) -> bool:
    return sum(px[:3]) >= 600  # near-white


def find_bounds(im):
    w, h = im.size
    px = im.load()
    mid_y, mid_x = h // 2, w // 2
    left = next(x for x in range(w) if not is_background(px[x, mid_y]))
    right = next(x for x in reversed(range(w)) if not is_background(px[x, mid_y]))
    top = next(y for y in range(h) if not is_background(px[mid_x, y]))
    bottom = next(y for y in reversed(range(h)) if not is_background(px[mid_x, y]))
    return left, top, right, bottom


def fit_radius(im, left, top, right, bottom) -> int:
    """Fits a circular corner radius to the top-left edge profile."""
    px = im.load()
    profile = []  # (depth below top edge, inset from left edge)
    for depth in range(0, min(400, (bottom - top) // 2), 4):
        y = top + depth
        x = next((x for x in range(left, right) if not is_background(px[x, y])), None)
        if x is not None:
            profile.append((depth, x - left))

    def error(r):
        total = 0.0
        for d, inset in profile:
            expected = r - math.sqrt(max(0.0, r * r - (r - d) ** 2)) if d < r else 0.0
            total += (expected - inset) ** 2
        return total

    return min(range(20, min(right - left, bottom - top) // 2), key=error)


def main() -> int:
    if not SRC.exists():
        print(f"missing {SRC}", file=sys.stderr)
        return 1
    src = Image.open(SRC).convert("RGB")
    left, top, right, bottom = find_bounds(src)
    radius = fit_radius(src, left, top, right, bottom)
    crop = src.crop((left, top, right + 1, bottom + 1))
    w, h = crop.size
    print(f"square {w}x{h} at ({left},{top}), corner radius {radius}px")

    ss = SUPERSAMPLE
    mask = Image.new("L", (w * ss, h * ss), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        (FRINGE_INSET * ss, FRINGE_INSET * ss, (w - FRINGE_INSET) * ss - 1, (h - FRINGE_INSET) * ss - 1),
        radius=radius * ss,
        fill=255,
    )
    tile = crop.convert("RGBA")
    tile.putalpha(mask.resize((w, h), Image.LANCZOS))
    tile = tile.resize((BODY, BODY), Image.LANCZOS)

    out = Image.new("RGBA", (CANVAS, CANVAS), (0, 0, 0, 0))
    offset = (CANVAS - BODY) // 2
    out.alpha_composite(tile, (offset, offset))
    OUT.parent.mkdir(parents=True, exist_ok=True)
    out.save(OUT)
    print(f"wrote {OUT.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

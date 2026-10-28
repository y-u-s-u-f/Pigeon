#!/usr/bin/env python3
"""Draw Pigeon's app icon and menu bar mark (MA-056, MA-057) and pack them.

One bird, drawn from signed distances: rendered at 1024 px and scaled down,
and drawn again at its own size for the smallest (up to 64 px): the app icon (a white bird on
a coral squircle), the menu bar's template image (the bird alone, black on
clear) and the picture on the sign-in screen. Standard library only: the PNG
is written by hand, `sips` and `iconutil` (both part of macOS) make the
sizes and the .icns.

    packaging/make_icon.py <out-dir> [preview-dir]

Writes Pigeon.icns, AppIcon.png (256 px, for the sign-in screen),
MenuBarIcon.png and MenuBarIcon@2x.png (18 pt) into <out-dir>; with a
preview dir, also icon-16/32/128/1024.png there to look at.
"""

import math
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import zlib

BASE = 1024
# Sizes up to this are drawn from the shapes at their own size: a 1024 px
# render scaled this far down blurs.
SMALL = 64
# The macOS icon grid: an 824/1024 rounded square, centred.
TILE = 824 / 1024
CORNER = 0.225
# Top to bottom: a light coral to a deeper one, away from Mail's blue.
TOP = (255, 138, 101)
BOTTOM = (233, 78, 74)
WHITE = (255, 255, 255)


# Signed distances, in the bird's unit square (y down, facing right).

def circle(x, y, cx, cy, r):
    return math.hypot(x - cx, y - cy) - r


def polygon(x, y, points):
    d = float("inf")
    inside = False
    n = len(points)
    for i in range(n):
        x0, y0 = points[i]
        x1, y1 = points[(i + 1) % n]
        vx, vy = x1 - x0, y1 - y0
        t = max(0.0, min(1.0, ((x - x0) * vx + (y - y0) * vy) / (vx * vx + vy * vy)))
        d = min(d, math.hypot(x - x0 - t * vx, y - y0 - t * vy))
        if (y0 > y) != (y1 > y) and x < x0 + (y - y0) * vx / vy:
            inside = not inside
    return -d if inside else d


def smooth_union(a, b, k):
    h = max(0.0, min(1.0, 0.5 + 0.5 * (b - a) / k))
    return b + (a - b) * h - k * h * (1 - h)


def arc(x, y, cx, cy, r, start, end, width):
    """A stroke along a circle from `start` to `end` (radians, y down)."""
    angle = math.atan2(y - cy, x - cx)
    if start <= angle <= end:
        return abs(math.hypot(x - cx, y - cy) - r) - width / 2
    ends = [(cx + r * math.cos(a), cy + r * math.sin(a)) for a in (start, end)]
    return min(math.hypot(x - ex, y - ey) for ex, ey in ends) - width / 2


def bird(x, y, detail):
    body = circle(x, y, 0.47, 0.58, 0.24)
    head = circle(x, y, 0.69, 0.31, 0.115)
    tail = polygon(x, y, [(0.30, 0.52), (0.04, 0.72), (0.08, 0.78), (0.38, 0.77)]) - 0.012
    beak = polygon(x, y, [(0.775, 0.265), (0.90, 0.325), (0.78, 0.365)])
    d = smooth_union(body, head, 0.16)
    d = smooth_union(d, tail, 0.06)
    d = smooth_union(d, beak, 0.02)
    if not detail:
        return d
    # Carved out: the wing's edge and the eye, left out where they'd blur.
    wing = arc(x, y, 0.30, 0.27, 0.37, math.radians(30), math.radians(92), 0.04)
    eye = circle(x, y, 0.715, 0.29, 0.025)
    return max(d, -wing, -eye)


def squircle(x, y):
    """The tile, in icon units (0..1)."""
    half = TILE / 2
    r = half * 2 * CORNER
    qx = abs(x - 0.5) - (half - r)
    qy = abs(y - 0.5) - (half - r)
    outside = math.hypot(max(qx, 0.0), max(qy, 0.0))
    return outside + min(max(qx, qy), 0.0) - r


def coverage(d, px):
    """How much of a pixel a shape covers, from its distance in units."""
    return max(0.0, min(1.0, 0.5 - d * px))


# The smallest bird, in px, that keeps its wing and eye.
DETAIL = 24
# The bird's square inside the icon.
BIRD_ORIGIN = 0.185
BIRD_SIZE = 0.63


def icon_pixel(x, y, size):
    u, v = (x + 0.5) / size, (y + 0.5) / size
    tile = coverage(squircle(u, v), size)
    if tile == 0:
        return (0, 0, 0, 0)
    t = (v - 0.5 + TILE / 2) / TILE
    bg = [a + (b - a) * t for a, b in zip(TOP, BOTTOM)]
    px = size * BIRD_SIZE
    b = coverage(bird((u - BIRD_ORIGIN) / BIRD_SIZE, (v - BIRD_ORIGIN) / BIRD_SIZE, px >= DETAIL), px)
    rgb = [int(round(c + (w - c) * b)) for c, w in zip(bg, WHITE)]
    return (*rgb, int(round(255 * tile)))


def mark_pixel(x, y, size):
    """The bird alone, black on clear, a little inset (template image)."""
    pad = 0.04
    u = ((x + 0.5) / size - pad) / (1 - 2 * pad)
    v = ((y + 0.5) / size - pad) / (1 - 2 * pad)
    px = size * (1 - 2 * pad)
    a = coverage(bird(u, v, px >= DETAIL), px)
    return (0, 0, 0, int(round(255 * a)))


def write_png(path, size, pixel):
    rows = bytearray()
    for y in range(size):
        rows.append(0)
        for x in range(size):
            rows.extend(pixel(x, y, size))

    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(bytes(rows), 9))
    png += chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(png)


def scale(src, px, out):
    subprocess.run(["sips", "-z", str(px), str(px), src, "--out", out], check=True, stdout=subprocess.DEVNULL)


def main():
    out = sys.argv[1]
    preview = sys.argv[2] if len(sys.argv) > 2 else None
    os.makedirs(out, exist_ok=True)
    work = tempfile.mkdtemp()
    try:
        base = os.path.join(work, "icon.png")
        write_png(base, BASE, icon_pixel)
        iconset = os.path.join(work, "Pigeon.iconset")
        os.mkdir(iconset)
        for size in (16, 32, 128, 256, 512):
            for factor in (1, 2):
                px = size * factor
                name = f"icon_{size}x{size}{'@2x' if factor == 2 else ''}.png"
                if px == BASE:
                    shutil.copy(base, os.path.join(iconset, name))
                elif px <= SMALL:
                    write_png(os.path.join(iconset, name), px, icon_pixel)
                else:
                    scale(base, px, os.path.join(iconset, name))
        subprocess.run(["iconutil", "-c", "icns", iconset, "-o", os.path.join(out, "Pigeon.icns")], check=True)
        scale(base, 256, os.path.join(out, "AppIcon.png"))

        write_png(os.path.join(out, "MenuBarIcon.png"), 18, mark_pixel)
        write_png(os.path.join(out, "MenuBarIcon@2x.png"), 36, mark_pixel)

        if preview:
            os.makedirs(preview, exist_ok=True)
            write_png(os.path.join(preview, "icon-16.png"), 16, icon_pixel)
            write_png(os.path.join(preview, "icon-32.png"), 32, icon_pixel)
            scale(base, 128, os.path.join(preview, "icon-128.png"))
            shutil.copy(base, os.path.join(preview, "icon-1024.png"))
            shutil.copy(os.path.join(out, "MenuBarIcon@2x.png"), os.path.join(preview, "menubar-36.png"))
    finally:
        shutil.rmtree(work)


if __name__ == "__main__":
    main()

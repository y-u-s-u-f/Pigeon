#!/usr/bin/env python3
"""Draw Pigeon's placeholder app icon (PER-4) and pack it as an .icns.

Standard library only: the PNG is written by hand, then `sips` and
`iconutil` (both part of macOS) make the sizes and the .icns.

    packaging/make_icon.py <out.icns>
"""

import os
import shutil
import struct
import subprocess
import sys
import tempfile
import zlib

SIZE = 512
SS = 2  # supersamples per axis


def inside_squircle(x, y):
    # The macOS icon grid: an 824/1024 rounded square, centred.
    half = SIZE * 824 / 1024 / 2
    r = half * 0.45
    cx = cy = SIZE / 2
    dx = max(abs(x - cx) - (half - r), 0)
    dy = max(abs(y - cy) - (half - r), 0)
    return abs(x - cx) <= half and abs(y - cy) <= half and dx * dx + dy * dy <= r * r


def on_envelope(x, y):
    """White envelope: an outlined rectangle with a V flap."""
    left, right = SIZE * 0.30, SIZE * 0.70
    top, bottom = SIZE * 0.36, SIZE * 0.64
    stroke = SIZE * 0.028
    if not (left - stroke <= x <= right + stroke and top - stroke <= y <= bottom + stroke):
        return False
    border = (
        abs(x - left) <= stroke
        or abs(x - right) <= stroke
        or abs(y - top) <= stroke
        or abs(y - bottom) <= stroke
    )
    # The flap: from both top corners down to the middle.
    mid_x, mid_y = SIZE / 2, SIZE * 0.52
    def near_line(x0, y0, x1, y1):
        vx, vy = x1 - x0, y1 - y0
        t = max(0.0, min(1.0, ((x - x0) * vx + (y - y0) * vy) / (vx * vx + vy * vy)))
        px, py = x0 + t * vx, y0 + t * vy
        return (x - px) ** 2 + (y - py) ** 2 <= stroke * stroke
    return border or near_line(left, top, mid_x, mid_y) or near_line(right, top, mid_x, mid_y)


def pixel(x, y):
    """RGBA for the pixel at (x, y), supersampled."""
    r = g = b = a = 0.0
    for sy in range(SS):
        for sx in range(SS):
            fx, fy = x + (sx + 0.5) / SS, y + (sy + 0.5) / SS
            if not inside_squircle(fx, fy):
                continue
            if on_envelope(fx, fy):
                cr, cg, cb = 255, 255, 255
            else:
                # Top to bottom: a lighter blue to a deeper one.
                t = fy / SIZE
                cr = 76 + (29 - 76) * t
                cg = 141 + (78 - 141) * t
                cb = 255 + (216 - 255) * t
            r, g, b, a = r + cr, g + cg, b + cb, a + 255
    n = SS * SS
    if a == 0:
        return (0, 0, 0, 0)
    covered = a / 255
    return (int(r / covered), int(g / covered), int(b / covered), int(a / n))


def write_png(path):
    rows = bytearray()
    for y in range(SIZE):
        rows.append(0)
        for x in range(SIZE):
            rows.extend(pixel(x, y))

    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", SIZE, SIZE, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(bytes(rows), 9))
    png += chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(png)


def main():
    out = sys.argv[1]
    work = tempfile.mkdtemp()
    try:
        base = os.path.join(work, "icon.png")
        write_png(base)
        iconset = os.path.join(work, "Pigeon.iconset")
        os.mkdir(iconset)
        for size in (16, 32, 128, 256, 512):
            for scale in (1, 2):
                px = size * scale
                name = f"icon_{size}x{size}{'@2x' if scale == 2 else ''}.png"
                subprocess.run(
                    ["sips", "-z", str(px), str(px), base, "--out", os.path.join(iconset, name)],
                    check=True,
                    stdout=subprocess.DEVNULL,
                )
        subprocess.run(["iconutil", "-c", "icns", iconset, "-o", out], check=True)
    finally:
        shutil.rmtree(work)


if __name__ == "__main__":
    main()

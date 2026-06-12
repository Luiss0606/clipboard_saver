#!/usr/bin/env python3
"""Generates src-tauri/icons/icon.png (1024px) and icon.icns.

Pure stdlib (zlib/struct) so it runs on any macOS python3. The artwork is a
blue rounded square (macOS Big Sur grid: 824px content inset 100px) with the
same clipboard glyph the tray uses.
"""
import struct
import subprocess
import tempfile
import zlib
from pathlib import Path

S = 1024
INSET = 100
RADIUS = 185  # ~22.4% of the 824 content box, Apple-ish squircle feel
GLYPH = [
    "......######......",
    "......#....#......",
    "..#####....#####..",
    "..#....####....#..",
    "..#............#..",
    "..#..########..#..",
    "..#............#..",
    "..#..########..#..",
    "..#............#..",
    "..#..########..#..",
    "..#............#..",
    "..#..######....#..",
    "..#............#..",
    "..#............#..",
    "..#............#..",
    "..##############..",
    "..................",
    "..................",
]
GLYPH_BOX = 600
TOP = (64, 156, 255)
BOTTOM = (0, 100, 220)

x0 = y0 = INSET
x1 = y1 = S - INSET - 1
gx0 = gy0 = (S - GLYPH_BOX) / 2
cell = GLYPH_BOX / 18


def pixel(x, y):
    if x < x0 or x > x1 or y < y0 or y > y1:
        return (0, 0, 0, 0)
    dx = max(x0 + RADIUS - x, x - (x1 - RADIUS), 0)
    dy = max(y0 + RADIUS - y, y - (y1 - RADIUS), 0)
    if dx * dx + dy * dy > RADIUS * RADIUS:
        return (0, 0, 0, 0)
    t = (y - y0) / (y1 - y0)
    col = tuple(int(a * (1 - t) + b * t) for a, b in zip(TOP, BOTTOM)) + (255,)
    gx = int((x - gx0) // cell)
    gy = int((y - gy0) // cell)
    if 0 <= gx < 18 and 0 <= gy < 18 and GLYPH[gy][gx] == "#":
        return (255, 255, 255, 255)
    return col


def main():
    out_dir = Path(__file__).resolve().parent.parent / "src-tauri" / "icons"
    out_dir.mkdir(parents=True, exist_ok=True)
    png_path = out_dir / "icon.png"

    raw = bytearray()
    for y in range(S):
        raw.append(0)  # PNG filter: none
        for x in range(S):
            raw.extend(pixel(x, y))

    def chunk(tag, data):
        body = tag + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    png = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", S, S, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
        + chunk(b"IEND", b"")
    )
    png_path.write_bytes(png)
    print(f"wrote {png_path}")

    with tempfile.TemporaryDirectory() as tmp:
        iconset = Path(tmp) / "icon.iconset"
        iconset.mkdir()
        for size in (16, 32, 64, 128, 256, 512):
            for scale in (1, 2):
                px = size * scale
                suffix = "" if scale == 1 else "@2x"
                name = iconset / f"icon_{size}x{size}{suffix}.png"
                subprocess.run(
                    ["sips", "-z", str(px), str(px), str(png_path), "--out", str(name)],
                    check=True,
                    capture_output=True,
                )
        subprocess.run(
            ["iconutil", "-c", "icns", str(iconset), "-o", str(out_dir / "icon.icns")],
            check=True,
        )
    print(f"wrote {out_dir / 'icon.icns'}")


if __name__ == "__main__":
    main()

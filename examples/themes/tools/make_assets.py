"""Generate the original Night City theme art.

The script uses only the Python standard library (``zlib`` and ``struct``)
and writes two 8-bit RGBA PNG files: ``icon.png`` (128x128) and
``grid.png`` (32x32, tiled as a background image).

Usage:
    python examples/themes/tools/make_assets.py <output-dir>
"""

import struct
import sys
import zlib
from pathlib import Path

BACKGROUND = (11, 14, 26, 255)
DEEP = (20, 24, 44, 255)
NEON = (252, 238, 10, 255)
CYAN = (0, 214, 255, 255)
MAGENTA = (255, 46, 136, 255)


def write_png(path, width, height, pixels):
    """Write RGBA pixels as a PNG with one filter byte per row."""
    raw = bytearray()
    for row in range(height):
        raw.append(0)
        for column in range(width):
            raw.extend(pixels[row * width + column])

    def chunk(tag, payload):
        body = tag + payload
        return struct.pack(">I", len(payload)) + body + struct.pack(
            ">I", zlib.crc32(body) & 0xFFFFFFFF
        )

    header = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    path.write_bytes(
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
        + chunk(b"IEND", b"")
    )


def fill(width, height, colour):
    return [colour] * (width * height)


def rectangle(pixels, width, x0, y0, x1, y1, colour):
    for y in range(max(y0, 0), min(y1, height_of(pixels, width))):
        for x in range(max(x0, 0), min(x1, width)):
            pixels[y * width + x] = colour


def height_of(pixels, width):
    return len(pixels) // width


def draw_icon():
    size = 128
    pixels = fill(size, size, BACKGROUND)
    towers = [(8, 78, 26, 128), (30, 58, 50, 128), (54, 90, 74, 128),
              (78, 44, 100, 128), (104, 70, 122, 128)]
    for index, tower in enumerate(towers):
        rectangle(pixels, size, *tower, DEEP if index % 2 else BACKGROUND)
    for y in range(24, 128, 12):
        rectangle(pixels, size, 6, y, 122, y + 1, CYAN)
    for step in range(90):
        x = 10 + step
        y = 118 - step
        rectangle(pixels, size, x, y, x + 3, y + 3, NEON)
    for y in range(size):
        for x in range(size):
            if (x - 96) ** 2 + (y - 30) ** 2 <= 144:
                pixels[y * size + x] = MAGENTA
    return pixels, size, size


def draw_grid():
    size = 32
    pixels = fill(size, size, BACKGROUND)
    for offset in range(0, size, 8):
        rectangle(pixels, size, offset, 0, offset + 1, size, (0, 92, 120, 255))
        rectangle(pixels, size, 0, offset, size, offset + 1, (0, 92, 120, 255))
    for y in range(0, size, 8):
        for x in range(0, size, 8):
            pixels[y * size + x] = CYAN
    return pixels, size, size


def main():
    if len(sys.argv) != 2:
        raise SystemExit("usage: make_assets.py <output-dir>")
    assets = Path(sys.argv[1])
    assets.mkdir(parents=True, exist_ok=True)
    for name, draw in (("icon.png", draw_icon), ("grid.png", draw_grid)):
        pixels, width, height = draw()
        write_png(assets / name, width, height, pixels)
        print(f"wrote {assets / name} ({width}x{height})")


if __name__ == "__main__":
    main()

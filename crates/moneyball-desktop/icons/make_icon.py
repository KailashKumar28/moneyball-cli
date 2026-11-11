"""Draws the Moneyball app icon and writes the set Tauri bundles.

The mark is original: an ink tile (the report's ink on its paper), three
rising paper bars, and one gold ball above the tallest. Standard library
only; sips and iconutil (both ship with macOS) do the resizing.

    python3 icons/make_icon.py        # from crates/moneyball-desktop
"""

import pathlib
import struct
import subprocess
import zlib

SIZE = 1024
SS = 3  # supersampling per axis
INK = (0x17, 0x14, 0x0F)
PAPER = (0xF4, 0xF1, 0xE8)
GOLD = (0xD3, 0xA5, 0x63)
HERE = pathlib.Path(__file__).parent


def in_round_rect(x, y, x0, y0, x1, y1, r):
    if not (x0 <= x <= x1 and y0 <= y <= y1):
        return False
    cx = min(max(x, x0 + r), x1 - r)
    cy = min(max(y, y0 + r), y1 - r)
    return (x - cx) ** 2 + (y - cy) ** 2 <= r * r


def colour_at(x, y):
    """The colour at a point, or None outside the tile."""
    if not in_round_rect(x, y, 100, 100, 924, 924, 185):
        return None
    if (x - 668) ** 2 + (y - 300) ** 2 <= 62 ** 2:
        return GOLD
    for x0, top in ((268, 610), (444, 500), (620, 410)):
        if in_round_rect(x, y, x0, top, x0 + 136, 756, 22):
            return PAPER
    return INK


def render():
    rows = []
    step = 1.0 / SS
    for py in range(SIZE):
        row = bytearray([0])
        for px in range(SIZE):
            r = g = b = hit = 0
            for sy in range(SS):
                for sx in range(SS):
                    c = colour_at(px + (sx + 0.5) * step, py + (sy + 0.5) * step)
                    if c:
                        r, g, b, hit = r + c[0], g + c[1], b + c[2], hit + 1
            if hit:
                row += bytes((r // hit, g // hit, b // hit, 255 * hit // (SS * SS)))
            else:
                row += b"\0\0\0\0"
        rows.append(bytes(row))
    return b"".join(rows)


def png(raw):
    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    head = struct.pack(">IIBBBBB", SIZE, SIZE, 8, 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", head) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")


def resized(src, dst, px):
    subprocess.run(["sips", "-z", str(px), str(px), str(src), "--out", str(dst)], check=True, capture_output=True)


def main():
    master = HERE / "icon-1024.png"
    master.write_bytes(png(render()))
    for name, px in (("32x32.png", 32), ("128x128.png", 128), ("128x128@2x.png", 256), ("icon.png", 512)):
        resized(master, HERE / name, px)
    iconset = HERE / "icon.iconset"
    iconset.mkdir(exist_ok=True)
    for px in (16, 32, 128, 256, 512):
        resized(master, iconset / f"icon_{px}x{px}.png", px)
        resized(master, iconset / f"icon_{px}x{px}@2x.png", px * 2)
    subprocess.run(["iconutil", "-c", "icns", str(iconset), "-o", str(HERE / "icon.icns")], check=True)
    for f in iconset.iterdir():
        f.unlink()
    iconset.rmdir()
    master.unlink()


if __name__ == "__main__":
    main()

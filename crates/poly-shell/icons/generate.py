#!/usr/bin/env python3
"""Draws the launcher icons of Poly's own apps (ADR 0146).

Original artwork, made for Poly: a rounded tile with a vertical gradient and
one white glyph, the same shape and weight for every app. Everything is
analytic (distance functions, 4x4 supersampling) and written with the
standard library only. Run `python generate.py` here to redraw the PNGs.
"""
import math
import struct
import zlib

SIZE = 128
SS = 4  # samples per pixel side


# ---- shapes: each returns whether the point (in a -1..1 box) is inside ----

def rrect(px, py, cx, cy, hw, hh, r):
    dx, dy = abs(px - cx) - (hw - r), abs(py - cy) - (hh - r)
    return math.hypot(max(dx, 0.0), max(dy, 0.0)) + min(max(dx, dy), 0.0) <= r


def disc(px, py, cx, cy, r):
    return (px - cx) ** 2 + (py - cy) ** 2 <= r * r


def stroke(px, py, ax, ay, bx, by, half):
    """A line segment with round ends, `half` wide on each side."""
    vx, vy = bx - ax, by - ay
    t = max(0.0, min(1.0, ((px - ax) * vx + (py - ay) * vy) / (vx * vx + vy * vy)))
    return math.hypot(px - (ax + t * vx), py - (ay + t * vy)) <= half


# ---- glyphs, white on the tile ----

def terminal(x, y):
    # A prompt ">" and a cursor bar.
    return (
        stroke(x, y, -0.42, -0.30, -0.08, 0.0, 0.085)
        or stroke(x, y, -0.42, 0.30, -0.08, 0.0, 0.085)
        or rrect(x, y, 0.22, 0.30, 0.22, 0.075, 0.075)
    )


def files(x, y):
    # A folder: the body and its tab.
    return rrect(x, y, 0.0, 0.10, 0.52, 0.36, 0.09) or rrect(x, y, -0.27, -0.26, 0.25, 0.12, 0.07)


def settings(x, y):
    # A gear: eight teeth around a ring with a round hole.
    r = math.hypot(x, y)
    a = math.atan2(y, x)
    step = math.pi / 4
    # Teeth are centred on the multiples of 45 degrees.
    off = min(a % step, step - a % step)
    tooth = off < 0.17
    body = r <= 0.46 or (r <= 0.62 and tooth)
    return body and r >= 0.20


def notifications(x, y):
    # A bell: a dome, flaring sides, a rim, and the clapper.
    dome = disc(x, y, 0.0, -0.08, 0.34) and y < -0.08
    sides = -0.10 <= y <= 0.30 and abs(x) <= 0.34 + (y + 0.10) * 0.22
    rim = rrect(x, y, 0.0, 0.32, 0.50, 0.075, 0.075)
    top = disc(x, y, 0.0, -0.46, 0.07)
    clapper = disc(x, y, 0.0, 0.53, 0.115) and y > 0.40
    return dome or sides or rim or top or clapper


ICONS = {
    # name: (glyph, top colour, bottom colour)
    "poly-terminal": (terminal, (0x3A, 0x41, 0x4E), (0x14, 0x17, 0x1D)),
    "poly-files": (files, (0x4F, 0xA3, 0xF7), (0x2A, 0x62, 0xD9)),
    "poly-settings": (settings, (0x8A, 0x94, 0xA6), (0x55, 0x5E, 0x70)),
    "poly-notifications": (notifications, (0xFF, 0x9F, 0x4A), (0xF0, 0x5A, 0x38)),
}


def render(glyph, top, bottom):
    rows = []
    for j in range(SIZE):
        row = bytearray()
        for i in range(SIZE):
            tile = ink = 0
            for sj in range(SS):
                for si in range(SS):
                    x = ((i + (si + 0.5) / SS) / SIZE) * 2 - 1
                    y = ((j + (sj + 0.5) / SS) / SIZE) * 2 - 1
                    if rrect(x, y, 0.0, 0.0, 0.94, 0.94, 0.42):
                        tile += 1
                        if glyph(x, y):
                            ink += 1
            n = SS * SS
            if tile == 0:
                row += bytes((0, 0, 0, 0))
                continue
            t = (j + 0.5) / SIZE
            base = [top[k] + (bottom[k] - top[k]) * t for k in range(3)]
            mix = ink / tile
            rgb = [round(base[k] * (1 - mix) + 255 * mix) for k in range(3)]
            row += bytes(rgb + [round(255 * tile / n)])
        rows.append(bytes(row))
    return rows


def png(rows):
    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    raw = b"".join(b"\x00" + r for r in rows)
    ihdr = struct.pack(">IIBBBBB", SIZE, SIZE, 8, 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")


if __name__ == "__main__":
    for name, (glyph, top, bottom) in ICONS.items():
        with open(f"{name}.png", "wb") as f:
            f.write(png(render(glyph, top, bottom)))
        print("wrote", name)

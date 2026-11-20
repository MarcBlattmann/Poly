#!/usr/bin/env python3
"""Writes the PolyTest TrueType fonts used by the CoreText tests (tests/macos/include/poly_mct.h).
Written for Poly from the public TrueType/OpenType table formats only (Microsoft/Apple font specs).
Usage: mkfont.py regular|bold  -> font bytes on stdout."""
import struct, sys

UPM = 1000

def rect(x0, y0, x1, y1):
    # one clockwise contour (TrueType outer contours are clockwise): points on-curve
    return [[(x0, y0), (x0, y1), (x1, y1), (x1, y0)]]

def glyph_simple(contours):
    pts = [p for c in contours for p in c]
    xs = [p[0] for p in pts]; ys = [p[1] for p in pts]
    out = struct.pack(">hhhhh", len(contours), min(xs), min(ys), max(xs), max(ys))
    end = -1; ends = []
    for c in contours:
        end += len(c); ends.append(end)
    out += b"".join(struct.pack(">H", e) for e in ends)
    out += struct.pack(">H", 0)  # no instructions
    flags = bytes([0x01] * len(pts))  # on curve, 16-bit x and y (no short vectors, no repeat)
    out += flags
    px = py = 0
    for (x, y) in pts:
        out += struct.pack(">h", x - px); px = x
    for (x, y) in pts:
        out += struct.pack(">h", y - py); py = y
    return out

def glyph_composite(parts, bbox):
    out = struct.pack(">hhhhh", -1, *bbox)
    for i, (gid, dx, dy) in enumerate(parts):
        more = 0x0020 if i < len(parts) - 1 else 0
        flags = 0x0001 | 0x0002 | more  # ARG_1_AND_2_ARE_WORDS | ARGS_ARE_XY_VALUES
        out += struct.pack(">HHhh", flags, gid, dx, dy)
    return out

def build(bold):
    w = 60 if bold else 0   # the bold variant is wider
    # glyph order: 0 .notdef, 1 space, 2 A, 3 B, 4 V, 5 W (composite), 6 a, 7 g, 8 eacute (composite), 9 acute-accent mark U+0301 (zero advance), 10 U+1F600 box, 11 O (ring: outer + inner contour)
    glyphs = []   # (glyph bytes or b"", advance, lsb)
    def add(data, adv, lsb): glyphs.append((data, adv, lsb))
    add(glyph_simple(rect(50, 0, 450, 700)), 500, 50)                          # .notdef: tall box
    add(b"", 300, 0)                                                            # space
    add(glyph_simple(rect(100, 0, 500 + w, 700)), 600 + w, 100)                 # A: solid box 400 wide, 700 high (cap height)
    add(glyph_simple(rect(100, 0, 400 + w, 500)), 500 + w, 100)                 # B: box to x-height
    tri = [[(0, 0), (350, 700), (700, 0)]]                                      # V: an upright triangle (outline is a diagonal pair)
    add(glyph_simple(tri), 700, 0)                                              # V
    add(glyph_composite([(4, 0, 0), (4, 700, 0)], (0, 0, 1400, 700)), 1400, 0)  # W: two triangles side by side
    add(glyph_simple(rect(50, 0, 450, 500)), 500, 50)                           # a: box to x-height
    add(glyph_simple(rect(50, -200, 450, 400)), 500, 50)                        # g: reaches the descender
    add(glyph_composite([(6, 0, 0), (9, 150, 520)], (50, 0, 450, 620)), 500, 50) # eacute = a + accent mark
    add(glyph_simple(rect(0, 0, 100, 100)), 0, 0)                               # U+0301 accent: small box, zero advance
    add(glyph_simple(rect(0, 0, 800, 800)), 900, 0)                             # U+1F600 box
    ring = [[(0, 0), (0, 700), (700, 700), (700, 0)], [(200, 200), (500, 200), (500, 500), (200, 500)]]  # outer clockwise, inner counter-clockwise
    add(glyph_simple(ring), 800, 0)                                             # O: a ring with a hole
    n = len(glyphs)
    cmap_map = {0x20: 1, 0x41: 2, 0x42: 3, 0x56: 4, 0x57: 5, 0x61: 6, 0x67: 7, 0xE9: 8, 0x301: 9, 0x4F: 11}
    astral = {0x1F600: 10}
    # glyf, loca
    glyf = b""; loca = []
    for data, adv, lsb in glyphs:
        loca.append(len(glyf)); glyf += data
        while len(glyf) % 4: glyf += b"\0"
    loca.append(len(glyf))
    loca_b = b"".join(struct.pack(">I", o) for o in loca)
    # hmtx: all glyphs have a long metric
    hmtx = b"".join(struct.pack(">Hh", adv, lsb) for _, adv, lsb in glyphs)
    # head
    xmin, ymin, xmax, ymax = -0, -200, 1400, 800
    head = struct.pack(">IIIIHHqqhhhhHHhhh", 0x00010000, 0x00010000, 0, 0x5F0F3CF5, 0x000B, UPM, 0, 0,
                       xmin, ymin, xmax, ymax, 0x0001 if bold else 0x0000, 8, 2, 1, 0)
    # hhea: ascent 800 descent -200 linegap 100
    hhea = struct.pack(">IhhhHhhhhhhhhhhhH", 0x00010000, 800, -200, 100, 1400, 0, 0, 1400, 1, 0, 0, 0, 0, 0, 0, 0, n)
    # maxp 1.0
    maxp = struct.pack(">IHHHHHHHHHHHHHH", 0x00010000, n, 12, 2, 0, 0, 2, 0, 0, 0, 0, 0, 0, 2, 2)
    # OS/2 version 4: typo 700/-300/50, win 900/250, capHeight 700, xHeight 500
    weight = 700 if bold else 400
    os2 = struct.pack(">H", 4) + struct.pack(">h", 500) + struct.pack(">HH", weight, 5) + struct.pack(">H", 0)  # version, xAvgCharWidth, weight class, width class, fsType
    os2 += struct.pack(">hhhhhhhhhh", 600, 600, 0, 150, 600, 600, 0, 350, 50, 300)  # subscript/superscript sizes+offsets, strikeout size+pos (10 values)
    os2 += struct.pack(">h", 0)  # sFamilyClass
    os2 += bytes(10)  # panose
    os2 += struct.pack(">IIII", 1, 0, 0, 0)  # ulUnicodeRange1..4
    os2 += b"PLYT"
    os2 += struct.pack(">HHH", 0x20 if bold else 0x40, 0x0020, 0xFFFF)  # fsSelection, first, last char index
    os2 += struct.pack(">hhh", 700, -300, 50)  # typo ascender, descender, linegap
    os2 += struct.pack(">HH", 900, 250)  # win ascent, win descent
    os2 += struct.pack(">II", 1, 0)  # code page ranges
    os2 += struct.pack(">hhHHH", 500, 700, 0, 0x20, 0)  # xHeight, capHeight, defaultChar, breakChar, maxContext
    # post format 3 (no glyph names)
    post = struct.pack(">IIhhIIIII", 0x00030000, 0, -100, 50, 0, 0, 0, 0, 0)
    # name table: platform 3 (Windows) encoding 1 and platform 1 (Mac)
    fam = "PolyTest"; sub = "Bold" if bold else "Regular"
    full = fam + " " + sub; ps = fam + "-" + sub
    names = {0: "Copyright: public domain test font made for Poly", 1: fam, 2: sub, 3: "PolyTest 1.0 " + sub, 4: full, 5: "Version 1.000", 6: ps, 16: fam, 17: sub}
    recs = []; strings = b""
    for nid in sorted(names):
        s = names[nid]
        u = s.encode("utf-16-be"); recs.append((3, 1, 0x409, nid, len(u), len(strings))); strings += u
    for nid in sorted(names):
        s = names[nid].encode("ascii"); recs.append((1, 0, 0, nid, len(s), len(strings))); strings += s
    recs.sort(key=lambda r: (r[0], r[1], r[2], r[3]))
    name = struct.pack(">HHH", 0, len(recs), 6 + 12 * len(recs))
    for r in recs: name += struct.pack(">HHHHHH", *r)
    name += strings
    # cmap: format 4 (platform 3 enc 1) and format 12 (platform 3 enc 10)
    def fmt4(m):
        keys = sorted(m)
        segs = [(k, k, m[k] - k) for k in keys]  # one segment per code, idDelta
        segs.append((0xFFFF, 0xFFFF, 1))
        segc = len(segs)
        sr = 2 * (1 << (segc.bit_length() - 1)); es = (segc.bit_length() - 1); rs = 2 * segc - sr
        body = struct.pack(">HHHH", segc * 2, sr, es, rs)
        body += b"".join(struct.pack(">H", s[1]) for s in segs) + b"\0\0"
        body += b"".join(struct.pack(">H", s[0]) for s in segs)
        body += b"".join(struct.pack(">h", ((s[2] + 0x8000) & 0xFFFF) - 0x8000) for s in segs)
        body += b"".join(struct.pack(">H", 0) for s in segs)
        length = 6 + len(body)
        return struct.pack(">HHH", 4, length, 0) + body
    def fmt12(m):
        groups = sorted(m.items())
        body = struct.pack(">HHIII", 12, 0, 16 + 12 * len(groups), 0, len(groups))
        for c, g in groups: body += struct.pack(">III", c, c, g)
        return body
    f4 = fmt4(cmap_map); f12 = fmt12({**cmap_map, **astral})
    cmap = struct.pack(">HH", 0, 2)
    off = 4 + 8 * 2
    cmap += struct.pack(">HHI", 3, 1, off) + struct.pack(">HHI", 3, 10, off + len(f4)) + f4 + f12
    # kern (format 0): A,V = -100 ; V,A = -80
    pairs = [(2, 4, -100), (4, 2, -80), (2, 2, 0)]
    pairs = sorted([p for p in pairs if p[2]])
    sub_len = 14 + 6 * len(pairs)
    kern = struct.pack(">HH", 0, 1) + struct.pack(">HHH", 0, sub_len, 0x0001)
    kern += struct.pack(">HHHH", len(pairs), 0, 0, 0) + b"".join(struct.pack(">HHh", *p) for p in pairs)
    tables = {b"OS/2": os2, b"cmap": cmap, b"glyf": glyf, b"head": head, b"hhea": hhea, b"hmtx": hmtx,
              b"kern": kern, b"loca": loca_b, b"maxp": maxp, b"name": name, b"post": post}
    tags = sorted(tables)
    nt = len(tags); sr = 16 * (1 << (nt.bit_length() - 1)); es = nt.bit_length() - 1
    out = struct.pack(">IHHHH", 0x00010000, nt, sr, es, nt * 16 - sr)
    off = 12 + 16 * nt; body = b""; dirs = b""
    def csum(b):
        b = b + b"\0" * (-len(b) % 4)
        return sum(struct.unpack(">%dI" % (len(b) // 4), b)) & 0xFFFFFFFF
    headpos = None
    for t in tags:
        data = tables[t]
        if t == b"head": headpos = off + len(body)
        dirs += struct.pack(">4sIII", t, csum(data), off + len(body), len(data))
        body += data + b"\0" * (-len(data) % 4)
    font = bytearray(out + dirs + body)
    adj = (0xB1B0AFBA - csum(bytes(font))) & 0xFFFFFFFF
    struct.pack_into(">I", font, headpos + 8, adj)
    return bytes(font)

if __name__ == "__main__":
    sys.stdout.buffer.write(build(sys.argv[1] == "bold"))

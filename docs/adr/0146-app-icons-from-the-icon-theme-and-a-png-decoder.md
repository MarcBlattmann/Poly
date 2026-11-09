# 0146: Installed apps show their own icons: a PNG decoder and the icon theme lookup

**Status:** Accepted. Extends ADR 0127 (the shell's look) and ADR 0134
(desktop entries).

## Context

The launcher drew only Lucide glyphs (ADR 0127), one per kind of app, and
read nothing of a desktop entry's `Icon=` key. Every Linux app installed
from a package (Chromium, GTK and Qt apps, VLC) looked the same as the next
one. The apps ship their icons as PNG files in the icon theme, and Poly had
no PNG reader (`poly-font` only writes PNGs, stored, for its own tests).

## Decision

- **`crates/poly-png`**: a `no_std` + `alloc`, `forbid(unsafe_code)` PNG
  decoder with its own DEFLATE/zlib inflater (nothing in the repo inflated:
  `poly-http` only parses `Transfer-Encoding`). It follows the W3C PNG
  specification (3rd edition) and RFC 1950 and 1951 and nothing else; no
  libpng, zlib, miniz or image-rs source was read (`CLEAN_ROOM.md`).
  - All colour types and bit depths, `PLTE` and `tRNS` (also the single
    transparent colour of grey and RGB), Adam7, the five filters. Sixteen-bit
    samples are scaled to eight; `gAMA`, `iCCP` and the other colour chunks
    are ignored (samples are sRGB). The result is straight `0xAARRGGBB`.
  - Damaged input is an error, never a panic: the signature, the chunk order,
    each critical chunk's CRC-32 and the zlib Adler-32 are checked (a bad
    ancillary chunk is dropped), any truncation is `Truncated`, an unknown
    critical chunk is refused. `Limits::max_pixels` is checked from `IHDR`
    before anything is allocated, and the inflater stops at the exact size
    the header implies, so a decompression bomb cannot grow.
  - Tests: an encoder written in the test module makes files of every colour
    type, depth, filter (fixed and mixed) and interlace method at nine sizes
    (1,620 round trips against independently computed pixels); handcrafted
    files and zlib streams (stored, fixed, dynamic, overlapping matches);
    every prefix and every single-bit flip of a real icon; header, chunk
    order, palette and `tRNS` rules; and Debian's Chromium icons, whose
    pixels match a reference decoding made with Python's `zlib`.
- **`poly-shell::appicon`** looks `Icon=` up as the freedesktop.org Icon
  Theme Specification (0.13) says: an absolute path is the file; a name is
  `/usr/share/icons/hicolor/<size>/apps/<name>.png` (also under
  `/usr/local/share/icons`), in the size directory nearest the wanted size
  (the smallest at least as big, so scaling down; else the biggest;
  `48x48@2` counts as 96), then `/usr/share/pixmaps/<name>.png`. An extension
  in the name is dropped; names that are paths (`..`, `/`) are refused. SVG
  and XPM are skipped: those apps keep their glyph.
- **Loading and memory:** lazily, when a row is first drawn, through an
  `IconFiles` trait the compositor implements on the disk (`DiskIcons`; the
  shell stays free of I/O and host-testable). Files over 2 MiB and images over
  4 Mpixels are not read. Each icon is scaled once to the size drawn
  (premultiplied alpha, area average down, bilinear up, aspect kept) and the
  decoded original is dropped. The cache holds 256 entries and 4 MiB, oldest
  first out, and remembers misses so the disk is not searched again at every
  frame.
- **Drawing:** the launcher's rows draw the app's icon over the whole 28-pixel
  box, at `28 x scale` pixels, in place of the glyph. Open windows are listed
  with their app's icon: the `Icon` of the desktop file that matches the
  window's app ID (by file name, or by the program it runs), else the app ID as
  an icon name. The window switcher's cards (16 pixels) do the same. Anything
  whose icon is not found keeps its Lucide glyph. There is no dock or taskbar.
- **Poly's own apps** (Settings, Notifications, Files, Terminal, and the
  windows of the last two) have original artwork made for Poly: a rounded
  tile with a vertical gradient and one white glyph, the same shape and weight
  for each, as 128-pixel PNGs in `crates/poly-shell/icons/`
  (`poly-settings`, `poly-notifications`, `poly-files`, `poly-terminal`),
  drawn by `icons/generate.py` (distance functions, standard library only; run
  it to redraw them). They are compiled into the shell (`include_bytes!`) under
  the built-in apps' `Icon` names, looked up before any theme, so they need no
  disk and no package can shadow them. They go through the same decode, scale
  and cache as the others.
- **Chromium:** Poly's launcher entry (`desktop_file()` in
  `tools/xtask/src/chromium.rs`) has `Icon=chromium`, Debian's name for its
  logo, which the package ships as hicolor PNGs from 16 to 256 pixels.

## Consequences

- Chromium, and any installed app with a PNG icon, shows its own icon in the
  launcher and the switcher. Apps with only SVG icons (many GTK apps use
  Adwaita's SVGs) still show the glyph until an SVG renderer exists.
- Only `hicolor` is searched; other themes' directories and `index.theme` are
  not read yet, so an icon only found in, say, Adwaita is not shown.
- `poly-png` is also the first step to PNG wallpapers, file thumbnails and
  notification images; none of that is done here.
- The icon loads on the desktop's thread the first time its row is drawn: a
  few milliseconds for a 48-pixel PNG, once.

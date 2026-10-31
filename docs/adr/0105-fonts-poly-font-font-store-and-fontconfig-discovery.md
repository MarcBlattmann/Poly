# 0105: Fonts: `poly-font` (TrueType/OpenType parsing, a signed-area rasteriser, a glyph cache, simple layout), a font store of pinned DejaVu fonts, and fontconfig-compatible discovery

**Status:** Accepted. Delivers the Phase 6 roadmap item "Fonts: font store,
rasterisation, fontconfig-compatible discovery" (started ahead of the last
Phase 5 work with the owner's approval). Builds on
[ADR 0019](0019-display-framebuffer-and-drawing.md) (the display and
`poly-gfx`'s bitmap fonts), [ADR 0059](0059-bash-coreutils-python-gcc-from-the-persistent-disk.md)
(the persistent root disk), [ADR 0073](0073-windows-file-system-layout.md)
(`C:\Windows\Fonts`) and [ADR 0087](0087-7-zip-command-line-on-poly.md)
(fetching a pinned third-party file instead of committing it).

## Context

The kernel draws text with two Spleen bitmap fonts (ADR 0019). The
desktop of Phase 6 (a compositor, a shell, GTK and Qt apps) and the
Windows GUI of Phase 7 (`gdi32` text, DirectWrite) need outline fonts:
something that reads the font files programs ship and expect, renders them
smoothly at any size, and finds "the bold sans-serif" the way Linux
programs ask for it, through fontconfig.

Sources (clean room; CLEAN_ROOM.md): the OpenType specification
(Microsoft Learn: "OpenType font file", `cmap`, `head`, `hhea`, `hmtx`,
`maxp`, `loca`, `glyf`, `name`, `OS/2`, `post`, `kern`, `GPOS` with its
"Common Table Formats" chapter for coverage and class tables, `CFF `),
Apple's TrueType Reference Manual (the same tables; Apple's `kern` header;
composite glyph transforms), Adobe Technical Notes #5176 (CFF) and #5177
(Type 2 charstrings), the PNG specification for the debug output, the
W3C XML 1.0 specification for `fonts.conf`, and the fontconfig user
documentation (`fontconfig-user.html`: font names, properties, constants,
font matching, the configuration file format). No FreeType, fontconfig,
HarfBuzz, stb_truetype or other rasteriser or font library source was
looked at, and the real fontconfig was not run to compare behaviour.

## Decision

### `crates/poly-font`, host-tested, `no_std` + `alloc`

A new crate, so that the kernel (a boot screen, a panic screen) or a
userland compositor can both use it; it builds for `x86_64-unknown-none`
but nothing in the kernel uses it yet. No dependencies. Nothing panics on
malformed data: every read is bounds-checked (`reader.rs`), recursion and
point counts are capped, sizes are clamped; tests truncate and corrupt the
test font byte by byte.

- **Parsing** (`font.rs`, `glyf.rs`, `cff.rs`, `kern.rs`). `Font::parse`
  borrows the file's bytes and reads the table directory (plain fonts,
  `true`, `OTTO` and `.ttc` collections by index); `head`, `hhea` and
  `maxp` are required, everything else read lazily. `cmap` formats 0, 4, 6
  and 12, preferring a full-repertoire Unicode subtable to a BMP one to
  Mac Roman. Metrics from `hhea` (or `OS/2`'s typo metrics when
  `USE_TYPO_METRICS` is set) and `hmtx`. `name` strings (US English
  Windows first). Style facts from `OS/2` (weight and width class,
  `fsSelection`), `post` (italic angle, fixed pitch) and `head.macStyle`.
  Outlines: `glyf` simple glyphs (flags with repeats, short and same
  coordinates) and composite glyphs (offsets or point matching, scale,
  x/y scale, 2×2; offsets unscaled unless `SCALED_COMPONENT_OFFSET`, the
  OpenType default), decoded to points first and turned into a path last
  (implied on-curve points between off-curve points, contours starting
  off-curve); CFF Type 2 charstrings with local and global subroutines,
  CID-keyed fonts' `FDSelect` (formats 0, 3), every path and hint
  operator (hints skipped, `seac` and CFF2 not supported). Kerning: `GPOS`
  pair adjustment (formats 1 and 2, through extension lookups) of every
  `kern` feature regardless of script, x advance only; else the `kern`
  table's format 0 subtables (Microsoft and Apple headers).
- **Rasteriser** (`raster.rs`): exact area coverage by signed-area
  accumulation. Curves are flattened to lines (to within 0.03 px); each
  line adds, per pixel cell it crosses, its signed height times the share
  of the cell right of it to that cell and the rest to the next; a
  running sum along each row then gives every pixel's signed area, whose
  absolute value clamped to 1 is the non-zero-winding coverage. Edges
  outside the bitmap are clipped without losing their effect on the pixels
  to their right.
- **Glyph rendering** (`render.rs`): `render_glyph(font, glyph, size_px,
  dx, mode)` scales to pixels per em, shifts by a subpixel offset and
  returns a bitmap placed relative to the pen position. `RenderMode::Gray`
  is one coverage byte per pixel; `RenderMode::Lcd` rasterises at three
  times the horizontal resolution and filters with a (1 2 3 2 1)/9
  triangle for RGB-stripe panels. Neither hints: no instructions are run,
  no stems snapped, which keeps shapes and spacing true at the cost of
  some blur at small sizes.
- **Glyph cache** (`cache.rs`): bitmaps keyed by (caller's font number,
  size in 1/64 px, glyph, subpixel step of 4 per pixel, mode), within a
  byte budget, least recently used evicted first, with hit/miss counts.
- **Layout** (`layout.rs`): one font, left to right, one glyph per
  character (`.notdef` for missing ones), advances, pair kerning, hard
  breaks at `\n` and greedy breaks at spaces for a maximum width (a word
  too long stays whole). `render_layout` draws through the cache at
  quarter-pixel positions into an `Image`. Shaping (ligatures, complex
  scripts, bidirectional text) and font fallback are not done.
- **`png.rs`**: a minimal PNG writer (stored deflate) to look at output.

### Discovery: `poly_font::fc`, fontconfig's documented behaviour

- **Patterns** (`fc/pattern.rs`): the documented name syntax
  `<families>-<point sizes>:<name>=<values>...`, comma lists, the escapes,
  the property types of the documentation's property table and its whole
  constant table (`:bold`, `slant=italic`, `spacing=mono`...), and text
  output that parses back. One leniency: a `-` not followed by numbers is
  part of the family name, so `sans-serif` needs no escape.
- **Describing a font** (`describe`): families (typographic name ID 16
  first, then ID 1), styles (17, then 2), full and PostScript names;
  weight mapped from `usWeightClass` linearly between the OpenType classes
  that name fontconfig's weight constants (400 → 80, 700 → 200, ...);
  width from `usWidthClass` (1–9 → the width constants); slant 100 for
  italic, 110 for oblique (`fsSelection` bit 9, or the italic bit with
  "Oblique" in the style name, as DejaVu's obliques have); spacing 100
  when `post.isFixedPitch`; file, index, `scalable`, `outline`,
  `fontformat`.
- **Matching** (`FontSet::font_match`, `sort`, `list`): the configuration's
  rules, then defaults (weight regular, slant roman, width normal, 12 pt at
  75 dpi and the pixel size they give), then a score per font compared
  property by property in the documented priority order (family, spacing,
  pixel size, style, slant, weight; width last, since the documentation
  defines it but does not rank it); ties keep the first font added.
  Family is the position of the first requested family the font has
  (case- and blank-insensitive); style is whether one requested style is
  the font's; the rest are distances; scalable fonts match every pixel
  size. Foundry, charset and language, also ranked by the documentation,
  are not modelled. `list` is `fc-list`: fonts that have one of the
  requested values of every requested property. `match_line` and
  `list_line` print in the familiar `fc-match` / `fc-list` forms.
- **`fonts.conf`** (`fc/config.rs`, `fc/xml.rs`): a small XML reader and
  the elements Poly uses: `<dir>`, `<cachedir>`, `<include>` (recorded;
  the caller does the I/O), `<alias>` with `<prefer>`/`<accept>`/
  `<default>` (prefer before the family, accept after it, default at the
  end of the list), and `<match target="pattern">` in the one form the
  documentation's example uses for a generic fallback (`qual="all"
  compare="not_eq"` family tests and an `append_last` family edit).
  Anything else is counted and ignored. Rules apply in document order.
- **Poly's `fonts.conf`** (`crates/poly-font/fonts.conf`,
  `fc::POLY_FONTS_CONF`), installed as `/etc/fonts/fonts.conf`: the font
  directories (`/usr/share/fonts`, `/usr/local/share/fonts`, the XDG
  one), cache directories, aliases that file common Windows and macOS
  names (Arial, Times New Roman, Courier New, Consolas...) under the
  generic families, then the sans-serif fallback, then the generic
  families' preferred DejaVu fonts, then `conf.d` and the user's file. It
  uses only elements of fontconfig's DTD, so the real fontconfig reads the
  same file when GTK and Qt arrive.

### The font store and the bundled fonts

DejaVu 2.37 (Sans, Serif, Sans Mono; regular, bold, oblique or italic,
bold oblique or italic: twelve files, 5.5 MB): broad coverage (Latin,
Greek, Cyrillic, symbols), the long-time Linux default, and permissively
licensed (Bitstream Vera and Arev licences, DejaVu's changes public
domain; THIRD_PARTY.md). Too large for the repository, so like 7-Zip
(ADR 0087) they are fetched: `cargo xtask fonts fetch` downloads the
release archive over HTTPS into the main checkout's `target/fonts/`,
checks its pinned size and SHA-256, unpacks only the twelve files and the
licence into a fresh directory, checks each file's pinned hash and drops
the archive (`tools/xtask/src/fonts.rs`). The licence text is committed
in `third_party/dejavu/LICENSE`.

On the root disk (`rootdisk.rs`): the fonts in
`/usr/share/fonts/truetype/dejavu/` with the licence beside them (the
`<format>/<family>` layout Linux distributions use), `/etc/fonts/fonts.conf`
always, `/var/cache/fontconfig`, and for Windows programs a relative
symbolic link per font in `C:\Windows\Fonts` (`/poly/nt/c/Windows/Fonts`
on the disk), so both worlds read one copy. Without fetched fonts the disk
has the configuration only, with a note.

`cargo xtask fonts list [pattern] [elements]`, `fonts match [--sort]
<pattern>` and `fonts render [--lcd] [--size] [--width] [--out] <pattern>
<text>` run the matcher and the renderer over the bundled fonts on the
host: the `fc-list` / `fc-match` of this item, as a library and a
developer tool. A program on Poly that wants them links `poly-font`;
real `fc-match` comes with fontconfig itself.

### Tests

- Unit tests on a TrueType font built in code (`src/testfont.rs`: a
  triangle with a hole, a contour of off-curve points only, a composite
  with a scaled component, a glyph past `numberOfHMetrics`, both `cmap`
  formats, a `kern` table, style variants and a collection), synthetic
  `GPOS` and CFF tables, and the rasteriser on shapes with known areas.
- Golden images (`tests/golden/*.pgm|ppm`, plain-text PGM/PPM): renderings
  by this crate, checked by eye as PNGs before they were committed,
  compared within a tolerance (8 per sample, 0.5 on average).
  `tests/golden.rs` (the test font: straight edges, implied points with a
  quarter-pixel offset, a composite, LCD, laid-out text) runs everywhere;
  `tests/dejavu.rs` (DejaVu text at 12–24 px, LCD mono, a 48 px glyph,
  one glyph at two subpixel offsets) also parses every glyph of every
  bundled font, checks names, metrics and kerning, and matches seventeen
  patterns with Poly's `fonts.conf`. It skips when the fonts are not
  fetched; xtask points it at its cache (`POLY_FONT_DIR`) and CI's
  host-test job fetches the fonts into a cache keyed by version.
- `POLY_FONT_BLESS=1` rewrites the golden files; `POLY_FONT_PNG_DIR`
  writes every rendering (and a 4× enlargement) as PNGs.

## Consequences

- Linux GUI toolkits will bring their own FreeType and fontconfig; this
  store and `fonts.conf` are what they will find. `poly-font` is for
  Poly's own pixels (compositor, shell, a nicer kernel console) and the
  NT personality's text APIs later (`gdi32`, DirectWrite in Phase 7),
  which can map GDI font requests through the same matcher.
- Left for later: rendering in the kernel or a boot screenshot (nothing
  draws with `poly-font` yet), hinting, shaping and fallback fonts, colour
  and bitmap glyphs (`CBDT`, `sbix`, `COLR`), variable fonts (`fvar`,
  CFF2), vertical metrics, `GPOS` beyond pair kerning, charset/language
  in matching, `<match>` and `<selectfont>` rules beyond the fallback form,
  and a font cache file. Point matching in composites is implemented but
  only lightly exercised.
- The root disk grows by 5.5 MB when the fonts are fetched.

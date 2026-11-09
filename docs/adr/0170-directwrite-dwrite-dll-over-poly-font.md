# 0170: DirectWrite: `dwrite.dll` over `poly-font` (factory and COM objects, a system collection of DejaVu under Windows names, glyph run analysis, fallback)

**Status:** Accepted. Delivers the part of "Chrome for Windows runs on Poly"
([0155](0155-chrome-for-windows-bring-up-stubs-lazy-images-and-the-stage-ladder.md))
that finds fonts and draws glyphs: Chrome's Skia asks DirectWrite for fonts and
glyph bitmaps (shaping is Chrome's own). Builds on
[0105](0105-fonts-poly-font-font-store-and-fontconfig-discovery.md) (`poly-font`
and the fonts in `C:\Windows\Fonts`), [0079](0079-our-kernelbase-and-kernel32.md)
(the Win32 workspace and how a DLL is built, embedded and installed),
[0128](0128-windows-gui-win32k-in-the-nt-personality-and-the-desktop-link.md)
(the GUI DLLs share an allocator and a lock) and
[0124](0124-native-desktop-alpine-shell-look-and-builds-on-windows-and-linux.md)
(pure Rust, `rust-lld`, builds on Windows and Linux hosts).

Clean room (CLEAN_ROOM.md): the implementer saw only the spec sheets and visible
tests of `specs/windows/dwrite.dll` (44 entries of `spec-win-dwrite`),
Microsoft Learn's DirectWrite reference, the public SDK headers (`dwrite.h`,
`dwrite_1.h` .. `dwrite_3.h`: interfaces, GUIDs, constants and the order of
the methods in the vtables) and the OpenType specification. No DirectWrite
binary was run or looked at; no Wine, ReactOS, Chromium or Skia source was read.

## Decision

### 1. Two layers

- **`crates/poly-dwrite`** (`no_std` + `alloc`, host-tested, no COM): `sfnt`
  (table directory, `Analyze`), `names` (the `name` table, locale tags,
  localized strings), `face` (weight, stretch, style, metrics, character map,
  glyph metrics), `model` (families, simulated faces, matching), `render`
  (glyph run geometry, the sampler behind `IDWriteGlyphRunAnalysis`, GDI-
  compatible metrics, recommended rendering mode) and `system` (the Windows
  names below). Outlines, advances and flattening of curves come from
  `poly-font`.
- **`personalities/nt/win32/dwrite`** (`dwrite.dll`, a cdylib built with the
  other Win32 modules, `/ENTRY:dwrite_main`, one export `DWriteCreateFactory`):
  the COM objects. Every object starts with a `Head` (vtable pointer,
  reference count, destructor, the interface ids it answers); `AddRef`,
  `Release` and `QueryInterface` are shared. Vtables are the SDK layouts,
  generated into `abi.rs`; slots for interfaces not implemented (text
  layout, GDI interop, ...) return `E_NOTIMPL`. It is registered like
  `gdi32.dll`: `tools/xtask/src/win32.rs` (`OUTPUTS` and the hashed crate list),
  `kernel/build.rs`, `kernel/src/nt/mod.rs` (`DWRITE`), `kernel/src/nt/layout.rs`
  (installed in `C:\Windows\System32`).

### 2. The factory and its interfaces

One object answers for `IDWriteFactory` .. `IDWriteFactory7`: its vtable is the
`IDWriteFactory7` table, a prefix-compatible extension of the others, so
`QueryInterface` for any of them returns the same pointer; `IUnknown` is a
separate identity pointer (a second vtable that forwards to the factory).
`DWRITE_FACTORY_TYPE_SHARED` is one process-wide object, `ISOLATED` a new one
(reference count 1) with its own loader registrations. Errors as recorded:
an unknown type is `E_INVALIDARG`, a non-factory interface `E_NOINTERFACE`,
the output set to null.

The vtable order of *overloaded* methods inside one interface
(`CreateFontFaceReference`, `ComputeGlyphOrigins`, `IDWriteFontSet`'s
`GetPropertyValues` and `GetMatchingFonts`) was not settled by the spec agent;
the table uses header declaration order (as the visible tests declare it).
MSVC may lay overloads out differently; Chrome's Skia does not call these.

### 3. Fonts, files and loaders

A **font file** is a `FileSource` (loader, key bytes, lazily read bytes)
shared by the `IDWriteFontFile` COM objects and by collection entries. The
loader is asked (`CreateStreamFromKey`, `GetFileSize`, `ReadFileFragment`,
once) only when the bytes are first needed (`Analyze`, creating a face, building
a collection). `IDWriteFactory::CreateFontFileReference` uses the built-in
**local loader** (a process-wide object answering `IDWriteLocalFontFileLoader`;
key = path in UTF-16 then the 8-byte last write time; a key of the wrong shape is
`DWRITE_E_CACHEFORMAT`). `RegisterFontFileLoader` / `RegisterFontCollectionLoader`
keep the registered pointers per factory (twice: `DWRITE_E_ALREADYREGISTERED`; a
loader that is not registered: `E_INVALIDARG`). `IDWriteInMemoryFontFileLoader`
(factory 5) keeps copies of the data and numbers them (key = 4 bytes).

### 4. Collections, families, fonts

A **collection** is an immutable `CollData` (the font entries, their sources,
the families), shared by the collection objects made from it. Families group
fonts by their WWS family (name ID 21), else typographic (16), else name ID 1,
ignoring case, ordered by en-us name. A family lists its real fonts (weight,
then distance of the stretch from normal, then style) followed by the faces
DirectWrite simulates: an oblique one of every upright face; a bold one (and
its oblique) for every class of style and stretch with no heavy face (weight
over 600). Weight is the OS/2 class (1..9 to hundreds, bold flag raising it to
700, clamped to 999); the style comes from the italic bit, else an "Oblique"
subfamily name. Face names of simulated fonts are generated in English
("Oblique", "Light Oblique", "Condensed Bold", "Italic Bold").

**Matching** (`GetFirstMatchingFont`, `GetMatchingFonts`) scores the best
variant of each real face (itself, bold-simulated, oblique-simulated or both):
350 per step of stretch, the distance of the weight (1 per unit toward the side
the request prefers, 1.75 away from it, preferring lighter up to 500 and
heavier above), style (an oblique request wants oblique then italic and the
reverse), plus a charge for each simulation. The constants were fitted to the
orderings the spec sheets record and are checked by `poly-dwrite`'s tests; the
first of `GetMatchingFonts` is `GetFirstMatchingFont`.

### 5. The system collection and Windows font names

The system collection is built on first use from the font files in
`C:\Windows\Fonts` (the DejaVu fonts, ADR 0105), once per process.
Every file's fonts are also grouped under the Windows family names that stand
for them (`poly_dwrite::system::ALIASES`), with the face names `Regular`,
`Bold`, `Italic`, `Bold Italic` Windows families use and the same files:

| DejaVu family | Windows names |
|---|---|
| DejaVu Sans | Arial, Arial Unicode MS, Calibri, Candara, Corbel, Franklin Gothic Medium, Lucida Sans Unicode, Malgun Gothic, Microsoft Sans Serif, Microsoft YaHei UI, Nirmala UI, Leelawadee UI, Segoe UI, Segoe UI Symbol, Segoe UI Emoji, Symbol, Tahoma, Trebuchet MS, Verdana, Webdings, Wingdings, Yu Gothic UI |
| DejaVu Serif | Book Antiqua, Cambria, Constantia, Georgia, Palatino Linotype, Times New Roman |
| DejaVu Sans Mono | Cascadia Code, Cascadia Mono, Consolas, Courier New, Lucida Console, Lucida Sans Typewriter |

`Symbol`, `Wingdings` and `Webdings` report `IsSymbolFont` TRUE; `Segoe UI
Emoji` claims only Latin-1, the euro sign, symbols and the emoji planes. The
DejaVu families keep their own names too. Real Windows fonts dropped into the
folder appear under their own names. The system fallback
(`GetSystemFontFallback`, `MapCharacters`) tries Segoe UI (then Arial and the
DejaVu families) for the character, Segoe UI Emoji and Symbol for emoji and
symbols; a character no font has gets a run of its length with no font.
Scripts DejaVu lacks (CJK, Devanagari, Thai) therefore map to no font.

### 6. Faces, metrics, glyphs

A face is the font file, index and simulations (`0..3`); asking again while a
reference is held returns the same object (a per-factory cache of weak
pointers, removed when the face is freed). Metrics: the OS/2 win ascent and
descent (typographic with `USE_TYPO_METRICS`), line gap from `hhea`, underline
from `post`, strike-through from OS/2, cap and x height from OS/2 version 2 or 0.7
and 0.5 of the em. Design glyph metrics use the `glyf` header box; the vertical
origin is the typographic ascender and the advance height ascender minus
descender. `GetGlyphIndices` honours symbol `cmap`s (the `0xF000` offset).
`TryGetFontTable` returns pointers into the face's own copy of the file.

### 7. Rendering

`IDWriteGlyphRunAnalysis` rasterises the run when it is created: the outline
is scaled to pixels (y down), placed, transformed, flattened and **point
sampled** (non-zero winding; samples strictly inside): ClearType textures take
six horizontal samples per pixel on 16 scan lines (one for GDI modes), count
them per stripe and filter three stripes with a box; aliased textures take one
sample at each pixel centre. The bounds are the pixels with a non-zero
result. This reproduces the recorded bounds and the stripe values of a
straight edge; byte-exact agreement on curved edges was not reached (the
recorded rows of the triangle differ by up to three levels). The oblique
simulation shears by 0.25; the bold simulation is not drawn yet.
`GetGlyphRunOutline` delivers `SetFillMode(WINDING)`, then per contour
`BeginFigure`, `AddLines`/`AddBeziers` (quadratics become cubics) and
`EndFigure(CLOSED)`, in pixels with y down; right-to-left runs extend to the
left, sideways glyphs are turned a quarter. `GetGdiCompatibleMetrics` rounds
each value to whole pixels (half down) and back to design units; the
glyph-metric variant is approximate. `GetRecommendedRenderingMode` is
natural-symmetric up to 72 px per em, GDI classic/natural for those measuring
modes, outline from 100.

## To be confirmed by the spec agent against real Windows

1. The vtable slot order of overloaded methods (above).
2. Byte-exact ClearType and aliased textures of curved edges, and the
   GDI-compatible glyph metrics (`GetGdiCompatibleGlyphMetrics`), which differ
   from the recorded values by a few design units.
3. Whether matching lists longer than four fonts (families with more than four
   real faces) return one entry per real face.

## Consequences

- `dwrite.dll` is in `C:\Windows\System32` of every Poly disk (about 130 KB).
- Windows programs find Arial, Segoe UI, Times New Roman, Courier New and
  Consolas; they all draw with DejaVu until other fonts are installed.
- Not implemented: text layout and formats, GDI interop, bitmap render
  targets, colour glyph layers (`TranslateColorGlyphRun` answers
  `DWRITE_E_NOCOLOR`), variable fonts, kerning queries, font download
  and the text analyzer.

# 0142: GDI text: a font mapper onto Poly's own fonts, outline text through `poly-font`, and Spleen-based bitmap faces for the stock fonts

**Status:** Accepted. Follows [ADR 0131](0131-windows-gui-slice-a-window-manager-messages-and-gdi.md)
(the system font's metrics and hand-set glyphs) and
[ADR 0128](0128-windows-gui-win32k-in-the-nt-personality-and-the-desktop-link.md)
Decision 5 (logical fonts are mapped in `gdi32` onto Poly's font store);
uses the fonts and the rasteriser of [ADR 0105](0105-fonts-poly-font-font-store-and-fontconfig-discovery.md)
and `poly-gfx`'s Spleen. One question is the owner's: ⚖️ see "Open".

## Context

Windows programs draw text with `TextOutW`, `ExtTextOutW` and `DrawTextW`
in the fonts they create with `CreateFontW` / `CreateFontIndirectW` (dialogs
and most older programs; Chrome draws through DirectWrite, which is a later
slice). Poly cannot ship Microsoft's fonts, and the specs record the output
of three bitmap stock fonts and one tracked behaviour, not a general font
engine:

- `TextOutW.yaml` gives `TEXTMETRICW` and advance widths for
  `SYSTEM_FONT`, `DEFAULT_GUI_FONT`, `ANSI_FIXED_FONT` and
  `OEM_FIXED_FONT`, and the exact pixels of the system font's `A`, `g`
  and `1`.
- `TextOutW.tests/to_glyphs.rs` also records a hash of the whole bitmap of
  `"The quick brown fox"` and its ink count (449 pixels in 0,3,129,16).
  That is a fingerprint of Microsoft's system font, glyph by glyph.
- There are no spec sheets yet for `CreateFontW`, `CreateFontIndirectW`,
  `GetTextMetricsW` on outline faces, `GetTextExtentPoint32W` on them,
  `GetTextFaceW`, `GetCharWidth32W`, `EnumFontFamiliesExW`, `ExtTextOutW`
  (clipping, `lpDx`), or `DrawTextW`'s ellipsis, `DT_MODIFYSTRING` and
  `DT_RTLREADING`. For these the implementation follows Microsoft Learn's
  reference pages; where Learn is silent the rule below is a choice of this
  ADR, marked "(choice)", for a spec agent to confirm or replace.

Before this ADR the repository drew the system font's glyphs for every
stock face and every `CreateFontIndirectW` font: `DEFAULT_GUI_FONT` and
`ANSI_FIXED_FONT` text came out with their descenders cut off and their
letters clipped on the right, any character outside ASCII was a box, and
`CreateFontIndirectW` picked a face from the height alone.

## Decisions

### 1. The font mapper (`poly_gdi::fontmap`)

`LogFont::parse` reads a 92-byte `LOGFONTW` (`wingdi.h`). `map` decides, in
this order:

1. **A raster font name** (`System`, `Terminal`, `Fixedsys`, `Courier`,
   `MS Sans Serif`, `MS Serif`, `Small Fonts`) is the stock bitmap face of
   that name at any size, as in Windows, where these names never scale.
2. **An outline name the table knows** gives a class: sans serif (Arial,
   Segoe UI, Tahoma, Verdana, Calibri, Microsoft Sans Serif, MS Shell Dlg
   and their relatives), serif (Times New Roman, Georgia, Cambria,
   Palatino Linotype...) or fixed pitch (Courier New, Consolas, Lucida
   Console, Cascadia...). Matching is ASCII case-insensitive, ignores
   trailing blanks and a leading `@`, and a trailing `*` in the table
   covers a family's variants ("Segoe UI Semibold", "Arial Narrow").
3. **Any other name** follows the program's `lfPitchAndFamily`: fixed pitch
   or `FF_MODERN` is fixed pitch, `FF_ROMAN` is serif, anything else
   (including no hint) is sans serif.

The class names the free font Poly has of that kind, the fonts of the font
store of ADR 0105: **DejaVu Sans, DejaVu Serif, DejaVu Sans Mono**. The
table is the same one `crates/poly-font/fonts.conf` applies for Linux
programs, so a face name means the same on both sides. `Mapping` carries
bold (`lfWeight >= 600`, the file `-Bold`) and italic (`-Oblique` or
`-Italic`).

### 2. Outline fonts (`poly_gdi::outline`)

`CreateFontIndirectW` makes an **`Outline`** for every mapped (not raster)
font whose file can be read: the file, the pixel size and the
`TEXTMETRICW`. No file (the font store is not installed) means the bitmap
face of Decision 3 draws instead, so nothing depends on the fonts being
there.

**Where the files come from.** `Gdi::set_font_loader`: `gdi32.dll` reads
`C:\Windows\Fonts\<file>` and then `Z:\usr\share\fonts\truetype\dejavu\<file>`
(the root disk links the first to the second, ADR 0105; the desktop image
has only the second) with `CreateFileW`/`ReadFile`, on first use, one file
at a time, and keeps the bytes. A file that does not parse with outlines
counts as missing. A missing bold or italic file falls back to the same
weight upright, then to the regular file (Poly does not slant or embolden
letters itself).

**Size** (`LOGFONTW.lfHeight`, Learn): negative is the character height, the
em, in pixels; positive is the cell height (ascent plus descent), for which
the largest em whose cell fits is taken; zero is a cell of 13 pixels, the
height of `DEFAULT_GUI_FONT` (choice). `lfWidth` greater than 0 scales the
glyphs and advances horizontally so that the average lowercase width is that
many pixels (`render_glyph_scaled`); 0 keeps the font's proportions.
`lfEscapement` and `lfOrientation` are not honoured: text is horizontal.

**Metrics.** Ascent and descent are the font's `hhea` values (or the typo
metrics `poly-font` selects) scaled to the em and rounded to the nearest
pixel (choice: Windows' rounding is not documented); `tmHeight` is their
sum, `tmInternalLeading` the cell less the em, `tmExternalLeading` the
scaled line gap, `tmAveCharWidth` the mean advance of `a`..`z`,
`tmMaxCharWidth` the scaled bounding box width, `tmWeight` 400 or 700,
`tmPitchAndFamily` `TMPF_VECTOR | TMPF_TRUETYPE` plus the family (and, as
`wingdi.h` has it, `TMPF_FIXED_PITCH` set for a *variable* pitch font),
aspect 96x96, first character 0x20, last and default 0xFFFD, break 0x20.

**Advances** are the font's `hmtx` advance scaled to the em and rounded to
whole pixels, character by character with no kerning, which is how GDI's
unhinted TrueType text is laid out; a control character is one pixel wide
(as the system font in `DrawTextW.yaml`) and draws nothing. Characters the
font lacks show its `.notdef` glyph. Only the Basic Multilingual Plane is
looked up; a surrogate pair is two `.notdef` glyphs. No shaping, no
bidirectional text, no font fallback.

**Rendering.** `poly_font::render_glyph_scaled` in grey scale (no hinting)
at the whole-pixel pen position, through `GlyphCache` (1 MB). The
coverage byte of each pixel blends the text colour over the surface by
`dst = (fg * a + dst * (255 - a)) / 255` per channel, inside the clip
region; a surface of 8 bits per pixel or fewer takes the text colour where
coverage is 128 or more. In `OPAQUE` mode the cell (the extent by the
height) is filled with the background colour first, as for the bitmap
faces. Underline and strike-out (`lfUnderline`, `lfStrikeOut`) are
rectangles a twelfth of the em thick (at least one pixel): the underline
from a third of the descent below the baseline, the strike-out through the
middle of the lowercase (choice).

### 3. Glyphs for the stock faces (`poly_gdi::bitfont`)

The stock fonts keep their bitmap faces, whose extents the sheets pin. The
system font's ASCII glyphs are what ADR 0131 made them: the oracle's
recorded rows for `A`, `g`, `1`, hand-set rows for the rest, advance widths
solved from the measured extents. Everything else is cut from **Spleen
8x16** (`poly-gfx`, BSD 2-Clause, `THIRD_PARTY.md`), whose baseline lies
after row 11:

- `ANSI_FIXED_FONT` (height 13, ascent 11, 8 wide) and `OEM_FIXED_FONT`
  (height 12, ascent 10) show Spleen's rows from `12 - ascent` on, so the
  baselines agree with the sheets; a descender that passes the cell is cut.
- `DEFAULT_GUI_FONT` does the same and then fits each glyph to the advance
  width the face reports (the sheets' measured widths for `A i W H d e o l
  r w , space`, three quarters of the system font's solved widths for the
  rest): the ink columns are thinned (the emptiest column first, so stems
  stay) or widened (the middle column repeats) to the advance minus a
  one-pixel gap. Advance widths are not touched.
- Characters of the system font above ASCII (Latin-1, Latin Extended-A,
  Greek, Cyrillic, `€`, `™`, box drawing...) are Spleen's glyph lowered one
  row for the system ascent of 13; a character Spleen lacks is the old
  box.

Where a mapped font has no outline (no files), `bitmap_face` stands in:
a raster name is its own face, fixed pitch is `ANSI_FIXED_FONT`, a
proportional font of `abs(lfHeight)` 13 or less (or 0) is
`DEFAULT_GUI_FONT`, a taller one `SYSTEM_FONT`. A `LOGFONTW` shorter than
92 bytes gets the system face.

### 4. The text calls

- **`TextOutW` / `ExtTextOutW` / the `A` variants** (`Gdi::ext_text_out`):
  alignment (`TA_*`, `TA_UPDATECP`) as `TextOutW.yaml`; `ETO_OPAQUE` fills
  the rectangle with the background colour (also for an empty string);
  `ETO_CLIPPED` clips the text to it; `lpDx` is each character's advance
  (and the extent used for alignment and the new current position).
  `ETO_GLYPH_INDEX`, `ETO_RTLREADING`, `ETO_PDY` are ignored. ANSI strings
  are read as Latin-1 (as the rest of Poly's `A` functions).
- **Measuring:** `GetTextExtentPoint32W/A`, `GetTextExtentPointW/A`,
  `GetTextExtentExPointW/A` (fit count and running widths), `GetTextMetricsW/A`,
  `GetCharWidth32W/A`, `GetCharWidthW/A`, `GetTextFaceW/A` (the name the
  program asked for, also where Poly draws a font of its own; the stock fonts
  report System, MS Shell Dlg, Courier, Terminal (choice)).
- **Creating:** `CreateFontW/A`, `CreateFontIndirectA` build the `LOGFONTW`
  and call the same path as `CreateFontIndirectW`.
- **`DrawTextW`, `DrawTextA`, `DrawTextExW/A`** (`Gdi::draw_text_ex`):
  everything of `DrawTextW.yaml` (unchanged results for the stock fonts)
  and from Learn: `DT_EXPANDTABS`/`DT_TABSTOP` (stop in average character
  widths; `DRAWTEXTPARAMS.iTabLength` overrides), `DT_EXTERNALLEADING`
  (line height plus the external leading), margins of `DrawTextEx`.
  *Prefixes:* `&x` underlines `x` (every marked character) unless
  `DT_NOPREFIX` or `DT_HIDEPREFIX`; `DT_PREFIXONLY` draws only the
  underlines; `DT_CALCRECT` is not affected. *Ellipses:* `DT_END_ELLIPSIS`
  cuts a line that is too wide and ends it with `...` (as many dots as fit
  when not even one character does); with `DT_WORDBREAK` text running past
  the rectangle's bottom ends in an ellipsis on the last line that fits;
  `DT_PATH_ELLIPSIS` replaces the middle and keeps what follows the last
  backslash; `DT_WORD_ELLIPSIS` cuts each too-wide word. `DT_MODIFYSTRING`
  writes the cut text back to the caller's buffer (NUL-terminated) when
  an ellipsis changed it. Ellipses do not change `DT_CALCRECT` (choice).
  `DT_RTLREADING`, `DT_EDITCONTROL`, `DT_INTERNAL` and
  `DT_NOFULLWIDTHCHARBREAK` are accepted and ignored. Lines wrap at spaces
  only, a word too long stays whole (as the sheet records).
- `user32.dll` exports `DrawTextA`, `DrawTextExA`, `DrawTextExW`; `gdi32.dll`
  the functions above; both lists are in `poly_win32k::exports` and checked
  by `cargo xtask win32`.

### 5. What was not done

- `to_glyphs` cannot pass. The hash and ink count are fingerprints of the
  Microsoft system font's pixels for fifteen letters the sheet does not
  show; only `A`, `g` and `1` are recorded. Nothing here approximates the
  hash.
- Rotated text (`lfEscapement`), `GetCharABCWidths`, `GetKerningPairs`,
  `GetOutlineTextMetrics`, `GetGlyphOutline`, `EnumFontFamiliesEx`,
  `AddFontResource`, `SetTextCharacterExtra`, `SetTextJustification`,
  `GetTextExtentExPointI`, ClearType-style LCD output (the rasteriser has
  it; GDI's `lfQuality` is not honoured, grey scale only).

## Open

⚖️ **`TextOutW/to_glyphs` (and the glyph pixels of `DrawTextW`).**
Options, recommendation first:

1. *Accept the difference:* mark `to_glyphs` as a known glyph-shape
   difference (metrics, cells, alignment and the three recorded glyphs
   stay exact). Held-out tests that hash glyph pixels fail the same way.
2. *Have the spec agent record every glyph of the system font* as rows and
   ship them as Poly's data. Passes the test, but it is a copy of
   Microsoft's bitmap font; a legal call, not a technical one.
3. *Drop glyph-hash checks* from the spec (keep the ink bounding box and
   cell checks).

Option 1 is what this branch does.

## Consequences

- A Windows program that creates a font gets real, antialiased text in
  Segoe UI / Arial / Tahoma / Times / Courier / Consolas looks (DejaVu
  underneath) at any size, in the colours and background mode of its DC; the
  `desktop-win32` boot test's program shows it (screenshot in the
  hand-off) and the boot test checks for grey scale pixels.
- `poly-gdi` depends on `poly-font` and `poly-gfx` (the Spleen 8x16 table,
  about 15 KB, is linked); the `Gdi` holds the font bytes it has read (up to
  twelve files, 5.5 MB) and a 1 MB glyph cache per process that draws text.
- The desktop image carries all twelve DejaVu files in
  `/usr/share/fonts/truetype/dejavu` (it had only the Sans regular).
- Visible tests: unchanged (140 of 141; `to_glyphs` still differs in ink and
  hash).
- Needs a spec agent: `CreateFontW`/`CreateFontIndirectW` (what `GetObject`
  returns, height and weight rounding, mapping of unknown names),
  `GetTextMetricsW` and `GetTextExtentPoint32W` on TrueType faces (Arial,
  Segoe UI, Tahoma, Courier New, Consolas at -11/-12/-16/-24),
  `GetTextFaceW`, `GetCharWidth32W`, `SetTextAlign`/`GetTextAlign`,
  `ExtTextOutW` (`ETO_OPAQUE`, `ETO_CLIPPED`, `lpDx`), `GetStockObject` for
  fonts, and the `DrawTextW` flags `DT_END_ELLIPSIS`, `DT_PATH_ELLIPSIS`,
  `DT_MODIFYSTRING`, `DT_HIDEPREFIX`, `DT_PREFIXONLY`, `DT_EDITCONTROL`,
  `DT_INTERNAL`. Each "(choice)" above is a rule to confirm.

## Update: the spec sheets of `spec-win-text` (impl-win-text-2)

The "(choice)" lines above were written before any spec sheet existed for
the text calls. `spec-win-text` has since observed them (CreateFontIndirectW,
GetTextMetricsW, GetTextExtentPoint32W, ExtTextOutW, TextOutW, DrawTextW,
...; `specs/windows/gdi32.dll/`, `user32.dll/`) with fonts the tests write
for themselves, so that Windows and Poly read the same font file. What
follows replaces the decisions it contradicts; everything else above stands.

### What is new

1. **Private fonts** (`poly_gdi::fonttab`). `AddFontMemResourceEx` copies the
   image, `AddFontResourceExW` reads the file (`FR_PRIVATE`, `FR_NOT_ENUM`;
   flags other than those two bits fail with error 87), `RemoveFont...` take
   one add away again (the file has to exist; the flags have to match). A
   font is a TrueType or OpenType font with outlines, whose table directory
   lies inside the data and which has a family name; anything else adds
   nothing (no error). The faces are found by family name (case-insensitive,
   exact: a blank more or less is another name) or by full name; of a
   family's faces the one that matches the bold and italic request best is
   taken (bold first). A request is mapped when the font is selected, and
   again whenever a font is added or removed. Only files add to
   `EnumFontFamiliesExW`'s list (not memory fonts, not `FR_NOT_ENUM`); its
   other entries are Poly's installed families under their Windows names
   (Regular, Bold, Italic, Bold Italic each).
2. **Metrics come from the font file** (`outline::Outline::new`):
   `OS/2.usWinAscent` and `usWinDescent` (or `hhea`), scaled and rounded to
   the nearest pixel, halves up; `hhea.lineGap` is the external leading;
   `OS/2.xAvgCharWidth` the average width; `hhea.advanceWidthMax` the
   maximum; the first character is the lowest code point of the `cmap` (at
   least 32), the last the highest of the BMP, the default one below the
   first, the break character 32; the family is FF_MODERN without
   TMPF_FIXED_PITCH for a font marked monospaced (`post`, panose), else
   TMPF_FIXED_PITCH (which means variable) with no family. The em is 16
   pixels for `lfHeight` 0, `|lfHeight|` below zero and at least 2; a
   positive `lfHeight` is the *cell*: ascent and descent are scaled together
   to add up to it, so the em is fractional (advances use it, the internal
   leading is the cell less the rounded em). `lfWidth` scales the horizontal
   size so that the average width is that many pixels.
3. **Advances** are the file's, scaled and rounded to whole pixels; tab, line
   feed and carriage return take no room in `GetTextExtentPoint32W` (but have
   the `.notdef` width in `GetCharWidth32W`); characters the font lacks are
   `.notdef`.
4. **Synthesis.** A bold request that the family has no face for makes every
   advance, the average and the maximum width one pixel wider and the ink a
   pixel wider; an italic one slants the ink (advances unchanged).
   `tmWeight` is 700 then. Underline and strike-out use the font's own `post`
   and `OS/2` values (position of the top of the line, thickness).
5. **Pixels.** `lfQuality` NONANTIALIASED and ANTIALIASED draw all or nothing
   (coverage of at least half a pixel, and a row or column the outline
   touches in a row always gets a pixel: the oracle drew ANTIALIASED the same way);
   DEFAULT, CLEARTYPE and CLEARTYPE_NATURAL give each colour channel its own
   coverage (three subpixels per pixel, each channel the mean of three, a
   gain of 1.4: the box glyphs of the sheet come out with the observed
   164 pure and 168 blended pixels); DRAFT and PROOF are grey scale. Glyphs are rendered
   through one affine map, which also carries the slant and
   `lfEscapement`/`lfOrientation`: the text runs along the escapement and
   turns about the reference point (a quarter turn puts the baseline one
   pixel further away than the ascent, as observed).
6. **No font files, still TrueType.** The test images of the diff runner (and
   a bare root disk) have no DejaVu, but a logical font must behave as a
   TrueType face everywhere. `poly_gdi::builtin` makes two TrueType fonts in
   memory from Spleen 8x16's pixels (one square of outline per ink pixel;
   units per em 1024, a pixel 64 units, ascent 12, descent 4 pixels), one
   proportional and one fixed pitch, and the mapper uses them where the
   store's files are missing. The bitmap faces now only stand for the stock
   objects and the raster font names.
7. **`GetTextFaceW`** reports the font's own spelling of its family name (its
   full name when that was asked for), a name Poly knows in its Windows
   spelling (`arial` is `Arial`), and for a name it does not know the family
   it stands in with (Arial, Times New Roman or Courier New by class).
8. **ExtTextOutW**: `ETO_GLYPH_INDEX`; an inverted rectangle is normalised
   for filling and clipping; all options set fail.
9. **DrawText**: a mnemonic underline is one row at baseline + 1 as wide as
   the character, drawn by default; text wraps at blanks, a blank that fits
   stays at the end of its line (counted for left aligned lines, not for
   centred or right aligned ones), one that does not is dropped, a word
   wider than the rectangle has its own line; `
`, `
` and `` break
   lines; lines are laid out while their top is not below the rectangle's
   bottom (that is the height returned and `uiLengthDrawn`); end and word
   ellipsis keep at least one character and the dots even where that is
   wider, path ellipsis keeps what follows the last backslash and leaves a
   string without one (or ending in one) alone, an ellipsis with
   `DT_WORDBREAK` cuts the last line laid out; `DT_MODIFYSTRING` keeps the
   `&` of the text; `DT_CALCRECT` measures the cut text; `DrawTextEx`'s
   margins, tab length (only with `DT_TABSTOP`), `cbSize` of 20 and
   `uiLengthDrawn` are done in `poly-gdi`.

### Open

- `to_glyphs` is gone from the sheet; the pixels of the stock bitmap fonts
  and of the installed families (DejaVu under Windows names) are still not
  specified, and are not matched.
- Rotation other than quarter turns, vertical fonts, kerning, charset-specific
  mapping, `GetCharABCWidths` and bidirectional text are not done.
- A font added with `FR_PRIVATE` is listed by `EnumFontFamiliesExW` with the
  metrics of a default-size face only in the log font; the
  `NEWTEXTMETRICEXW` argument carries just weight, italic and pitch.

# 0142: GDI text: a font mapper onto Poly's own fonts, and Spleen-based bitmap faces for the stock fonts

**Status:** Accepted. Follows [ADR 0131](0131-windows-gui-slice-a-window-manager-messages-and-gdi.md)
(the system font's metrics and hand-set glyphs) and
[ADR 0128](0128-windows-gui-win32k-in-the-nt-personality-and-the-desktop-link.md)
Decision 5 (logical fonts are mapped in `gdi32` onto Poly's font store);
uses the fonts of [ADR 0105](0105-fonts-poly-font-font-store-and-fontconfig-discovery.md)
and `poly-gfx`'s Spleen. One question is the owner's: ⚖️ see "Open".

## Context

Windows text reaches Poly through `TextOutW`, `DrawTextW` and (soon) the
rest of GDI's text calls. Poly cannot ship Microsoft's fonts, and the
specs record the output of three bitmap stock fonts and one tracked
behaviour, not a general font engine:

- `TextOutW.yaml` gives `TEXTMETRICW` and advance widths for
  `SYSTEM_FONT`, `DEFAULT_GUI_FONT`, `ANSI_FIXED_FONT` and
  `OEM_FIXED_FONT`, and the exact pixels of the system font's `A`, `g`
  and `1`.
- `TextOutW.tests/to_glyphs.rs` also records a hash of the whole bitmap of
  `"The quick brown fox"` and its ink count (449 pixels in 0,3,129,16).
  That is a fingerprint of Microsoft's system font, glyph by glyph.
- There are no spec sheets yet for `CreateFontW`, `CreateFontIndirectW`,
  `GetTextMetricsW` on outline faces, `GetTextExtentPoint32W` on them,
  `GetTextFaceW`, `GetCharWidth32W`, `EnumFontFamiliesExW`, `SetTextAlign`
  beyond `TextOutW`'s alignment rules, `ExtTextOutW` (clipping, `lpDx`),
  or `DrawTextW`'s ellipsis, `DT_MODIFYSTRING` and `DT_RTLREADING`.

Before this ADR the repository drew the system font's glyphs for every
stock face and every `CreateFontIndirectW` font: `DEFAULT_GUI_FONT` and
`ANSI_FIXED_FONT` text came out with their descenders cut off and their
letters clipped on the right (the glyph rows of a 16-pixel bold face in a
13-pixel cell), and any character outside ASCII was a box. `CreateFontIndirectW`
picked a face from the height alone.

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
programs (Arial, Helvetica, Verdana, Tahoma, Segoe UI -> sans-serif; Times
New Roman, Georgia -> serif; Courier New, Consolas, Lucida Console ->
monospace), so a face name means the same on both sides. `Mapping` also
carries bold (`lfWeight >= 600`) and italic for the outline path.

### 2. Which bitmap face draws a mapped font today

The outline path (`poly-font` rasterising DejaVu at the `LOGFONT`'s
height, with metrics from the font) needs the spec sheets listed above,
or its extents and `TEXTMETRICW` would be guesses the oracle never saw. Until
they exist `bitmap_face` chooses among the four bitmap faces:

| Mapped font | Face |
|---|---|
| a raster name | its own face |
| fixed pitch | `ANSI_FIXED_FONT` (8x13) |
| sans serif or serif, `abs(lfHeight)` of 13 or less, or 0 (the default size) | `DEFAULT_GUI_FONT` (13, proportional) |
| sans serif or serif, taller | `SYSTEM_FONT` (16, bold, proportional) |

A `LOGFONTW` shorter than 92 bytes gets the system face. These are
choices of this ADR, not observations; the table is one function and a
test, so the outline slice replaces it in one place.

### 3. Glyphs for the stock faces (`poly_gdi::bitfont`)

The system font's ASCII glyphs stay what ADR 0131 made them: the oracle's
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
  one-pixel gap. Advance widths are not touched: extents stay what the
  sheets and ADR 0131 settled.
- Characters of the system font above ASCII (Latin-1, Latin Extended-A,
  Greek, Cyrillic, `€`, `™`, box drawing...) are Spleen's glyph lowered one
  row for the system ascent of 13; a character Spleen lacks is the old
  box. Every face now shows `ä ö ü é ñ ß € ™`, which German- and
  French-language programs need.

### 4. What was not done

`to_glyphs` cannot pass. The hash and ink count are fingerprints of the
Microsoft system font's pixels for fifteen letters the sheet does not
show; only `A`, `g` and `1` are recorded. Reproducing the font would
need either its glyph data (not ours to ship, and forbidden to look
inside) or the spec agent recording all 95 glyphs of the font as pixel
rows, which would be a complete copy of the font's bitmaps in the
repository. Nothing here approximates the hash.

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

- Text in the GUI and fixed faces is legible: descenders, right edges and
  non-ASCII characters were wrong or missing.
- `CreateFontIndirectW` picks a face by name, hint and size, in a table the
  outline slice will reuse.
- `poly-gdi` depends on `poly-gfx` (no further dependencies; only the
  Spleen 8x16 table, about 15 KB, is linked).
- Visible tests: unchanged. `to_glyphs` still differs in ink and hash.
- Needs a spec agent: `CreateFontW`/`CreateFontIndirectW` (what `GetObject`
  returns, height and weight rounding, mapping of unknown names),
  `GetTextMetricsW` and `GetTextExtentPoint32W` on TrueType faces (Arial,
  Segoe UI, Tahoma, Courier New, Consolas at -11/-12/-16/-24),
  `GetTextFaceW`, `GetCharWidth32W`, `GetCharABCWidthsW`,
  `SetTextAlign`/`GetTextAlign`, `ExtTextOutW` (`ETO_OPAQUE`,
  `ETO_CLIPPED`, `lpDx`), `GetStockObject` for fonts, `EnumFontFamiliesExW`,
  `SetTextCharacterExtra`, and the `DrawTextW` flags
  `DT_END_ELLIPSIS`, `DT_PATH_ELLIPSIS`, `DT_MODIFYSTRING`, `DT_RTLREADING`,
  `DT_HIDEPREFIX`, `DT_PREFIXONLY`, `DT_EDITCONTROL`, `DT_INTERNAL`.

# 0181: Fonts for Windows programs: where Poly's fonts appear, what a missing face gets, and `CreateNumberSubstitution`

**Status:** Accepted. Follows [ADR 0142](0142-gdi-font-mapper-and-bitmap-faces-for-windows-text.md)
(GDI maps every face name onto Poly's own fonts), [ADR 0170](0170-directwrite-dwrite-dll-over-poly-font.md)
(DirectWrite's system collection is made of the files in `C:\Windows\Fonts`)
and the stage-1 stop of [ADR 0155](0155-chrome-for-windows-bring-up-stubs-lazy-images-and-the-stage-ladder.md).

Clean room (CLEAN_ROOM.md): the implementer saw the spec sheets and visible
tests of `spec-chrome-win-6` (`specs/windows`: `GetObjectW`, `CreateFontIndirectW`,
`GetTextFaceW`, `GetTextMetricsW`, `EnumFontFamiliesExW`, `SystemParametersInfoW`,
`GetDC`, `IDWriteFontCollection.FindFamilyName`, `IDWriteFactory.GetSystemFontCollection`,
`IDWriteFactory.CreateNumberSubstitution`, `IDWriteFontFallback.MapCharacters`),
Microsoft Learn and the public headers (`dwrite.h`, `wingdi.h`). No Wine, ReactOS
or Chromium source.

## Context

Chrome's UI-font set-up asks `SystemParametersInfoW(SPI_GETNONCLIENTMETRICS)` for
five fonts, makes each with `CreateFontIndirectW`, reads the face name back with
`GetObjectW` and asks DirectWrite for a family of that name (`FindFamilyName`). On
Poly the S1 stage disk had no `C:\Windows\Fonts`, so DirectWrite's system
collection was empty, nothing was found, the fallback (`CreateNumberSubstitution`
then `MapCharacters`) failed too because `CreateNumberSubstitution` was a stub
(`E_NOTIMPL`), and Chrome started its UI-font set-up again, 5,226 times, until the
8 MB stack was full. Windows never gets there: the system collection is never empty
and a face that is not in it is still mapped by the system fallback.

## Decisions

### 1. Where the fonts are

Poly ships no Microsoft fonts. The fonts are the pinned DejaVu 2.37 files of
`cargo xtask fonts fetch` (ADR 0105) and nothing else. For Windows programs they
appear in two places, and both are looked at:

1. **`C:\Windows\Fonts`**, the folder of the Windows layout (ADR 0073). Every
   image that runs Windows programs *and is made by a tool* carries the twelve
   files there: the root disk (symbolic links into `usr/share/fonts/truetype/dejavu`,
   `fonts::add_to_root`, as before), **the Chrome stage disk** (new: `chrome-win
   disk` fetches the fonts when they are not cached and adds them the same way),
   and the diff runner's initramfs for tests that import `dwrite.dll` or create
   GDI fonts (as before). A font a program or the user puts there is seen by both
   libraries and wins over a file of the same name in the store.
2. **Poly's font store**, `Z:\usr\share\fonts\truetype\dejavu` (`Z:` is `/`), which
   the desktop image already carries for Linux programs and for `gdi32.dll`
   (ADR 0142). `dwrite.dll`'s system collection now reads this folder as well
   (`personalities/nt/win32/dwrite/src/system.rs`), after `C:\Windows\Fonts`,
   skipping a file name it has already taken. So the desktop image needs no
   second copy, a disk that links the store into `C:\Windows\Fonts` and an image
   that only has the store give the same collection, and `gdi32.dll` and
   `dwrite.dll` look in the same two folders in the same order.

The registry's font list, `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts`
(`"<face> (TrueType)"` = file name, as Windows has it), lists the twelve files; the
layout sync writes the values when they are missing at every boot
(`layout::sync::FONT_LIST`, policy `Default`, so a program's own values stay).

Not done: a built-in fallback collection inside `dwrite.dll` for an image with no
fonts at all (`GetSystemFontCollection.yaml` says the collection is never empty;
on Poly it is empty exactly when the fonts were never fetched). Every tool that
makes an image for Windows programs fetches them (`chrome-win disk` does now);
the font bytes of `poly_gdi::builtin` (Spleen as TrueType) could stand in later.

### 2. What a missing face gets

`CreateFontIndirectW` makes the font whatever the face, `GetObjectW` gives the
request back as written, and only when the font is selected into a DC does GDI map
it (`CreateFontIndirectW.yaml`: `names`, `absent_face`). The face name is compared
as a **whole name**, ignoring ASCII case: a blank before, after or inside, a prefix
or an extension of a name is another name. The mapper table (`poly_gdi::fontmap`)
had patterns that ended in `*` ("segoe ui*", "arial*", "calibri*", "courier new*",
...) and ignored trailing blanks, so "Segoe UI PolyNoSuch" counted as Segoe UI and
`GetTextFaceW` said so; the table now lists the names Windows has (Segoe UI
Semibold, Light, Semilight, Black, Emoji, Symbol, Historic, Variable ...; Arial
Black, Narrow, Rounded MT Bold; Calibri Light; the Yu Gothic and Meiryo
variants) one by one and nothing else matches.

A name the table does not know follows `lfPitchAndFamily`: fixed pitch or
`FF_MODERN` is a fixed-pitch TrueType font, anything else a proportional one; it
draws with the DejaVu family of that class, `GetTextFaceW` names the face it
stands in with (Arial, Times New Roman or Courier New by class: the same face an
empty name gets) and `EnumFontFamiliesExW` lists that face, as the sheets require.
`tmHeight - tmInternalLeading` is the requested em height whatever face is chosen,
and `tmCharSet` is the requested charset (`DEFAULT_CHARSET` is ANSI).

DirectWrite does not substitute: `FindFamilyName` finds exactly the system
collection's families (the 78 Windows names of ADR 0170 plus DejaVu's three: Segoe
UI is among them, so the face of the message font is always a family) and
generic names (`sans`, `sans-serif`, `serif`, `monospace`, `System`, `MS Shell
Dlg`) are not families. A face that is not found goes through the **system
fallback** (`MapCharacters`), which already answers Segoe UI for Latin text.

`GetObjectW` on a font reads a negative size as unsigned (a font gives its 356
bytes for -1, `GetObjectW.yaml` `larger_buffer`).

### 3. `IDWriteFactory::CreateNumberSubstitution`

No longer a stub. The method must be 0 to 4, else `E_INVALIDARG`; only
`FROM_CULTURE` looks at the locale name: NULL is `E_INVALIDARG`, and so is a name
that is not made of ASCII letters, digits, `-` and `_` ("not a locale at all"); `xx-YY`,
`EN-US`, `en_US` and the empty string are fine, as the sheet records. Every failure
sets the out pointer to NULL. The object (`IDWriteNumberSubstitution`, no methods
of its own) answers `QueryInterface` for its own interface and `IUnknown` with the
same pointer, has one reference, and each call makes a new one. The check is
`poly_dwrite::numsub::check`, host-tested. It keeps the arguments and nothing else:
Poly has no text analyzer.

## Doc-derived, to be confirmed against Windows

- The rule that makes "a sentence" an invalid `FROM_CULTURE` locale: the sheet only
  has `"not a locale at all"` and `"a sentence"` as invalid and `xx-YY`, `EN-US`,
  `en_US`, `""` as valid. Poly takes any name with a character outside
  `[A-Za-z0-9_-]` as invalid; the length of a `FROM_CULTURE` name is not checked
  (85 is `LOCALE_NAME_MAX_LENGTH`; the sheet has a 94-character name only with `NONE`).
- Which Windows names the GDI table treats as installed (the lists above are the
  families of a default Windows 11 install plus a few common ones); a name of a
  family Windows does not have is "absent" there too.
- The registry font list's exact value names and file names are Poly's (DejaVu).

## Consequences

- The Chrome stage disk grows by the 5.5 MB of the fonts (the `fonts-0181` stamp
  makes `chrome-win disk` build it again once).
- S1's browser no longer recurses in the UI-font set-up; where it goes next is in
  the hand-off `chrome-win-s10.md`.
- `CreateFontIndirectW` with "Arial " (a trailing blank) no longer maps to Arial
  by name; the font gets the stand-in of its hints (Windows does the same).

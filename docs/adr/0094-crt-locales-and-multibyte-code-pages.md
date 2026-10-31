# 0094: Locales other than "C" and multibyte code pages in our C runtime: one code page table set in `kernelbase` (generated from the Unicode Consortium's mapping files), a shared `poly-nls` crate, interned locale objects, Poly's own documented collation

**Status:** Accepted. Extends [ADR 0084](0084-our-c-runtime-ucrtbase-and-msvcrt.md)
(our `ucrtbase.dll` and `msvcrt.dll`, which had the "C" locale only) and
[ADR 0079](0079-our-kernelbase-and-kernel32.md) (`kernelbase`'s code pages:
1252, 437 and UTF-8 until now).

## Context

The Phase 5 item "C runtime" left "locales other than "C" and multibyte
code pages (`_mbs*`)". Console programs call `setlocale(LC_ALL, "")` at
start-up, print numbers with `printf` in the user's locale, convert
arguments and file names with `mbstowcs`/`wcstombs`, and programs written
for Japanese, Chinese and Korean Windows walk strings with the `_mbs*`
functions, which must not split a double-byte character (the classic
example: 表 is `95 5C` in Shift-JIS, and its trail byte is `\`).

Sources (clean room, CLEAN_ROOM.md): Microsoft Learn's `setlocale`/
`_wsetlocale` page (the forms of the locale argument, "UTF-8 support"),
"Locale names, Languages, and Country/Region strings", "Language strings",
"Country/Region strings", "Code Pages", "Code Page Identifiers",
"Interpretation of multibyte-character sequences", the pages of
`localeconv`, `_create_locale`, `_free_locale`, `_configthreadlocale`,
`_setmbcp`, `_getmbcp`, `MB_CUR_MAX`, each `_mbs*`/`_mbc*`/`_ismb*`
function (with the code page 932 ranges of `_ismbchira`, `_ismbckata`,
`_ismbcsymbol`, `_ismbcl0/1/2`, `_mbctohira`, `_mbctokata`), `strcoll`,
`strxfrm`, the `mbstowcs_s` family, `MultiByteToWideChar`,
`WideCharToMultiByte`, `GetCPInfo`, `IsDBCSLeadByteEx`, the `LOCALE_*`
constants and [MS-LCID]; the public headers `locale.h` (`LC_*`, `struct
lconv` with the UCRT's wide members), `corecrt.h` (`__crt_locale_pointers`,
`__crt_locale_data_public`), the Visual C++ 2005–2013 `crtdefs.h`
(`threadlocaleinfostruct`, `threadmbcinfostruct`), `mbctype.h` (`_MS` …
`_SBLOW`, `_MB_CP_*`), `mbstring.h` (`_MBC_*`), `ctype.h` (`_LEADBYTE`),
`winnls.h`, `string.h` (`_NLSCMPERROR`); the Unicode Consortium's public
code page mapping files and Character Database; the Unicode CLDR's
published regional formats; JIS X 0208's published Shift-JIS arithmetic.
No Windows binary, UCRT, Wine, ReactOS or ICU source was looked at, and
nothing ran on Windows.

## Decision

### One set of code page tables, generated, in a shared crate

`crates/poly-nls` (host-tested, `no_std`) holds what `kernelbase` and the C
runtime share: the code pages ([`codepage`]), the locale table
([`locale`]), Unicode classes and case ([`unicode`]), and Poly's collation
([`collate`]).

- **Code pages.** Single-byte 437, 737, 775, 850, 852, 855, 857, 860, 861,
  862, 863, 865, 866, 869, 874, 1250–1258; double-byte 932 (Shift-JIS),
  936 (GBK), 949 (Unified Hangul Code), 950 (Big5); UTF-8. 1252 stays
  `poly_rtl::codepage`'s table (ADR 0071), read at compile time.
- **Generated, reproducibly.** `cargo xtask nls-tables` downloads the
  Unicode Consortium's "Microsoft vendor" mapping files
  (`MAPPINGS/VENDORS/MICSFT/WINDOWS/CPnnn.TXT` and `PC/CPnnn.TXT`) into
  `target/nls-mappings/` (or reads `--from <dir>`), checks each against
  the SHA-256 pinned in `tools/xtask/src/nls.rs`, and writes
  `crates/poly-nls/src/tables.rs` (single-byte high halves, double-byte
  single bytes, lead-byte rows, lead and trail byte ranges) and
  `crates/poly-nls/data/cpNNN.bin` (a double-byte code page's cells, row by
  row, little-endian `u16`; 22–48 KB each). Both are checked in; the
  mapping files are not.
- **Kernelbase converts.** `poly_kernelbase::nls` now resolves any of these
  code pages (`MultiByteToWideChar`, `WideCharToMultiByte`, `GetCPInfo`
  with the lead byte ranges, `IsValidCodePage`, new `IsDBCSLeadByte` and
  `IsDBCSLeadByteEx`). The second copy of code page 437 it had is gone. A
  double-byte code page's reverse index (character → code) is built from
  the forward table on first use and kept.
- **The C runtime converts through `kernel32`**, as the real one does, and
  links only the small parts of `poly-nls` (the lead/trail byte ranges, the
  locale table, classes, collation): the double-byte cells exist once, in
  `kernelbase.dll`.

Choices where Learn is silent (to be confirmed by the spec agent): an
unassigned byte or double byte decodes to `?` (U+003F) and is invalid for
`MB_ERR_INVALID_CHARS` (1252's five unassigned bytes stay C1 controls,
ADR 0079); a lead byte before a byte that is not a trail byte, or at the
end, is one `?` and the next byte is decoded alone; no best-fit mapping;
when 932 maps several codes to one character (NEC and IBM extensions),
the character converts back to the lowest code, codes with lead byte
0x87, 0xED or 0xEE losing to any other.

### Locale names and data

`poly_nls::locale::LOCALES` has 48 locales (English, German, French,
Italian, Spanish, Portuguese and Dutch variants, the Nordic languages,
Polish, Czech, Slovak, Hungarian, Russian, Ukrainian, Greek, Turkish,
Hebrew, Lithuanian, Vietnamese, Thai, Japanese, Chinese Simplified and
Traditional, Korean) with LCID, English language and country names, the
three-letter abbreviations, ANSI and OEM code pages, decimal and thousands
separators, grouping, currency symbol and ISO code, and the currency
formats (`ICURRENCY`, `INEGCURR`, `ICURRDIGITS`). Poly's user and system
locale is en-US (1252/437).

`poly_crt::locale::parse` reads a category's locale:

| Name | Answered as |
|---|---|
| `"C"`, `"POSIX"` | `C` |
| `""` | `English_United States.1252` |
| `.ACP`, `.OCP`, `.utf8`/`.UTF-8`, `.850` | `English_United States.1252` / `.437` / `.utf8` / `.850` |
| `German_Switzerland.850`, `german`, `german-swiss`, `deu`, `French_Canada.OCP` (Learn's language and country strings) | `German_Switzerland.850`, `German_Germany.1252`, `German_Switzerland.1252`, … |
| `de-CH`, `de_CH`, `de-CH.utf8`, `pl-PL.1252` (locale names) | `de-CH`, `de-CH.utf8`, `pl-PL.1252` (the code page shown when it is not the locale's ANSI one) |

The code page must be one `kernel32` converts; UTF-8 only in `ucrtbase`
(Learn: a UCRT feature). When categories differ, `setlocale(LC_ALL, NULL)`
answers `LC_COLLATE=…;LC_CTYPE=…;LC_MONETARY=…;LC_NUMERIC=…;LC_TIME=…`,
which `setlocale(LC_ALL, …)` accepts back.

### The runtime's locale objects

- A locale (`Loc`) is built once per distinct set of five category
  settings and **interned** for the process's life, so every pointer handed
  out (`setlocale`'s string, `localeconv`'s `lconv`, `__pctype_func`'s
  table) stays valid whatever another thread sets later. The multibyte
  code page's tables (`_mbctype`, `_mbcasemap`) likewise.
- `LC_CTYPE` decides the `_pctype` table (ASCII as "C"; another byte by its
  character's Unicode class; a lead byte `_LEADBYTE` only; nothing above
  0x7F in UTF-8), `toupper`/`tolower` (through UTF-16 when the other case
  is a byte of the code page), `MB_CUR_MAX` (1, 2, or 4 for UTF-8), and the
  multibyte conversions. `towupper`/`towlower` use full Unicode case
  outside "C" (Latin-1 in "C", as before). Wide classes do not depend on
  the locale; above U+00FF they now come from `poly_nls::unicode` (Latin
  Extended, Greek, Cyrillic, Armenian, Hebrew, Arabic, Thai, CJK, the
  fullwidth forms) where they had none.
- `LC_NUMERIC`'s decimal point reaches `printf`, `scanf` and `strtod` as
  bits 56–63 of the format engines' option word
  (`poly_crt::locale::point_option`; the program's own bits there are
  cleared), and `float::parse_point`. `LC_MONETARY` fills `lconv`'s
  currency members (`int_curr_symbol` is the ISO code without C's
  separator; `p_sign_posn` 3). `lconv`'s strings are in each category's
  code page.
- `LC_TIME` is not localised: `strftime` keeps the "C" names
  (`_strftime_l` / `_wcsftime_l` ignore their locale).
- `setlocale` of `LC_CTYPE` or `LC_ALL` also sets the multibyte code page
  to the new `LC_CTYPE` code page (Poly's reading of `_MB_CP_LOCALE`);
  `_setmbcp` takes `_MB_CP_SBCS`, `_MB_CP_OEM`, `_MB_CP_ANSI`,
  `_MB_CP_LOCALE` or a code page Poly converts other than UTF-8; the
  start-up multibyte code page is 0 (single byte), as `_getmbcp` reported.
- **`_locale_t`**: our block, `locinfo` and `mbcinfo` first as
  `corecrt.h` declares. For `ucrtbase` `locinfo` points at
  `__crt_locale_data_public` (`ctype.h`'s inline functions read it); for
  `msvcrt` at the classic `threadlocaleinfostruct` (offsets checked at
  compile time) and `mbcinfo` at `threadmbcinfostruct`.
  `_create_locale(category, name)` starts from "C"; `_free_locale` frees
  only the block.
- **Per-thread locales**: `_configthreadlocale(_ENABLE_PER_THREAD_LOCALE)`
  gives the thread a copy of the global locale (and multibyte code page)
  that its `setlocale`/`_setmbcp` then change; a TLS slot of the locale
  module's own, freed at `DLL_THREAD_DETACH`. While no thread has its own,
  nobody reads TLS.
- **`msvcrt`'s exported variables** (`_pctype`, `__mb_cur_max`, `_mbctype`,
  `__lc_codepage`, `__lc_collate_cp`, `__lc_handle`) and `ucrtbase`'s
  `_mbcasemap` are bound at attach and kept current on every global change.

### Collation: Poly's own, documented

Windows collates with `CompareStringEx` and the locale's sort tables, which
Poly does not have and does not pretend to have. In a locale other than
"C", `strcoll`, `wcscoll`, `_stricoll` … and the `_mbscoll` family use
`poly_nls::collate`: three levels (characters by class — punctuation and
symbols, then digits by value, then Latin letters by base letter with
`æ`/`œ`/`ß`/`ĳ`/`þ` expanded, then other letters by code point; then
diacritics; then case, lower first), the same in every locale, with no
tailoring. `strxfrm`/`wcsxfrm` produce sort keys that `strcmp`/`wcscmp`
order the same way. In "C", byte order as before.

### The multibyte string functions

`poly_crt::mbcs` has the logic (host-tested on code page 932): character
walking (`_mbsinc`, `_mbsdec`, `_mbsninc`, `_mbsnbcnt`, `_mbsnccnt`,
`_mbsbtype`), searching that never matches a trail byte (`_mbschr`,
`_mbsrchr`, `_mbsstr`, `_mbspbrk`, `_mbsspn`, `_mbscspn`, `_mbstok`),
comparison, case (single bytes through `_mbcasemap`, double bytes through
UTF-16: fullwidth Ａ ↔ ａ), `_mbsrev` keeping pairs, `_mbsset` and the copy
and append functions that never cut a character, the `_s` forms, and the
code page 932 functions (hiragana/katakana, JIS ↔ Shift-JIS, `_mbbtombc`/
`_mbctombb` through the fullwidth and halfwidth forms). Every function is
exported with its `_l` form. Poly's choices: comparisons are byte by byte
(case-insensitive ones fold single-byte letters); `_mbsncpy` pads with one
NUL per character short.

### Exports

Each new function is written once in the shared runtime; what both DLLs
list is exported from there, what only one lists is exported by that DLL
(`ucrtbase/src/locale.rs`, `msvcrt/src/locale.rs`). The `_o_*` aliases of
`ucrtbase` and `msvcrt`'s `_printf_l`-style variadic functions are left
out (the latter overlap the positional `printf` work).

## To be confirmed by the spec agent against real Windows

1. The names `setlocale` answers: `""` → `English_United States.1252`;
   `.utf8` → `English_United States.utf8`; locale names answered in
   canonical case (`de-ch` → `de-CH`); `Chinese (Simplified)_China.936`
   for `chs`; `msvcrt` refusing UTF-8.
2. The locale table's separators, groupings and currency formats (Windows
   changes them between releases, e.g. de-CH's thousands separator);
   `int_curr_symbol` without a fourth character; `p_sign_posn`.
3. `_pctype` of bytes above 0x7F (Poly: the character's Unicode class);
   `toupper` of bytes whose other case is not in the code page; UTF-8
   locales' `MB_CUR_MAX` (4) and classes (none above 0x7F); `isleadbyte`
   in UTF-8 (never).
4. That `setlocale(LC_CTYPE)` changes the multibyte code page; `_getmbcp`
   after `setlocale(".utf8")` (Poly: 65001, the `_mbs` functions treating
   it as single-byte).
5. `mbrtowc` of a UTF-8 character above U+FFFF (Poly: `EILSEQ`);
   `wcrtomb` with a surrogate pair (Poly: the high surrogate kept in the
   state); the `_s` functions' results on truncation.
6. The undefined-byte and duplicate-mapping choices of the code pages
   (above), `IsDBCSLeadByte(Ex)`'s API-set contract.
7. Collation: Windows' order differs (word sort, tables for every script).
8. `_mbsicmp`'s folding of double-byte letters, `_mbsncpy`'s padding,
   `_mbcjistojms` outside code page 932 (Poly: unchanged).

## Consequences

- Host tests: `cargo test -p poly-nls` (every single-byte code page round
  trips; Shift-JIS, GBK, UHC and Big5 samples both ways, invalid and
  truncated input, the NEC duplicate; UTF-8; the locale table's
  consistency and lookups; Unicode classes and that case maps round trip
  over the whole BMP; collation order and that sort keys order as
  `compare`), `cargo test -p poly-crt` (`setlocale` names, composite
  strings, `lconv`, `LC_CTYPE` tables for 1252/1251/932/UTF-8, `printf`
  and `strtod` with a decimal comma, the `_mbs*` logic on 932, JIS ↔
  Shift-JIS over the whole JIS grid), `cargo test -p poly-kernelbase`
  (double-byte `CPINFO`, conversions, `IsDBCSLeadByteEx`).
- The ring-3 test `nt::test_crt_locale` runs the new `crtloctest.exe`:
  about 70 checks of names, `localeconv`, `printf`/`strtod` with a decimal
  comma, classes and case in 1252 and 1251, conversions in 1252, 932 and
  UTF-8 (so `kernelbase`'s generated tables), `mbrtowc` across calls, the
  `_mbs*` functions on 932, `_create_locale` and `_l` forms, collation, a
  thread's own locale, and `msvcrt.dll`'s `__mb_cur_max`, `_pctype` and
  `_mbctype`. `crttest`'s `setlocale(LC_ALL, "")` check now expects
  `English_United States.1252`.
- Seen while writing the test, not fixed here: a thread made by
  `CreateThread` with the default stack faulted below its first guard
  page instead of growing (the test gives its thread a committed 256 KiB
  stack), and the kernel's backtrace of the nested fault that followed in
  `KiUserExceptionDispatcher` overflowed the kernel stack.
- Sizes: `kernelbase.dll` grows by the double-byte tables (~170 KB),
  `ucrtbase.dll` and `msvcrt.dll` by ~80 KB each (no tables).
- Shared files changed (additively): the host workspace members, the Win32
  workspace members, `xtask`'s `main.rs` (`nls-tables`) and `win32.rs`
  (the new program), `kernel/build.rs`, `kernel/src/nt/mod.rs`,
  `KERNEL32_FORWARDS`; in the runtime, the ctype, collation, case and
  multibyte functions moved out of `strings.rs`, `conv.rs` and `misc.rs`
  into the new modules, and `stdio.rs`'s four format calls carry the
  decimal point.
- Coverage: the new exports of `ucrtbase.dll`, `msvcrt.dll`,
  `kernelbase.dll` and `kernel32.dll` are claimed by `impl-crt-locale`.
- **Left**: localised `LC_TIME` (`strftime` names), the `_l` locale
  argument of `__stdio_common_*` (the current locale is used), `msvcrt`'s
  `_printf_l` family, `mbrtoc16`/`c16rtomb`, locales beyond the table,
  `GetLocaleInfoEx`/`CompareStringEx` in `kernelbase`, Windows' sort
  tables.

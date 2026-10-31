# 0101: System message text: Learn's "System Error Codes" pages as a generated table, `FormatMessageW`/`FormatMessageA` in full, and the 7-Zip scenario made required

**Status:** Accepted. Extends `FormatMessageW` of
[ADR 0079](0079-our-kernelbase-and-kernel32.md) (which had 51 hand-copied
messages). Makes the 7-Zip scenario of
[ADR 0088](0088-7-zip-scenario-driver.md) a required check. The Phase 5
milestone "7-Zip command line passes its full recorded trace" stays
unticked (see "The milestone").

## Context

With C++ exceptions in `msvcrt` ([ADR 0099](0099-cpp-exception-handling-in-msvcrt.md)),
15 of the scenario's 16 steps matched Windows. The last one,
`missing-archive` (`7za x -y missing.7z`), printed `Error #80070002` on
stderr where Windows prints the system's text for that error:
`FormatMessageW` had no message for the `HRESULT` 0x80070002
(`HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)`), and only 51 Win32 codes at
all. `FormatMessageA`, module message tables, `*` widths and line wrapping
were missing too.

Sources (clean room, CLEAN_ROOM.md; implementer): Microsoft Learn's "System
Error Codes" page and its ten range pages (0-499, 500-999, 1000-1299,
1300-1699, 1700-3999, 4000-5999, 6000-8199, 8200-8999, 9000-11999,
12000-15999), the `FormatMessageW` / `FormatMessageA` reference pages,
"Message Tables", the `MESSAGE_RESOURCE_DATA` / `MESSAGE_RESOURCE_BLOCK` /
`MESSAGE_RESOURCE_ENTRY` pages, the `HRESULT_FROM_WIN32` macro page, the PE
format specification (the resource tree), and the constants of the public
headers `winerror.h`, `winnt.h` and `winuser.h` (`RT_MESSAGETABLE`). The
scenario's recorded Windows output was not used as a source of text: every
message comes from Learn's tables. No Windows binary, Wine, ReactOS or
7-Zip source was looked at.

## Decision

### The table: `cargo xtask system-messages`, from Learn, checked in

`tools/xtask/src/sysmsg.rs` downloads the ten range pages from
learn.microsoft.com with `curl` into `target/learn-system-error-codes/`
(or reads them with `--from <dir>`) and extracts every code. On each page
a code is three paragraphs: the constant's name in bold (after its
anchors), `N (0xH)` (both forms must agree), and the message text. A few
codes have a further paragraph (an older name of the constant, a note on
Windows XP / Server 2003), which documents the code and is not message
text; it is skipped. Inside the text, tags (links, bold) are dropped,
entities decoded, and white space runs become one space. The tool
refuses a page with fewer than 100 codes, a code outside the page's
range, a code listed twice, or one above 65535.

It writes two files, both checked in so a build needs neither the network
nor the tool:

- `crates/poly-kernelbase/data/system-messages.tsv`: the extracted data,
  `code<TAB>name<TAB>text` per line, by code (2,745 codes on 2026-10-31);
- `crates/poly-kernelbase/src/sysmsg.rs`: the same as `CODES: [u16; N]`,
  `ENDS: [u32; N]` and one `TEXT` string (`concat!` of one literal per
  message, each under a comment with its code and name): about 195 KB of
  text, no relocations, a binary search per lookup.

The pages are not pinned by hash (unlike ADR 0094's mapping files):
Learn's HTML changes with every site build while the texts do not, so a
hash would break on every run. Re-running the tool and reviewing the TSV's
diff is the update procedure.

Every range Learn lists is in: the base codes (0-1299, including the
registry's 1009-1021 and the network connection codes of the 1200s),
security and services (1300-1699), RPC and networking (1700-3999,
including the network connection codes 2108-2404), clusters, the directory
service, DNS (9000s), Winsock (10004-11999), IPsec and side-by-side
(12000-15999).

### `FormatMessageW` and `FormatMessageA`

Both are in `personalities/nt/win32/kernelbase/src/misc.rs` on top of
`poly_kernelbase::message` (host-tested), and `kernel32` forwards both
through `api-ms-win-core-localization-l1-2-0`.

- **Sources.** `FORMAT_MESSAGE_FROM_STRING` formats the caller's string
  (UTF-16 for W, ANSI for A). `FORMAT_MESSAGE_FROM_HMODULE` searches the
  module's `RT_MESSAGETABLE` resource (ID 1; the executable for a NULL
  handle); a module without one is `ERROR_RESOURCE_TYPE_NOT_FOUND`
  (Learn), a message ID no block holds is `ERROR_MR_MID_NOT_FOUND`.
  With `FORMAT_MESSAGE_FROM_SYSTEM` too, the system table is searched when
  the module does not have the message (Learn). `FORMAT_MESSAGE_FROM_SYSTEM`
  alone searches the system table; an unknown ID is
  `ERROR_MR_MID_NOT_FOUND`.
- **`HRESULT`s.** A message ID `0x8007xxxx` (`FACILITY_WIN32` with the
  severity bit, `HRESULT_FROM_WIN32`) has the message of Win32 code
  `xxxx`. Other facilities are not mapped.
- **The text.** A system message is Learn's text followed by CR LF (the
  "regular line break" a message definition ends with); the low byte of
  the flags then applies (below).
- **Message tables.** `MESSAGE_RESOURCE_DATA`: a block count, blocks of
  `LowId`, `HighId`, `OffsetToEntries`; entries of `Length`, `Flags` (0
  ANSI, `MESSAGE_RESOURCE_UNICODE` 1, `MESSAGE_RESOURCE_UTF8` 2) and
  NUL-padded text, consecutive per block. Our `kernelbase` reads the
  module's resource tree with `poly-pe` (`Pe::parse_mapped` over the
  mapped image, `SizeOfImage` long).
- **Languages.** A specific LANGID gets that language only, else
  `ERROR_RESOURCE_LANG_NOT_FOUND` (Learn); `LANG_NEUTRAL` with
  `SUBLANG_DEFAULT` (0x0400) or `SUBLANG_SYS_DEFAULT` (0x0800) names the
  user's or system's default, which on Poly is English (United States),
  0x0409. Language 0 follows Learn's order: neutral, the thread's, user's
  and system's defaults (all 0x0409 on Poly), English (United States),
  then any language present. The system table is English (United States)
  only, so it serves 0, 0x0400, 0x0800 and 0x0409.
- **Inserts.** `%1`-`%99` with an optional `!printf format!` (default
  `!s!`): flags `- 0 + space #`, width and precision as digits or `*`,
  sizes `h l w ll I I32 I64`, types `s S c C d i u x X o p`; floating-point
  types are refused (Learn: not supported) with `ERROR_INVALID_PARAMETER`,
  like a missing argument or an unknown type. In `FormatMessageA` a plain
  `s`/`c` is ANSI and `S`/`C` wide; in `FormatMessageW` the reverse; `h`
  makes either ANSI, `l`/`w` wide. A `*` takes an argument before the
  value. Learn's numbering of the inserts after one with asterisks is
  followed exactly: with an argument array insert `n` starts at element
  `n`; with a `va_list` every earlier insert number that has a `*` shifts
  the later ones by one argument (the page's two examples, `"%1!*.*s! %4
  %5!*s!"` and `"%1!*.*s! %3 %4!*s!"`, are host and ring-3 tests). With
  `FORMAT_MESSAGE_IGNORE_INSERTS` the arguments are ignored (Learn).
  On x64 a `va_list` and an argument array are both 8-byte slots.
- **Escapes.** `%0` ends the message (no line break), `%n` a hard line
  break (CR LF), `%r` a bare CR, `%t` a tab, any other character after `%`
  is copied without the `%` (`%%`, `%.`, `%!`, `% `).
- **Line width** (the low byte). 0 keeps the text's line breaks.
  `FORMAT_MESSAGE_MAX_WIDTH_MASK` turns each regular line break into one
  space and keeps `%n` breaks. Any other value does the same and then
  wraps every line at that many characters, breaking with CR LF in place of
  the space before a word that does not fit; a word longer than the width
  stays whole (Learn: never splits a string delimited by white space).
- **The buffer.** Without `FORMAT_MESSAGE_ALLOCATE_BUFFER` a message that
  does not fit `nSize` units with its NUL is `ERROR_INSUFFICIENT_BUFFER`.
  With it, the buffer is `LocalAlloc`'d for at least `nSize` units and its
  address stored at `lpBuffer`; a message over 128K bytes is
  `ERROR_MORE_DATA` (Learn). The result is the length without the NUL, in
  `TCHAR`s (bytes for A, converted to the ANSI code page).

### The scenario is required

`cargo xtask win 7zip-diff --build-kernel` now passes all 16 steps (setup
and 15 `7za.exe` runs: exit codes, stdout and stderr bytes, files; the two
documented normalizations of ADR 0088 for local times). As ADR 0088
prescribes, `SCENARIO_REQUIRED` in `tools/xtask/src/sevenzip.rs` is now
`true`: a difference, or a failure to run the scenario, fails the `7zip`
boot group (CI shard `userland`).

### The milestone

"7-Zip command line passes its full recorded trace" is **not ticked**.
ADR 0087 ("The recording on Windows") asks for five things; ADR 0088 built
points 1, 2, 4 and 5 (inputs, commands, outputs and files, the comparison)
and said, for the moment the scenario passes: make it required "and tick
nothing else: the milestone also needs the API trace (ADR 0087, point 3)".
ADR 0090 built the tracer, but the trace part of the recording is not
there yet:

1. **No recorded Windows trace of the scenario.** ADR 0090 traced three
   separate runs (`i`, `a`, `a -mmt1`) on `homeserver2` and keeps them out
   of the repository; the tracer is not yet run as the scenario driver's
   "traced" variant (ADR 0090, "Next"), so there is no trace of the 15
   steps to compare with.
2. **The traces differ.** ADR 0090's comparison found 43 and 160
   divergences. Some are the set-up (TCG's `PF_XSAVE_ENABLED`, the
   reported build number, `_isatty` on a console instead of redirected
   handles, the working directory's length) and need a normalization or a
   matching set-up; others are Poly's: `__getmainargs` leaves the last
   error where Windows sets 0, `SetDefaultDllDirectories` is missing
   (`GetProcAddress` NULL), `AdjustTokenPrivileges` fails with
   `ERROR_ACCESS_DENIED`, `FindFirstFileW` sets the last error to 0. These
   need spec-agent observations before they are changed.
3. **Nothing checks it.** The `wintrace` boot group is report-only and
   has no committed reference.

The milestone is met when the traced scenario (all 15 steps, worker
threads included) recorded on Windows and on Poly compares without
differences other than documented normalizations, and that comparison is
a required check like the scenario's. The roadmap's "left:" text says so.

## Results

- Scenario (`cargo xtask win 7zip-diff --build-kernel`, TCG): **16 of 16**
  steps pass; `missing-archive` now matches byte for byte.
- Host tests: `poly-kernelbase` `message` (the whole table: sorted, no
  empty or multi-line text, every Learn range present; `HRESULT`s;
  languages; inserts, ANSI inserts, `*` widths with Learn's examples,
  escapes, widths and wrapping, bad inserts; message table parsing and
  module language choice) and `xtask` `sysmsg` (page parsing, a broken
  entry, the generated table's offsets and escaping).
- Ring 3: `k32test` checks the `HRESULT` text, a Winsock code with
  LANGID 0x0409, `ERROR_MR_MID_NOT_FOUND` and
  `ERROR_RESOURCE_LANG_NOT_FOUND`, an allocated buffer freed with
  `LocalFree`, `MAX_WIDTH_MASK`, Learn's two `*` examples (argument array
  and `va_list`), a system message's insert, `%0`/`%%`/`%n`, a missing
  argument, `FROM_HMODULE` on itself (`ERROR_RESOURCE_TYPE_NOT_FOUND`) and
  with `FROM_SYSTEM`, and `FormatMessageA` (system text, ANSI and `!S!`
  inserts, a short buffer, an allocated buffer).

## To be confirmed by the spec agent

1. Messages Windows splits over several lines: Learn shows each as one
   paragraph, so ours are one line (e.g. the `{Title}` messages of
   500-999). The exact texts of codes Learn does not list are absent
   (`ERROR_MR_MID_NOT_FOUND`).
2. Wrapping (a width other than 0 and `MAX_WIDTH_MASK`): where the break
   goes, whether trailing spaces stay.
3. `IGNORE_INSERTS` with escapes (ours: the text unchanged, ADR 0079).
4. A specific English LANGID other than 0x0409 (ours:
   `ERROR_RESOURCE_LANG_NOT_FOUND`, as Learn says for "that LANGID only").
5. A NULL string insert, a NULL `lpBuffer`, no source flag at all (ours:
   an empty string, `ERROR_INVALID_PARAMETER`, `ERROR_MR_MID_NOT_FOUND`),
   and `HRESULT`s of other facilities that have system texts.

## Consequences

- New: `tools/xtask/src/sysmsg.rs` (`cargo xtask system-messages`),
  `crates/poly-kernelbase/data/system-messages.tsv`,
  `crates/poly-kernelbase/src/sysmsg.rs` (generated).
- Changed: `crates/poly-kernelbase/src/message.rs` (the table, `HRESULT`s,
  languages, ANSI inserts, `*`, wrapping, message tables),
  `error.rs` (`ERROR_RESOURCE_TYPE_NOT_FOUND`), `exports.rs`
  (`FormatMessageA`); `kernelbase`'s `misc.rs` and `Cargo.toml` (`poly-pe`
  for the resource tree); `k32test`; `sevenzip.rs` (`SCENARIO_REQUIRED`);
  the coverage map (`FormatMessageA` in `kernel32` and `kernelbase`
  claimed by `impl-formatmessage`); the roadmap's "left:" text.
- `kernelbase.dll` grows by the table (about 200 KB).

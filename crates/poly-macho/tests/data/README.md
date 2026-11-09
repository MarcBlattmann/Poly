# Mach-O test files

Small Mach-O files for `poly-macho`'s tests. They are **built here**, from the
C sources in `src/`, with LLVM's `clang` and `ld64.lld`. No Apple SDK, no Apple
binary and no file from a Mac was used or read. `generate.sh` makes all of
them; the tests read the committed files and need none of these tools.

## What is in here

| File | What it is |
|---|---|
| `arm64_chained.exe` | arm64 executable with chained fixups (`-fixup_chains`, pointer format 2), imports from `libtiny` and `libweakdep`, rebases in `__DATA` |
| `arm64_classic.exe` | the same program with classic `LC_DYLD_INFO_ONLY` opcodes (`-no_fixup_chains`): rebase, bind, weak bind and lazy bind streams |
| `x86_64_classic.exe` | x86-64 executable, macOS 10.15 minimum, classic opcodes, 4 KiB pages |
| `x86_64_chained.exe` | x86-64 executable with chained fixups (these need macOS 11 as the minimum) |
| `x86_64_nopie.exe` | x86-64 executable without `MH_PIE`: fixed address, no rebases, an absolute `__mh_execute_header` export |
| `libtiny.dylib` | arm64 dylib with six exports (functions, data, a weak definition) that share name prefixes: an export trie with several edges |
| `libre.dylib` | arm64 dylib that re-exports `libtiny` (`LC_REEXPORT_DYLIB`) and has an `LC_RPATH` |
| `universal.bin` | `llvm-lipo -create` of `arm64_chained.exe` and `x86_64_classic.exe`: a fat file with two slices |

Every executable has symbols (the linker's default), `LC_RPATH` entries (two
on the arm64 programs, one on the x86-64 ones), a weak dependency (`LC_LOAD_WEAK_DYLIB`), `LC_MAIN`, `LC_UUID`,
`LC_BUILD_VERSION`, `LC_FUNCTION_STARTS`, `LC_DATA_IN_CODE`, and (arm64, which
`ld64.lld` signs by default) `LC_CODE_SIGNATURE`.

`src/` holds the C: `tiny.c` (the library), `app.c` (the program, with a table
of function pointers and data pointers so there are binds and rebases),
`weakdep.c`, `empty.c`, and `system.c`. The last is a stand-in written here
for the system library: classic lazy binding needs a library that exports the
binder symbol, so the build links against a two-line dylib named
`/usr/lib/libSystem.B.dylib`, not Apple's.

What the files do **not** have: arm64e code. `ld64.lld` 18 accepts
`-arch arm64e` but writes a plain arm64 file, so the arm64e pointer formats
(authenticated rebases and binds) are tested on images the tests build
themselves (`tests/support`, `tests/synthetic.rs`), from the bit layouts of
`<mach-o/fixup-chains.h>`.

## How they were made

On Ubuntu 24.04 (aarch64 was used; the output does not depend on the host)
with the `clang`, `lld` and `llvm` packages installed, from the repository
root:

```sh
bash crates/poly-macho/tests/data/generate.sh
```

Tools, as installed by those packages (`clang`, `lld`, `llvm`):

| Tool | Version |
|---|---|
| `clang` | Ubuntu clang 18.1.3 (1ubuntu1) |
| `ld64.lld` | Ubuntu LLD 18.1.3 |
| `llvm-lipo` | LLVM 18.1.3 |
| `llvm-objdump` (to check, not to build) | Ubuntu LLVM 18.1.3 |

Targets: `-target arm64-apple-macos11` and `-target x86_64-apple-macos10.15`
with `-ffreestanding -nostdlib -fno-stack-protector -fno-ident -Os`, so no SDK
header is read. The link line gives the platform itself
(`-platform_version macos 11.0 11.0`).

Regenerating gives the same bytes (the script was run twice and the SHA-256 of
every file compared). With other tool versions the files change, and the
literal values in `tests/fixtures.rs` have to be checked again.

## Where the expected values come from

`tests/fixtures.rs` has the values as literals, recorded from `llvm-objdump
--macho` run on these files (`--private-headers`, `--rebase`, `--bind`,
`--lazy-bind`, `--weak-bind`, `--exports-trie`, `--chained-fixups`,
`--function-starts`, `--indirect-symbols`, `--universal-headers`), `llvm-nm`
and `llvm-lipo`. Chained fixup chains are not printed by that tool; their
walk is checked against the slots decoded by hand and against the classic
build of the same program.

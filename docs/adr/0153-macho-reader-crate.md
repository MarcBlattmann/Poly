# 0153: A Mach-O reader: `poly-macho`

**Status:** Accepted. Second piece of the plan in
[ADR 0151](0151-macos-plan-arm64-translator-and-aarch64-port.md); Roadmap
Phase 9, groundwork for "Mach-O loading and our own `dyld`".

## Context

Every macOS program and library is a Mach-O file, and a universal file is
several of them in one. The arm64 translator of ADR 0151 needs the code and
the entry point of an arm64 program. Our own `dyld` needs everything else:
the segments to map, the libraries to load, the fixups to apply, the
exports to look symbols up in. A kernel port to aarch64 needs the same
reader for its first user program. Reading the format is the same work for
all three, so it is done once, apart from the work that needs an address
space.

The format has several layers that all show up in real files today:
universal headers, load commands, the symbol table, the classic rebase and
bind opcode streams (older linkers and deployment targets), chained fixups
(what newer linkers write by default, `-fixup_chains`), export tries, and arm64e's pointer authentication fields.

Untrusted input is the normal case: a loader reads files downloaded from the
network. The reader must hold up against files made to break it.

## Decision

A new `no_std` crate `crates/poly-macho` with no dependencies reads and
checks Mach-O files. It **only reads**: it maps nothing, applies no fixups,
loads no libraries and resolves no symbols.

### Design

- **Zero-copy over `&[u8]`.** Everything borrows the file's bytes. There is
  no allocator. Lists are iterators; names are `&[u8]` (symbols) or `&str`
  (load command paths, which must be UTF-8). The one fixed-size copy is a
  table of up to 32 segments inside `MachO`, so the fixup walkers can check
  addresses without holding the load commands.
- **Check once, then trust the tables you opened.** `MachO::parse` checks the
  header, every load command's size and structure, every segment against the
  file and against the other segments, and remembers the first command of
  each kind. The accessors check only the table they open (a symbol table's
  range, a trie, a fixup stream).
- **Thin and universal files.** `Fat` reads `FAT_MAGIC` and `FAT_MAGIC_64`
  (big-endian), checks every slice for bounds, size and overlap, and finds a
  slice by CPU (`select_slice`). Thin files must be 64-bit little-endian;
  32-bit and big-endian files are recognised and refused with their own
  errors. arm64 and arm64e are told apart by the CPU subtype with the
  capability bits masked off.
- **Typed errors.** `Error` is a `Copy` enum with one variant per kind of
  corruption. Nothing returns a bare "invalid".

### What is covered

- Header, file type, flags, CPU (x86-64, arm64, arm64e, others by number).
- Load commands: segments and sections; `LC_SYMTAB` with `nlist_64` and
  strings; `LC_DYSYMTAB` and the indirect symbol table; every dylib kind
  (`LOAD`, `WEAK`, `REEXPORT`, `LAZY_LOAD`, `UPWARD`, `ID`, and the
  `dylib_use_command` encoding of macOS 15); dylinker; `LC_MAIN`;
  `LC_UNIXTHREAD`/`LC_THREAD` with the program counter and stack of the x86-64
  and arm64 states; UUID; `LC_BUILD_VERSION` and `LC_VERSION_MIN_*`;
  `LC_RPATH`; source version; target triple; encryption info; the
  `linkedit_data` family (code signature, function starts, data in code,
  export trie, chained fixups, ...); `LC_DYLD_INFO(_ONLY)`.
- Export tries: walking, lookup, and the regular, thread-local, absolute,
  weak, re-export and stub-and-resolver forms.
- Classic fixups: the rebase, bind, weak bind and lazy bind opcode streams as
  iterators of records (segment, offset, address, type, library ordinal,
  symbol, flags, addend; for lazy binds the stream offset a stub pushes).
- Chained fixups: the header, the segment starts, the three import formats
  with their names and addends, and a walk of every chain for pointer
  formats `ARM64E`, `64`, `64_OFFSET`, `ARM64E_KERNEL`, `ARM64E_USERLAND`
  and `ARM64E_USERLAND24`: rebase targets and `high8`, bind ordinals and
  addends, and the pointer authentication key, diversity and address
  diversity.
- A summary for a loader: segments with ranges and protections, the entry
  point, dependencies in ordinal order, run paths, PIE, platform and minimum
  OS, UUID, install name.

### Robustness

The input is hostile. Whatever the bytes are, the crate does not panic, read
out of bounds, overflow or run for long.

- Every offset and size is checked against the data it indexes, with checked
  arithmetic. Every table is checked against the file when it is opened.
- Every loop either consumes input or has a bound: slices of a universal file
  (64), segments (32), trie depth (128, no recursion) and nodes visited
  (no more than the trie has bytes), name length (1 KiB in a trie, 64 KiB in
  a string table), LEB128 length (10 bytes), and a budget of records per
  rebase or bind stream (one per pointer slot in the segments' file
  contents). A chain walk always moves forward inside its page and the
  segment's file bytes.
- A bind, rebase or chained fixup must lie in the file-backed part of its
  segment, so no stream can ask for more records than the file holds.
- `ADD_ADDR` opcodes wrap around 2^64 on purpose (a linker encodes a
  negative skip that way); every record is checked after the addition.
- Iterators yield one `Err` and stop.
- A deterministic fuzz test (`tests/fuzz.rs`) mutates every test file with a
  seeded PRNG (bit flips, byte overwrites, interesting values, splices,
  truncations; 300 thousand inputs, a few seconds) and runs every parser and
  iterator to the end on each. Hand-made corrupt files in `tests/corrupt.rs`
  check the exact error for each kind of damage.

### Test files and provenance

The test files in `crates/poly-macho/tests/data/` are built here, by
`generate.sh` from C sources written for this purpose, with LLVM's `clang`
and `ld64.lld` 18.1.3 as black boxes, for `arm64-apple-macos11` and
`x86_64-apple-macos10.15`, freestanding and without an Apple SDK. They link
against a two-line stand-in for the system library written here. No Apple
binary, and no file from a Mac, is used as input. The expected values in the
tests were recorded from `llvm-objdump --macho`, `llvm-nm` and `llvm-lipo` run
on those files. The README there says how, with versions. The arm64e pointer
formats, which `ld64.lld` 18 does not produce, are tested on images the tests
build from the bit layouts of `<mach-o/fixup-chains.h>`.

### Sources

Constants and layouts come from Apple's public SDK headers
`<mach-o/loader.h>`, `<mach-o/fat.h>`, `<mach-o/nlist.h>`,
`<mach-o/fixup-chains.h>` and `<mach/machine.h>`, plus `<mach/vm_prot.h>`
and the thread state headers `<mach/i386/thread_status.h>` and
`<mach/arm/thread_status.h>` (for the three protection bits and the
register state flavors and offsets of the entry point). Each constant group
names its header. How the opcodes advance (what `ADD_ADDR_ULEB` adds, how
a lazy stream is divided into entries) is described in few words by the
headers; it was settled by comparing our decoders with `llvm-objdump` on
our own files. No source of `dyld`, `ld64`, `cctools`, XNU, Darling, LLVM's
object file code, `lld` or any other Mach-O implementation was read.

## Not in this change

What is left for our own `dyld` and its neighbours:

- **Mapping.** Picking addresses (ASLR, `MH_PIE`), mapping segments with
  their protections, zero fill, `__PAGEZERO`, the 16 KiB pages of arm64,
  `SG_READ_ONLY` after fixups, the entry point's stack and `apple[]` strings.
- **Applying fixups.** Rebasing by the slide and binding to symbols; chained
  fixups' `high8` and arm64e signing (which the translator treats as no-ops
  to begin with); threaded binds (`BIND_OPCODE_THREADED`, refused here) and
  the pointer formats of 32-bit, kernel cache, firmware and shared cache
  images (refused as unsupported formats).
- **Symbol resolution.** Searching dependencies in ordinal order, `@rpath`,
  `@executable_path` and `@loader_path`, re-exports, weak coalescing, lazy
  binding through the binder, flat lookup, interposing.
- **The dyld shared cache.** A cache holds system libraries, split into
  pieces that this reader does not know. How Poly provides `libSystem` and the
  frameworks (rebuilt from specs, or a cache format of our own) is for the
  Mach core and `libSystem` milestones.
- Initialisers, thread-local variables (the descriptors are readable, the
  setup is not), `__DATA,__interpose`, Objective-C and Swift metadata,
  code signature checking, `LC_NOTE`, filesets and 32-bit files.

## Consequences

- The translator, our `dyld` and an aarch64 port read Mach-O files with the
  same code, host-tested on Linux, macOS and Windows hosts.
- The kernel can link the crate as it is: no allocator, no `std`.
- Fixture generation needs Linux (clang, `ld64.lld`, `llvm-lipo`); the tests
  themselves do not.
- Some real files are refused that a lenient reader would take: commands
  that do not fill `sizeofcmds` exactly, segments that overlap, and more
  than 32 segments or 64 universal slices. Linkers do not produce these.
  If a real file needs one of the checks relaxed, the limit is a constant.
- Phase 9 stays open: "Mach-O loading and our own `dyld`" is not done by a
  reader alone.

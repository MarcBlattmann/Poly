# 0024: ELF loader: a host-tested crate that plans mappings and builds the initial stack

**Status:** Accepted

## Context

Phase 3 runs a static Linux "hello world"; Phase 4 runs glibc's own `ld.so`
unchanged. Both need the kernel to validate an ELF executable, map its
`PT_LOAD` segments, place the program break, and hand the program `argc`,
`argv`, `envp` and an auxiliary vector exactly where the System V AMD64
psABI (§3.4.1) says. None of that touches hardware, and a mistake in it is a
kernel bug reachable from any file a user can exec, so it must be
exhaustively host-tested and must never panic on hostile input.

Inputs allowed by `CLEAN_ROOM.md`: the System V gABI, the AMD64 psABI, the
public `<elf.h>` for constants, and the getauxval(3) / execve(2) man pages.

## Decision

**`crates/poly-elf`** (`no_std`, no allocation, no dependencies) has three
steps, each a plain value the kernel can inspect:

1. `Elf::parse(&[u8])` validates the header (ELF64, little-endian,
   `EV_CURRENT`, OS ABI System V or GNU, x86-64, `ET_EXEC` or `ET_DYN`,
   `e_phentsize == 56`, no `PN_XNUM`) and the program headers: every `PT_LOAD`
   and `PT_INTERP` lies in the file, nothing wraps, `PT_LOAD`s are sorted by
   address (the gABI requires it) and `p_vaddr ≡ p_offset` modulo the page
   and `p_align`. `PT_INTERP`, `PT_PHDR`, `PT_TLS`, `PT_DYNAMIC` and
   `PT_GNU_STACK` may occur once. The entry point, `PT_PHDR` and the
   initialised part of `PT_TLS` must lie in loaded segments.
2. `LoadPlan::new(&elf, &LoadOptions)` produces up to 32 page-aligned
   `Mapping`s (two per segment, at most 16 segments): file-backed pages,
   then anonymous pages for the rest of `.bss`. It also gives the load bias,
   entry point, `AT_PHDR`, initial break, executable-stack flag and
   warnings. The kernel either copies `Mapping::file_bytes` or fills pages on
   demand with `Mapping::fill_page`.
3. `build_initial_stack(buf, stack_top, &StackParams)` writes the vectors
   and the information block into a buffer standing for the top of the
   stack and returns the 16-byte aligned `rsp`. `InitialStack` reads such a
   stack back (tests, later `/proc/<pid>/auxv`).

Policies:

- **Segments may not share a page.** A page has one protection, so two
  segments in one page could only be merged by widening one of them. Every
  linker we know of keeps segments on separate pages; such files are refused
  (`SegmentsOverlap`) instead of being made more permissive.
- **The zero tail is always zeroed.** Bytes of the last file page past
  `p_filesz` read as zero even when `p_memsz == p_filesz`. The ABI leaves
  them unspecified; zeros leak no file contents past the segment.
- **`ET_DYN` base 0x5555_5555_4000,** raised to the image's largest
  `p_align`, which is where Linux puts PIEs with randomisation off. Traces
  from both systems then show the same addresses. Randomisation comes later,
  as a different `LoadOptions::dyn_base`.
- **User space is `[0x1_0000, 0x7fff_ffff_f000)`.** The lower bound matches
  the usual `vm.mmap_min_addr`, so NULL plus a small offset faults. The
  upper bound leaves the last canonical page unmapped, so `sysret` never
  returns to a non-canonical address. Both are `LoadOptions` fields.
- **W^X is reported, not enforced.** RWX segments, `PT_GNU_STACK` with
  `PF_X`, a missing `PT_GNU_STACK` and an entry point in a non-executable
  segment are `Warnings`, so old binaries still run. Without
  `PT_GNU_STACK` the stack is non-executable.
- **`AT_PHDR`** is `PT_PHDR`'s address if present. Otherwise it comes from
  the `PT_LOAD` whose file bytes contain the table, as in static binaries
  that start their first segment at offset 0. If no segment maps the table,
  `build_initial_stack` copies it onto the stack and points `AT_PHDR` there.
- **Auxiliary vector:** `AT_SYSINFO_EHDR` (once a vDSO exists), `AT_HWCAP`,
  `AT_PAGESZ`, `AT_CLKTCK` (100), `AT_PHDR`, `AT_PHENT`, `AT_PHNUM`,
  `AT_BASE`, `AT_FLAGS`, `AT_ENTRY`, `AT_UID`, `AT_EUID`, `AT_GID`,
  `AT_EGID`, `AT_SECURE`, `AT_RANDOM`, `AT_HWCAP2` (optional), `AT_EXECFN`,
  `AT_PLATFORM` ("x86_64"), caller extras, `AT_NULL`. The caller supplies
  the 16 random bytes and the CPUID-derived `hwcap`.

**Dynamic binaries (Phase 4).** `Elf::interp()` gives the interpreter path.
The kernel parses the interpreter (an `ET_DYN` image) and plans it with its
own `dyn_base`, taken from free space in the new address space. It then
starts at `start_address(&exe, Some(&interp))` and builds the auxiliary
vector with `AuxInfo::new(&exe, Some(&interp), creds)`, which sets
`AT_BASE` to the interpreter's bias and `AT_ENTRY` to the program's entry.
Until then the kernel refuses images with `PT_INTERP`.

## Consequences

- The kernel's exec path is thin: look up the file, `Elf::parse`,
  `LoadPlan::new`, create one VMA per mapping, `build_initial_stack` into
  the new stack, enter ring 3 at the entry point with the returned `rsp`.
- `LoadPlan` is about 1.6 KiB, a fixed-size value; the kernel should keep it
  off small stacks (box it or hold it in the process).
- Images with more than 16 `PT_LOAD` segments, `PN_XNUM` program header
  counts or segments sharing a page are refused. None are produced by the
  toolchains we target.
- Host tests compile real static, static-PIE and dynamic programs with the
  host C compiler, when present, and compare the result with `readelf -lW`.
  They also check that every planned page holds exactly the segment's bytes
  and zeros, and they mutate headers at random, so the parser and planner
  have to hold their invariants on damaged input.

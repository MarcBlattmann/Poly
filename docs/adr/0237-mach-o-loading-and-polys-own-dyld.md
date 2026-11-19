# 0237: Mach-O loading and Poly's own dyld

**Status:** Accepted. Track `impl-macos-dyld`, Roadmap Phase 9 "Mach-O loading and our own
`dyld`". Builds on ADR 0151 (the macOS plan), ADR 0153 (the `poly-macho` reader) and ADR 0157
(Chrome's import map: x86-64 first, chained fixups plus classic binds). It takes the place of the
planned ADRs 0158 ("a static x86_64 Mach-O runs") and 0160 ("our dyld v0"), which were never
written. Inputs: the dyld(1) manual page (search paths), Apple's public Mach-O headers through
`poly-macho`, the spec sheets `specs/macos/bsd/write.yaml` and `exit.yaml`, and test programs
of our own built with clang and lld as black boxes. No source of Apple's dyld, XNU, ld64 or any
other loader was read; no Apple binary was disassembled.

## What was there

The XNU personality answered every call with `ENOSYS`; the kernel refused Mach-O files
("xnu programs cannot run yet"). `poly-macho` read every structure a loader needs but placed and
fixed up nothing.

## Decisions

1. **The loader is a host-tested crate, `crates/poly-dyld`.** `link` reads a program and every
   library it needs through a `FileSystem` trait, places the images, and works out entry point,
   initializers and thread-local variables; `Program::fixups` yields every pointer to write. The
   crate maps nothing, so the kernel, a test's simulated memory, or later a user-space dyld can
   drive it. It reads arm64 files the same way as x86-64 ones (with 16 KiB pages); only the start
   code is per CPU.
2. **The work happens at exec time, in the kernel (`kernel/src/xnu.rs`), plus a page of start
   code in user space.** The kernel maps every segment, writes every fixup, sets the final
   protections (`SG_READ_ONLY` segments become read-only after fixups), maps one thread-local
   block per image for the main thread and maps the start code with its data block. The start
   code (`crates/poly-dyld/stub/start_x86_64.s`, assembled once and embedded) calls the
   initializers with `(argc, argv, envp, apple, 0)`, then `main(argc, argv, envp, apple)`, then
   libSystem's `_exit` with the result (the raw exit call if there is no libSystem). A real
   user-space dyld (for `dlopen` and lazy binding) can be built on the same crate later.
3. **The contract programs observe:** the stack has the program's header address below `argc`
   when dyld runs (as the kernel hands it to dyld), then `argc`, `argv`, `envp`, `apple`;
   `apple[0]` is `executable_path=<path>`; `__mh_execute_header` is the mapped header; dependencies
   are initialized before dependents, the program last. A static program (`LC_UNIXTHREAD` without
   `LC_LOAD_DYLINKER`) starts at its own entry point with the stack pointer at `argc`. An
   `LC_UNIXTHREAD` program that asks for dyld gets its initializers run, then a jump to its entry.
4. **Placement:** the program at its preferred address (a PIE one can be moved by an option, which
   tests use); libraries from 32 GiB on, one after another with a guard page. No ASLR yet.
5. **Library search:** `@executable_path`, `@loader_path`, `@rpath` (the run paths of the image
   that names the library, then of the images that loaded it, up to the program), plain paths;
   a library already loaded under the same install name or path is reused. A missing
   `LC_LOAD_WEAK_DYLIB` is no error; a missing library is `Library not loaded: <name>, referenced
   from <path>`.
6. **Symbols:** export tries with re-exports (trie entries and `LC_REEXPORT_DYLIB`), absolute and
   thread-local exports, stub-and-resolver exports bound to their stub, the two-level namespace,
   flat lookups (`-2`, and every bind of a flat-namespace image), weak lookups and classic weak
   binds coalesced to the first strong definition in load order. A missing weak import is 0; a
   missing symbol is `Symbol not found: <sym>, referenced from <path>, expected in <lib>`.
7. **Fixups:** chained fixups (`DYLD_CHAINED_PTR_64`, `64_OFFSET` and the arm64e formats, whose
   signatures are not applied: the translator treats authentication as a no-op), and the classic
   rebase, bind, lazy bind and weak bind opcodes. Lazy binds are bound at load, so
   `dyld_stub_binder` is never called (and may be missing). 32-bit text relocations are refused.
8. **Thread-local variables:** each descriptor's thunk becomes Poly's (in the start code page:
   `rax = key + offset`, no other register changed) and its key the address of the image's block,
   a copy of its `__thread_data` / `__thread_bss` template. The thunk words are not bound, so TLV
   works without a libSystem.
9. **First BSD calls:** `write` and `exit` (the spec sheets: carry flag and Darwin errno),
   decoded in `personalities/xnu/src/bsd.rs` behind a `BsdKernel` trait; everything else stays
   `ENOSYS`. Only processes the kernel started as XNU processes get them.
10. **Only x86-64 programs run.** An arm64 Mach-O is refused with a reason until the arm64
    translator lands (ADR 0151); the crate already links arm64 programs in host tests.

## Tests

- `crates/poly-dyld/tests/fixtures.rs`: the committed test programs (`tests/data`, rebuilt by
  `generate.sh` on a Mac from our C sources; they link a stand-in libSystem of ours) linked into a
  simulated memory: chained x86-64 and arm64, classic x86-64, moved and unmoved, static; every
  bound and rebased pointer, TLV descriptors, initializer order, `_exit`, errors, and a mutation
  sweep that must never panic. Unit tests for the search paths, the stack and the start data.
- Boot test group `macho` (shard `userland`): `/bin/mstatic`, `/bin/mhello` (chained) and
  `/bin/mhello-classic` (classic opcodes) run as the first process on Poly and print every check.

## Not in this change

Threads (each needs its own TLV blocks: with `bsdthread_*`), `dlopen` and `dlsym`, the dyld API
(`_dyld_*`, `_NSGetExecutablePath`), file-backed segment mappings (files are read whole, up to
64 MiB), ASLR, code signatures, the shared cache, `__interpose`, `DYLD_*` variables, and a real
libSystem (ADR 0157's next step).

# 0007: In-kernel tests in a linker section; frame-pointer backtraces symbolised on the host

**Status:** Accepted.

## Context

Some things can only be tested on the real machine: CPU exceptions, page
tables, interrupt controllers, devices. The boot self-test was a hand-written
array in `selftest.rs`, so every new test meant editing one central file, and
it reported a single PASS/FAIL. When something crashed, the panic message
gave a `rip` and nothing else.

The kernel has no heap yet and uses no third-party crates, so Rust's
`custom_test_frameworks` (unstable) and crates like `linkme` are out.

## Decision

**Tests.** `kernel_test!` wraps a function and emits a static
`KernelTest { path, run }` record into its own input section
`.kernel_tests.<module path>::<name>`. `kernel/linker.ld` collects them with
`KEEP(*(SORT_BY_NAME(.kernel_tests.*)))` between `__kernel_tests_start` and
`__kernel_tests_end`, so tests live next to their code, need no registration
list and no heap, and always run in the same (alphabetical) order. The runner
prints one `test <name> ... ok|FAILED: <reason>` line per test and a
`test result:` summary, filtered by `poly.test=<substring>`. `cargo xtask
test` parses these lines and fails on any failure or a missing summary.

**Backtraces.** The kernel already builds with frame pointers. The panic
handler walks the `rbp` chain with `poly-backtrace`, a host-tested crate that
only reads frame records lying inside a known kernel stack, requires each
frame pointer to be aligned and to move up the stack, and stops after 32
frames, so a corrupt stack cannot fault the panic path or loop. A fatal CPU
exception records its trap frame's `rip`/`rbp` so the backtrace starts at the
faulting instruction. The kernel prints raw addresses only; `cargo xtask`
reads `.symtab` from the kernel ELF (a small ELF64 reader) and demangles with
`rustc-demangle`.

## Consequences

- No symbol table in the kernel image and no demangler in ring 0; a raw log
  must be symbolised with the ELF it came from (`cargo xtask symbolize`).
- Every kernel stack must be listed for the walker. Kernel threads will add
  their stacks when they arrive.
- Unwinding is not supported: a panic is still fatal, and a failing test
  reports an `Err`, not a panic. A test that panics ends the run; the missing
  summary line and the backtrace identify it.
- Frames from precompiled `core` code built without frame pointers may be
  skipped; frames are never invented.

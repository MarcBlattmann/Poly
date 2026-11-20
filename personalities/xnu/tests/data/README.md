# x86-64 builds of Mach core spec tests

The boot test group `macho-mach` (`tools/xtask/src/macho.rs`, ADR 0238)
runs these programs on Poly, each as the first process, and compares their
output with what macOS printed for the same test (the arm64 recording in the
test's `macos.json`).

- `tests.txt`: the visible spec tests built here, the ones that run on Poly
  today (no fork, mmap or sysctl yet).
- `generate.sh`: rebuilds the programs on a Mac with Apple's clang and Rust's
  rust-lld, used as black boxes. They link the stand-in libSystem of
  `crates/poly-dyld/tests/data` (Poly's own, built from our sources) and
  import nothing from it; no Apple library is linked or committed.

The sources are the visible tests under `specs/macos/` and the test header
`tests/macos/include/poly_mtest.h`.

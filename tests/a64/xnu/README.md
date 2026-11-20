# arm64 test programs of the translated XNU process

Built by `build.sh` on a Mac (Apple clang, ld and lipo for the static program,
Rust's `rust-lld` in its `ld64.lld` flavour for the dynamic ones, all as black
boxes) from the C sources here, written for Poly (ADR 0257). The dynamic
programs link `libsystem.c`, a stand-in for libSystem of ours, never Apple's.

| File | What |
|---|---|
| `bin/hello-static` | (a) static (`LC_UNIXTHREAD`): `write`, `exit`, the carry flag and `EBADF` |
| `bin/crash` | (d) static: reads address 8, ends with SIGSEGV |
| `bin/hello-fat` | the same as a universal file with only its arm64 slice |
| `bin/libSystem.B.dylib` | the stand-in: `exit`, `poly_write`, `malloc` on `mmap`, string functions |
| `bin/hello` | (b) dynamically linked: initializers, thread-local variables, `malloc` |
| `bin/heap` | (c) malloc-heavy: `heap <rounds>` |
| `bin/heap-2.out`, `bin/heap-6.out` | what `heap.c` prints, built for the Mac with its own C library |

`crates/poly-a64-xnu`'s tests run them under the interpreter (and the JIT on
x86-64 hosts); the boot test group `macho-a64` (`tools/xtask/src/macho_a64.rs`)
runs them on Poly.

# poly-dyld test programs

Built by `generate.sh` on a Mac (Apple clang 21.0.0 and `ld` for the static
programs, Rust's `rust-lld` in its `ld64.lld` flavour for the dynamic ones,
all as black boxes) from the C sources in `src/`, written for Poly. They link
against `src/system.c`, a stand-in for libSystem of ours, never Apple's.

| File | What |
|---|---|
| `hello_x64`, `hello_a64` | program: chained fixups, `__init_offsets`, macOS 13 |
| `libgreet_x64.dylib`, `libgreet_a64.dylib` | `@rpath/libgreet.dylib`, chained |
| `libSystem_x64.dylib`, `libSystem_a64.dylib` | the stand-in libSystem, chained |
| `helloc_x64`, `libgreetc_x64.dylib`, `libSystemc_x64.dylib` | the same with classic rebase/bind opcodes and `__mod_init_func`, macOS 10.15 |
| `static_x64`, `static_a64` | `LC_UNIXTHREAD`, no dyld |

The expected offsets in `tests/fixtures.rs` were recorded with
`llvm-objdump --macho --dyld-info --exports-trie` and `otool -lv`. The boot
test group `macho` (`tools/xtask/src/macho.rs`) runs the x86-64 ones on Poly,
`macho-a64` (`tools/xtask/src/macho_a64.rs`, ADR 0257) runs `hello_a64` with
`libgreet_a64.dylib` through the arm64 translator.

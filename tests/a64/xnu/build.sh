#!/usr/bin/env bash
# Builds the arm64 test programs of the translated XNU process (ADR 0257)
# into bin/ from the C sources here (written for Poly). Dev-time only: the
# host tests (crates/poly-a64-xnu) and the boot test group `macho-a64` read
# the committed files, so neither needs a compiler.
#
# Needs a Mac with Apple's command line tools (clang, ld and lipo, used as
# black boxes) and Rust's rust-lld (`ld64.lld` flavour). No Apple library is
# linked: the dynamic programs link libsystem.c, our stand-in for
# /usr/lib/libSystem.B.dylib.
#
#   bash tests/a64/xnu/build.sh
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
out=$here/bin
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cd "$tmp"

host=$(rustc -vV | sed -n 's/^host: //p')
lld="$(rustc --print sysroot)/lib/rustlib/$host/bin/rust-lld"

CFLAGS="-arch arm64 -mmacosx-version-min=13.0 -ffreestanding -nostdlib -fno-stack-protector -fno-stack-check
    -fno-ident -O2 -fno-asynchronous-unwind-tables -fno-unwind-tables"
link() { # rust-lld as ld64: chained fixups and __init_offsets, macOS 13
    "$lld" -flavor darwin -arch arm64 -platform_version macos 13.0 13.0 -fixup_chains -init_offsets "$@"
}

# (a) static: LC_UNIXTHREAD, linked by Apple's ld (rust-lld has no -static),
# and the same program as a universal file with only its arm64 slice.
clang $CFLAGS -c "$here/hello_static.c" -o hello_static.o
clang -arch arm64 -static -nostdlib -e start -Wl,-no_uuid -o "$out/hello-static" hello_static.o
lipo -create "$out/hello-static" -output "$out/hello-fat"
# (d) static, ends with a fault on purpose.
clang $CFLAGS -c "$here/crash.c" -o crash.o
clang -arch arm64 -static -nostdlib -e start -Wl,-no_uuid -o "$out/crash" crash.o

# The libSystem stand-in, then (b) and (c) against it.
clang $CFLAGS -c "$here/libsystem.c" -o libsystem.o
link -dylib -install_name /usr/lib/libSystem.B.dylib -o "$out/libSystem.B.dylib" libsystem.o
for prog in hello heap; do
    clang $CFLAGS -c "$here/$prog.c" -o "$prog.o"
    link -o "$out/$prog" "$prog.o" "$out/libSystem.B.dylib"
done

# The expected output of (c): the same source built for this Mac with its
# own C library (only `poly_write` is ours). heap.c prints nothing that
# depends on addresses or on the malloc behind it.
cat > shim.c <<'C'
#include <unistd.h>
long poly_write(int fd, const char *buf, long len) { return write(fd, buf, (size_t)len); }
C
clang -O2 -o heap-host "$here/heap.c" shim.c
for rounds in 2 6; do ./heap-host "$rounds" > "$out/heap-$rounds.out"; done
ls -l "$out"

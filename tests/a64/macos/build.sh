#!/bin/sh
# Builds the arm64 macOS test programs (ADR 0239) into bin/. Needs an Apple
# toolchain (clang and ld); the binaries are committed, so the tests that run
# them (poly-a64-jit, on any host) never need this script.
#
# Each program is built at -O2 and at -O3 (the vectoriser), linked against
# libSystem only because dyld insists on it; no libSystem function is called.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
cd "$here"
mkdir -p bin
CFLAGS="-arch arm64 -mmacosx-version-min=12.0 -ffreestanding -fno-builtin -fno-stack-protector -fno-stack-check -fno-asynchronous-unwind-tables -fno-unwind-tables -fno-ident"
clang $CFLAGS -O2 -c rt.c -o bin/rt.o
for src in *.c; do
    name=${src%.c}
    [ "$name" = rt ] && continue
    for opt in O2 O3; do
        clang $CFLAGS -$opt -fbuiltin -c "$src" -o "bin/$name.o"
        clang -arch arm64 -mmacosx-version-min=12.0 -nostartfiles -Wl,-dead_strip -o "bin/$name-$opt" "bin/$name.o" bin/rt.o
        strip -x "bin/$name-$opt" 2>/dev/null || true
    done
    rm -f "bin/$name.o"
done
rm -f bin/rt.o
# The expected output of each, from the real Mac.
for b in bin/*-O2 bin/*-O3; do
    "./$b" > "$b.out"; echo "exit $?" >> "$b.out"
done

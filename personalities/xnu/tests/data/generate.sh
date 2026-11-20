#!/usr/bin/env bash
# Builds the x86-64 programs of the boot test group `macho-mach`
# (tools/xtask/src/macho.rs, ADR 0238) from the visible spec tests that run
# on Poly today: they need no fork, mmap or sysctl yet. Dev-time only: the
# boot test reads the committed files.
#
# Needs a Mac with Apple's clang (a black box) and Rust's rust-lld. No Apple
# library is linked: the programs link Poly's stand-in libSystem of
# crates/poly-dyld/tests/data, and import nothing from it (the test header
# makes the raw calls itself).
#
#   bash personalities/xnu/tests/data/generate.sh
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

host=$(rustc -vV | sed -n 's/^host: //p')
lld="$(rustc --print sysroot)/lib/rustlib/$host/bin/rust-lld"
CFLAGS="-ffreestanding -nostdlib -fno-stack-protector -fno-builtin -fno-ident -O1 -fno-asynchronous-unwind-tables -fno-unwind-tables"
libsystem="$repo/crates/poly-dyld/tests/data/libSystem_x64.dylib"

while read -r src; do
    [ -z "$src" ] && continue
    name=$(basename "$src" .c)
    clang -arch x86_64 -mmacosx-version-min=13.0 $CFLAGS -I"$repo/tests/macos/include" -c "$repo/$src" -o "$tmp/$name.o"
    "$lld" -flavor darwin -arch x86_64 -platform_version macos 13.0 13.0 -fixup_chains \
        -o "$here/$name" "$tmp/$name.o" "$libsystem"
done < "$here/tests.txt"
ls -l "$here"

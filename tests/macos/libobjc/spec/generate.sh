#!/usr/bin/env bash
# Builds the x86-64 programs of the boot test group `objc-spec`
# (tools/xtask/src/libobjc.rs, ADR 0259): every visible test of
# specs/macos/libobjc.A.dylib, compiled as `cargo xtask pipeline mac build`
# compiles them (Objective-C, ARC where the test says so, linked the ordinary
# way against the SDK's libobjc and libSystem), and the runner that starts
# them on Poly (runner.c, raw system calls, linked by rust-lld against the
# stand-in libSystem of crates/poly-dyld). Dev-time only: the boot test reads
# the committed files.
#
# Needs a Mac with Apple's clang and ld (black boxes) and Rust's rust-lld.
# No Apple library or code ends up in the files.
#
#   bash tests/macos/libobjc/spec/generate.sh
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
host=$(rustc -vV | sed -n 's/^host: //p')
lld="$(rustc --print sysroot)/lib/rustlib/$host/bin/rust-lld"

rm -rf "$here/bin"
mkdir -p "$here/bin"
: > "$here/tests.txt"
for src in "$repo"/specs/macos/libobjc.A.dylib/*.tests/*.c; do
    name=$(basename "$src" .c)
    arc=()
    grep -q "poly-mac: arc" "$src" && arc=(-fobjc-arc)
    clang -arch x86_64 -march=x86-64 -mmacosx-version-min=13.0 -O1 -g0 -std=c11 -fno-builtin -fno-stack-protector \
        -fno-ident -Wl,-S -Wl,-dead_strip_dylibs ${arc[@]+"${arc[@]}"} -I"$repo/tests/macos/include" \
        -x objective-c "$src" -lobjc -o "$here/bin/$name"
    echo "$name" >> "$here/tests.txt"
done

clang -arch x86_64 -mmacosx-version-min=13.0 -ffreestanding -nostdlib -fno-stack-protector -fno-builtin -fno-ident -O1 \
    -fno-asynchronous-unwind-tables -c "$here/runner.c" -o "$tmp/runner.o"
"$lld" -flavor darwin -arch x86_64 -platform_version macos 13.0 13.0 -fixup_chains -e start \
    -o "$here/runner" "$tmp/runner.o" "$repo/crates/poly-dyld/tests/data/libSystem_x64.dylib"
echo "$(wc -l < "$here/tests.txt") tests"

#!/usr/bin/env bash
# Regenerates the poly-objc test images from src/ (written for Poly). Dev-time
# only: the tests read the committed files.
#
# Needs a Mac (clang and Apple's ld as black boxes, the SDK's objc headers
# for NSObject's interface) and Rust's rust-lld. classes_ld64.dylib is
# linked by Apple's ld (relative method lists in __TEXT,__objc_methlist),
# classes_lld by rust-lld (plain method lists); both against our
# NSObject stand-in (src/nsobject.s), installed as /usr/lib/libobjc.A.dylib.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
src=$here/src
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
host=$(rustc -vV | sed -n 's/^host: //p')
lld="$(rustc --print sysroot)/lib/rustlib/$host/bin/rust-lld"
L="$lld -flavor darwin -arch x86_64 -platform_version macos 13.0 13.0"

clang -arch x86_64 -mmacosx-version-min=13.0 -c "$src/nsobject.s" -o "$tmp/nsobject.o"
$L -dylib -install_name /usr/lib/libobjc.A.dylib -o "$here/libobjc_stub.dylib" "$tmp/nsobject.o"
clang -arch x86_64 -mmacosx-version-min=13.0 -fno-objc-arc -O1 -fno-ident -c "$src/classes.m" -o "$tmp/classes.o"
clang -arch x86_64 -mmacosx-version-min=13.0 -O1 -fno-ident -c "$src/main.c" -o "$tmp/main.o"
# libSystem: the stand-in of crates/poly-dyld (our own source).
sys="$here/../../../poly-dyld/tests/data/libSystem_x64.dylib"
$L -fixup_chains -o "$here/classes_lld" "$tmp/classes.o" "$tmp/main.o" "$here/libobjc_stub.dylib" "$sys"
clang -arch x86_64 -mmacosx-version-min=13.0 -nostdlib -o "$here/classes_ld64" \
    "$tmp/classes.o" "$tmp/main.o" "$here/libobjc_stub.dylib" "$sys" -Wl,-e,_main
rm -f "$here"/classes_*.dylib
ls -l "$here"/classes_* "$here"/libobjc_stub.dylib

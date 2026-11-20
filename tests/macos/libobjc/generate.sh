#!/usr/bin/env bash
# Rebuilds hello_x64 from hello.m (written for Poly). Dev-time only: the boot
# test (`cargo xtask boot-tests --only objc`) reads the committed file.
#
# Needs a Mac with Xcode or the command line tools: clang and Apple's ld
# (black boxes) compile it with ARC and link it the ordinary way against the
# SDK's libobjc.tbd and libSystem.tbd, so it imports their names as any
# macOS program does. No Apple library or code ends up in the file.
#
#   bash tests/macos/libobjc/generate.sh
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
clang -arch x86_64 -march=x86-64 -mmacosx-version-min=13.0 -O1 -fobjc-arc -fno-builtin -fno-ident -fno-objc-exceptions \
    -Wall -Wno-format -o "$here/hello_x64" "$here/hello.m"
ls -l "$here/hello_x64"

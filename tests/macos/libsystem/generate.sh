#!/usr/bin/env bash
# Rebuilds hello_x64 from hello.c (written for Poly). Dev-time only: the
# boot test (`cargo xtask boot-tests --only libsystem`) reads the committed
# file, so no other host needs a Mac.
#
# Needs a Mac with Xcode or the command line tools: clang and Apple's ld
# (black boxes) link it the ordinary way against the SDK's libSystem.tbd,
# so it imports libSystem's names as any macOS program does. No Apple
# library or code ends up in the file.
#
#   bash tests/macos/libsystem/generate.sh
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
clang -arch x86_64 -march=x86-64 -mmacosx-version-min=13.0 -O1 -fno-builtin -fstack-protector-strong -fno-ident \
    -Wall -Wno-format -o "$here/hello_x64" "$here/hello.c"
ls -l "$here/hello_x64"

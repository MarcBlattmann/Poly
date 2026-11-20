#!/usr/bin/env bash
# Builds the x86-64 programs of the boot test group `corefoundation`
# (tools/xtask/src/corefoundation.rs, ADR 0260) into bin/: the visible
# CoreFoundation spec tests listed in tests.txt, cfdemo.c and runner.c.
# Dev-time only: the boot test reads the committed files, so no other host
# needs a Mac.
#
# Needs a Mac with Xcode or the command line tools: clang and Apple's ld
# (black boxes) link them the ordinary way against the SDK's
# CoreFoundation.tbd and libSystem.tbd, so they import those names as any
# macOS program does. No Apple library or code ends up in the files.
#
#   bash tests/macos/corefoundation/generate.sh
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../.." && pwd)
# The flags of the spec tests' own builds (tools/pipeline/src/mac.rs), for
# the baseline x86-64 CPU and the oldest macOS Chrome asks for.
CFLAGS="-arch x86_64 -march=x86-64 -mmacosx-version-min=13.0 -O1 -g0 -std=c11 -Wall -Wextra -Werror -fno-builtin -fno-stack-protector -fno-ident -Wl,-S"
mkdir -p "$here/bin"
rm -f "$here"/bin/*
while read -r src _; do
    case "$src" in '' | '#'*) continue ;; esac
    name=$(basename "$src" .c)
    clang $CFLAGS -I"$repo/tests/macos/include" -framework CoreFoundation -o "$here/bin/$name" "$repo/$src"
done < "$here/tests.txt"
clang $CFLAGS -framework CoreFoundation -o "$here/bin/cfdemo" "$here/cfdemo.c"
clang $CFLAGS -o "$here/bin/cf-runner" "$here/runner.c"
ls -l "$here/bin"

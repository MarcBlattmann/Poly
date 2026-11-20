#!/usr/bin/env bash
# Builds the exception test programs of the boot test group `macho-mach`
# (ADR 0238) for x86-64: our C sources, plus MIG stubs that Apple's `mig`
# (a black box, like the compiler) generates from the public SDK's
# exc.defs, mach_exc.defs, task.defs and thread_act.defs. Only the built
# programs are committed. Links Poly's stand-in libSystem, nothing of Apple.
#
#   bash personalities/xnu/tests/data/exc/build.sh <output directory>
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../../.." && pwd)
out=$(cd "$1" && pwd)
sdk=$(xcrun --show-sdk-path)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cd "$tmp"
for d in exc mach_exc; do
    mig -arch x86_64 -isysroot "$sdk" -user /dev/null -header $d.h -server ${d}Server.c -sheader ${d}Server.h "$sdk/usr/include/mach/$d.defs"
done
for d in task thread_act; do
    mig -arch x86_64 -isysroot "$sdk" -user ${d}User.c -header $d.h -server /dev/null "$sdk/usr/include/mach/$d.defs"
done
host=$(rustc -vV | sed -n 's/^host: //p')
lld="$(rustc --print sysroot)/lib/rustlib/$host/bin/rust-lld"
CFLAGS="-arch x86_64 -mmacosx-version-min=13.0 -ffreestanding -fno-builtin -fno-stack-protector -D_FORTIFY_SOURCE=0 -fno-ident -O1 -fno-asynchronous-unwind-tables -fno-unwind-tables -I. -I$here"
for f in excServer mach_excServer taskUser thread_actUser "$here/support"; do
    clang $CFLAGS -c "$f.c" -o "$(basename "$f").o"
done
clang -arch x86_64 -mmacosx-version-min=13.0 -c "$here/trap.s" -o trap.o
common="support.o trap.o taskUser.o thread_actUser.o"
for t in exc_state:mach_excServer exc_default:excServer; do
    name=${t%%:*}; server=${t##*:}
    clang $CFLAGS -c "$here/$name.c" -o "$name.o"
    "$lld" -flavor darwin -arch x86_64 -platform_version macos 13.0 13.0 -fixup_chains -dead_strip \
        -o "$out/$name" "$name.o" $server.o $common "$repo/crates/poly-dyld/tests/data/libSystem_x64.dylib"
done

#!/usr/bin/env bash
# Regenerates the Mach-O test files in this directory. Dev-time only: the
# tests read the committed files and never need these tools.
#
# Needs LLVM's clang, ld64.lld and llvm-lipo (18.1.x), and no Apple SDK:
# the C sources are freestanding and the linker is told the platform itself.
#
#   bash crates/poly-macho/tests/data/generate.sh   (e.g. on Ubuntu 24.04 with clang, lld, llvm)
#
# (On Ubuntu the tools live in /usr/lib/llvm-18/bin.)
set -euo pipefail
export PATH=/usr/lib/llvm-18/bin:$PATH
here=$(cd "$(dirname "$0")" && pwd)
src=$here/src
out=$here
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cd "$tmp"

CFLAGS="-ffreestanding -nostdlib -fno-stack-protector -fno-ident -Os -fno-asynchronous-unwind-tables"
# macOS 11.0 is the first release for arm64; 10.15 is a x86-64 release that
# predates chained fixups.
A64="-target arm64-apple-macos11"
X64="-target x86_64-apple-macos10.15"
LD_A64="-arch arm64 -platform_version macos 11.0 11.0"
LD_X64="-arch x86_64 -platform_version macos 10.15 10.15"

cc() { clang "$@" $CFLAGS; }

# ---- libtiny: a dylib with exports ----------------------------------------
cc $A64 -c "$src/tiny.c" -o tiny_a64.o
ld64.lld $LD_A64 -dylib -install_name @rpath/libtiny.dylib \
    -current_version 1.2.3 -compatibility_version 1.0.0 \
    -o "$out/libtiny.dylib" tiny_a64.o
cc $X64 -c "$src/tiny.c" -o tiny_x64.o
ld64.lld $LD_X64 -dylib -install_name @rpath/libtiny.dylib \
    -current_version 1.2.3 -compatibility_version 1.0.0 \
    -o libtiny_x64.dylib tiny_x64.o

# ---- libre: a dylib that re-exports libtiny and has an rpath --------------
cc $A64 -c "$src/empty.c" -o empty_a64.o
ld64.lld $LD_A64 -dylib -install_name @rpath/libre.dylib \
    -reexport_library "$out/libtiny.dylib" -rpath @loader_path/../lib \
    -o "$out/libre.dylib" empty_a64.o

# A second small library for a weak dependency (only linked against; the
# tests never open it).
cc $A64 -c "$src/weakdep.c" -o weakdep_a64.o
ld64.lld $LD_A64 -dylib -install_name @rpath/libweakdep.dylib \
    -o libweakdep.dylib weakdep_a64.o
cc $X64 -c "$src/weakdep.c" -o weakdep_x64.o
ld64.lld $LD_X64 -dylib -install_name @rpath/libweakdep.dylib \
    -o libweakdep_x64.dylib weakdep_x64.o

# A stand-in for libSystem, which classic lazy binding needs for its binder.
# Our own two-line source, not Apple's library.
mkdir sys
cc $A64 -c "$src/system.c" -o system_a64.o
ld64.lld $LD_A64 -dylib -install_name /usr/lib/libSystem.B.dylib -o sys/libSystem.dylib system_a64.o
cc $X64 -c "$src/system.c" -o system_x64.o
ld64.lld $LD_X64 -dylib -install_name /usr/lib/libSystem.B.dylib -o x64sys.dylib system_x64.o

# ---- arm64 executables -----------------------------------------------------
cc $A64 -c "$src/app.c" -o app_a64.o
LINK_APP_A64="$LD_A64 -e _main -rpath @executable_path/../lib -rpath /opt/poly/lib \
    -Lsys -lSystem -L$out -ltiny -weak_library libweakdep.dylib app_a64.o"
ld64.lld $LINK_APP_A64 -fixup_chains -o "$out/arm64_chained.exe"
ld64.lld $LINK_APP_A64 -no_fixup_chains -o "$out/arm64_classic.exe"

# ---- x86-64 executables ----------------------------------------------------
cc $X64 -c "$src/app.c" -o app_x64.o
# (-L. finds the x86-64 builds of the libraries, which are not committed.)
mkdir x64 && cp libtiny_x64.dylib x64/libtiny.dylib && cp libweakdep_x64.dylib x64/libweakdep.dylib && cp x64sys.dylib x64/libSystem.dylib
LINK_APP_X64="$LD_X64 -e _main -rpath @executable_path/../lib -Lx64 -ltiny -Lx64 -lSystem \
    -weak_library x64/libweakdep.dylib app_x64.o"
ld64.lld $LINK_APP_X64 -no_fixup_chains -o "$out/x86_64_classic.exe"
# A position dependent executable: no MH_PIE, a fixed load address.
ld64.lld $LINK_APP_X64 -no_fixup_chains -no_pie -o "$out/x86_64_nopie.exe"
# Chained fixups need macOS 11 as the minimum, even for x86-64.
cc -target x86_64-apple-macos11 -c "$src/app.c" -o app_x64_11.o
ld64.lld -arch x86_64 -platform_version macos 11.0 11.0 -e _main \
    -rpath @executable_path/../lib -Lx64 -ltiny -Lx64 -lSystem \
    -weak_library x64/libweakdep.dylib app_x64_11.o -fixup_chains -o "$out/x86_64_chained.exe"

# ---- a universal (fat) file: arm64 + x86-64, 16 KiB alignment by default ---
llvm-lipo -create "$out/arm64_chained.exe" "$out/x86_64_classic.exe" \
    -output "$out/universal.bin"

chmod 644 "$out"/*.exe "$out"/*.dylib "$out"/universal.bin
ls -l "$out"/*.exe "$out"/*.dylib "$out"/universal.bin

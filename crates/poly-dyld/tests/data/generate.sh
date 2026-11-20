#!/usr/bin/env bash
# Regenerates the Mach-O test programs in this directory from the C sources
# in src/ (written for Poly). Dev-time only: the tests and the boot test read
# the committed files.
#
# Needs a Mac with Apple's command line tools (clang and ld, used as black
# boxes) and Rust's rust-lld (`ld64.lld` flavour, for the classic-fixup
# builds). No Apple library is linked: the programs link against our own
# stand-in for libSystem (src/system.c).
#
#   bash crates/poly-dyld/tests/data/generate.sh
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
src=$here/src
out=$here
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cd "$tmp"

host=$(rustc -vV | sed -n 's/^host: //p')
lld="$(rustc --print sysroot)/lib/rustlib/$host/bin/rust-lld"

CFLAGS="-ffreestanding -nostdlib -fno-stack-protector -fno-builtin -fno-ident -O1 -fno-asynchronous-unwind-tables -fno-unwind-tables"
cc() { # arch min src obj
    clang -arch "$1" -mmacosx-version-min="$2" $CFLAGS -I"$src" -c "$3" -o "$4"
}
# rust-lld links every dynamic program: Apple's ld refuses a dylib or program
# that does not link Apple's libSystem. Chained fixups and __init_offsets at
# macOS 13 (the minimum Chrome asks for), classic rebase and bind opcodes and
# __mod_init_func at macOS 10.15 (before chained fixups). Apple's ld links
# the LC_UNIXTHREAD static program (rust-lld does not implement -static).
link() { # arch min extra-flags... -- args...
    local arch=$1 min=$2
    shift 2
    "$lld" -flavor darwin -arch "$arch" -platform_version macos "$min" "$min" "$@"
}

build() { # arch tag min suffix link-flags
    local arch=$1 tag=$2 min=$3 sfx=$4 flags=$5
    cc $arch $min "$src/system.c" system.o
    cc $arch $min "$src/greet.c" greet.o
    cc $arch $min "$src/hello.c" hello.o
    rm -rf sys && mkdir sys
    link $arch $min $flags -dylib -install_name /usr/lib/libSystem.B.dylib -o sys/libSystem.B.dylib system.o
    link $arch $min $flags -dylib -install_name "@rpath/libgreet$sfx.dylib" -o "libgreet$sfx.dylib" greet.o sys/libSystem.B.dylib
    link $arch $min $flags -o "hello${sfx}_$tag" hello.o "libgreet$sfx.dylib" sys/libSystem.B.dylib \
        -rpath @executable_path/../usr/lib/poly-dyld-test -U _poly_missing
    cp sys/libSystem.B.dylib "$out/libSystem${sfx}_$tag.dylib"
    cp "libgreet$sfx.dylib" "$out/libgreet${sfx}_$tag.dylib"
    cp "hello${sfx}_$tag" "$out/"
    if [ "$sfx$tag" = x64 ]; then
        cc $arch $min "$src/bsd.c" bsd.o
        link $arch $min $flags -o "$out/bsd_x64" bsd.o sys/libSystem.B.dylib
    fi
}

build x86_64 x64 13.0 "" "-fixup_chains -init_offsets"
build arm64 a64 13.0 "" "-fixup_chains -init_offsets"
# The classic build's library has its own install name so it can sit beside
# the chained one on a disk.
build x86_64 x64 10.15 c "-no_fixup_chains"

for arch in x86_64 arm64; do
    tag=$([ $arch = x86_64 ] && echo x64 || echo a64)
    cc $arch 13.0 "$src/static.c" static.o
    clang -arch $arch -static -nostdlib -e start -Wl,-no_uuid -o "$out/static_$tag" static.o
done

ls -l "$out"

#!/usr/bin/env bash
# Assembles start_x86_64.s with clang (black box) and prints its bytes as
# the Rust array in src/start.rs. Dev-time only; needs clang with an x86_64
# Mach-O target (any Mac, or LLVM's clang with `-target x86_64-apple-macos11`).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
clang -target x86_64-apple-macos11 -c "$here/start_x86_64.s" -o "$tmp/start.o"
otool -t "$tmp/start.o" | tail -n +3 |
    awk '{ for (i = 2; i <= NF; i++) { printf "0x%s, ", $i; if (++n % 12 == 0) printf "\n" } } END { printf "\n" }'

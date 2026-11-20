#!/usr/bin/env bash
# Assembles the start code with clang (black box) and prints its bytes as
# the Rust arrays in src/start.rs: start_x86_64.s as bytes, start_arm64.s as
# 32-bit instruction words. Dev-time only; needs clang with Mach-O targets
# (any Mac, or LLVM's clang with `-target x86_64-apple-macos11` and
# `-target arm64-apple-macos11`).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
clang -target x86_64-apple-macos11 -c "$here/start_x86_64.s" -o "$tmp/start.o"
echo "// X86_64_START"
otool -t "$tmp/start.o" | tail -n +3 |
    awk '{ for (i = 2; i <= NF; i++) { printf "0x%s, ", $i; if (++n % 12 == 0) printf "\n" } } END { printf "\n" }'
clang -target arm64-apple-macos11 -c "$here/start_arm64.s" -o "$tmp/start_arm64.o"
echo "// ARM64_START"
otool -t "$tmp/start_arm64.o" | tail -n +3 |
    awk '{ for (i = 2; i <= NF; i++) { printf "0x%s, ", $i; if (++n % 6 == 0) printf "\n" } } END { printf "\n" }'

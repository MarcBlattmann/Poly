#!/usr/bin/env bash
# Visible slice A+B run (untracked helper, never committed).
# Usage (inside the container): bash .slice-run.sh [out-dir] [only-dir-glob-substring]
set -u
out=${1:-/repo/target/slice}
filter=${2:-}
mkdir -p "$out"
dirs=()
names=()
for d in specs/windows/user32.dll/*.tests specs/windows/gdi32.dll/*.tests; do
  [[ -n "$filter" ]] && ! grep -q -E "$filter" <<<"$d" && continue
  dirs+=(--tests-dir "$d")
  for s in "$d"/*.rs "$d"/*.cpp; do
    [[ -f "$s" ]] || continue
    n=${s##*/}
    names+=("${n%.*}")
  done
done
list=$(IFS=,; printf '%s' "${names[*]}")
cargo xtask pipeline win build "${dirs[@]}" --tests "$list" >"$out/build.log" 2>&1 || { echo "BUILD FAILED"; tail -30 "$out/build.log"; exit 2; }
cargo xtask pipeline win poly --build-kernel --jobs "${JOBS:-2}" --tests "$list" --out "$out/poly.json" >"$out/poly.log" 2>&1 || { echo "POLY RUN FAILED"; tail -30 "$out/poly.log"; exit 3; }
: >"$out/diff.log"
for d in specs/windows/user32.dll/*.tests specs/windows/gdi32.dll/*.tests; do
  [[ -n "$filter" ]] && ! grep -q -E "$filter" <<<"$d" && continue
  tl=()
  for s in "$d"/*.rs "$d"/*.cpp; do
    [[ -f "$s" ]] || continue
    n=${s##*/}
    tl+=("${n%.*}")
  done
  echo "== $d" >>"$out/diff.log"
  cargo xtask pipeline win diff --reference "$d/windows.json" --target "$out/poly.json" --tests "$(IFS=,; printf '%s' "${tl[*]}")" 2>&1 | grep -E '^(PASS|FAIL|ERROR|MISSING|UNSUPPORTED)' >>"$out/diff.log"
done
echo "PASS: $(grep -c '^PASS' "$out/diff.log")  FAIL/other: $(grep -c -E '^(FAIL|ERROR|MISSING|UNSUPPORTED)' "$out/diff.log")"
grep -E '^(FAIL|ERROR|MISSING|UNSUPPORTED)' "$out/diff.log"

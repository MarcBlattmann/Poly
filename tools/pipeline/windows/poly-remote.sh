#!/bin/sh
# Run Poly's Windows tests on the Windows oracle machine over SSH, from a
# host whose builds run in a container (ADR 0097). Three steps:
#
#   1. in the build environment: cargo xtask pipeline win remote prepare
#      (build, bundle, write .oracle-remote/<host>/ with transfer.sh)
#   2. here, where `ssh <host>` works: sh .oracle-remote/<host>/transfer.sh
#      (copy the bundle and runner over, run them, fetch results.json)
#   3. in the build environment: cargo xtask pipeline win remote finish
#      (check the results; --record, --diff)
#
# Usage, from the repository (or a worktree) root:
#   POLY_RUN=~/poly-dev/polyd tools/pipeline/windows/poly-remote.sh --host homeserver2 [--tests a,b] [--record] [--diff]
#
# POLY_RUN runs one command line in the build environment (it gets it as one
# argument); without it, the command runs here with `sh -c`. Every option
# goes to both `prepare` and `finish` (see tools/pipeline/src/main.rs).
set -eu

host=
prev=
for a in "$@"; do
    if [ "$prev" = --host ]; then host=$a; fi
    prev=$a
done
if [ -z "$host" ]; then
    echo "usage: $0 --host <ssh host> [win remote options]" >&2
    exit 2
fi

# Quote one argument for the build environment's shell.
quote() { printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"; }
args=
for a in "$@"; do args="$args $(quote "$a")"; done

run() {
    if [ -n "${POLY_RUN:-}" ]; then "$POLY_RUN" "$1"; else sh -c "$1"; fi
}

stage=".oracle-remote/$host"
run "cargo xtask pipeline win remote prepare --stage $stage$args"
status=0
sh "$stage/transfer.sh" || status=$?
if [ ! -f "$stage/results.json" ]; then
    echo "$0: no results fetched from $host (transfer exit $status)" >&2
    exit 1
fi
run "cargo xtask pipeline win remote finish --stage $stage$args"

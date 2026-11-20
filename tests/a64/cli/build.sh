#!/usr/bin/env bash
# Builds the arm64 command-line programs of the boot test group
# `macho-a64-cli` (ADR 0257 addendum) into bin/: Lua and the SQLite shell
# from their pinned, checksummed releases (downloaded here, never
# committed), and Poly's own C test programs. Dev-time only: the boot test
# reads the committed binaries.
#
# Needs a Mac with Apple's command line tools (clang and ld, black boxes).
# The programs link the SDK's libSystem the ordinary way; on Poly, Poly's
# own libSystem takes its place.
#
#   bash tests/a64/cli/build.sh
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
out=$here/bin
mkdir -p "$out"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cd "$tmp"

LUA_VERSION=5.4.7
LUA_SHA256=9fbf5e28ef86c69858f6d3d34eccc32e911c1a28b4120ff3e84aaa70cfbf1e30
SQLITE_TAG=3470200
SQLITE_YEAR=2024
SQLITE_SHA256=aa73d8748095808471deaa8e6f34aa700e37f2f787f4425744f53fdd15a89c40

fetch() { # url sha256 file
    curl -sSfL -o "$3" "$1"
    echo "$2  $3" | shasum -a 256 -c - >/dev/null
}

CFLAGS="-arch arm64 -mmacosx-version-min=13.0 -O2 -fno-ident"

# Lua: the interpreter without readline or dlopen (LUA_USE_POSIX).
fetch "https://www.lua.org/ftp/lua-$LUA_VERSION.tar.gz" "$LUA_SHA256" lua.tar.gz
tar xzf lua.tar.gz
(cd "lua-$LUA_VERSION/src" && clang $CFLAGS -DLUA_USE_POSIX -o "$out/lua" $(ls ./*.c | grep -v luac.c) -lm)

# The SQLite shell: without extension loading; single-threaded (the
# translator's guest threads are tested by threads.c).
fetch "https://www.sqlite.org/$SQLITE_YEAR/sqlite-amalgamation-$SQLITE_TAG.zip" "$SQLITE_SHA256" sqlite.zip
unzip -q sqlite.zip
(cd "sqlite-amalgamation-$SQLITE_TAG" &&
    clang $CFLAGS -DSQLITE_OMIT_LOAD_EXTENSION -DSQLITE_THREADSAFE=0 -o "$out/sqlite3" shell.c sqlite3.c)

strip -S "$out/lua" "$out/sqlite3"
ls -l "$out"

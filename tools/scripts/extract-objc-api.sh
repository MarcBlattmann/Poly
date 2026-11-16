#!/bin/sh
# Write the Objective-C API of one SDK framework as a symbol graph.
# Helper of import-macos-api.sh (ADR 0187); run on a Mac.
#
#   extract-objc-api.sh <sdk> <framework dir> <out dir> <target>
#
# `clang -extract-api` reads the framework's public headers (the umbrella
# header first, then every other header directly in Headers/) and writes
# Apple's documented JSON symbol graph of their declarations to
# <out dir>/<Name>.framework.json. Only the headers named on the command
# line are described, not the ones they include. Prints `ok <Name>` or
# `FAIL <Name>` (the compiler's messages are in <out dir>/<Name>.log); a
# framework without public headers prints nothing.
set -u

SDK=$1
FRAMEWORK=$2
OUT=$3
TARGET=$4

NAME=$(basename "$FRAMEWORK" .framework)
HEADERS="$FRAMEWORK/Headers"
[ -d "$HEADERS" ] || exit 0
ls "$HEADERS"/*.h >/dev/null 2>&1 || exit 0

UMBRELLA="$HEADERS/$NAME.h"
INPUTS=$( [ -f "$UMBRELLA" ] && echo "$UMBRELLA"; ls "$HEADERS"/*.h | grep -v "/$NAME\.h\$" )

# shellcheck disable=SC2086 # INPUTS is a list of paths without blanks
if xcrun clang -extract-api --product-name="$NAME" -x objective-c-header \
    -target "$TARGET" -isysroot "$SDK" -F "$SDK/System/Library/Frameworks" -fobjc-arc \
    -o "$OUT/$NAME.framework.json" $INPUTS >"$OUT/$NAME.log" 2>&1; then
    echo "ok $NAME"
else
    rm -f "$OUT/$NAME.framework.json"
    echo "FAIL $NAME"
fi

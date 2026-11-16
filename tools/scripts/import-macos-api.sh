#!/bin/sh
# Map the Objective-C and Swift API of the macOS frameworks into the
# coverage map (ADR 0187), next to the exported names that
# import-macos-sdk.sh records. Run this on a Mac, again when a new SDK
# ships (use the same SDK for both scripts so the revisions match).
#
#   tools/scripts/import-macos-api.sh [<MacOSX.sdk>]
#
# Without an argument the SDK is `xcrun --sdk macosx --show-sdk-path`; the
# Command Line Tools' SDK, /Library/Developer/CommandLineTools/SDKs/MacOSX<version>.sdk,
# works too.
#
# Objective-C: `clang -extract-api` (Apple clang, a black box) describes
# the public headers of each framework; tools/scripts/extract-objc-api.sh
# runs it per framework into a scratch directory of symbol graphs, and
# `coverage import macos-objc` keeps the names of classes, protocols,
# methods and properties. Swift: `coverage import macos-swift` reads the
# SDK's .swiftinterface files lexically. Nothing but names reaches the
# repository (see CLEAN_ROOM.md); the scratch directory is removed.
set -eu

cd "$(dirname "$0")/../.."

if [ "$(uname -s)" != "Darwin" ]; then
    echo "This script reads the macOS SDK and must run on a Mac." >&2
    exit 1
fi
if ! command -v cargo >/dev/null 2>&1; then
    echo "Rust is not installed. Install it from https://rustup.rs, then open a new terminal." >&2
    exit 1
fi

SDK=${1:-$(xcrun --sdk macosx --show-sdk-path)}
if [ ! -f "$SDK/SDKSettings.json" ] && [ ! -f "$SDK/SDKSettings.plist" ]; then
    echo "$SDK does not look like a macOS SDK (no SDKSettings)." >&2
    exit 1
fi

PLIST="$SDK/System/Library/CoreServices/SystemVersion.plist"
if [ -f "$PLIST" ]; then
    VERSION=$(/usr/libexec/PlistBuddy -c "Print :ProductVersion" "$PLIST")
    BUILD=$(/usr/libexec/PlistBuddy -c "Print :ProductBuildVersion" "$PLIST")
else
    VERSION=$(xcrun --sdk macosx --show-sdk-version)
    BUILD=$(xcrun --sdk macosx --show-sdk-build-version 2>/dev/null || echo unknown)
fi
REVISION="macOS SDK $VERSION ($BUILD)"
TARGET="arm64-apple-macos$VERSION"
JOBS=$(sysctl -n hw.ncpu)

GRAPHS=$(mktemp -d "${TMPDIR:-/tmp}/poly-objc-graphs.XXXXXX")
trap 'rm -rf "$GRAPHS"' EXIT

echo "==> Reading $SDK ($REVISION)"
echo "==> Objective-C: extracting the API of every framework with public headers ($JOBS at a time)"
ls -d "$SDK"/System/Library/Frameworks/*.framework |
    xargs -P "$JOBS" -I{} sh tools/scripts/extract-objc-api.sh "$SDK" {} "$GRAPHS" "$TARGET" >"$GRAPHS/result.txt"
echo "    $(grep -c '^ok ' "$GRAPHS/result.txt") frameworks extracted"
if grep -q '^FAIL ' "$GRAPHS/result.txt"; then
    echo "    not extracted (the compiler could not read their headers): $(grep '^FAIL ' "$GRAPHS/result.txt" | cut -d' ' -f2 | sort | tr '\n' ' ')"
fi
cargo xtask coverage import macos-objc "$GRAPHS" --revision "$REVISION"

echo "==> Swift: reading the .swiftinterface files"
cargo xtask coverage import macos-swift "$SDK" --revision "$REVISION"

echo "==> Regenerating the README progress section"
cargo xtask coverage readme

echo
echo "Done. Check the result, then commit and push:"
echo "    git add coverage README.md docs/progress"
echo "    git commit -m \"Map the Objective-C and Swift API from $REVISION\""
echo "    git push"

#!/bin/sh
# Map the macOS framework API into the coverage map. Run this on a Mac.
#
# Reads the .tbd stub files of the macOS SDK that comes with Xcode or the
# Command Line Tools, and records only the exported *names* in coverage/.
# No SDK file is copied into the repository (see CLEAN_ROOM.md).
set -eu

cd "$(dirname "$0")/../.."

if [ "$(uname -s)" != "Darwin" ]; then
    echo "This script reads the macOS SDK and must run on a Mac." >&2
    exit 1
fi
if ! xcrun --sdk macosx --show-sdk-path >/dev/null 2>&1; then
    echo "No macOS SDK found. Install the Command Line Tools first:" >&2
    echo "    xcode-select --install" >&2
    exit 1
fi
if ! command -v cargo >/dev/null 2>&1; then
    echo "Rust is not installed. Install it from https://rustup.rs, then open a new terminal:" >&2
    echo "    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh" >&2
    exit 1
fi

SDK=$(xcrun --sdk macosx --show-sdk-path)
VERSION=$(xcrun --sdk macosx --show-sdk-version)
BUILD=$(xcrun --sdk macosx --show-sdk-build-version 2>/dev/null || echo unknown)
REVISION="macOS SDK $VERSION ($BUILD)"

echo "==> Reading $SDK ($REVISION)"
cargo xtask coverage import macos-sdk "$SDK" --revision "$REVISION"

echo "==> Ticking the roadmap milestone"
sed -i '' 's/^- \[ \] macOS framework API mapped from the SDK/- [x] macOS framework API mapped from the SDK/' docs/ROADMAP.md

echo "==> Regenerating the README progress section"
cargo xtask coverage readme

echo
echo "Done. Check the result, then commit and push:"
echo "    git add coverage README.md docs/progress docs/ROADMAP.md"
echo "    git commit -m \"Map the macOS framework API from $REVISION\""
echo "    git push"

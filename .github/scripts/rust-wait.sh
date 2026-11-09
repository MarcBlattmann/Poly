#!/usr/bin/env bash
# Waits for what rust-start.sh started, and fails if the install failed.
set -u
log="$HOME/rust-install.log"
for _ in $(seq 1 300); do
  if grep -q '^exit=' "$log" 2>/dev/null; then break; fi
  sleep 1
done
cat "$log"
grep -q '^exit=0' "$log" || { echo "::error::installing Rust failed"; exit 1; }
rustc --version

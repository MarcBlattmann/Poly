#!/usr/bin/env bash
# Starts installing the Rust toolchain of rust-toolchain.toml in the background
# (detached, so it outlives this step); rust-wait.sh waits for it. The boot
# and pipeline jobs spend their first half minute on packages and artifacts,
# which Rust's 15 s of downloads can overlap.
set -u
log="$HOME/rust-install.log"
setsid nohup bash -c 'rustup show active-toolchain || rustup toolchain install; echo "exit=$?"' > "$log" 2>&1 < /dev/null &
echo "started installing Rust (log $log)"

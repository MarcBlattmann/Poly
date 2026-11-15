#!/bin/sh
# usage: chk.sh pkg...
cd /repo
VENDOR=$(rustc --print sysroot)/lib/rustlib/src/rust/library/vendor
PK=""
for p in "$@"; do PK="$PK -p $p"; done
RUSTC_BOOTSTRAP=1 CARGO_INCREMENTAL=0 RUSTFLAGS="-Clinker=rust-lld -Clinker-flavor=lld-link -Clink-arg=/NODEFAULTLIB" \
cargo check --release --manifest-path personalities/nt/win32/Cargo.toml $PK -Zbuild-std=core,alloc,compiler_builtins -Zbuild-std-features=compiler-builtins-mem \
 --target i686-pc-windows-msvc --target-dir target/wow64-check --offline --keep-going --message-format short \
 --config 'source.crates-io.replace-with="rustsrc"' --config "source.rustsrc.directory=\"$VENDOR\"" 2>&1

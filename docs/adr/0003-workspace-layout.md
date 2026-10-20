# 0003: Host-testable crates, thin kernel

**Status:** Accepted

## Context

Most of this project is pure logic: binary format parsing, syscall argument
decoding, memory maps, later VFS path rules, NT object semantics. Kernel code
is slow to test (boot in QEMU) and hard to debug. AI agents need fast,
precise feedback.

## Decision

- Logic that does not touch hardware lives in `no_std` crates under `crates/`
  and `personalities/`, unit-tested with plain `cargo test` on the host.
- The `kernel/` crate wires them to hardware and stays thin. It is a separate
  Cargo workspace because it only builds for `x86_64-unknown-none`.
- `cargo xtask` is the one entry point that builds, boots and tests both.
- The kernel uses no third-party crates. Host tools may.
- The compiler is pinned in `rust-toolchain.toml` and bumped deliberately.

## Consequences

- `cargo test --workspace` stays fast enough to run on every change.
- Shared crates must stay `no_std` and allocation-free until the kernel heap exists.

# 0001: Three personalities on one Rust kernel

**Status:** Accepted

## Context

Linux, Windows and macOS apps must run side by side on our own kernel,
without Wine and without a Linux kernel underneath.

## Decision

One shared core (scheduler, memory, VFS, IPC, networking, drivers) hosts three
system call personalities: Linux, NT and XNU. A process's personality is fixed
at exec from its binary format (ELF → Linux, PE → NT, Mach-O → XNU) and its
system calls are routed to that personality's table.

The shared types live in `poly-abi`; each personality is its own crate that
implements `SyscallDispatch` and is unit-tested on the host.

## Consequences

- Precedent: Windows NT's original subsystems, WSL1 pico processes, FreeBSD's
  Linux ABI layer.
- The NT and XNU personalities choose their own service numbering, because
  only our own `ntdll`/`libsystem` call into them.
- Personality crates will need kernel services (memory, files, scheduling).
  These will come in through a trait the kernel implements, so the crates stay
  host-testable with a fake kernel.

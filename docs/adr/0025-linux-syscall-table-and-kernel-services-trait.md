# 0025: Linux system calls: a generated table, man-page handlers, a kernel-services trait

**Status:** Accepted

## Context

Phase 3 runs the first static Linux program, and the Linux personality grows
from "everything is `-ENOSYS`" to real calls. About 370 x86-64 calls exist;
a few dozen are needed for a static C or Rust "hello world" (traced with
`strace -f` on a Linux host), and the rest arrive one by one from the agent
pipeline. Every call needs kernel subsystems (VFS, VMA manager, threads,
clocks) that other crates own, but ADR 0001 wants the personality
host-tested with a fake kernel. `SyscallDispatch::dispatch(regs)` has no way
to reach the kernel.

## Decision

- **Table.** `personalities/linux/src/numbers.rs` holds every call's number
  and name and an `nr::*` constant per call. It is generated from
  `coverage/linux/syscall/x86_64.json` by a crate test, which fails when the
  two drift and rewrites the file with `POLY_BLESS=1`. Handlers are
  registered by number in one list (`table::IMPLEMENTED`), turned into a
  `[Option<Handler>; 473]` at compile time; lookup is one bounds-checked index.
- **Handlers** have the signature
  `fn(&mut dyn LinuxKernel, &Args) -> Result<u64, Errno>`. The dispatcher
  writes the value or `-errno` to `rax` and nothing else, and reports
  `Outcome::Handled`; numbers without a handler keep returning `-ENOSYS` with
  `Outcome::Unimplemented`. `Args` decodes `rdi, rsi, rdx, r10, r8, r9` by
  C type (`int` uses the low 32 bits); pointers are `UserPtr`, never
  dereferenced.
- **Kernel services** come through one object-safe trait, `LinuxKernel`, that
  stands for the calling thread: ids and credentials; `copy_from_user`,
  `copy_to_user`, `strncpy_from_user` (all `EFAULT`); file operations on
  kernel buffers (`write`, `read`, `close`, `fstat`, `ioctl`) for the VFS;
  `mmap` (a validated `MapRequest`), `munmap`, `mprotect`, `brk`/`set_brk`
  for the VMA manager; FS/GS base; `clear_child_tid`; the signal mask and
  dispositions (types owned by the personality, stored by the kernel);
  thread and group exit; clocks; random bytes; host and domain name.
  `dyn` keeps the table a plain array of function pointers; the cost is one
  indirect call per service, small next to a system call.
- **Split of work.** The personality does what the man pages describe about
  arguments: flags, alignment, rounding, sizes, `ssize_t` overflow, the
  order of copies, and Linux's odd cases (brk returns the current break on
  failure, `MAP_SHARED_VALIDATE` vs. unknown flags, SIGKILL/SIGSTOP never
  blocked). The kernel does what needs state it owns. Where the man pages
  list errors but not their order, the personality's order is documented in
  the handler and left to the diff runner to confirm.
- **Data path.** File data moves through heap bounce buffers of up to 64 KiB
  (`alloc` is used for these and for iovec arrays): a write loops over
  chunks until done or short, a read does one chunk. This keeps the VFS
  interface free of user pointers.
- **`SyscallDispatch::dispatch`** (no kernel) still answers `-ENOSYS`; the
  kernel calls `Linux::dispatch_with(&mut kernel, regs)` once it implements
  `LinuxKernel`.
- **Clean room.** Behaviour comes from the Linux man pages (section 2);
  numbers, constants and structure layouts from the UAPI headers, cited at
  each definition. No Linux source was read.

## Consequences

- Adding a call is a handler, a line in `IMPLEMENTED`, and tests against
  the fake kernel in `src/tests/fake.rs`.
- The kernel implements one trait over its VFS, VMA manager and thread
  state; it never parses Linux argument encodings.
- Writes larger than 64 KiB reach the file in several pieces, so they are
  not atomic as one block (pipes only promise atomicity up to 4 KiB).
- Signals are recorded but not delivered until Phase 4; programs that rely
  on handlers running will not work before then.
- `exit`/`exit_group` return to the dispatcher; the kernel must not resume
  the thread afterwards.

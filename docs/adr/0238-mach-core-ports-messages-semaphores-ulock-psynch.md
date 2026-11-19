# 0238: The Mach core: `poly-mach` and the XNU personality's kernel interface

**Status:** Accepted. Roadmap Phase 9, "Mach core: tasks, threads, ports,
port rights, messages, Mach VM". Takes the place of the planned ADR 0161
("Mach IPC core"), which was never written; nothing of it is on `main`.

## Context

macOS programs reach the kernel through two tables: BSD system calls and
Mach traps. Before any program gets past libSystem's start-up it uses the
Mach side: `mach_task_self`, reply ports, `mach_port_*` on its own name
space, `mach_vm_*` for memory, and the locks of libSystem and libpthread
(`ulock_wait` / `ulock_wake`, `psynch_*`), plus `mach_msg` for everything
that talks to another process or a kernel server.

Spec batch 3 (`specs/macos/mach/`, `specs/macos/bsd/`, PR #315) records
what macOS 27 on arm64 does for 39 of these calls. Its tests are arm64
programs; Poly cannot load Mach-O programs yet (the loader and the arm64
translator are other tracks), so nothing of this can run on Poly today.

## Decision

1. **A host-tested crate, `crates/poly-mach`** (`no_std` + `alloc`), holds
   the Mach state of the whole machine and never blocks:
   - `Ipc`: one name space per task (32-bit names; one name per port for
     its receive and send rights, user references for send rights and dead
     names, a name of its own for each send-once right), port sets, message
     queues with a queue limit, kernel-object ports (task, thread, host,
     semaphore). Destroying a receive right turns every right to the port,
     in every space, into a dead name with the same references, destroys the
     queued messages and wakes the receivers with `MACH_RCV_PORT_DIED`.
   - Messages: the `mach_msg_header_t` layout and the port dispositions of
     the public `<mach/message.h>`, inline bodies and port descriptors,
     the reply right and the swap of remote and local on receive, the
     format-0 trailer cut to the size asked for, `MACH_RCV_LARGE`. Sending
     to a kernel object is reported as such (Poly has no MIG servers yet).
   - Semaphores (count, waiters, signal one / all / a thread) and the
     lookup order the sheets record (`KERN_INVALID_NAME`,
     `KERN_INVALID_RIGHT`, `KERN_INVALID_CAPABILITY`).
   - Wait tables for ulock (private and shared keys) and psynch (condition
     variables with pre-posted signals, mutex hand-off).
   A call that has to wait queues the calling thread; a waker leaves a result
   and returns the threads to unblock (`waitq`).
2. **The XNU personality gets a kernel interface, `XnuKernel`**, in the
   pattern of the Linux personality (ADR 0025): one value per calling
   thread; user memory copies, the address space as regions with current and
   maximum protection, block / unblock with deadlines, and the Mach state
   behind one lock (`with_mach`). The personality decodes the traps and the
   BSD memory and synchronisation calls, checks arguments in the order the
   sheets record, and drives the crate. `call()` is architecture-neutral
   (the arm64 translator decodes `x16` with `decode_arm64` and passes
   `x0..x7`); `dispatch_with()` is the x86-64 entry (seventh and eighth
   arguments from the user stack).
3. **The architecture of the caller decides two things**: the
   `mach_absolute_time` unit (arm64: 24 MHz, timebase 125/3; x86-64:
   nanoseconds, 1/1) and whether anonymous memory may be writable and
   executable at once (refused on arm64, as recorded).
4. **mach_msg_trap keeps what the oracle does**: a send option ends the
   process with SIGKILL (programs send with `mach_msg2_trap`, which is not
   specified yet); receive works on ports and sets with timeouts.
5. **Verification on the host**: `personalities/xnu/tests` runs every
   visible test of the 37 entries, transliterated from C, against a host
   model of the kernel (threads as processes, fork with inheritance,
   signals, 16 KiB pages) and compares the output byte for byte with the
   arm64 output recorded in `macos.json`.

## Consequences

- The kernel keeps calling the stateless `Xnu::dispatch` until Mach-O
  processes exist; implementing `XnuKernel` in the kernel (on `poly-vm`
  regions, the scheduler's block and wake, one `MachState` behind a lock) is
  the next step and needs a loaded Mach-O program to be useful.
- Values the sheets mark as values (not checks) follow the recordings where
  the model can, for example `EOWNERDEAD` for an unfair lock whose word does
  not name a thread, `316` for a timed-out `psynch_cvwait`.
- Not done: `mach_msg2_trap`, out-of-line memory in messages, no-senders
  and dead-name notifications, MIG servers for task, thread, host and
  semaphore ports (`semaphore_create` is a crate call until they exist),
  process-shared psynch objects.

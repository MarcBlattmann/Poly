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

- The kernel implements `XnuKernel` (`kernel/src/xnu_mach.rs`) on the
  process's `user::Memory` and the scheduler's park and unpark, with one
  `MachState` behind a spin lock. A Mach task is a pid; its name space is
  made on the process's first Mach call and dropped at exit. `xnu.rs` asks
  the BSD layer (ADR 0237) first and falls back to the Mach core for calls
  that layer does not have. Lowering a maximum protection is a `poly-vm`
  operation (`VmMap::limit_max_prot`).
- The boot test group `macho-mach` runs the x86-64 builds of the visible
  tests that need no fork, mmap or sysctl (24 programs,
  `personalities/xnu/tests/data`) on Poly and compares their output with
  the macOS recordings. The others join it as the BSD layer gains those
  calls.
- Values the sheets mark as values (not checks) follow the recordings where
  the model can, for example `EOWNERDEAD` for an unfair lock whose word does
  not name a thread, `316` for a timed-out `psynch_cvwait`.
- Not done: `mach_msg2_trap`, out-of-line memory in messages, no-senders
  and dead-name notifications, MIG servers for task, thread, host and
  semaphore ports (`semaphore_create` is a crate call until they exist),
  process-shared psynch objects.

## Addendum (2026-11-20): Mach exceptions

What Chrome's crash handling (Crashpad) and its process model need: handlers
for a task's or thread's exceptions, the messages the kernel sends them, and
the state of a thread that stopped.

1. **Sending.** `mach_msg_trap` sends for x86-64 programs (alone and as the
   send half of an RPC); a full queue waits, with `MACH_SEND_TIMEOUT` at
   most the timeout. arm64 programs keep what the sheet recorded (a send
   with this trap ends the process; they send with `mach_msg2_trap`, which
   no public header describes yet).
2. **The kernel answers MIG requests** sent to a task or thread port, in
   `poly-mach` (`ipc/mig.rs`): `task_` / `thread_set_exception_ports`,
   `_get_` and `_swap_`, `thread_get_state` / `thread_set_state`,
   `semaphore_create` / `semaphore_destroy`; anything else gets
   `MIG_BAD_ID`. Layouts are those of the SDK's MIG-generated headers
   (`mach/task.h`, `mach/thread_act.h`, `mach/exc.h`), ids the subsystem
   base plus the routine's index in the preprocessed `.defs`; inline arrays
   of variable size travel packed (as MIG documents), a get's reply carries
   32 port descriptors with the handlers first.
3. **Handlers** live in tables per task, per thread and for the host: per
   exception type a send right, a behaviour (`EXCEPTION_DEFAULT`,
   `_STATE`, `_STATE_IDENTITY`, with `MACH_EXCEPTION_CODES`) and a state
   flavour (`x86_THREAD_STATE64`, `x86_THREAD_STATE`,
   `x86_EXCEPTION_STATE64`). A new task inherits its parent's task
   handlers (a child of `posix_spawn` or fork).
4. **Raising.** A CPU fault of an XNU thread that demand paging did not
   resolve becomes `EXC_BAD_ACCESS` (`KERN_INVALID_ADDRESS` or
   `KERN_PROTECTION_FAILURE` and the address; #GP: `EXC_I386_GPFLT`),
   `EXC_BAD_INSTRUCTION` (#UD), `EXC_ARITHMETIC` (#DE) or `EXC_BREAKPOINT`
   (#BP, #DB), with the codes of `mach/i386/exception.h`. The thread stops;
   its state is kept for `thread_get_state` / `thread_set_state`. The
   thread's handler, then the task's, then the host's gets
   `exception_raise*` (2401-2403) or `mach_exception_raise*` (2405-2407)
   with a send-once reply right to a port the kernel holds. `KERN_SUCCESS`
   resumes the thread with the reply's state (state behaviours) or the one
   `thread_set_state` gave; any other reply, or a reply right that dies,
   passes the exception on; without a taker the default action follows.
5. **Tests.** On the host: `poly-mach`'s message tests and the
   personality's `tests/exceptions.rs`. On Poly (boot group `macho-mach`):
   two x86-64 programs built with MIG stubs that Apple's `mig` generates,
   as a black box, from the SDK's `.defs` (`personalities/xnu/tests/data/
   exc/build.sh`): a parent registers a handler, spawns itself, and its
   MIG server answers the child's faults (a state reply that moves the
   child on; a decline that leaves the default action).

Not yet: the protected behaviours, `EXC_CRASH` and corpse notifications,
`host_set_exception_ports` by message (the host port has no server yet),
`posix_spawnattr_setexceptionports_np` (its sheet is on the way), arm64
thread state (with the translator), and the float and debug flavours.

## Addendum (2026-11-20): bootstrap, task ports, more of `mach_port_*`

The spec sheets of `spec-macos-bootstrap` (#435) cover what Chrome-era
programs need before their first window: the bootstrap server, task ports of
other processes, the rest of the `mach_port_*` family, the spawn attributes
for special and exception ports, and XPC's basics.

1. **The `mach_port_*` routines** live in `poly-mach` (`ipc/ports.rs`) and
   are reached two ways: the Mach traps where XNU has one
   (`_kernelrpc_mach_port_{insert,extract}_member`, `construct`,
   `destruct`, `get_attributes`, `guard`, `unguard`,
   `request_notification`: 22-25, 40-42, 77) and the `mach_port` MIG
   subsystem on the task port (3200 + the routine's index in
   `mach/mach_port.h`) for the ones without a trap (`destroy`, `get_refs`,
   `peek`, `set_mscount`, `extract_right`, `set_attributes`, the context
   pair, ...). Two replies are Poly's own because Poly's libSystem is their
   only client: `mach_port_extract_right` answers inline with the name and
   type acquired (a right that leaves the space and comes back would get a
   new name, where the sheet keeps it), and `mach_port_get_set_status`
   with the names inline (Poly's messages have no out-of-line memory yet).
2. **Port state** gained what those routines read: a context and a guard
   (strict or not), the make-send and send-once counts, membership in
   several port sets at once, and notification requests. No-senders is
   checked after every call (`Ipc::take_woken`) for the few ports that
   asked; dead-name requests sit in the name space and add the
   notification's reference to the dead name; port-destroyed moves the
   receive right into the notification instead of killing the port. A
   guard violation (destruct, destroy, `mod_refs(-1)` of a guarded receive
   right without its guard, unguard of what is not guarded) is not an error
   return: the caller ends with SIGKILL, from a trap or from a request it
   sent.
3. **Task ports of others.** `task_for_pid` gives only the caller's own task
   port; `task_name_for_pid` a name port (one per task, made on first use,
   `KObject::TaskName`) for the caller, its parent and its running children,
   never pid 0 or 1; `pid_for_task` reads a task or name port. A task's name
   port dies when the process ends, whatever ended it: the kernel's
   `Process::ended` now drops the Mach task of every XNU process (a child
   killed by a signal used to keep it).
4. **Special ports.** `task_get_special_port` / `task_set_special_port`
   (MIG 3409/3410): the task's own port, the host port, its name port and
   the bootstrap port by default; a port set by the task replaces the
   default and a child inherits it. `posix_spawnattr_setspecialport_np` and
   `posix_spawnattr_setexceptionports_np` have no effect for an
   unprivileged caller (the sheets), and every caller on Poly is one, so
   the kernel's posix_spawn ignores those lists.
5. **The bootstrap server** is a kernel object, one for the machine, behind
   every task's bootstrap port. Its messages are Poly's own (the ids
   `0x504f42xx` in `poly_mach::consts::bootstrap`; Apple's protocol with
   launchd is private): check-in makes a receive right with a queue limit
   of 2 for the caller, register keeps a send right, look-up hands out a
   send right. A name belongs to the task that holds it and is free again
   once that task has ended or the port has died; the results are
   `servers/bootstrap_defs.h`'s.
6. **Tests.** On the host: `poly-mach`'s `tests_ports.rs` (every routine,
   the notifications, the server by message). On Poly: the `mach/` sheets'
   visible tests in the `macho-specs` group. `task_name_for_pid_others`
   fails there by construction: the test's parent on Poly is the test
   init, pid 1, and the test expects both "the parent works" and "pid 1 is
   refused".

The libSystem side (the `bootstrap_*` functions, the `mach_port_*` stubs,
the spawn attributes and XPC) builds on Poly's libSystem (ADR 0258) and
follows in its own change; the visible tests of those entries need it.

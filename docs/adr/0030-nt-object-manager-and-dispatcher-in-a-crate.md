# 0030: NT object manager and dispatcher logic in a host-tested crate (`poly-ntobj`)

**Status:** Accepted. Builds on [ADR 0003](0003-workspace-layout.md)
(host-testable crates, thin kernel) and follows the policy/mechanism split
of [ADR 0012](0012-scheduler-policy-in-a-crate-context-switch-in-the-kernel.md).

## Context

Phase 5 needs the NT object manager (handles, the object namespace,
reference counts) before almost any other NT service: files, sections,
processes, the registry and every synchronisation call go through handles
and names. The synchronisation milestone (events, mutants, semaphores,
timers, `NtWaitForMultipleObjects`) needs the dispatcher: who a signal
satisfies, in what order, with which status.

Both are bookkeeping-heavy and full of error paths whose exact `NTSTATUS`
matters to applications. None of it touches hardware. There is no spec
sheet for any of it yet (no Windows oracle VM), so the implementation can
only use public documentation and must say where it guessed (see
`CLEAN_ROOM.md`).

## Decision

**One `no_std` crate, `crates/poly-ntobj`, with `alloc`.** It holds the
whole object manager state in an `ObjectManager` value the kernel keeps
behind one lock. The namespace, handle tables and wait queues grow without a
bound known in advance, and the kernel has a heap (ADR 0003's
"allocation-free until the kernel heap exists" no longer binds), so the
crate uses `Vec`, `VecDeque`, `BTreeMap`. It has no dependencies, not even
on `poly-sched`: threads and processes are opaque `ThreadRef(u64)` and
`ProcessId(u32)` the kernel picks.

**Services mirror the system calls.** `nt_create_event`, `nt_open_event`,
`nt_set_event`, `nt_wait_for_multiple_objects`, `nt_close`,
`nt_duplicate_object`, `nt_query_object_*` ... take a `Caller` (process,
thread, previous mode, privileges), captured `ObjectAttributes` and plain
values, and return `Result<T, NtStatus>`. The personality's job shrinks to
copying arguments in and results out. Object-level calls (`set_event(id)`,
`wait_objects`, `reference`/`dereference`, `new_object`) serve kernel code.

**The crate decides, the kernel sleeps.** A wait returns `Done(status)` or
`Blocked(WaitId)`. Any later call that satisfies blocked waits queues
`Wake { wait, thread, status }`; the kernel drains them with `pop_wake`
after the call (under the same lock) and makes those threads runnable.
Timeouts, alerts and user APCs are the kernel's: it ends a blocked wait with
`cancel_wait(id, reason)` and gets the status to return. A thread that ends
calls `thread_terminated`, which drops its wait and abandons its mutants.

**Ids carry generations.** `ObjectId` and `WaitId` are slot + generation,
so a stale id never reaches a reused slot.

**Reference counts as documented.** Handle count plus pointer count (which
includes one per handle). A temporary named object loses its name when its
handle count reaches zero; an object is deleted when its pointer count
reaches zero and it has no name. A name holds a reference on its directory;
a blocked wait holds one on each object. Deleting an opaque object queues a
`Deleted { id, kind, cookie }` for the kernel.

**Opaque types are placeholders with hooks.** Section, File, Process,
Thread, Key and Device objects are created with a kernel cookie, an
optional signal state (so processes and threads can be waited on) and an
optional parse flag. A lookup that reaches a parse object hands the rest
of the path to a `ParseHook` the caller passes in, which returns an object,
a reparse path or a failure. That is where devices and file systems will
plug in.

**Access without security descriptors.** Each object has an *allowed* mask
standing in for its DACL (default: every valid right for the type). Opens
map generic rights, honour `MAXIMUM_ALLOWED`, and check user-mode requests
against the allowed mask; kernel-mode requests skip the check unless
`OBJ_FORCE_ACCESS_CHECK`. Handle use checks the granted mask (kernel mode
skips it). Tokens and real ACL evaluation replace the allowed mask later
without changing the service signatures.

**Guesses are labelled.** Wherever public documentation does not settle a
behaviour, the code picks one and its comment says "to be confirmed by the
spec agent against real Windows". The list below collects them so the
spec agent can turn each into a spec-sheet test.

## To be confirmed by the spec agent against real Windows

Namespace and names:

1. Case folding: per UTF-16 code unit, Unicode uppercase via Rust's `core`
   only when it maps to one BMP character; surrogates unchanged (examples to
   check: `ı`, `ſ`, `ÿ`, `ß`, Greek with ypogegrammeni).
2. Lookups without `OBJ_CASE_INSENSITIVE` compare exactly (Learn says
   "default system settings"); case-sensitive creates of `Foo` and `foo`
   coexist and a case-insensitive lookup finds the older.
3. `\??` is a plain directory (no per-session DOS-devices lookup with
   `\GLOBAL??` fallback yet); the starting set of directories and the
   `\DosDevices`, `\BaseNamedObjects\Global` and `\BaseNamedObjects\Local`
   links.
4. Statuses: a name without a leading `\` and no root, or a relative name
   with one, `STATUS_OBJECT_PATH_SYNTAX_BAD`; an empty component or trailing
   `\`, `STATUS_OBJECT_NAME_INVALID`; a non-directory on the way,
   `STATUS_OBJECT_TYPE_MISMATCH`; an open without a name,
   `STATUS_OBJECT_PATH_SYNTAX_BAD`; a create with a root handle and an empty
   name, `STATUS_OBJECT_NAME_INVALID`; an open with a root handle and an
   empty name opens the root.
5. Symbolic links: targets must be fully qualified
   (`STATUS_OBJECT_PATH_SYNTAX_BAD`); at most 32 reparses, then
   `STATUS_REPARSE_POINT_NOT_RESOLVED`; `OBJ_DONT_REPARSE` refuses object
   manager links with `STATUS_REPARSE_POINT_ENCOUNTERED`; `OBJ_OPENLINK`
   opens the link itself; a create never follows a final link (collision, or
   type mismatch with `OBJ_OPENIF`).
6. The root directory handle is not access-checked
   (`DIRECTORY_CREATE_OBJECT`, `DIRECTORY_TRAVERSE`), and traversal is not
   checked at all.
7. `NtQueryDirectoryObject` order (ours: sorted by upcased name).
8. The name `NtQueryObject` reports for an object whose directory lost its
   own name.
9. Type names (`Directory`, `SymbolicLink`, `Event`, `Mutant`, `Semaphore`,
   `Timer`, `Section`, `File`, `Process`, `Thread`, `Key`, `Device`).

Handles and access:

10. Freed handle values are reused lowest first; at most 2^24 handles per
    table (`STATUS_INSUFFICIENT_RESOURCES`).
11. Kernel handles carry the high bits `0xFFFF_FFFF_8000_0000`; from user
    mode they are `STATUS_INVALID_HANDLE`; `OBJ_KERNEL_HANDLE` from user
    mode is ignored.
12. `NtClose` on a pseudo handle succeeds and does nothing; on a protected
    handle it fails with `STATUS_HANDLE_NOT_CLOSABLE`.
13. `NtDuplicateObject`: the new handle may get more access than the source
    when the object allows it; no target without `DUPLICATE_CLOSE_SOURCE` is
    `STATUS_INVALID_PARAMETER`; a protected source survives
    `DUPLICATE_CLOSE_SOURCE`; unknown options are `STATUS_INVALID_PARAMETER`;
    `OBJ_PROTECT_CLOSE` (0x1) is accepted as a handle attribute;
    `NtCurrentThread()` as source means the caller's thread.
14. `OBJ_EXCLUSIVE`: another process gets `STATUS_ACCESS_DENIED` (open and
    duplicate); asking for it on an object not created exclusive is
    `STATUS_INVALID_PARAMETER`; exclusivity ends when the handle count
    reaches zero; inherited handles are not refused.
15. Unknown `OBJ_*` bits (outside `OBJ_VALID_ATTRIBUTES`, 0x1FF2) are
    `STATUS_INVALID_PARAMETER`.
16. Handle use checks the type before the access.
17. Generic mappings of every type except File and Key (which come from
    `winnt.h`).
18. A right the type does not have is denied rather than ignored;
    `ACCESS_SYSTEM_SECURITY` without the privilege is
    `STATUS_PRIVILEGE_NOT_HELD`.
19. Access needed by `NtReleaseMutant` (none), `NtMakeTemporaryObject`
    (`DELETE`), `NtQueryObject` (none).
20. `NtQueryObject(ObjectBasicInformation)`: the counts (any bias, or the
    query's own reference) and which attribute bits are reported.

Dispatcher:

21. Strict FIFO per object (Learn says FIFO is not guaranteed), and
    abandoned mutants freed in ascending object id order when their owner
    ends.
22. A satisfied wait-all with abandoned mutants returns
    `STATUS_ABANDONED_WAIT_0 +` the first abandoned index.
23. A mutant's abandoned state clears on its next acquisition.
24. The same object twice: `STATUS_INVALID_PARAMETER_MIX` in a wait-all
    (also through two different handles), allowed in a wait-any.
25. Zero or more than 64 objects: `STATUS_INVALID_PARAMETER`; a
    non-waitable object: `STATUS_OBJECT_TYPE_MISMATCH`.
26. Mutant recursion limit (`i32::MAX`) and the status a wait returns at the
    limit (`STATUS_MUTANT_LIMIT_EXCEEDED`); what `NtReleaseMutant` reports
    as `PreviousCount` (ours: the recursion count before the release).
27. `NtReleaseSemaphore` with a count of zero or less:
    `STATUS_INVALID_PARAMETER`.
28. Closing the last handle during a wait keeps the wait going.
29. The previous state `NtSetTimer` and `NtCancelTimer` report.

## Consequences

- Every rule above has a host test, including a randomized check of the
  dispatcher against a reference model with many simulated threads; the
  kernel side only has to prove locking and sleeping.
- The kernel must call `pop_wake` after every call that can signal, and
  must cancel (or let satisfy) every blocked wait before it forgets the
  thread; the crate cannot check either.
- One lock around the whole manager is simple and correct on one CPU; with
  SMP it may need splitting (namespace vs. dispatcher), which the API
  allows because ids, not references, cross the boundary.
- `ThreadRef` and `ProcessId` are the kernel's to keep unique while in use;
  reusing a `ThreadRef` after `thread_terminated` is safe.
- Security descriptors, sessions (`\Sessions\<n>`, per-session `\??`),
  APC delivery and the timer clock are left for later milestones; their
  hooks (allowed mask, `\Sessions`, alertable flag, `timer_expired`) exist.

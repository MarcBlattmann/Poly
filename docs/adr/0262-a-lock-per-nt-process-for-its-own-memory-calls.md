# 0262: A lock per NT process for its own memory calls

**Status:** Accepted. Track `nt-per-process-vm-lock`. Builds on [ADR 0030](0030-nt-object-manager-and-dispatcher-in-a-crate.md)
(one NT lock), [ADR 0247](0247-a-timeline-of-one-key-press-and-fewer-nt-lock-waits.md) (the timeline that measured it) and
[ADR 0250](0250-out-of-memory-never-panics-the-kernel.md) (the commit charge).

## Context

All NT state sat behind one lock (`STATE`, the timeline's `nt-state-lock`): the object manager, the dispatcher, the
section store, and every process's memory model. Every memory call of every thread of every process took it.

The timeline of a warm Chrome start (chrome-first-start, PR #455) showed what that costs:
- **About 3.4 s of waiting for the lock**, summed over all threads, in a start of about 2.2 s.
- **The browser's main thread lost 458 ms** in `NtAllocateVirtualMemory` alone (2,884 calls), waiting for other
  processes' calls (`NtMapViewOfSection` of the renderers, the GPU process's allocations).
- **Allocate and free churn is a quarter of the Viz thread's time**, and the typing and Ctrl+Tab lock waits are the
  same lock.
- **`NtMapViewOfSection` held the lock about 3.6 ms per relocated DLL view**: the view read and checked the image's
  whole relocation table again each time (about 2 ms for `chrome.dll`, which moves in every process).

Most memory calls are a process's calls on its own private memory: `VirtualAlloc`, `VirtualFree`, `VirtualProtect`,
`VirtualQuery` of heaps, stacks, the V8 heap and Skia's buffers. Those need nothing of the system but the commit
charge and the job limits.

## Decision

1. **Each NT process's state has its own lock** (`kernel/src/nt/proctable.rs`).
   - The NT state's table of processes holds `Arc<Mutex<NtProcess>>`. `ProcTable::get`/`get_mut` lock a process and
     give a guard; most code reads as before.
   - A registry of the same cells (`proctable::own`, a leaf lock) finds the calling process's cell without the NT
     state's lock.
2. **A process's own memory calls take only its lock** (`NtKernel::with_own_vm`, `personalities/nt/src/memory.rs`):
   - `NtAllocateVirtualMemory` and `NtFreeVirtualMemory` on `NtCurrentProcess()`, when the range is private memory
     (`VirtualMemory::only_private`: no view, no placeholder) and no special flag (`MEM_RESET`, `MEM_RESET_UNDO`, the
     placeholder flags) is given;
   - `NtProtectVirtualMemory` and `NtQueryVirtualMemory` (basic information) on `NtCurrentProcess()`, any range: they
     change or read only the process's own pages.
   - The backend of this path has no section store. A call that would reach one (a view, a section commit) takes the
     full path instead; the personality's test kernel fails any test whose own-memory path reaches a section.
   - Everything else (other processes' memory, views, placeholders, the other information classes) runs as before:
     the NT state's lock, then the process's.
3. **The lock order is the NT state, then a process; never the reverse.**
   - Code that holds the NT state locks processes through the table.
   - The own-memory path holds a process's lock alone and takes no other NT lock: not the NT state's, not another
     process's. What it calls (the memory map's `map_anonymous`, `unmap`, `protect`) takes only the memory map's and
     the frame allocator's locks, as under the NT state's lock before.
   - A thread holding two processes (a parent and its new child, a thread made in another process) holds the NT
     state's lock, which keeps those apart: no two threads can each hold one process and wait for the other's.
   - Page faults (`resolve_fault`) take the NT state's lock and then the process's, as every other path. The
     own-memory path touches no user memory, so it never faults while it holds the process's lock.
   - The out-of-memory policy (ADR 0250) ends processes from its own thread, through the NT state's lock; a thread on
     the own-memory path never waits for it.
4. **The commit charge is atomics shared by the system and every process** (`CommitCharge`): the own-memory path
   charges and refunds it without the system's state. The system adopts each new process (`NtSystem::adopt`).
   - The check before a commit and the charge after it are two steps. Processes committing at the same moment can
     each pass the check and together pass the limit by one call's commit each. The limit is RAM less the kernel's
     reserve; the reserve (ADR 0250) is what never gives way, so this slack is harmless.
5. **Job memory limits are kept per process** (`Jobs::commit_limit_of`, a per-process `AtomicU64`): the jobs update
   it when a process joins a job and when any job's limits change, and the own-memory path reads it.
6. **A process that ends is taken out of the table under its lock.** A thread of it that found its cell before and
   locks it after gets an empty process in its place, whose calls touch nothing that is still used.
7. **Big image files keep their checked relocation table** (`image::RelocTables` in `NtSystem`, by file identity, at
   most 256 files): the next relocated view of the same file uses it instead of reading and checking the table again.

Sections and views (`NtMapViewOfSection`, `NtUnmapViewOfSection`, commits inside views) stay under the NT state's lock:
the section store and the section objects are the system's, shared by every process that maps them. Item 7 cuts the
longest of those holds.

## Consequences

- A process's heap and stack traffic no longer waits for other processes' calls, nor they for it. Threads of one
  process still take turns on its lock, as on Windows (its address space lock).
- `NtProcess` is reached through a guard. Locking the same process twice in one thread panics (the kernel `Mutex`
  catches it); `ProcTable::get_mut` takes `&mut self`, so the compiler refuses two guards from it in one scope.
- The kernel test `nt_memory_calls_of_many_threads_and_processes_at_once` runs `k32test.exe --vm-stress`: four
  threads in each of four processes allocating, protecting, querying, decommitting, releasing and mapping at once,
  while the parent works on each child's memory (`VirtualAllocEx`, `WriteProcessMemory`, `ReadProcessMemory`,
  `VirtualProtectEx`, `VirtualQueryEx`, `VirtualFreeEx`).
- Open: views under a process's lock (a section store split from the NT state's), and the other memory calls on
  other processes without the NT state's lock.

# Architecture

The full vision is in [PROJECT_BRIEF.md](PROJECT_BRIEF.md). This page shows
what exists **today** and how it fits that plan.

```
                         (later) compositor + shell
   ┌──────────────────────┬──────────────────────┬──────────────────────┐
   Linux apps             Windows apps            macOS apps
   │                      │ (later) our DLLs       │ (later) our frameworks
   ▼                      ▼                        ▼
   personalities/linux    personalities/nt        personalities/xnu
   -errno in rax          NTSTATUS in rax         CF + errno / kern_return_t
   └──────────────────────┼────────────────────────┘
                          │  poly-abi: Personality, SyscallRegs, SyscallDispatch
                 kernel/src/syscall.rs  ← routes by the process's personality tag
                 kernel/src/process.rs  ← tag set at exec from poly-binfmt
                          │
            shared core: boot, cpu (GDT/TSS/IDT), memory map, frames, threads
            arch/x86_64 (PVH entry, serial, exceptions, context switch)
```

## Boot flow

1. QEMU (`-kernel`) reads the PVH ELF note and jumps to `pvh_start32` in
   32-bit mode with `ebx` pointing at `hvm_start_info`.
2. `arch/x86_64/boot.rs` checks for long mode, maps physical [0, 1 GiB) at
   0 and at `0xffffffff80000000`, enables paging and jumps to `kernel_entry`.
3. `kernel_entry` brings up serial, loads our GDT/TSS/IDT, reads the boot info,
   normalises the memory map (reserving the first MiB and the kernel image),
   hands all usable RAM to the frame allocator, reads the ACPI tables and runs
   the boot self-test.
4. With `poly.qemu_exit` on the command line, the result is reported through
   QEMU's exit status, which is how `cargo xtask test` checks it.

## Firmware tables (ACPI)

- `poly-acpi` validates the RSDP, RSDT/XSDT and table checksums, and decodes
  the MADT (CPUs, I/O APICs, interrupt overrides, NMIs), HPET, FADT and MCFG.
  No AML yet. It never touches physical memory itself: it asks a
  `PhysMemory` implementation for bytes, so the host tests feed it synthetic
  tables and the kernel decides what is safe to read.
- `kernel/src/acpi.rs` implements `PhysMemory` over the boot mapping (only
  the first 1 GiB, and never usable RAM or the kernel image), prints a
  summary at boot and keeps the counts the self-test checks.

## Personality routing

- `poly-binfmt` identifies ELF / PE / Mach-O (and fat Mach-O, `#!` scripts)
  from the image header. It never panics on any input.
- `Process::from_image` tags the process with its personality once, at exec.
- All three worlds enter through the same `syscall` instruction. The core
  saves registers into `SyscallRegs` and `syscall::dispatch` hands them to the
  personality, which reads arguments and writes results in its own convention.
- Unimplemented calls return each OS's native "not implemented" result and are
  logged. That log line is the seed of the API tracer.

## Testing strategy

| Level | Where | Runs |
|---|---|---|
| Unit tests for pure logic | `crates/`, `personalities/`, `tools/` | `cargo test --workspace` (host) |
| In-kernel tests (`kernel_test!`, next to the code) | `kernel/src/ktest.rs` | every boot; `cargo xtask test` reports each test |
| Negative boot tests (faults, failures and panics must be reported, with a backtrace) | `poly.selftest.*` flags | `cargo xtask test` |
| Conformance vs. real OS (diff runner) | `specs/` + oracle VMs | later |
| App trace replay | recorded sessions | later |

Backtraces, symbolisation and attaching gdb/lldb are described in
[DEBUGGING.md](DEBUGGING.md).

## Memory layout (current)

| Virtual | Physical | What |
|---|---|---|
| `0x0` – `0x4000_0000` | same | identity map used by the boot stub (to be removed) |
| `0xffff_ffff_8000_0000` + phys | 0 – 1 GiB | kernel image and early access to low memory |

The real virtual memory manager (direct map of all RAM, per-process address
spaces) replaces the boot mapping in the next milestone.

## Physical memory

- `poly-mem` normalises the firmware memory map (sorted, non-overlapping,
  most restrictive type wins, usable RAM trimmed to whole pages).
- `poly_mem::BuddyAllocator` manages every usable 4 KiB frame in blocks of
  2^0 to 2^10 frames (4 KiB to 4 MiB), aligned to their size; freed buddies
  merge again. It never touches the memory it manages: its state is a few
  bitmaps per order in a caller-provided buffer (about 4 bits per frame of
  span, 256 KiB for 2 GiB), with summary levels so finding a free block costs
  one step per level rather than a scan. `free` checks a per-order
  "allocated" bitmap and rejects double, misaligned, out-of-range and
  wrong-order frees. See [ADR 0007](adr/0007-buddy-allocator-out-of-band-metadata.md).
- `kernel/src/frames.rs` carves that buffer from usable RAM below 1 GiB (the
  only RAM reachable through the boot mapping), marks it `Kernel` in the map,
  and puts the allocator in the global `frames::FRAMES` spinlock. RAM above
  1 GiB is managed too; nothing maps it yet. `cargo xtask test` boots with
  256 MiB and with 2 GiB to cover both cases. `allocate_below(order, limit)`
  takes the lowest suitable block under a physical limit, for memory the
  kernel must reach through the boot mapping (thread stacks, for now).

## Threads and scheduling

Kernel threads only, on one CPU, round robin. See
[ADR 0012](adr/0012-scheduler-policy-in-a-crate-context-switch-in-the-kernel.md).

- **Policy in `poly-sched`** (host-tested, including a randomized check
  against a reference model). `Scheduler<N>` is a fixed table of `N` thread
  slots with no other memory: a FIFO run queue and caller-owned
  `WaitQueue`s, all linked through one `next` field per slot (a thread is on
  at most one list). `ThreadId` is slot + generation, so a stale id never
  names a later thread in the same slot. Every operation that changes the
  running thread (`yield_now`, `block_current`, `join`, `exit_current`)
  returns a `Switch { from, to }` and leaves the switching to the kernel.
- **Time slices.** Each thread gets `TIME_SLICE_TICKS` (10) ticks when it is
  switched in. `tick()` returns true when the slice is used up *and*
  another thread is runnable (a lone thread just gets a new slice); the idle
  thread gives way as soon as anything is runnable.
- **Thread 0 and idle.** `thread::init()` (right after the frame allocator)
  makes the boot code thread 0, still on the boot stack, and creates the
  idle thread, which runs only when nothing else can. Without interrupts
  nothing could wake a thread at that point, so today it panics with a
  "deadlock" message and a backtrace instead of hanging.
- **Mechanism in the kernel.** `kernel/src/thread.rs` keeps the scheduler
  and each thread's stack in one spinlock (`THREADS`), and saved stack
  pointers in a static array. Stacks are 16 KiB (one order-2 buddy block)
  below the boot-mapping limit, registered with the backtrace walker. There
  is no guard page yet (TODO once page tables are dynamic).
  `arch/x86_64/context.rs` switches by pushing the six callee-saved
  registers, swapping `rsp` and popping; a new thread's stack is primed so
  that the switch "returns" into `thread_trampoline`, which calls
  `thread_start(f, arg)` with `rbp = 0` (the end of every thread's
  backtrace) and exits with `f`'s return value.
- **API:** `spawn(fn, arg)`, `yield_now()`, `exit(code)`, `current()`,
  `join(id)` (returns the exit code), `detach(id)`, `state(id)`, and
  `WaitQueue::{wait_until(cond), wake_one(), wake_all()}`. `wait_until`
  checks its condition with the scheduler locked, so a wakeup between the
  check and going to sleep cannot be lost.
- **Exit.** An exiting thread is still on its stack, so the next thread
  frees it (`finish_switch` after every switch). The table slot is freed
  once the thread is also joined or detached.
- **Interrupts and locking.** Every `THREADS` critical section runs with
  interrupts off, and the lock is dropped before the switch; with one CPU
  and interrupts off nothing can run in between. Each thread restores its
  own interrupt state when the switch returns into it. SMP needs the lock
  (or an "on CPU" flag) held until the outgoing thread's registers are
  saved.
- **Preemption hook.** `thread::preempt_tick()` is all the timer handler has
  to call: with interrupts off, after EOI, on the interrupted thread's own
  stack (not an IST stack, since the handler's frames stay there while the
  thread is switched out). New threads start with interrupts off; once the
  timer exists, `thread_start` must enable them.

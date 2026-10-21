# 0050: Kernel thread stacks in a stack area of their own, above unmapped guards

**Status:** Accepted. Amends [0010](0010-kernel-address-space.md) (a new
region in the kernel address space; thread stacks get guards too). Amended
by [0061](0061-per-cpu-interrupt-stacks.md): a second 2 MiB area right
after this one holds each CPU's interrupt stack, and stacks are painted to
measure their depth.

## Context

ADR 0010 put guard pages below the stacks in the kernel image (boot, double
fault, NMI). Kernel threads got 16 KiB stacks, one order-2 buddy block each,
used through the direct map, where the frames below a stack are whatever the
buddy allocator handed out next: another thread's stack, a heap page, a
page table. A thread that ran off the bottom of its stack corrupted that
memory silently, and the damage showed up later and elsewhere. The direct
map cannot have holes for guards: it maps physical memory linearly with
2 MiB or 1 GiB pages.

## Decision

**A stack area.** The kernel half gets one more region: 2 MiB at
`0xffff_c080_0000_0000` (PML4 slot 385, right after the MMIO window),
exactly what one page table maps. `paging::init` builds its PDPT, page
directory and page table at boot, empty, so the area never needs a new table
and the PML4 entry exists before any user address space copies the kernel
half.

**One slot per thread-table slot.** `thread::STACKS` (a `poly_mem::StackArea`,
host-tested) cuts the area into 64 slots of 32 KiB: slot `i` holds the stack
of the thread in table slot `i` in its upper 16 KiB, and its lower 16 KiB is
never mapped. A thread's frames are still one buddy block; `install` maps
them into the slot (4 PTEs, global, read-write, no-execute) and `reap` unmaps
them (with `invlpg`) before it frees them. Both run with the scheduler lock
held, which is what owns a slot's PTEs, so they take no page-table lock and
allocate nothing.

**Overflow reports.** Running off a stack writes into the guard; the page
fault cannot be pushed onto the exhausted stack, so it becomes a double
fault on its IST stack (ADR 0010). The exception handler asks
`thread::stack_guard_hit` whether CR2 or the saved RSP lies in a slot's
guard and reports `kernel stack overflow in thread <name> (slot <i>)`. It
looks the name up only if the thread table is not locked (the overflow may
have happened inside the scheduler).

**DMA from a thread stack** stays allowed (`dma.rs`): `virt_range_to_phys`
accepts a range in the stack area when every page of it is mapped and the
pages are physically consecutive, which they are within one stack.

## Consequences

- An overflow of up to 16 KiB below a thread stack faults and names the
  thread; `poly.selftest.thread_stack_overflow` checks that on every
  `cargo xtask test`. A single frame larger than the guard could still jump
  it, but Rust probes stack frames larger than a page, so it touches the
  guard first.
- Thread stacks are no longer in the direct map; code that turned a stack
  address into a physical one by subtracting `DIRECT_MAP_BASE` would be
  wrong (nothing did; DMA goes through `virt_range_to_phys`).
- The area holds `MAX_THREADS` (64) stacks of at most 16 KiB; more threads or
  larger stacks need a larger area (more page tables, still made at boot).
- Unmapping relies on the kernel threads running on one CPU: once other CPUs
  run kernel code, `unmap_thread_stack` needs a TLB shootdown.

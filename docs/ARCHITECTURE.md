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
            shared core: boot, cpu (GDT/TSS/IDT), memory map, frames
            arch/x86_64 (PVH entry, serial, exceptions)
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
  `allocate_below` limits a request to memory under an address, and
  `allocate_run` / `free_run` hand out runs of any length up to 4 MiB
  without rounding them up to a power of two.
- `kernel/src/frames.rs` carves that buffer from usable RAM below 1 GiB (the
  only RAM reachable through the boot mapping), marks it `Kernel` in the map,
  and puts the allocator in the global `frames::FRAMES` spinlock. RAM above
  1 GiB is managed too; nothing maps it yet. `cargo xtask test` boots with
  256 MiB and with 2 GiB to cover both cases.

## Kernel heap

- `poly-heap` is the allocator: size-class slabs (16 B to 2 KiB, slabs of
  one page up to 32 KiB, aligned to their size so a free finds its slab by
  rounding the pointer down) for small objects, and whole-page runs for
  everything larger or more strictly aligned. It takes pages from a
  `PageSource` and is host-tested against a mock one.
- `kernel/src/heap.rs` makes it the `#[global_allocator]`, so the kernel
  uses `alloc` (`Box`, `Vec`, `String`, `BTreeMap`). Its page source takes
  runs of frames from `frames::FRAMES` with `BuddyAllocator::allocate_run`,
  which frees the unused tail of the buddy block, and maps them with
  `arch::phys_to_virt`. One allocation is at most one buddy block (4 MiB).
  Until the direct map lands only the first 1 GiB is reachable, so the page
  source asks for frames there first and the heap fails cleanly when it is
  used up.
- Lock order is heap, then `FRAMES`: never allocate while holding `FRAMES`.
  Neither lock masks interrupts yet.
- Out of memory: fallible APIs (`try_reserve`) see the error; everything
  else panics with "memory allocation of N bytes failed" and a backtrace
  (stable Rust's default handler in `no_std`), checked by a boot test.
  Boot prints the heap's numbers after the self-test.

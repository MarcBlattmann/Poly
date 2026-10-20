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
            shared core: boot, cpu (GDT/TSS/IDT), memory map, frames, heap, threads
            block layer: request queue + buffer cache per disk (virtio0, ram0)
            devices: PCI (ECAM, MSI-X, MSI), virtio-blk (MSI-X interrupts), RAM disk
            arch/x86_64 (PVH entry, serial, exceptions, page tables, context switch)
```

## Boot flow

1. QEMU (`-kernel`) reads the PVH ELF note and jumps to `pvh_start32` in
   32-bit mode with `ebx` pointing at `hvm_start_info`.
2. `arch/x86_64/boot.rs` checks for long mode, maps physical [0, 1 GiB) at
   0 and at `0xffffffff80000000`, enables paging and jumps to `kernel_entry`.
3. `kernel_entry` brings up serial, loads our GDT/TSS/IDT, reads the boot info,
   normalises the memory map (reserving the first MiB and the kernel image),
   switches to the kernel page tables (`mm::init`; the boot mapping is gone
   after this), hands all usable RAM to the frame allocator, starts the
   scheduler, reads the ACPI tables, sets up interrupts and time
   (`arch::init_interrupts`) and enables them, enumerates PCI, brings up the
   block layer (the virtio-blk disk if there is one, and a RAM disk) and runs
   the boot self-test with the timer preempting threads.
4. With `poly.qemu_exit` on the command line, the result is reported through
   QEMU's exit status, which is how `cargo xtask test` checks it.

## Firmware tables (ACPI)

- `poly-acpi` validates the RSDP, RSDT/XSDT and table checksums, and decodes
  the MADT (CPUs, I/O APICs, interrupt overrides, NMIs), HPET, FADT and MCFG.
  No AML yet. It never touches physical memory itself: it asks a
  `PhysMemory` implementation for bytes, so the host tests feed it synthetic
  tables and the kernel decides what is safe to read.
- `kernel/src/acpi.rs` implements `PhysMemory` over the direct map (never
  usable RAM or the kernel image), prints a summary at boot and keeps the
  counts the self-test checks.

## Devices: PCI and virtio

Protocol logic lives in host-tested crates behind access traits
([ADR 0009](adr/0009-drivers-behind-access-traits.md)); the kernel maps
memory, implements the traits and drives the devices. Decisions are in
[ADR 0015](adr/0015-pci-and-virtio-in-the-kernel.md).

- **Configuration space.** `kernel/src/pci.rs` takes the ECAM windows from
  the ACPI MCFG (`acpi::ecam_regions`), maps each with `mm::map_mmio` and
  implements `poly_pci::ConfigSpace` over it with volatile accesses of the
  width asked for. The MCFG base is the address of bus 0 even when a window
  starts higher, so a window for buses N-M is mapped from base + N MiB. No
  MCFG means no PCI (the legacy 0xcf8 port mechanism is not implemented).
- **Enumeration.** `pci::init` probes every bus the windows decode
  (`poly_pci::scan_all`), sizes BARs (`probe_bars`; host and ISA bridges are
  skipped, since sizing briefly turns off decoding the kernel depends on)
  and prints one line per function. The list (`pci::devices()`) lives for
  the kernel's lifetime; drivers find their device in it.
- **BARs.** `Device::map_bar(i)` maps a memory BAR once (uncached) and
  returns an `MmioRegion` (`arch/x86_64/mmio.rs`, the kernel's one
  `poly_pci::Mmio`, also used for the APICs and the HPET): bounds- and
  alignment-checked volatile accesses, with `subregion` for the virtio
  structures inside a BAR. Drivers enable memory decoding and bus mastering
  themselves (`poly_pci::enable`).
- **Interrupts: MSI-X and MSI** ([ADR 0018](adr/0018-msi-x-and-interrupt-driven-disk-io.md)).
  INTx is never used. `Device::enable_msix` maps the MSI-X table and PBA
  (`map_bar`), turns INTx and MSI off and enables MSI-X with the function
  mask set and every entry masked; the driver then `attach`es the entries
  it uses (`MsixVectors::attach`: a kernel vector from
  `arch::irq::allocate_vector` running its handler, and the entry written
  with `pci::msi_message(vector)`) and clears the function mask
  (`set_function_masked(false)`), so nothing is sent before its entry is
  programmed. `MsixVectors` also masks and unmasks entries, reads entries
  and pending bits back, and `disable` turns MSI-X off and frees the
  vectors. Functions without MSI-X get plain MSI with one vector
  (`Device::enable_msi`; several would need an aligned block of
  consecutive vectors); `Device::enable_interrupt` picks MSI-X entry 0 or
  MSI. `to_pci_message` / `from_pci_message` convert between
  `poly_irq::MsiMessage` and `poly_pci::Message`. Messages are memory
  writes, so the function needs bus mastering, which its driver enables.
- **DMA.** `kernel/src/dma.rs`: `DmaBuffer` is a zeroed block of frames from
  `FRAMES`, used through the direct map. There is no IOMMU, so the bus
  address a device gets is the physical address. Drivers that take the
  caller's memory translate it with `dma::bus_address` (`KernelDma` for
  `poly-virtio-blk`), which accepts only ranges inside the direct map: the
  map is linear, so such a range is physically contiguous. Heap memory
  qualifies (every heap allocation lies in one run of frames from one buddy
  block, reached through the direct map), and so do `DmaBuffer`s and thread
  stacks; the kernel image (statics, the boot stack), MMIO mappings and
  everything else are refused, which fails the request with `BadBuffer`.
- **virtio-blk** (`kernel/src/virtio_blk.rs`): the driver is the host-tested
  `poly-virtio-blk` crate. The kernel finds the first virtio block function,
  discovers its structures (`poly_virtio::PciTransport`), maps them, turns
  on memory decoding, resets the device, turns on bus mastering and hands
  everything to `poly_virtio_blk::init` with DMA memory for the virtqueue
  (128 entries) and the request headers, tables from the heap and
  `KernelDma`. `init` negotiates features (VERSION_1 required; SIZE_MAX,
  SEG_MAX, RO, BLK_SIZE, FLUSH, EVENT_IDX and ACCESS_PLATFORM taken when
  offered) and sets DRIVER_OK. Data moves straight between the device and
  the caller's buffers, several requests at a time. If the function has
  MSI-X, it is enabled (masked) before `init`, which maps the request
  queue to entry 0; once the block layer has made the disk,
  `virtio_blk::start` attaches entry 0 to the disk's interrupt handler,
  asks the device for interrupts and clears the function mask. Without
  MSI-X (or if the device refuses the vector) the disk is polled; the boot
  log says which. The driver sits behind a `poly_block::Watchdog`: when
  requests are out and none has come back for 10 seconds (monotonic
  clock), it resets the device (`Abort`) and fails them, and every later
  request fails too. All of it lives as long as the kernel.

```text
pci: segment 0 buses 00-ff: ECAM at 0xb0000000, mapped at 0xffffc00000000000
pci: 00:03.0 1af4:1042 class 01.00.00, BAR1 mem32 0xfebd5000 4 KiB, BAR4 mem64 pf 0xfe000000 16 KiB
virtio-blk: 00:03.0: MSI-X with 3 vectors, table BAR1+0x0, PBA BAR1+0x800
virtio-blk: 00:03.0: 1 MiB (2048 sectors), block size 512, features 0x120000244, queue 128 entries, 126 segments per request, write cache
virtio-blk: 00:03.0: request queue on MSI-X entry 0 -> vector 0x30; interrupt-driven, waiters sleep
```

`cargo xtask` writes a fresh 1 MiB test image (`target/poly-test-disk.img`,
every 512-byte sector starting with "POLY" and its number) before each boot
and attaches it as `-device virtio-blk-pci,disable-legacy=on` with the
kernel flag `poly.test.disk`. With the flag, the block layer tests (below)
run against the disk as well as the RAM disk. Without it they check only
what is there, so the kernel boots without a disk too (`cargo xtask run
--no-disk`). Two more test devices come with it: a spare virtio-blk device
on QEMU's null block driver (1 MiB of zeros), which the kernel leaves alone
except for the watchdog test, and QEMU's `edu` device (MSI, no MSI-X;
flag `poly.test.msi`) for the MSI test. One self-test boot attaches the
test disk and the spare device without MSI-X (`vectors=0`), so the block
layer is tested polled as well.

## Block layer

The block layer is the host-tested `poly-block` crate
([ADR 0014](adr/0014-block-layer.md)); `kernel/src/block.rs` gives it
memory, locks, interrupt handling and sleeping waits per disk, and keeps
the list of disks ([ADR 0017](adr/0017-block-layer-in-the-kernel.md),
[ADR 0018](adr/0018-msi-x-and-interrupt-driven-disk-io.md)).

```
   file systems (later)           block::root(), block::ram(), block::find(name)
          │  Disk: read_block, read, write_block, sync, stats, lock() → Cached
          ▼
   BufferCache   64 KiB per disk: write-back, exact LRU, pinning, read-ahead
          │
   RequestQueue  32 requests; ordering, C-LOOK elevator, merging, splitting;
          │      4 requests of up to 16 segments at the device
          ▼
   block::Device ── Virtio: Watchdog<poly_virtio_blk::VirtioBlk, clock>  ("virtio0", MSI-X)
                └── Ram:    poly_block::RamDisk, 256 KiB of heap           ("ram0", polled)
```

- **Disks.** `block::init` (after `pci::init`) registers the virtio-blk disk
  as `virtio0` if there is one, and always a 256 KiB RAM disk as `ram0`, so
  file system code can be tested without a disk. `block::root()` is the
  disk a file system mounts (`virtio0`), `block::ram()` the RAM disk,
  `block::disks()` all of them.
- **Each disk** is a `Disk`: a name, the device's `DeviceInfo`, the device
  in a `RequestQueue` behind a `SpinLock` (held only to enqueue, run the
  queue or collect a result, and taken by the disk's interrupt handler),
  and a `BufferCache` behind a sleeping `Mutex` (held across waits); lock
  order: cache, then queue. The tables of both and the cache's buffers come
  from the heap, once, and live as long as the kernel; the cache's buffers
  are one heap allocation, so the device can DMA into them.
- **Reads** (`read_block`, and `read` for several blocks, which reads the
  missing ones ahead in one batch so the queue merges them) copy out of the
  cache. **Writes** (`write_block`) go into the cache without reading the
  block first and reach the device when the block is evicted or on `sync`,
  which writes every dirty block and flushes the device's write cache; only
  then are they durable. `Disk::lock()` gives the rest (a `Cached`, the
  cache locked): pinned buffers (`get`, `data`, `release`), `invalidate`
  (drop clean blocks). Reads and writes that go past the cache
  (`Disk::read_uncached`, `write_uncached`) take only the queue lock.
- **Waiting** ([ADR 0018](adr/0018-msi-x-and-interrupt-driven-disk-io.md)).
  Every operation is synchronous for its caller, who waits for its ticket
  through `block::Io`, the kernel's `poly_block::IoQueue`: the hook the
  cache does its I/O through (a bare `RequestQueue` implements it by
  spinning). The waiter runs the queue once, which starts its request.
  On an interrupt-driven disk it then sleeps on the disk's `WaitQueue`
  until the completion counter moves; the interrupt handler runs the queue
  (collecting finished requests, starting waiting ones) and, if requests
  finished, bumps the counter and wakes the waiters, each of which collects
  its own result. Whoever finishes requests wakes the waiters, so a
  completion a waiter picked up instead of the handler is not lost. The
  sleep is a timed wait that ends at the watchdog's deadline at the latest,
  since a lost request raises no interrupt: the waiter then runs the queue
  and the watchdog fails the request. A polled disk (the RAM disk, which
  finishes requests as they are submitted; virtio-blk without MSI-X) runs
  the queue and yields between tries. Either way interrupts stay on: the
  tick keeps time and preempts threads during disk I/O, and other threads
  run while a reader waits. All threads share each disk's 32 request
  slots.

```text
block: virtio0: virtio-blk 00:03.0, 2048 blocks of 512 bytes (1 MiB), write cache; queue 32 requests, cache 128 buffers (64 KiB); interrupt-driven (vector 0x30)
block: ram0: RAM disk, 512 blocks of 512 bytes (256 KiB); queue 32 requests, cache 128 buffers (64 KiB); polled
block: virtio0: block 0: 50 4f 4c 59 00 00 00 00 08 09 0a 0b 0c 0d 0e 0f  POLY............
```

The kernel tests (`block::*`) run the same suite against `ram0` (filled
with the test pattern first) and, with `poly.test.disk`, `virtio0`: the
pattern read through the cache (first and last block, a multi-block read
that must be read ahead in merged requests), hit and miss counters, a write
that stays in the cache until `sync` and comes back from the disk after the
cache is dropped, dirty blocks written back when more blocks are written
than the cache holds, pinned blocks surviving a full cache, a read-only RAM
disk refusing writes, DMA into a heap buffer that crosses a page
boundary (a buffer on the boot stack is refused), and two threads reading
overlapping ranges at once. On `virtio0` they also check that a CPU-bound
thread runs while another thread does 40 reads (and the tick keeps
coming), that reads finish with MSI-X interrupts and sleeping waiters, and
on the spare device that a masked MSI-X entry holds its interrupt back
until unmasked and that a request whose notification was dropped
(`VirtioBlk::set_drop_notifications`) fails at the watchdog's deadline
while its reader sleeps. Each test puts the disks back as it found them.

## File systems (host-tested; not yet wired into the kernel)

Decisions are in [ADR 0023](adr/0023-vfs-tmpfs-and-cpio.md).

```
   personalities (openat, read, write, getdents64, ...)   kernel: console, exec
          │  FdTable (per process) → Arc<OpenFile> (offset, flags) → NodeRef
          ▼
   poly_vfs::Vfs   path resolution, permissions (Cred), mounts, char devices
          │  FileSystem trait: lookup/create/rename/read/write/readdir by inode
          ▼
   Tmpfs (in poly-vfs)        later: ext2, devfs, procfs
          ▲
   poly_cpio::unpack_fs       initramfs (newc / crc) → files, links, nodes
```

- **`poly-vfs`** (`no_std` + `alloc`): `Vfs` resolves paths as
  `path_resolution(7)` describes (`.`/`..` across mount points, 40 symbolic
  links then `ELOOP`, trailing slashes, `ENAMETOOLONG`), checks permissions
  from a `Cred`, and implements the path-based calls; `FdTable` and
  `OpenFile` implement descriptors (`dup2`/`dup3`, close-on-exec,
  `O_APPEND`, `lseek`, `getdents` positions). Errors are `Errno` with
  Linux's values; flags take Linux's bits.
- **tmpfs**: sparse files in 4 KiB pages, stable `readdir` positions,
  unlinked files and removed directories live while open.
- **`poly-cpio`**: a parser for `newc`/`crc` that never panics, a writer,
  and an unpacker into any file system through the VFS. Tested against
  archives made by GNU cpio (stored, and live if `cpio` is installed).

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
| Negative boot tests (faults, failures and panics must be reported, with a backtrace): unmapped access, write to text, execute stack/data, stack overflow | `poly.selftest.*` flags | `cargo xtask test` |
| Device tests against QEMU's devices: PCI config space, MSI on `edu`, the block layer on the virtio-blk test image (interrupt-driven, and in another boot polled) and the RAM disk, the watchdog on a spare virtio-blk device (and a boot that expects the disk without one must fail) | `kernel_test!` + `poly.test.disk`, `poly.test.msi` | `cargo xtask test` |
| Conformance vs. real OS (diff runner) | `specs/` + oracle VMs | later |
| App trace replay | recorded sessions | later |

Backtraces, symbolisation and attaching gdb/lldb are described in
[DEBUGGING.md](DEBUGGING.md).

## Memory layout (current)

The kernel page tables (`kernel/src/arch/x86_64/paging.rs`, decisions in
[ADR 0010](adr/0010-kernel-address-space.md)) replace the boot stub's
mapping early in `kernel_entry`:

| Virtual | Physical | What |
|---|---|---|
| `0x0` – `0x7fff_ffff_ffff` | – | nothing (user space later); the boot identity map is gone |
| `0xffff_8000_0000_0000` + phys | every region of the memory map, up to 64 TiB | direct map: RW, NX; RAM write-back, reserved ranges uncached; holes unmapped |
| `0xffff_c000_0000_0000` – `+512 GiB` | device ranges | `mm::map_mmio`: uncached (PCD+PWT), NX, a gap page after each; PCI ECAM windows and BARs |
| `0xffff_ffff_8000_0000` + phys | kernel image | `.text` R+X, `.rodata`/`.kernel_tests` R, `.data`/`.bss` RW+NX, stacks RW+NX |

Kernel image detail (`kernel/linker.ld`, every boundary page-aligned):

| Part | Mapped | Note |
|---|---|---|
| `.boot`, `.boot.bss` (physical 1 MiB) | direct map only | 32-bit entry stub and its page tables, unused after boot |
| `__text_start` – `__text_end` | read-only, executable | also read-only through the direct map |
| `__rodata_start` – `__rodata_end` | read-only, no-execute | includes `.kernel_tests` |
| `__data_start` – `__data_end` | read-write, no-execute | `.data`, `.bss` |
| `__boot_stack_guard` (4 KiB) | **unmapped** | an overflow faults, reported as a double fault |
| boot stack (64 KiB) | read-write, no-execute | |
| `__double_fault_stack_guard` (4 KiB) | **unmapped** | |
| double-fault IST stack (16 KiB) | read-write, no-execute | |

All kernel mappings are global and use 2 MiB pages (1 GiB where the CPU has
them) wherever alignment allows. No-execute needs EFER.NXE, which is enabled
when CPUID reports NX (without it, pages stay executable and the boot log
says `NX: no`). `arch::phys_to_virt(phys)` translates through the direct map and returns
`None` for addresses outside it; `arch::phys_range_to_virt` checks a whole
range. Before the switch it uses the boot mapping, which is why the first page
tables come from a bump allocator over RAM below 1 GiB, before the buddy
allocator exists. Per-process address spaces come in Phase 2.

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
- `kernel/src/frames.rs` carves that buffer from usable RAM, reached through
  the direct map, marks it `Kernel` in the map, and puts the allocator in the
  global `frames::FRAMES` spinlock. Every frame it hands out is reachable with
  `arch::phys_to_virt`. `cargo xtask test` boots with 256 MiB and with 2 GiB,
  so RAM and ACPI tables above 1 GiB are covered too.

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
  `arch::phys_to_virt` (the direct map). One allocation is at most one buddy
  block (4 MiB).
- Lock order is heap, then `FRAMES`: never allocate while holding `FRAMES`.
  Both locks mask interrupts while held (the heap through
  `sync::RawIrqSpinLock`), so an interrupt handler may allocate.
- Out of memory: fallible APIs (`try_reserve`) see the error; everything
  else panics with "memory allocation of N bytes failed" and a backtrace
  (stable Rust's default handler in `no_std`), checked by a boot test.
  Boot prints the heap's numbers after the self-test.

## Interrupts and time

Single CPU (the application processors stay parked). The controllers and
the arithmetic are in `poly-irq`
([ADR 0013](adr/0013-interrupt-controllers-and-time-base.md)); the kernel
wires them up ([ADR 0016](adr/0016-interrupt-dispatch-tick-and-kernel-locks.md)).

- **Bring-up** (`arch::init_interrupts`, after the heap, `thread::init` and
  ACPI, with interrupts off): CPUID features; the 8259s remapped to
  0x20-0x2f and fully masked; the local APIC in x2APIC mode when the CPU has
  it, else xAPIC through `map_mmio` (every LVT entry masked except the error
  entry and the LINT inputs the MADT wires to NMI; spurious vector 0xff);
  every I/O APIC from the MADT mapped with all inputs masked, ISA routes
  from the interrupt source overrides; the HPET mapped, its timers stopped,
  started (the ACPI PM timer if there is no HPET); the TSC and the local
  APIC timer calibrated against it; the monotonic clock; the APIC timer
  started periodic at `TICK_HZ` = 1000 on vector 0xf0. Then `kernel_entry`
  enables interrupts.
- **Hardware access** for `poly-irq`: `arch/x86_64/mmio.rs` (`MmioRegion`,
  the kernel's one `Mmio` implementation, over `map_mmio`, shared with PCI and
  virtio), `apic::CpuMsr` (only the APIC's MSRs) and `apic::CpuPorts` (only
  the 8259's ports).
- **Vectors:** 0x00-0x1f exceptions, 0x20-0x2f the masked 8259s (only a
  spurious IRQ 7/15 can arrive), 0x30-0xef devices, 0xf0 the tick,
  0xf1-0xfd reserved for IPIs, 0xfe APIC error, 0xff APIC spurious (never an
  EOI). Every vector has an interrupt-gate stub in `idt.rs` pushing the same
  `TrapFrame`; `arch::irq::dispatch` handles 32-255.
- **Drivers:** `arch::irq::allocate_vector(handler, data)` claims a device
  vector (lock-free; `free_vector` releases it); the handler runs as
  `handler(data)` with interrupts off, before the EOI. Point a device at the
  vector with `arch::irq::msi_message(vector)` (address / data for MSI or
  MSI-X, targeting the boot CPU) or route an I/O APIC input with
  `route_gsi` / `route_isa_irq` (undo with `unroute_gsi`). PCI functions do
  all of it through `pci::Device::enable_msix` / `enable_msi` /
  `enable_interrupt` (see "Devices"). Handlers must not block; they may
  wake a `WaitQueue`.
- **Time:** `time::now_ns()` is nanoseconds since boot from
  `poly_irq::MonotonicClock`: the TSC when it is invariant and calibrated,
  otherwise the reference timer (under QEMU's TCG: the HPET). It never goes
  backwards, also across threads. `time::ticks()` counts tick interrupts.
  The boot log prints the APIC mode, the HPET, the TSC rate, the clock
  source and the APIC timer rate.

## Threads and scheduling

Kernel threads only, on one CPU, round robin, preempted by the timer. See
[ADR 0012](adr/0012-scheduler-policy-in-a-crate-context-switch-in-the-kernel.md)
and [ADR 0016](adr/0016-interrupt-dispatch-tick-and-kernel-locks.md).

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
  idle thread, which runs only when nothing else can. It halts with
  interrupts enabled (`sti; hlt`, atomic) until an interrupt makes a thread
  runnable. Before interrupts are set up nothing could, so then it panics
  with a "deadlock" message and a backtrace instead of hanging.
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
  `join(id)` (returns the exit code), `detach(id)`, `state(id)`,
  `sleep(duration)` / `sleep_until(ns)`, and
  `WaitQueue::{wait_until(cond), wait_until_deadline(cond, ns), wake_one(),
  wake_all()}`. `wait_until` checks its condition with the scheduler
  locked, so a wakeup between the check and going to sleep cannot be lost.
  `wait_until_deadline` also gives up at a deadline: the thread table keeps
  one (deadline, queue) entry per thread in a timed wait, and the tick wakes
  the queue of each one that is due (the other waiters on it check their
  conditions and sleep again).
- **Exit.** An exiting thread is still on its stack, so the next thread
  frees it (`finish_switch` after every switch). The table slot is freed
  once the thread is also joined or detached.
- **Interrupts and locking.** Every `THREADS` critical section runs with
  interrupts off, and the lock is dropped before the switch; with one CPU
  and interrupts off nothing can run in between. Each thread restores its
  own interrupt state when the switch returns into it; new threads start
  with interrupts enabled. `switch` panics if the thread holds a spinlock
  or is inside an interrupt handler. SMP needs the lock (or an "on CPU"
  flag) held until the outgoing thread's registers are saved.
- **Preemption.** The tick handler counts the tick, sends the EOI and calls
  `thread::on_timer_tick()`: it wakes sleepers whose deadline has passed
  (the earliest deadline is kept in an atomic, so most ticks do nothing
  there) and calls `preempt_tick()`, which switches once the running thread
  has used its 10-tick slice and another is runnable. This runs on the
  interrupted thread's own stack (no IST), since the handler's frames stay
  there while the thread is switched out.

## Synchronisation

`kernel/src/sync.rs`:

- `SpinLock<T>` busy-waits and keeps interrupts disabled from before it is
  taken until after it is released (restoring the previous state), so an
  interrupt handler can never deadlock on a lock held by the code it
  interrupted, and holders are never preempted. `RawIrqSpinLock` is the same
  for the heap. Critical sections must not block; the scheduler checks.
- `Mutex<T>` sleeps: a thread that finds it held blocks on a `WaitQueue`
  and is woken when it is released. It may be held across blocking calls,
  not taken in interrupt handlers, and locking it twice from one thread
  panics with "deadlock".
- `SetOnce<T>`: written once at boot, read without a lock (local APIC,
  clock).


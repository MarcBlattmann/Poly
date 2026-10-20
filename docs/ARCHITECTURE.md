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
            shared core: boot, cpu (GDT/TSS/IDT), memory map, frames, heap
            devices: PCI (ECAM), virtio-blk (polled)
            arch/x86_64 (PVH entry, serial, exceptions, page tables)
```

## Boot flow

1. QEMU (`-kernel`) reads the PVH ELF note and jumps to `pvh_start32` in
   32-bit mode with `ebx` pointing at `hvm_start_info`.
2. `arch/x86_64/boot.rs` checks for long mode, maps physical [0, 1 GiB) at
   0 and at `0xffffffff80000000`, enables paging and jumps to `kernel_entry`.
3. `kernel_entry` brings up serial, loads our GDT/TSS/IDT, reads the boot info,
   normalises the memory map (reserving the first MiB and the kernel image),
   switches to the kernel page tables (`mm::init`; the boot mapping is gone
   after this), hands all usable RAM to the frame allocator, reads the ACPI
   tables, enumerates PCI, brings up the virtio-blk disk if there is one and
   runs the boot self-test.
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
  returns an `MmioRegion`, the kernel's `poly_pci::Mmio`: bounds- and
  alignment-checked volatile accesses, with `subregion` for the virtio
  structures inside a BAR. Drivers enable memory decoding and bus mastering
  themselves (`poly_pci::enable`).
- **Interrupts.** None yet: `Device::msix()` parses and checks a function's
  MSI-X capability so drivers can log it, but nothing enables MSI-X or INTx.
  The steps to switch virtio-blk to MSI-X are in `kernel/src/virtio_blk/mod.rs`.
- **DMA.** `kernel/src/dma.rs`: `DmaBuffer` is a zeroed block of frames from
  `FRAMES`, used through the direct map. There is no IOMMU, so the bus
  address a device gets is the physical address.
- **virtio-blk** (`kernel/src/virtio_blk/`): finds the first virtio block
  function, discovers its structures (`poly_virtio::PciTransport`), maps
  them, resets it and negotiates features (VERSION_1 required; RO,
  BLK_SIZE, FLUSH and ACCESS_PLATFORM accepted), reads capacity and block
  size, sets up the request queue (16 entries) in a `DmaBuffer` and sets
  DRIVER_OK. `read`, `write` and `flush` send one request at a time through
  a one-page bounce buffer and poll the used ring with a spin budget; a lost
  request or a protocol error resets the device and stops the driver. The
  request format (header, status, feature bits, configuration offsets) is in
  `virtio_blk/protocol.rs`, apart from the transport. Boot reads sector 0
  and prints its first bytes.

```text
pci: segment 0 buses 00-ff: ECAM at 0xb0000000, mapped at 0xffffc00000000000
pci: 00:03.0 1af4:1042 class 01.00.00, BAR1 mem32 0xfebd5000 4 KiB, BAR4 mem64 pf 0xfe000000 16 KiB
virtio-blk: 00:03.0: MSI-X with 3 vectors, table BAR1+0x0, PBA BAR1+0x800; left disabled, polling
virtio-blk: 00:03.0: 1 MiB (2048 sectors), block size 512, features 0x100000240 of 0x10130007e54, queue 16 entries of 256
virtio-blk: sector 0: 50 4f 4c 59 00 00 00 00 08 09 0a 0b 0c 0d 0e 0f  POLY............
```

`cargo xtask` writes a fresh 1 MiB test image (`target/poly-test-disk.img`,
every 512-byte sector starting with "POLY" and its number) before each boot
and attaches it as `-device virtio-blk-pci,disable-legacy=on` with the
kernel flag `poly.test.disk`. With the flag, the kernel tests require the
disk, check the pattern and write, flush and read back a sector (restoring
it). Without it they check only what is present, so the kernel boots
without a disk too (`cargo xtask run --no-disk`).

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
| Device tests against QEMU's devices: PCI config space, virtio-blk read/write/flush on a test image (and a boot that expects the disk without one must fail) | `kernel_test!` + `poly.test.disk` | `cargo xtask test` |
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
  Neither lock masks interrupts yet.
- Out of memory: fallible APIs (`try_reserve`) see the error; everything
  else panics with "memory allocation of N bytes failed" and a backtrace
  (stable Rust's default handler in `no_std`), checked by a boot test.
  Boot prints the heap's numbers after the self-test.

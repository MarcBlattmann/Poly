# 0015: PCI and virtio in the kernel: ECAM only, a boot-time device list, DMA from the frame allocator, polling first

**Status:** Accepted

## Context

`poly-pci` and `poly-virtio` hold the protocol logic for PCI and virtio
([ADR 0009](0009-drivers-behind-access-traits.md)); the kernel has to give
them hardware access and drive a first device. The first device is a disk
(virtio-blk), because everything after Phase 1 needs storage. Interrupts are
not available yet: there is no local APIC or I/O APIC driver, and a parallel
effort builds the timer and APIC code. Neither is there an IOMMU driver.

## Decision

**Configuration space through ECAM only.** The kernel reads the ECAM windows
from the ACPI MCFG and maps each with `mm::map_mmio` (uncached). The MCFG
base address is where bus 0 would be, also when a window starts at a higher
bus (PCI Firmware Specification 3.2, §4.1.2), so it goes into
`poly_pci::Ecam` unchanged and the window starts `start_bus` MiB above it.
`kernel/src/pci.rs` implements `ConfigSpace` with volatile accesses exactly
as wide as asked; misaligned or out-of-window accesses read all ones and
drop writes. The legacy port mechanism (0xcf8/0xcfc) is not implemented:
every PCI Express machine has ECAM, and QEMU's `q35` does.

**One enumeration at boot, a device list after it.** `pci::init` probes every
bus each window decodes (`poly_pci::scan_all`; root buses that no bridge
points at, such as those of extra host bridges, are found without parsing
AML), sizes every BAR and keeps the functions in a list that lives as long as
the kernel. Drivers look their device up there. Host and ISA/LPC bridges are
not BAR-probed: sizing turns decoding off for a moment, and those bridges
decode RAM, the ECAM window, the serial port and ACPI registers. Memory BARs
are mapped on demand by `Device::map_bar` (once per BAR), which hands out an
`MmioRegion` implementing `poly_pci::Mmio` with bounds checks. Configuration
space is not locked: enumeration runs once on the boot CPU, and afterwards
each driver only touches its own functions.

**DMA memory comes from the frame allocator.** `kernel/src/dma.rs` hands out
zeroed, physically contiguous blocks of frames from `FRAMES`, used by the
CPU through the direct map. Without an IOMMU, bus address equals physical
address; virtio's ACCESS_PLATFORM is accepted when offered because nothing
translates. x86-64 DMA snoops the caches, so write-back memory needs no
flushing; ordering is the virtqueue code's job (its fences).

**Polling first.** The virtio-blk driver (`kernel/src/virtio_blk/`) sends one
request at a time and spins on the used ring with a fixed poll budget
(about 5 seconds under TCG), because there is no timer. On a timeout or a
protocol violation it resets the device, which is the only way to take the
in-flight buffers back, and refuses all later requests. INTx is disabled in
the command register, MSI-X is parsed and logged but left disabled, and no
queue gets an MSI-X vector. The driver's module documentation lists the
steps to switch to MSI-X once interrupts exist.

**The virtio-blk wire format is its own module.** Request header, status
byte, feature bits and configuration offsets live in
`kernel/src/virtio_blk/protocol.rs`, so a shared virtio-blk library can
replace them without touching transport and queue handling.

**The disk is tested in every boot test.** `cargo xtask` writes a fresh
1 MiB raw image with a known pattern into `target/` before each boot and
attaches it as a modern-only `virtio-blk-pci`, passing `poly.test.disk`.
With that flag the kernel tests require the device, check the pattern,
and write, flush and read back a sector (restoring it afterwards). Without
the flag they only check what is there, so the kernel still boots and passes
without a disk. A boot test sets the flag without attaching the disk and
expects the failure.

## Consequences

- PCI on machines without an MCFG (QEMU's `pc`, very old hardware) is
  unavailable until a port-I/O `ConfigSpace` is added; nothing else changes.
- Hot-plug is not handled: the list is fixed at boot.
- `map_mmio` never unmaps, so BAR mappings are permanent; mapping each BAR
  once keeps that from growing.
- Requests block the CPU while the device works. The interrupt-driven path
  needs the APIC driver, an IDT vector allocator and a way to sleep; until
  then a disk request costs a busy CPU for its duration.
- Data goes through a one-page bounce buffer, an extra copy per request.
  Passing caller buffers directly needs a virtual-to-physical translation
  for heap memory and pinning rules; worth it once throughput matters.
- When an IOMMU driver arrives, `dma.rs` is where bus addresses stop being
  physical addresses.

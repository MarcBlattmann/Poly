# 0010: Kernel address space: a direct map of the memory map's regions, W^X, guard pages

**Status:** Accepted. Amends [0007](0007-buddy-allocator-out-of-band-metadata.md):
the buddy allocator's metadata may now live in any usable RAM.

## Context

The boot stub maps physical [0, 1 GiB) twice, at 0 and at the kernel's link
address, with 2 MiB pages that are writable and executable. That cannot reach
RAM or ACPI tables above 1 GiB (QEMU puts its tables there once the guest has
more than 1 GiB), leaves the lower half occupied, lets the kernel write its
own code, execute its stack, and run off the end of a stack into whatever lies
below it. Drivers for the APIC, HPET and PCI need uncached mappings of device
memory, which the boot mapping does not offer (it does not even cover
0xfee00000).

## Decision

**Layout** (x86-64, 48-bit addresses; `arch/x86_64/paging.rs` has the table):
the lower half stays empty for user space; the direct map starts at
`0xffff_8000_0000_0000` (PML4 slot 256, up to 64 TiB of physical memory); an
MMIO window of 512 GiB starts at `0xffff_c000_0000_0000`; the kernel image
stays at its link address `0xffff_ffff_8000_0000` + phys. All of it is
global.

**The direct map covers the regions of the memory map, not the whole physical
address space.** Holes are not mapped, so mapping a large reserved range far
above RAM (QEMU reserves 12 GiB just below 1 TiB) costs a few page tables,
not one per GiB of address space, and `phys_to_virt` returns `None` for a
hole instead of an address that faults. `poly_mem::DirectMap` (host-tested)
turns the normalized map into page-aligned spans; when rounding makes two
regions share a page, the more restrictive kind gets it. RAM kinds (usable,
kernel, ACPI) are mapped write-back; reserved ranges may be device memory and
are mapped uncached (PCD|PWT, UC with the power-on PAT), like `map_mmio`, so
no cached alias of device memory exists. Pages are as large as the CPU allows
(1 GiB only with CPUID Page1GB, which QEMU's default CPU lacks; else 2 MiB).

**W^X.** The linker script page-aligns and names the boundaries of text,
read-only data and data. Text is read-only and executable, everything else is
no-execute (IA32_EFER.NXE, enabled when CPUID reports NX), and nothing is
both writable and executable. The direct-map alias of text and read-only data
is read-only too. CR0.WP (set by the boot stub) makes read-only pages
binding in ring 0 too.

**Guard pages.** Each kernel stack (boot, double fault) is its own page-aligned
section with an unmapped page below it. An overflow faults; the page fault
cannot be delivered on the exhausted stack, so it becomes a double fault on
the IST stack, and the exception handler names the stack whose guard page CR2
or RSP points into.

**Bootstrapping.** The new tables are built while the boot tables are loaded,
so their frames must be reachable through the boot mapping. `paging::init`
runs before the buddy allocator and takes frames from a bump allocator over
usable RAM below 1 GiB, then returns the memory map with those frames marked
`Kernel`. `phys_to_virt` uses the boot mapping until CR3 is switched and the
direct map afterwards; page-table memory is always reached through it. The
buddy allocator is built after the switch, so its metadata is reached through
the direct map from the start and can come from any usable RAM.

**Device memory.** `mm::map_mmio(phys, len)` maps a range uncached and
no-execute in the MMIO window, with an unmapped page after each mapping. It
refuses ranges that overlap RAM, since an uncached alias of write-back memory
breaks coherence. Mappings are not removed yet.

## Consequences

- `phys_to_virt` looks the address up in up to 128 sorted spans (a binary
  search) instead of a bounds check.
- Code that kept a pointer from the boot mapping across `mm::init` would
  dangle; `kernel_entry` only copies boot information before that point.
- The boot stub's code and its four page tables (16 KiB) stay in the kernel
  image region, unused.
- Kernel-mode writes through the direct map to free frames are possible by
  design; only the image's code is protected from writes.
- Per-process address spaces will copy the kernel half of the PML4 (slots
  256-511); because those entries point at shared tables, kernel mappings
  added later must not need new PML4 entries, or must be propagated.

# 0002: Boot via PVH under QEMU first

**Status:** Accepted

## Context

The roadmap develops entirely in QEMU before supporting one real laptop. We
want fast, deterministic boots in CI with no third-party binaries.

## Decision

The kernel is an ELF with a `XEN_ELFNOTE_PHYS32_ENTRY` note and boots through
the PVH direct-boot ABI (`qemu-system-x86_64 -kernel`). Our own ~100-line stub
enters long mode. No bootloader, no disk image, no ISO tooling.

Everything protocol-specific is confined to `arch/x86_64/boot.rs` and
`boot_info.rs`, which produce a protocol-neutral `BootInfo`.

## Consequences

- A boot test takes about a second, with nothing to download.
- The same ELF also boots under Firecracker and Cloud Hypervisor.
- Real hardware needs UEFI. That comes with roadmap step 7 as a second
  producer of `BootInfo` (a small UEFI loader of our own, or Limine). Nothing
  above `BootInfo` changes.

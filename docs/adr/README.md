# Architecture decision records

One file per decision: context, decision, consequences. Never rewrite an
accepted ADR. Supersede it with a new one and link both ways.

| # | Decision | Status |
|---|---|---|
| [0001](0001-multiple-personalities-one-kernel.md) | Three personalities on one Rust kernel, chosen by binary format | Accepted |
| [0002](0002-pvh-boot-first.md) | Boot via PVH under QEMU first; UEFI later | Accepted |
| [0003](0003-workspace-layout.md) | Host-testable crates + thin kernel, kernel in its own workspace | Accepted |
| [0004](0004-coverage-map-format.md) | `coverage.json` format, seeded from public name lists | Accepted |
| [0005](0005-license.md) | Apache-2.0 | Accepted |
| [0006](0006-coverage-map-one-file-per-component.md) | Coverage map stored as one file per component | Accepted |
| [0007](0007-buddy-allocator-out-of-band-metadata.md) | Buddy frame allocator with its metadata outside the memory it manages | Accepted |
| [0008](0008-in-kernel-tests-and-backtraces.md) | In-kernel tests in a linker section; frame-pointer backtraces symbolised on the host | Accepted |
| [0009](0009-drivers-behind-access-traits.md) | Device drivers as host-tested crates behind access traits | Accepted |
| [0010](0010-kernel-address-space.md) | Kernel address space: direct map of the memory map's regions, W^X, guard pages | Accepted |
| [0012](0012-scheduler-policy-in-a-crate-context-switch-in-the-kernel.md) | Scheduler policy in a host-tested crate; context switch and stacks in the kernel | Accepted |
| [0013](0013-interrupt-controllers-and-time-base.md) | Interrupt controllers and the time base in one host-tested crate (`poly-irq`) | Accepted |
| [0014](0014-block-layer.md) | Block layer: submit/poll devices, ordering request queue, pinning buffer cache | Accepted |
| [0015](0015-pci-and-virtio-in-the-kernel.md) | PCI and virtio in the kernel: ECAM only, a boot-time device list, DMA from the frame allocator, polling first | Accepted (virtio-blk driver parts superseded by 0017) |
| [0017](0017-block-layer-in-the-kernel.md) | The block layer in the kernel: poly-virtio-blk on direct-map DMA, a watchdog, a queue and a cache per disk | Accepted (waiting and watchdog superseded by 0018) |
| [0016](0016-interrupt-dispatch-tick-and-kernel-locks.md) | Interrupt dispatch and vector allocation, a 1 kHz periodic tick, spinlocks that mask interrupts, a sleeping mutex | Accepted |
| [0018](0018-msi-x-and-interrupt-driven-disk-io.md) | MSI and MSI-X for PCI functions, interrupt-driven disks with sleeping waiters, a watchdog on the clock | Accepted |
| [0021](0021-kernel-shell.md) | The kernel shell: a host-tested line editor on `alloc`, a `Console` trait, serial input on IRQ 4 | Accepted (amends 0003: shared crates may use `alloc`) |

## Open decisions

These are also marked ⚖️ in [ROADMAP.md](../ROADMAP.md) at the point where they block work.

- **Global menu bar vs. per-window menus.** Needed before compositor work (roadmap Phase 6).
- **Window frames:** native per app (macOS traffic lights, Windows frames) or one neutral frame.
- **MSVC / DirectX redistributables:** 62% of Windows entries are versioned C++
  runtime DLLs (`msvcp*`, `msvcr*`, `vcruntime*`) that apps normally ship
  themselves. Decide whether we reimplement them or run the app's own copies.
- **aarch64:** port the kernel to 64-bit ARM, or stay x86-64 only. Modern Mac apps
  are increasingly arm64-only, so this decides how much of macOS is reachable.

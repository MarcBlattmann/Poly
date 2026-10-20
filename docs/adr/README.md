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
| [0022](0022-user-mode-address-spaces-and-system-calls.md) | User mode: one PML4 per process sharing the kernel half, PCIDs, syscall/SYSRET with an IRETQ fallback, user copies through an exception table, eager FPU switching | Accepted |
| [0024](0024-elf-loader.md) | ELF loader in a host-tested crate: validated headers, page mapping plans, the psABI initial stack | Accepted |
| [0026](0026-user-address-space-regions.md) | User address spaces: a region map in a host-tested crate (`poly-vm`), page state in the page tables | Accepted |
| [0036](0036-demand-paging-and-exec.md) | Demand paging and exec: a per-process memory map, faults resolved against it (user copies too), ELF segments mapped lazily through file windows | Accepted |
| [0025](0025-linux-syscall-table-and-kernel-services-trait.md) | Linux system calls: a generated table, man-page handlers, a kernel-services trait | Accepted |
| [0020](0020-ps2-keyboard-and-layouts.md) | PS/2 keyboard: decoding and layouts in `poly-input`, an i8042 driver on IRQ 1 with a byte ring | Accepted |
| [0021](0021-kernel-shell.md) | The kernel shell: a host-tested line editor on `alloc`, a `Console` trait, serial input on IRQ 4 | Accepted (amends 0003: shared crates may use `alloc`) |
| [0027](0027-agent-pipeline-tools-and-test-run-protocol.md) | Agent pipeline tools: own ptrace tracer, per-test diff runner, a serial test-run protocol for Poly, held-out store outside the repository | Accepted |
| [0023](0023-vfs-tmpfs-and-cpio.md) | The VFS in a host-tested crate with `alloc`, tmpfs beside it, cpio unpacked through the VFS | Accepted |
| [0030](0030-nt-object-manager-and-dispatcher-in-a-crate.md) | NT object manager and dispatcher logic in a host-tested crate (`poly-ntobj`) | Accepted |
| [0029](0029-pe-loader-logic-in-poly-pe.md) | PE loader logic in a host-tested crate (`poly-pe`), working on mapped images in place | Accepted |
| [0019](0019-display-framebuffer-and-drawing.md) | The display: Bochs VBE mode setting, a write-combining framebuffer behind a RAM back buffer, a drawing library and text grid over caller memory | Accepted |
| [0028](0028-tcp-ip-stack-in-a-crate.md) | Our own TCP/IP stack in `poly-net`: RFC-based, host-tested in a simulated network, driven by `poll(now)`, Linux errno | Accepted |
| [0031](0031-linux-signals-and-futexes.md) | Linux signals and futexes: personality-owned state, kernel-provided blocking | Accepted |
| [0035](0035-screen-terminal.md) | The screen terminal: a status bar and a text grid on the display, a second shell on the keyboard, the kernel log mirrored through a ring (`poly-term`) | Accepted |
| [0037](0037-vfs-and-initramfs-in-the-kernel.md) | The VFS in the kernel: one global `Vfs` on a tmpfs root, the initramfs as PVH module 0 held by the frame allocator until unpacked, per-process `Files` behind sleeping mutexes, console reads at end of file | Accepted |
| [0032](0032-ext2-driver.md) | ext2 in a host-tested crate over a block access trait, metadata ordered by flushes instead of a journal | Accepted (volume lock, page cache and writeback: see 0040) |
| [0038](0038-linux-processes-in-the-kernel-and-the-first-program.md) | Linux processes in the kernel: `LinuxKernel` per running thread, threads registered with their process and ended at their next return to user mode, signals on every way out (faults included) through the trap frame, programs exec'd from the VFS, `poly.init` and the ADR 0027 records, the CMOS RTC as wall clock | Accepted |
| [0039](0039-networking-in-the-kernel.md) | Networking in the kernel: virtio-net in a host-tested crate (`poly-virtio-net`), one stack behind a sleeping mutex, a network thread, readiness generations for blocking sockets | Accepted |
| [0040](0040-page-cache-and-ext2-on-virtio-blk.md) | A page cache in a host-tested crate (`poly-pagecache`) wrapping disk file systems, shared with memory mappings; ext2 mounted on virtio-blk with a sleeping volume lock, a writeback thread, clean unmount at reboot | Accepted |
| [0041](0041-processes-threads-fork-exec-wait.md) | Processes and threads: copy-on-write fork with `Arc`-counted shared frames, clone and execve in the kernel, the process tree (parents, zombies, groups, sessions, wait) in the Linux personality | Accepted |
| [0042](0042-readiness-pipes-poll-epoll-and-event-files.md) | Readiness: wait queues, file objects (pipes, event counters, timers, epoll sets) and `poll` in `poly-vfs`, the wait and signal rules in the personality (`FileIo`), park / unpark and a timer thread in the kernel | Accepted |
| [0043](0043-dev-proc-sys-and-file-system-calls.md) | `/dev`, `/proc` and `/sys`: a ChaCha20 fast-key-erasure generator (`poly-random`), procfs over a kernel-provided source and a static sysfs (`poly-procfs`), open files that know their names; the Linux file system calls behind one `LinuxFs` services trait, resource limits in the personality | Accepted |
| [0044](0044-time-syscalls-timers-and-the-vdso.md) | Time system calls, process timers (state in the personality, one kernel timer thread), CPU time per thread measured at context switches and split by tick samples, a settable wall clock, and a vDSO built from our own Rust by `kernel/build.rs` reading a seqlock data page (`poly-vdso`) shared through the page cache | Accepted |
| [0045](0045-credentials-and-capabilities.md) | Credentials and capabilities: per-thread `Creds` (ids, groups, capability sets) in the Linux personality, capability-based permission checks in the VFS, execve's set-user-ID and capability transformation | Accepted |
| [0046](0046-terminals-line-discipline-ptys-job-control.md) | Terminals: the line discipline, terminals with job control, ptys and devpts in a host-tested crate (`poly-tty`) on a kernel `Host`; a device may open as another (`CharDevice::open`); a device's input goes to its terminal while a program has it open, else to the kernel shell | Accepted |
| [0048](0048-sockets-inet-unix-netlink.md) | Sockets: socket files over the stack with per-socket wake-ups (`Stack::take_changed`), Unix sockets with names, credentials and descriptor passing in `poly-unix` (in-flight cycles collected), `NETLINK_ROUTE` dumps in the personality, the socket calls behind one `LinuxNet` services trait | Accepted |

## Open decisions

These are also marked ⚖️ in [ROADMAP.md](../ROADMAP.md) at the point where they block work.

- **Global menu bar vs. per-window menus.** Needed before compositor work (roadmap Phase 6).
- **Window frames:** native per app (macOS traffic lights, Windows frames) or one neutral frame.
- **MSVC / DirectX redistributables:** 62% of Windows entries are versioned C++
  runtime DLLs (`msvcp*`, `msvcr*`, `vcruntime*`) that apps normally ship
  themselves. Decide whether we reimplement them or run the app's own copies.
- **aarch64:** port the kernel to 64-bit ARM, or stay x86-64 only. Modern Mac apps
  are increasingly arm64-only, so this decides how much of macOS is reachable.

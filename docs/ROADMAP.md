# Roadmap

Everything Poly needs, in the order it has to be built. Each milestone only
depends on milestones above it. Tick a box (`- [x]`) when the milestone is
merged with tests; `cargo xtask coverage readme` turns this file into the
progress chart in the README.

**Rules for this file**
- One line per milestone, written so a reviewer can say "done" or "not done".
- Phases are `## Phase N: Title`; milestones are `- [ ]` / `- [x]` lines.
- Add milestones where they belong in the order, not at the end.
- ⚖️ marks a decision the project owner must make before that milestone starts.

---

## Phase 0: Foundation

- [x] Repository layout, pinned toolchain, `cargo xtask` build/run/test driver
- [x] Kernel boots in QEMU via PVH, enters 64-bit long mode, higher-half kernel
- [x] Serial console logging, panic handler, QEMU exit codes for automated tests
- [x] GDT, TSS (double-fault stack), IDT with handlers for all CPU exceptions
- [x] Boot info parsing and normalised physical memory map, early frame allocator
- [x] Binary format detection (ELF / PE / Mach-O) and personality tag per process
- [x] System call routing to Linux / NT / XNU personalities (native error conventions)
- [x] Boot self-test on every boot, plus negative test (page fault is reported)
- [x] Coverage map seeded from Wine `.spec`, Linux syscall table, XNU syscall/trap tables
- [x] macOS framework API mapped from the SDK `.tbd` files (owner runs `tools/scripts/import-macos-sdk.sh` on a Mac)
- [x] Clean-room rules, architecture docs, ADRs, spec sheet template, CI

## Phase 1: Kernel core

The machinery every personality needs. Single CPU, kernel threads only.

- [x] Physical frame allocator for all RAM (buddy allocator), host-tested
- [x] Kernel page tables: direct map of all physical memory, drop the boot identity map
- [x] W^X for the kernel: text read-only + executable, data non-executable, guard pages
- [x] Kernel heap (slab + large allocations) so the kernel can use `alloc` (`Box`, `Vec`)
- [x] In-kernel test harness: unit tests that run inside QEMU and report per test
- [x] Symbolised kernel backtraces on panic; documented GDB workflow (`qemu -s`)
- [x] ACPI table parsing (RSDP, MADT, HPET, FADT); no AML interpreter yet
- [x] Interrupt controllers: local APIC + I/O APIC, legacy PIC masked
- [x] Time: TSC calibrated against HPET, monotonic clock, APIC timer interrupts
- [x] Kernel threads, context switching, preemptive round-robin scheduler
- [x] Synchronisation: interrupt-safe spinlocks, mutexes, wait queues, sleep/wakeup
- [x] PCI/PCIe enumeration (ECAM), MSI/MSI-X interrupts
- [x] virtio core (virtqueues) and virtio-blk driver: first real disk I/O
- [x] Block layer with request queue and buffer cache

## Phase 2: Kernel terminal

A terminal built into the kernel: a debug console on screen until user-mode
programs (and later the real desktop) exist. The drivers stay for good.

- [x] Display: QEMU standard VGA (Bochs VBE) linear framebuffer driver and mode setting
- [x] Drawing library: pixels, rectangles, a permissively licensed bitmap font, scrolling text grid
- [x] Keyboard: PS/2 (i8042) driver on interrupts, scancode decoding, US and Swiss German layouts
- [x] Kernel shell: line editor with history and completion, commands (help, mem, ps, lspci, disks, uptime, test, clear, reboot)
- [x] **Milestone:** the kernel boots into a full-screen terminal with a status bar, usable with the keyboard

## Phase 3: User space and the first Linux program

- [x] Per-process address spaces, switching on context switch (PCID where available)
- [x] User mode (ring 3) and the `syscall`/`sysret` entry path with per-CPU kernel stacks
- [x] Safe user-memory access (`copy_from_user` / `copy_to_user` with fault recovery)
- [x] Virtual memory areas, demand paging, page-fault handler for user space
- [x] VFS core: inodes, dentries, mounts, file descriptor tables, path lookup
- [x] Initial RAM disk (cpio) mounted as the root file system; tmpfs
- [x] ELF loader for static binaries: segments, initial stack, auxiliary vector
- [x] First Linux syscalls: `write`, `exit_group`, `brk`, `mmap`, `munmap`, `arch_prctl`
- [x] **Milestone:** a static Linux "hello world" runs and exits cleanly
- [ ] Agent pipeline v0: Linux syscall tracer on a real Linux VM, spec agent, implementer agent
- [x] Diff runner v0: same tests on real Linux and on Poly, results compared per test
- [x] Held-out test store outside this repo; hand-off logging in `specs/HANDOFF_LOG.md`

## Phase 4: Linux userland

- [x] Processes and threads: `clone`, `fork` (copy-on-write), `execve`, `exit`, `wait4`
- [x] Thread-local storage, `futex`, robust futex lists, `set_tid_address`
- [ ] Signals: delivery, handlers, `sigaltstack`, `rt_sigreturn`, process groups
- [x] Pipes, `poll` / `select`, `epoll`, `eventfd`, `timerfd`, `signalfd`
- [x] Time syscalls and a vDSO (`clock_gettime` without a syscall)
- [ ] Dynamic linking: glibc's own `ld.so` loads shared libraries unchanged
- [x] `/dev` (null, zero, random, urandom, tty, console), `/proc` (self, maps, status, cpuinfo), minimal `/sys`
- [x] TTY layer: line discipline, pseudo-terminals, job control
- [x] Users, groups, credentials, file permissions, `umask`
- [x] Persistent file system on virtio-blk (ext2), page cache and writeback
- [ ] ext4 on the persistent disk (extents, journal)
- [ ] **Milestone:** BusyBox shell boots as init and runs interactively
- [ ] SMP: start all CPUs, per-CPU data, TLB shootdown, multi-core scheduler
- [x] Networking: virtio-net driver, own TCP/IP stack (IPv4, IPv6, TCP, UDP, ICMP)
- [ ] Sockets: `AF_INET`, `AF_INET6`, `AF_UNIX` (incl. fd passing), `AF_NETLINK` basics
- [ ] Linux namespaces / seccomp basics (needed by browsers' sandboxes)
- [ ] **Milestone (proof of concept a):** a recorded trace of a simple Linux CLI app passes
- [ ] bash, coreutils, Python and a C compiler run from the persistent disk

## Phase 5: NT personality and Windows console apps

- [ ] ⚖️ Decide: reimplement MSVC / DirectX redistributable DLLs, or run apps' own copies
- [ ] Windows test oracle VM, Windows API tracer, Wine conformance tests imported as spec seeds
- [ ] NT object manager: handles, object namespace (`\??`, `\Device`, `\BaseNamedObjects`), reference counts
- [ ] NT memory services: sections, `NtAllocateVirtualMemory`, `NtMapViewOfSection`, protections
- [ ] NT processes and threads, TEB/PEB, thread contexts, APCs
- [ ] NT synchronisation: events, mutants, semaphores, timers, `NtWaitForMultipleObjects`
- [ ] NT file I/O: case-insensitive lookup, drive letters, share modes, byte-range locks, IRP-style async I/O
- [ ] Structured exception handling: kernel-to-user exception dispatch, unwinding
- [ ] Registry: hive storage, `NtOpenKey` family, default system hives
- [ ] Our `ntdll`: PE loader (imports, relocations, TLS, delay-load), heap, `Rtl*`, loader lock
- [ ] Our `kernelbase` / `kernel32`: files, processes, threads, memory, synchronisation, errors
- [ ] C runtime: `ucrtbase` / `msvcrt` (or app-local copies per the decision above)
- [ ] `advapi32` basics: registry API, security descriptors, tokens (single user)
- [ ] Console: our console host, `AllocConsole`, `ReadConsole` / `WriteConsole`, standard handles
- [ ] Windows file system layout: `C:\Windows`, `System32`, `Program Files`, user profile
- [ ] **Milestone (proof of concept b):** 7-Zip command line (`7z.exe`) passes its full recorded trace
- [ ] Winsock (`ws2_32`) on our sockets; `winhttp` / `wininet` basics
- [ ] Cryptography: `bcrypt`, `crypt32` certificate stores (needed by almost every networked app)

## Phase 6: Graphics, input, audio and the desktop

- [ ] ⚖️ Decide: global menu bar or per-window menus
- [ ] ⚖️ Decide: native window frames per app, or one neutral frame
- [ ] virtio-gpu driver: modesetting and 2D framebuffer
- [ ] Input: virtio-input / PS/2 keyboard and mouse, input event layer, keymaps
- [ ] Kernel GPU interface compatible with Linux DRM/KMS so Mesa runs unchanged
- [ ] 3D acceleration: virtio-gpu with Venus (Vulkan) / virgl (OpenGL) through Mesa
- [ ] Audio: virtio-sound (then Intel HDA) driver, audio server, ALSA-compatible interface
- [ ] Compositor: Wayland server, surface composition, HiDPI scaling, vsync
- [ ] Shell: launcher, window management, notifications, settings (own non-glass design)
- [ ] Compositor exposes the content behind each window (needed later for Liquid Glass)
- [ ] XWayland for X11 apps
- [ ] Fonts: font store, rasterisation, fontconfig-compatible discovery
- [ ] **Milestone:** GTK and Qt apps run; VLC plays a video with sound

## Phase 7: Windows GUI apps

- [ ] win32k services: windows, message queues, input routing, hooks, timers
- [ ] `user32`: window classes, messages, dialogs, menus, clipboard, DPI awareness
- [ ] `gdi32`: device contexts, bitmaps, regions, text output; surfaces handed to the compositor
- [ ] Native look: `uxtheme` visual styles, `comctl32` common controls matching real Windows
- [ ] Text: DirectWrite, GDI font mapping, Uniscribe / complex scripts
- [ ] COM runtime: `combase` / `ole32`, apartments, marshalling, RPC, `oleaut32` automation
- [ ] Shell: `shell32` / `shlwapi`, known folders, file open/save dialogs
- [ ] Graphics: DXGI, Direct3D 11 on Vulkan, Direct2D; then Direct3D 12
- [ ] Audio: MMDevice / WASAPI and `winmm` on our audio server
- [ ] Security model: tokens, ACLs, integrity levels, job objects, restricted tokens (Chrome sandbox)
- [ ] Services: service control manager, scheduled tasks, Windows event log
- [ ] **Milestone:** Notepad++ and the 7-Zip GUI run with native-looking controls
- [ ] **Milestone:** Paint.NET (.NET desktop) and VS Code (Electron) run
- [ ] **Milestone:** Chrome runs with its sandbox enabled

## Phase 8: Integration bridges

- [ ] Shared clipboard across Linux, Windows and macOS formats
- [ ] Drag and drop between app worlds
- [ ] One file picker for every app world, mapped to each OS's path conventions
- [ ] Unified notifications, app launcher entries and file associations
- [ ] Shared fonts, HiDPI settings, keyboard layouts and input methods (IME)
- [ ] One user home visible as `~`, `C:\Users\<name>` and `/Users/<name>`

## Phase 9: XNU personality and macOS foundations

- [ ] ⚖️ Decide: aarch64 kernel port or x86-64 only (modern Mac apps are increasingly arm64-only)
- [ ] macOS test oracle (licensed Mac); extend the framework map with Objective-C methods and Swift API
- [ ] Mach-O loading and our own `dyld` (shared cache optional, chained fixups)
- [ ] Mach core: tasks, threads, ports, port rights, messages, Mach VM
- [ ] BSD layer: XNU syscalls, `kqueue`, `posix_spawn`, signals with Darwin semantics
- [ ] File system semantics: case-insensitive APFS behaviour, extended attributes, resource forks
- [ ] `libSystem`: libc, `libpthread` (incl. workqueues), `libdispatch` (GCD), `libxpc`
- [ ] Objective-C runtime (`libobjc`) and Swift runtime
- [ ] System daemons: `launchd`, XPC services, preferences (`cfprefsd`), `notifyd`
- [ ] CoreFoundation, Foundation, Security framework, keychain
- [ ] Networking: CFNetwork, Network.framework on our sockets
- [ ] **Milestone:** macOS command-line tools run (Swift and Objective-C)

## Phase 10: macOS GUI apps and Liquid Glass

- [ ] Window server: Quartz / CoreGraphics windows composited by our compositor, global menu bar handling
- [ ] Core Text and font rendering matching macOS
- [ ] Core Animation: layer tree, implicit animations, timing curves
- [ ] Metal on Vulkan; Core Image
- [ ] AppKit: windows, views, controls, menus, event handling, accessibility
- [ ] SwiftUI on our AppKit / Core Animation
- [ ] Liquid Glass: materials, blur, refraction, sampling the content behind the window
- [ ] Visual fidelity runner: noise-floor calibration, exact layout checks, perceptual checks
- [ ] Audio: Core Audio / AVFoundation on our audio server
- [ ] **Milestone:** a native AppKit app and a SwiftUI app match the real Mac within the noise floor

## Phase 11: Real hardware

- [ ] UEFI boot (own loader or Limine) producing the same `BootInfo` as PVH
- [ ] ACPI AML interpreter, power buttons, battery, thermal, suspend/resume
- [ ] NVMe storage, xHCI USB with HID (keyboard, mouse, touchpad)
- [ ] GPU driver for the chosen laptop (Intel or AMD), display output and 3D
- [ ] Wi-Fi, Bluetooth, laptop audio codec, backlight, webcam
- [ ] Installer, disk encryption, updates
- [ ] **Milestone:** Poly runs as the only OS on one real laptop for a full workday

## Ongoing (every phase)

- Security hardening and fuzzing of every parser and syscall
- Performance work guided by real app traces
- New app traces turn into work items at the top of the queue
- Keeping up with new Windows and macOS releases (the coverage map is a treadmill)

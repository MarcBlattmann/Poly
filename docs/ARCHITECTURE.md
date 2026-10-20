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
                 kernel/src/linux       ← LinuxKernel for the running thread: files, memory, signals, futexes;
                                          clone/fork/execve, the process tree, wait4 (task.rs)
                 kernel/src/process.rs  ← tag set at exec from poly-binfmt; threads, group exit, wait
                 kernel/src/program.rs  ← exec of programs from the VFS; poly.init + test-run records (testrun.rs)
                          │
            kernel shell: poly-line editor, commands; on COM1 and on the screen terminal
            screen terminal: status bar + text grid, keys as terminal input, kernel log mirrored (poly-term)
            kernel/src/user  ← user threads: ring 3, system call hook, faults, copy_*_user,
                               memory maps with demand paging (poly-vm), exec of static ELF (poly-elf)
            file systems: VFS (poly-vfs), tmpfs root from the initramfs, ext2 disks behind the page cache (poly-pagecache)
            shared core: boot, cpu (GDT/TSS/IDT), memory map, frames, heap, threads
            block layer: request queue + buffer cache per disk (virtio0, ram0; virtio1... on demand)
            network: eth0 on virtio-net (MSI-X), poly-net TCP/IP stack, network thread, blocking sockets
            devices: PCI (ECAM, MSI-X, MSI), virtio-blk (MSI-X interrupts), RAM disk, PS/2 keyboard (IRQ 1)
            display: QEMU standard VGA framebuffer + RAM back buffer; poly-gfx (drawing, font, text grid)
            arch/x86_64 (PVH entry, serial, exceptions, page tables, address spaces,
                         syscall/sysret, FPU state, context switch)
```

## Boot flow

1. QEMU (`-kernel`) reads the PVH ELF note and jumps to `pvh_start32` in
   32-bit mode with `ebx` pointing at `hvm_start_info`.
2. `arch/x86_64/boot.rs` checks for long mode, maps physical [0, 1 GiB) at
   0 and at `0xffffffff80000000`, enables paging and jumps to `kernel_entry`.
3. `kernel_entry` brings up serial, loads our GDT/TSS/IDT, reads the boot info,
   normalises the memory map (reserving the first MiB and the kernel image),
   switches to the kernel page tables (`mm::init`; the boot mapping is gone
   after this), sets the CPU up for user mode (`arch::user::init`: SYSCALL,
   FPU state, SMEP/SMAP/PCIDs when present), hands all usable RAM to the
   frame allocator, starts the
   scheduler, reads the ACPI tables, sets up interrupts and time
   (`arch::init_interrupts`) and enables them, enumerates PCI, brings up the
   block layer (the virtio-blk disk if there is one, and a RAM disk) and the
   PS/2 keyboard and the display (which shows the boot screen), mounts a
   tmpfs on `/` filled from the initramfs, makes the page cache, starts the
   writeback thread and mounts the disks `poly.mount=` names (see "File
   systems"), brings up the network, runs the first program if
   `poly.init=<path>` names one (see "Linux processes"; with
   `poly.qemu_exit` the boot ends when it does), and runs the boot
   self-test with the timer preempting threads.
4. With `poly.qemu_exit` on the command line, the result is reported through
   QEMU's exit status, which is how `cargo xtask test` checks it.
5. Otherwise (or with `poly.shell` as well, once the tests passed) the
   kernel puts the screen terminal on the display with a shell of its own
   (see "Screen terminal"), starts the shell on COM1 in a thread of its
   own, and the boot thread exits (see "Kernel shell").

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
  file system code can be tested without a disk. The other virtio-blk
  functions are `virtio1`, `virtio2`, ... in PCI order, brought up the first
  time something names them (`block::open`, which a mount calls); one
  nobody names (the watchdog test's spare) is left alone. `block::root()` is the
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

## Networking

One interface, `eth0`: the host-tested TCP/IP stack `poly-net`
([ADR 0028](adr/0028-tcp-ip-stack-in-a-crate.md)) on a virtio-net device
driven by the host-tested `poly-virtio-net`, a network thread that polls
the stack, and blocking sockets for the system call layers. Decisions are
in [ADR 0039](adr/0039-networking-in-the-kernel.md).

```text
 socket::Socket (blocking) ──net::call(op)──► IFACE: Mutex<Interface { Stack, Device }>
        ▲  wait_for_change(generation)            ▲ poll(dev, now) -> next deadline
        │                                         │
 CHANGED += 1, wake ◄───── network thread ────────┘  sleeps until WAKE moves or the deadline
                                ▲
             MSI-X entry 0 ─────┘ on_interrupt: WAKE += 1, wake the thread (touches nothing else)
```

- **`poly-virtio-net`** (VIRTIO 1.2 §5.1) negotiates VERSION_1, MAC,
  STATUS, MTU (only if the buffers can hold a frame of that size),
  MRG_RXBUF, EVENT_IDX and ACCESS_PLATFORM, never checksum or segmentation
  offloads, the control queue or several queue pairs. Every packet buffer
  is 2 KiB of driver-owned DMA memory with one descriptor: transmit puts
  the 12-byte `virtio_net_hdr` (all zeros) and the frame in one
  device-readable buffer; receive buffers are all handed to the device at
  start and each goes back as soon as its frame is copied out. With
  MRG_RXBUF a packet spanning several buffers is gathered (and held until
  all of them are back). Lengths, buffer counts and used-ring entries are
  checked; a device that breaks the queue protocol is reset and refused
  from then on. Transmit completions interrupt only while every transmit
  buffer is in flight (otherwise they are reclaimed by the next send). It
  implements `poly_net::NetDevice`; frames are copied between its buffers
  and the stack's, so the device never sees other memory. Host tests run
  it against a simulated device written from the specification, including
  a `poly-net` stack doing ARP, ping and a 300 KB TCP echo over it.
- **The kernel side** (`kernel/src/net/virtio.rs`) finds the first
  virtio-net function, maps its structures, enables MSI-X masked, resets
  the device before bus mastering and gives `poly_virtio_net::init` 128
  receive and 128 transmit buffers (`DmaBuffer`s, 2 x 256 KiB) plus rings
  and tables, with both queues and configuration changes on MSI-X entry 0.
  Without MSI-X the network thread polls every 10 ms.
- **`kernel/src/net/mod.rs`** keeps the stack and its device together in
  one `Interface` behind a sleeping `Mutex` (the stack allocates and a poll
  can take a while; no interrupt handler takes the lock). Without a
  device, the stack sits on a stand-in that drops everything, so loopback
  (127.0.0.0/8) works in every boot. The MSI-X handler only bumps a counter
  and wakes the network thread, which polls, asks the device for
  interrupts again (polling again if work arrived meanwhile), and sleeps
  until the next interrupt or the deadline `poll` returned. Socket calls go
  through `net::call`: the operation, then a `poll` right away, then the
  network thread is kicked if the stack's deadline moved earlier.
- **Blocking** (`kernel/src/net/socket.rs`): `Socket` wraps a handle with
  blocking `connect`, `accept`, `send`, `recv` and friends (optional
  deadlines; `EAGAIN` or, for `connect`, `ETIMEDOUT` when they pass) and a
  one-socket `wait(interest)`; `Pinger` sends ICMP echo through a ping
  socket. A blocking call tries, and on `EAGAIN` sleeps on one wait queue
  until the readiness generation moves away from the value its attempt
  saw (`call_and_generation`, `wait_for_change`). The generation moves,
  with the interface locked, whenever a poll moved packets (device frames
  or the stack's loopback), ran a due timer or saw the link change, and on
  `shutdown` and `close`. So no change after an attempt is missed, and a
  poll that moved nothing wakes nobody.
- **Configuration**: `poly.net=<addr>/<prefix>[,gw=<addr>][,dns=<addr>]`,
  `poly.net=qemu` (QEMU's user networking: 10.0.2.15/24, gateway
  10.0.2.2, DNS 10.0.2.3) or `poly.net=off`, parsed by
  `poly_net::setting`; without it the interface has only its IPv6
  link-local address. The stack's secret (initial sequence numbers and
  ports, RFC 6528) and a MAC address for devices without one come from
  `arch::user::random_bytes` (RDRAND, or the TSC mixed).

```text
virtio-net: 00:02.0: MSI-X with 4 vectors, table BAR1+0x0, PBA BAR1+0x800
virtio-net: 00:02.0: MAC 52:54:00:12:34:56, MTU 1500, link up, features 0x120018020, queues 128 rx / 128 tx entries
net: eth0: virtio-net 00:02.0; queues on MSI-X entry 0 -> vector 0x31; interrupt-driven
net: eth0: inet 10.0.2.15/24 gateway 10.0.2.2 dns 10.0.2.3; inet6 fe80::5054:ff:fe12:3456/64
```

The kernel tests (`net::tests`) run TCP between two kernel threads over
loopback (the server asleep in `accept` first), UDP and ping over loopback
and a receive timing out in every boot. With `poly.test.net=<port>` (the
network boot tests, on QEMU's user networking) they also ping the gateway
10.0.2.2, push 1 MiB through a TCP echo server and datagrams through a UDP
echo server that `cargo xtask` runs on the host's `<port>` (reached at
10.0.2.2), and accept a connection the host makes through a port forward
to guest port 7777, checking the bytes at both ends.

## Input

Keyboard input is the host-tested `poly-input` crate; `kernel/src/keyboard.rs`
drives the PS/2 keyboard with it, and `arch/x86_64/i8042.rs` gives it the
controller's ports ([ADR 0020](adr/0020-ps2-keyboard-and-layouts.md)).

```
   IRQ 1 ─▶ keyboard::on_irq: data port ─▶ ByteRing (256 bytes, lock-free) ─▶ wake waiters
                                                           │
   keyboard::next_event / try_next_event (thread) ◀────────┘
      poly_input::Keyboard
        Set1Decoder   bytes ─▶ (Key, pressed): E0 / E1 prefixes, fake shifts, Print Screen, Pause
        KeyState      ─▶ KeyEvent { key, pressed, repeat, modifiers }: Shift/Ctrl/Alt/AltGr/Meta, 3 locks
        Composer      ─▶ Output: Char, Chars (accent + char), Dead, Special (Enter, arrows, F-keys, ...)
          + Layout    US, Swiss German (de-CH); six levels per key, as data
```

- **Keys** (`poly_input::Key`) are physical positions named after the US
  legends (W3C UI Events `code` names): 105 keys, the ISO key next to the
  left Shift (`IntlBackslash`) included. `Modifiers` is a bit set (Shift,
  Control, Alt, AltGr, Meta, Caps / Num / Scroll Lock), `Leds` the PS/2 LED
  mask.
- **Decoding.** The controller translates the keyboard's scancode set 2
  into set 1, which `Set1Decoder` decodes; it ignores replies and unknown
  codes, resynchronises after broken sequences and never panics.
  `set1::encode` is the reverse (tests, `keyboard::inject`). `KeyState`
  flags a press of a key that is down as a repeat (locks toggle only on
  the first press) and makes the right Alt key AltGr on layouts that have
  it.
- **Layouts** (`poly_input::layout`) are tables built at compile time:
  base, Shift, Caps Lock, Caps Lock + Shift, AltGr and AltGr + Shift, each a
  character, a dead key (\` ´ ^ ~ ¨) or nothing. `Composer` picks the
  level, types the keypad's digits when Num Lock is on (navigation keys
  otherwise, Shift reverses it), turns Ctrl + letter into a control
  character and composes dead keys (accent + letter: the accented letter;
  + Space: the accent; + anything else: both; a special key drops it).
  `layout::by_name` finds one (`us`, `de-ch`).
- **Bring-up** (`keyboard::init`, after the block layer). If the ACPI FADT
  (revision 2+) says there is no 8042, nothing is touched. Otherwise
  `poly_input::i8042::init`, polled with both ports' interrupts off:
  disable both ports, flush the output buffer, clear the interrupt and
  translation bits in the configuration byte, controller self-test (0xAA
  → 0x55) and the configuration written again, keyboard port test (0xAB →
  0x00), enable the port, reset the keyboard (0xFF → ACK, 0xAA), scancode
  set 2 (0xF0 0x02), enable scanning (0xF4). Every wait is bounded; a
  missing controller (status 0xFF), a failed self-test or no keyboard is
  logged and the kernel runs on without one. Then ISA IRQ 1 is routed
  (`arch::irq::route_isa_irq`), the configuration byte gets the keyboard
  interrupt and translation on (mouse port off), and the controller is
  drained once. The layout comes from `poly.keymap=` (default `us`); Num
  Lock starts on.
- **Interrupts and waiting.** The handler moves every waiting byte into
  the ring and wakes the readers; decoding happens in the reader's thread
  under a sleeping `Mutex`, which is never held while waiting for input. A
  reader that sleeps looks at the controller itself every 100 ms, so a
  lost interrupt costs latency, not the keyboard. LED updates (0xED + mask)
  follow the locks: the reader writes each byte and sleeps until the
  handler hands it the keyboard's ACK (or RESEND) instead of queueing it.
- **API** (what the terminal uses): `keyboard::next_event()` (sleeps),
  `next_event_until(deadline_ns)`, `try_next_event()`, each an `Input {
  event: KeyEvent, output: Output }` for every press and release;
  `set_layout(&'static Layout)`, `layout()`, `modifiers()`,
  `is_present()`, `inject(bytes)` (set 1 bytes as if typed), `stats()`.

```text
keyboard: i8042 config 0x71 -> 0x61, 0 stale byte(s) flushed; keyboard reset ok, scancode set 2 translated to set 1
keyboard: ISA IRQ 1 -> GSI 1 -> vector 0x31; layout us (US English (QWERTY)), Num Lock on
```

The kernel tests (`keyboard::*`) decode injected scancodes through the ring
(US and de-CH, with AltGr and dead keys), check modifiers on events, the
I/O APIC route for IRQ 1, bytes the controller fakes with its "write
keyboard output" command (0xD2) arriving through the interrupt, and Caps
Lock LED updates acknowledged by the keyboard. With
`poly.test.keyboard=<text>`, `keyboard::typed_text_matches_the_host_keys`
waits up to 30 s for keys from the host, which `cargo xtask test` types
through the QEMU monitor (`sendkey`), with the US layout and with
`poly.keymap=de-ch` (see [DEBUGGING.md](DEBUGGING.md)).

## File systems

The logic is host-tested in `poly-vfs` and `poly-cpio`
([ADR 0023](adr/0023-vfs-tmpfs-and-cpio.md)); `kernel/src/fs/` wires it in
([ADR 0037](adr/0037-vfs-and-initramfs-in-the-kernel.md)). The ext2 driver is
`poly-ext2` ([ADR 0032](adr/0032-ext2-driver.md)), mounted on virtio-blk disks
behind the page cache, `poly-pagecache`
([ADR 0040](adr/0040-page-cache-and-ext2-on-virtio-blk.md)); both host-tested.

```
   personalities (later: openat, read, write, getdents64, ...)   kernel shell: ls, cat, mount
          │  user memory copied by the system call layer; fs works on kernel buffers
          ▼
   fs::Files (on each Process)   FsContext + FdTable (sleeping mutexes) + Cred
          │  FdTable → Arc<OpenFile> (offset, flags) → NodeRef; I/O runs unlocked
          ▼
   fs::vfs(): poly_vfs::Vfs   path resolution, permissions (Cred), mounts, char devices
          │  FileSystem trait: lookup/create/rename/read/write/readdir by inode   └─ /dev/console, /dev/tty → kernel log
          ▼                                                                         /dev/null, /dev/zero, /dev/full
   Tmpfs on /                 CachedFs on /mnt: page cache (poly-pagecache) ◀── user mappings (FileSource)
          ▲                          │  fill with read-ahead / writeback (thread, sync, unmount)
   poly_cpio::unpack                 ▼
   initramfs (PVH module 0)   poly_ext2::Ext2<DiskIo, RawMutex>        later: devfs, procfs, ext4
                                     │  BlockIo: read/write/flush device blocks
                                     ▼
                              block::Disk virtio2 (queue + buffer cache)
```

- **The initramfs** is the first PVH boot module (`hvm_start_info.nr_modules`
  / `modlist_paddr`, `hvm_modlist_entry` in Xen's public
  `arch-x86/hvm/start_info.h`); `boot_info` keeps it as `BootInfo::initrd`.
  Before the page tables are built, `fs::initrd::reserve` marks its pages
  `Kernel` in the memory map, so nothing allocates them early.
  `frames::init(memory, held)` then manages them but starts with them
  allocated (`BuddyAllocator::allocate_range`), so the allocator spans them
  even at the very top of RAM, where QEMU puts the archive. `fs::init` (after
  the display, before the self-test) unpacks the archive into a tmpfs
  mounted on `/` with `poly_cpio::unpack`, as root and with the archive's
  owners, modes and times, and gives the pages back (`frames::free_range`).
  Without a module `/` starts empty. Either way it then gets `/dev`, `/tmp`
  (`01777`) and `/proc` (empty, for procfs later) and the device nodes,
  unless the archive brought them. tmpfs times count from boot (no RTC yet).
- **Devices**: `/dev/console` (5, 1) and `/dev/tty` (5, 0) write through
  the kernel log's path (serial now; a UTF-8 character split across writes
  stays whole); their reads return end of file until the TTY layer
  (Phase 4) shares the serial input with the shell. `/dev/null`, `/dev/zero`,
  `/dev/full` are `poly-vfs`'s. `ioctl` on them is `ENOTTY`.
- **`fs::Files`** is a process's file state (`Process::files`): `FsContext`
  (root, working directory, umask) and `FdTable` behind sleeping mutexes
  (lock order: context, then table), and the `Cred` (root for now). New
  processes get `/dev/console` on 0, 1 and 2 (one open file description).
  The API takes kernel byte buffers and returns `Errno`s with Linux's
  numbers: `open`/`openat`, `read`, `write`, `pread`, `pwrite`, `close`,
  `lseek`, `dup`/`dup2`/`dup3`/`dup_from`, close-on-exec and status flags,
  `fstat`/`statat`, `getdents` (a callback per entry), `ioctl`, `ftruncate`,
  `fsync`, `readlinkat`, `mkdirat`, `unlinkat`, `symlinkat`, `chdir`,
  `fchdir`, `getcwd`, `umask`, `close_on_exec`, `close_all`, `fork`. Reads
  and writes run with no lock held, so a blocking read stalls nothing else.
  The `Vfs` locks itself (ADR 0023); no interrupt handler enters it.

```text
boot: initramfs at 0xffcd000-0xffd7884 (43 KiB)
fs: initramfs unpacked into /: 7 directories, 8 files (40263 bytes), 3 symbolic links, 2 device nodes, 1 hard links
fs: initramfs: 11 pages (44 KiB) at 0xffcd000-0xffd8000 returned to the frame allocator
fs: devices console (5,1), tty (5,0), null (1,3), zero (1,5), full (1,7); 6 of 8 directories and nodes made
```

The kernel tests (`fs::*`) check, with the test initramfs `cargo xtask`
builds (`poly.test.initrd`), every entry's type, mode, size, owner and time,
file contents (through relative and absolute links), link targets, hard
links sharing an inode, a device node from the archive; that the archive's
pages came back, and on an archive written into frames taken the same way,
that unpacking frees exactly them (interrupts off while counting); the
memory devices and the console; descriptor rules (lowest free number, `dup`
sharing offsets, `dup2`/`dup3`, close-on-exec kept, cleared and applied at
exec, `fork` copies); tmpfs I/O across pages, holes, `O_APPEND`, `O_EXCL`,
`getdents`; working directory and umask. One self-test boot has no
initramfs.

- **`poly-vfs`** (`no_std` + `alloc`): `Vfs` resolves paths as
  `path_resolution(7)` describes (`.`/`..` across mount points, 40 symbolic
  links then `ELOOP`, trailing slashes, `ENAMETOOLONG`), checks permissions
  from a `Cred`, and implements the path-based calls; `FdTable` and
  `OpenFile` implement descriptors (`dup2`/`dup3`, close-on-exec,
  `O_APPEND`, `lseek`, `getdents` positions). Errors are `Errno` with
  Linux's values; flags take Linux's bits.
- **tmpfs**: sparse files in 4 KiB pages, stable `readdir` positions,
  unlinked files and removed directories live while open.
- **Readiness and file objects** ([ADR 0042](adr/0042-readiness-pipes-poll-epoll-and-event-files.md)):
  every open file reports `PollEvents` (Linux's `POLL*` / `EPOLL*` bits);
  one whose readiness changes has a `WaitQueue` that whoever changes it
  notifies, and waiters register a `Waker` on it, check again and block
  through a `Blocker` (the kernel's park / unpark). `FileObject`s are open
  files that are not on a file system, on inodes of an anonymous file
  system no path reaches (`Vfs::anon_node`): pipes (64 KiB, `PIPE_BUF`
  atomicity, end of file, `EPIPE`, packet mode), event counters, timers
  (expirations counted from the clock, a `TimerService` callback to wake
  waiters) and epoll sets (an interest entry is a waker that puts itself
  on the ready list; level- and edge-triggered, one-shot, nested). Their
  operations never block; `OpenFile::read_wait` / `write_wait` wait on
  the queue unless the file is `O_NONBLOCK`, and `poll::poll` waits over
  many files.

```text
 pipe write ──► WaitQueue::notify(IN) ──► waker: thread in read / poll ──► unpark
                                      └─► waker: epoll entry ──► ready list ──► epoll's WaitQueue ──► thread in epoll_wait
 timer thread (timers) ──► TimerFd::expire ──► notify(IN)        signal queued ──► signal files' queue
```
- **`poly-cpio`**: a parser for `newc`/`crc` that never panics, a writer,
  and an unpacker into any file system through the VFS. Tested against
  archives made by GNU cpio (stored, and live if `cpio` is installed).
- **`poly-ext2`**: ext2 (revisions 0 and 1, 1/2/4 KiB blocks, `filetype`,
  `sparse_super`, `large_file`; unknown incompatible features refuse the
  mount, unknown read-only ones mount read-only) over a `BlockIo` (the
  kernel's `block::Disk`, or `CachedIo`/`DeviceIo` over `poly-block`).
  Group-local allocation, indirect block maps with holes, directory records
  split and merged, `format` for new volumes. Without a journal, metadata
  writes are ordered by flushes so a crash leaves a volume e2fsck repairs
  without shared blocks or lost names. Corrupt volumes give `EUCLEAN`,
  never a panic. Tested against `mke2fs`/`debugfs`/`e2fsck` images, a
  random model test against tmpfs, byte flips and simulated crashes.
  Its volume lock is a type parameter (`poly_vfs::lock::RawLock`): a
  spinlock on the host, the kernel's sleeping `sync::RawMutex` when mounted.
- **Disk mounts** (`kernel/src/fs/disk.rs`): `fs::disk::mount(disk, path,
  ro)` brings the disk up (`block::open`), mounts ext2 on it through
  `DiskIo` (the disk's buffer cache as `BlockIo`, `flush` = `Disk::sync`)
  with a `RawMutex`, registers it with the page cache and mounts the
  result on `path`, made if missing. `poly.mount=<disk>:<path>[:ro]`
  (comma-separated) mounts at boot; the shell has `mount <disk> <path>
  [ro]`, `umount`, `sync` and `write`. `unmount` refuses a busy mount, then
  writes back, syncs and marks the volume clean; `power::reboot` first runs
  `fs::disk::shutdown`, which does the same for every volume, files open
  or not.
- **The page cache** (`poly-pagecache`, kernel glue in
  `kernel/src/fs/cache.rs`): file pages in frames, keyed by file system,
  inode and page index. `PageCache::register(ext2)` returns a `CachedFs`,
  the `FileSystem` the VFS mounts: regular files' reads and writes go
  through it (a miss reads up to 16 pages ahead; writes dirty pages and
  update the size and time at once, the data reaches ext2 at writeback),
  everything else passes through; files that shrink or are deleted lose
  their pages. Per-inode locks (the kernel's `RawMutex`) are held across a
  file's page I/O. Frames come from `FRAMES` (`KernelFrame`); the cache
  keeps at most a quarter of RAM, evicting clean unused pages least
  recently used first (and half of them when free memory drops below 1/16
  of RAM). The `writeback` thread syncs every disk mount that changed every
  5 seconds; `sync` and unmounting do it at once. `FileSource` makes a VFS
  file a `user::PageSource` whose pages on a disk mount are the cache's,
  for mappings (see "Memory maps").

```text
fs: page cache of up to 16190 pages (63 MiB); writeback every 5 s
block: virtio2: virtio-blk 00:06.0, 16384 blocks of 512 bytes (8 MiB), write cache; queue 32 requests, cache 128 buffers (64 KiB); interrupt-driven (vector 0x32)
fs: virtio2 mounted on /mnt: ext2, 2048 blocks of 4096 bytes (1901 free), 1024 inodes (1008 free), read-write
```

The disk file system tests (`fs::disk_tests::*`) format the RAM disk as
ext2 and mount, write, unmount and check it in every boot; with
`poly.test.ext2` (the ext2 test disk that `cargo xtask` builds, mounted on
`/mnt`) they read its files, write and unmount and mount it again
(read-only too, with the disk's buffer cache dropped), check that `read`
and a private and a shared mapping get the same page (copied on a private
write, written to the file through the shared one), that the writeback
thread cleans a dirty page, and that evicting clean pages gives their
frames back.

## Display

A linear framebuffer on QEMU's standard VGA, a back buffer in RAM, and the
host-tested drawing library `poly-gfx`
([ADR 0019](adr/0019-display-framebuffer-and-drawing.md)).

```
   screen terminal (below), a panic screen (later)
          │  display::with_canvas(|canvas| terminal.render(canvas)), display::flush(rect)
          ▼
   poly-gfx      Canvas: fill, clip, lines, blit, copy_within, text
          │      Font: Spleen 8x16 and 12x24     TextGrid: cells, scrollback, escapes, dirty cells
          ▼
   back buffer   width × height × 4 bytes of contiguous frames, write-back (direct map)
          │  display::flush(rect): copies those rows
          ▼
   framebuffer   BAR 0 of PCI 1234:1111, mapped write-combining (PAT entry 1)
                 mode set with poly-bochs-vbe through BAR 2 + 0x500 (or I/O ports 0x1ce/0x1cf)
```

- **Bring-up** (`display::init`, after `block::init`): finds PCI function
  1234:1111 (the standard VGA that `q35` always has, or `bochs-display`),
  turns on decoding, checks the Bochs VBE interface ID, reads the largest
  mode (GETCAPS) and sets `poly.display=<width>x<height>` from the command
  line or 1280x800, at 32 bits per pixel (XRGB8888), falling back to the
  default when the device refuses. The registers are read back to confirm
  the mode. It maps all of BAR 0 write-combining (`mm::map_mmio_wc`,
  uncached without PAT), allocates the back buffer
  (`frames::allocate_contiguous`: more than one 4 MiB block for modes past
  1280x800), draws the boot screen (`poly_gfx::boot_screen`: "Poly", the
  version and the mode) and flushes it. `poly.display=off` skips it all.
  The protocol (`poly-bochs-vbe`) is tested on the host against a
  simulated device, including devices that refuse modes.
- **Drawing.** `display::with_canvas(|c| ...)` hands out the back buffer as
  a `poly_gfx::Canvas`; `display::flush(rect)` / `flush_all()` copy rows of
  it to the framebuffer; `display::draw(|c| -> Option<Rect>)` does both,
  flushing the rectangle the closure returns (what `TextGrid::render`
  returns). `display::info()` gives width, height, stride, pixel format,
  the framebuffer's address and size, and whether it is write-combining.
  Everything sits behind one spinlock (interrupts off while drawing), so
  any context may draw. Only the flush touches device memory, and only
  with writes.
- **`poly-gfx`** draws on any `Canvas` (a byte slice, width, height,
  stride, one of four pixel formats), clipping everything: rectangles,
  lines, blits with format conversion, `copy_within` (overlapping moves, for
  scrolling), and text in a `Font` (Spleen 8x16 and 12x24, BSD 2-Clause,
  converted from BDF by `cargo xtask font`; see `THIRD_PARTY.md`; ASCII,
  Latin-1 and Extended-A, Greek, Cyrillic, box drawing, blocks, Braille;
  a box for anything else), optionally scaled up. `TextGrid` is a terminal
  screen over caller-provided cells: wrap with a pending wrap, scrolling
  into a bounded scrollback (a ring, so scrolling moves no cells), a view
  that scrolls back into it, `\n` `\r` `\b` `\t`, the common escape
  sequences (SGR colours: 16, 256 and RGB; bold, faint, underline, inverse,
  hidden, crossed out; cursor movement, erase, insert / delete characters
  and lines, scroll, save / restore, cursor on / off, wrap on / off) and a
  cursor in three shapes. It tracks changed cells per row, scrolled lines
  and the drawn cursor; `render` moves scrolled pixels and draws only the
  changed cells at any offset on a canvas, so a status bar can sit next to
  it. Nothing in `poly-gfx` allocates.

```text
mm: direct map at 0xffff800000000000: ...; NX: yes, 1 GiB pages: no, global pages: yes, write-combining (PAT): yes
display: 00:01.0 1280x800x32, 5120 bytes per line; framebuffer 0xfd000000 (16 MiB) mapped write-combining; registers through MMIO (BAR 2)
```

The kernel tests (`display::*`) check that the registers read back the
mode asked for, that the I/O ports and the MMIO BAR show the same
registers, that the framebuffer mapping is write-combining and holds a
pixel written to it, that drawing reaches the back buffer and not the
screen, that a flush copies exactly its rectangle (and nothing around it),
and that `draw` flushes what it drew; each puts the screen back as it found
it. With `poly.test.display` (which `cargo xtask` passes) they require a
display. Under TCG a full redraw of 1280x800 in the 8x16 font takes about
40 ms and a full flush about 20 ms, so a terminal renders once per batch of
output.

## Screen terminal

A status bar and a text grid on the display, a second kernel shell on the
keyboard, and the kernel log mirrored into the grid
([ADR 0035](adr/0035-screen-terminal.md)). The logic is the host-tested
`poly-term` crate; `kernel/src/terminal.rs` gives it memory, locks,
threads and the devices.

```
   kprintln ─▶ log::_print ─▶ COM1
                     └──────▶ terminal::mirror ─▶ LOG: LogRing, 64 KiB (spinlock; never draws)
                                                        │ drained by whoever holds the terminal
   keyboard::next_shared_event_until ─▶ poly_term::keys ─▶ bytes ─▶ TermDecoder ─▶ Edit ─▶ shell "shell-screen"
                                                                                            │ output
   SCREEN.term (sleeping mutex): poly_term::Terminal = TextGrid + prompt state ◀────────────┘ poke
          │ render thread "terminal": drain LOG, status line, Terminal::render
          ▼
   display back buffer ─▶ display::flush in bands of 64 rows
```

- **Layout** (`poly_term::Screen`): the status bar is the top text row,
  the grid fills the rest from the left edge in Spleen 8x16 (12x24 from
  1920x1200 up): 160x49 cells at 1280x800. The scrollback holds up to 1000
  lines, fewer if the cells (16 bytes each) would pass 3 MiB.
- **Status bar** (`poly_term::status`): " Poly <version> | up h:mm:ss |
  <free> free of <total> | <layout> | 1 of N CPUs" on the left, a
  Shift+PgUp hint (or the scrollback position) on the right, light text on
  slate; refreshed every second, on the second.
- **Keys** (`poly_term::keys::translate`) become the bytes an xterm sends
  (UTF-8, `CR`, `DEL`, `ESC [ A`, `ESC [ 1 ; 5 D`, `ESC [ 3 ~`, `ESC O P`,
  Alt as an `ESC` prefix), decoded by the line editor's `TermDecoder` as on
  the serial line. Shift+Page Up / Page Down scroll the view a page
  (`TextGrid::scroll_view`); a key that types scrolls back. The console
  reads with `keyboard::next_shared_event_until`, which takes no keys
  while kernel tests run (the keyboard tests read the keyboard).
- **The log**: `log::_print` also calls `terminal::mirror`, which appends
  to a `LogRing` (overwrites the oldest bytes; readers keep their own
  position and learn how much they lost) under a spinlock nothing holds
  longer than a copy. The shell drains it into the grid before its own
  output (keeping the order), the render thread at least every 25 ms. The
  terminal starts with what the ring holds of the boot log. Log text is
  grey (`poly_term::LOG_STYLE`).
- **The prompt**: as on the serial console, the console marks it shown
  while it waits; log output erases its row first, and within 50 ms the
  console returns `Edit::Redraw`, which draws prompt and line below the
  log (`Terminal::prompt_shown`, `take_redraw`).
- **Rendering**: writers change cells under `SCREEN.term` and poke the
  render thread, which renders at most every 15 ms (`TextGrid`'s dirty
  cells, scrolling by moving pixels, the status bar when its text
  changed) into the back buffer and flushes the changed rectangle in bands
  of 64 rows. It pauses while a `display::` test runs.
- **Off**: no display, or `poly.terminal=off`, starts nothing (the mirror
  turns itself off); the serial shell is the same either way.

```text
terminal: 160x49 cells in Spleen 8x16 on the display, status bar on top, 1000 lines of scrollback; a shell on the keyboard
```

The kernel tests (`terminal::*`) run the real shell on a screen console
with scripted key events (Tab completion, editing, Ctrl-C, history, the
keypad's Enter), log output arriving while a line is typed (it goes above,
the prompt is drawn again below), Shift+Page Up and typing scrolling the
view, a `kprintln` reaching a terminal through the ring, and a render on a
display-sized canvas. `cargo xtask test` boots, types `ech`, Tab, `Polx`,
Backspace, `y-Term`, Enter through QEMU's monitor and compares screenshots
with what `poly-gfx` and `poly-term` draw on the host (see
[DEBUGGING.md](DEBUGGING.md)).

## Personality routing

- `poly-binfmt` identifies ELF / PE / Mach-O (and fat Mach-O, `#!` scripts)
  from the image header. It never panics on any input.
- `Process::from_image` tags the process with its personality once, at exec.
- All three worlds enter through the same `syscall` instruction. The core
  saves registers into `SyscallRegs` and `syscall::dispatch` hands them to the
  personality, which reads arguments and writes results in its own convention
  (see "User mode and system calls": a user thread's system calls go to its
  hook, `user::DEFAULT_HOOK` unless it was given another).
- Unimplemented calls return each OS's native "not implemented" result and are
  logged. That log line is the seed of the API tracer.
- **Linux** ([ADR 0025](adr/0025-linux-syscall-table-and-kernel-services-trait.md)):
  `personalities/linux` has the whole x86-64 table (numbers and names
  generated from the coverage map) and handlers for the calls a static C or
  Rust program makes at startup (`read`, `write`, `readv`, `writev`, `close`,
  `fstat`, `ioctl`, `mmap`, `munmap`, `mprotect`, `brk`, `arch_prctl`,
  `set_tid_address`, `exit`, `exit_group`, the id getters, `uname`,
  `rt_sigaction`, `rt_sigprocmask`, `clock_gettime`, `getrandom`; the time
  calls of ADR 0044 are listed under "Time, timers and the vDSO"). Handlers
  validate arguments per the man pages and do the work through
  `LinuxKernel`, a trait the kernel implements over its VFS, VMA manager and
  thread state; `Linux::dispatch_with(&mut kernel, regs)` is the entry point.
  Host tests run every call, and scripted startup sequences of real static
  binaries, against a fake kernel.
- **Linux signals and futexes**
  ([ADR 0031](adr/0031-linux-signals-and-futexes.md)): the personality
  owns each process's signal state (`LinuxProcess`: dispositions, pending
  queues, per-thread masks, alternate stacks, robust list heads), which
  the kernel stores and lends out under the process's lock, and the
  futex wait queues (`FutexTable`), which the kernel locks system-wide.
  `kill`, `tgkill`, `tkill`, `rt_sigqueueinfo`, `rt_sigpending`,
  `rt_sigsuspend`, `rt_sigtimedwait`, `sigaltstack`, `rt_sigreturn`,
  `futex` (no PI operations), `set_robust_list` / `get_robust_list` and
  `restart_syscall` are implemented. The kernel calls
  `Linux::exit_to_user` before every return to user mode (delivery: frames
  on the user stack, stops, kills, and `SA_RESTART` / `EINTR` for
  interrupted calls) and `Linux::thread_exit` when a thread ends (robust
  list, `clear_child_tid`); blocking uses park/unpark (`block`,
  `unblock`) plus a per-thread signal notification.
- **Linux processes and threads**
  ([ADR 0041](adr/0041-processes-threads-fork-exec-wait.md)): `clone`,
  `clone3`, `fork`, `vfork`, `execve`, `wait4`, `waitid`, `setpgid`,
  `getpgid`, `getpgrp`, `setsid` and `getsid`. The personality decodes
  and checks them (`task::clone`, `task::exec`, `task::wait`) and owns the
  process tree's rules (`task::tree::ProcessTree`, lent by the kernel
  through `with_tree`); the kernel creates threads and processes
  (`clone_task`), replaces programs (`execve`) and blocks waiters
  (`wait_tree`), and calls `task::process_exited`, `process_stopped` and
  `process_continued`, which signal the parent.
- **Pipes, waits and event files**
  ([ADR 0042](adr/0042-readiness-pipes-poll-epoll-and-event-files.md)):
  `pipe`, `pipe2`, `poll`, `ppoll`, `select`, `pselect6`, `epoll_create`,
  `epoll_create1`, `epoll_ctl`, `epoll_wait`, `epoll_pwait`,
  `epoll_pwait2`, `eventfd`, `eventfd2`, `timerfd_create`,
  `timerfd_settime`, `timerfd_gettime`, `signalfd`, `signalfd4`, and
  `SIGPIPE` / `FIONREAD`. The handlers check arguments, turn timeouts into
  deadlines, convert `select`'s sets, swap the signal mask of the `p*`
  variants (rt_sigsuspend's `saved_mask`) and decide `EINTR` or a restart;
  the kernel's part is `fileio::FileIo` (`LinuxKernel::file_io`): making
  the objects, looking descriptors up, waiting. Signal files read the
  reading thread's pending signals out of its `LinuxProcess`;
  `LinuxKernel::signal_queued` wakes their readers.

## Linux processes

`kernel/src/linux/`, `process.rs`, `program.rs`, `testrun.rs`; decisions
in [ADR 0038](adr/0038-linux-processes-in-the-kernel-and-the-first-program.md).

- **Kernel services.** `linux::Thread` implements `LinuxKernel` for the
  running thread of a Linux process, one value per system call (or per
  return to user mode): files on the process's `fs::Files`, memory on
  `user::current_memory()`, FS/GS through the MSRs, `with_process` on each
  process's `LinuxState` (the personality's `LinuxProcess` behind a
  spinlock), park/unpark for `block`, one global futex table, the trap
  frame for `user_context`, the thread's FPU save area for signal frames,
  the monotonic clock and the CMOS RTC (`time::realtime_ns`, decoded by
  `poly-rtc`) for clocks.
- **Routing.** A process made with `Process::new_linux` has a `LinuxState`;
  `syscall::dispatch` sends its calls to `linux::syscall`
  (`Linux::dispatch_with`, then `Linux::exit_to_user`). Bare records
  (`Process::new`, the kernel tests' own hooks) still get only `-ENOSYS`
  from the personality.
- **Threads and exits.** Threads register with their process
  (`attach_thread`: the first gets the pid as its id, later ones fresh ids
  from the pid counter) and end themselves (`user::end_thread`): the
  personality's thread-exit duties, then the last thread ends the process
  (files closed, `ExitStatus` set, `Process::wait` returns). `exit_group`
  and fatal signals set a group exit; every other thread ends at its next
  return to user mode (after a system call, or after an interrupt or
  exception in ring 3, which `trap_dispatch` hands to
  `user::return_to_user`), parked threads are woken for it. Processes are
  listed by pid (`process::find`, `find_thread`, `all`).
- **clone, fork, vfork, execve, wait4**
  ([ADR 0041](adr/0041-processes-threads-fork-exec-wait.md),
  `linux/task.rs`). The personality decodes the calls (`task::clone`,
  `task::exec`) and keeps the rules of the process tree (`task::tree`:
  parents, zombies, orphans to init or the kernel, process groups and
  sessions, wait selectors); the kernel lends it the one tree behind a
  spinlock and does the work: `clone_task` starts a thread of the caller's
  process, or a new process with a copy-on-write copy of its memory
  (`Memory::fork`) or the same memory (`CLONE_VM`), copies of its files
  and signal state; the child starts with the caller's registers and FPU
  state (`user::spawn_child`), its stack and TLS, its tids stored and
  `clear_child_tid` set; a vfork parent waits for the exec or the end.
  `execve` follows `#!` lines, loads the ELF, ends the other threads (a
  per-thread `killed` flag), takes over the pid, swaps the memory, closes
  close-on-exec files and resets signals, FPU and FS/GS. When the last
  thread leaves a Linux process, `task::process_exited` makes it a zombie
  (its record stays, for kill(2) and the parent) or reaps it, re-parents
  its children and sends the parent its exit signal; stops and continues
  are reported too (`CLD_STOPPED`, `CLD_CONTINUED`). `wait4` / `waitid`
  sleep on the process event queue until the tree changes.
- **Faults are signals.** An exception in ring 3 that demand paging does not
  resolve becomes a signal (`signal::fault::fault_info`: `SIGSEGV` with
  `SEGV_MAPERR` / `SEGV_ACCERR`, `SIGBUS`, `SIGILL`, `SIGFPE`, `SIGTRAP`),
  delivered at once: a handler runs (its frame on the user stack, the trap
  frame rewritten and the return made with IRETQ) or the process dies with
  the signal.
- **Programs from the VFS.** `program::exec_path(path, argv, envp, traced)`
  opens an executable regular file, checks its format, loads a static ELF
  through `fs::cache::FileSource` (pages read from the file at fault time) and
  starts a Linux process with the console on fds 0-2. The shell's
  `run <program> [args]` runs one and waits for it.
- **The first program and the test-run protocol** (ADR 0027).
  `poly.init=<path>` runs a program as pid 1 before the self-test and
  reports `POLY-INIT` / `POLY-EXIT`; `poly.test.capture` adds a
  `POLY-SYSCALL` record per call, `POLY-OUT` for bytes written to fds 1
  and 2 and `POLY-SIGNAL`; with `poly.qemu_exit` QEMU exits after it. The
  diff runner (`cargo xtask pipeline diff --target poly`) boots one kernel
  per corpus test this way.

```text
init: running /test-binary (traced)
POLY-INIT: exec /test-binary
POLY-SYSCALL: pid=1 nr=12 args=0x0,0x0,0x0,0x0,0x0,0x0 ret=2109440
POLY-OUT: fd=1 hex=48656c6c6f2c20776f726c64210a
POLY-SYSCALL: pid=1 nr=1 args=0x1,0x20019f,0xe,0x0,0x0,0x0 ret=14
POLY-SYSCALL: pid=1 nr=231 args=0x0,0x0,0x0,0x0,0x0,0x0 ret=?
POLY-EXIT: pid=1 code=0
```

## Time, timers and the vDSO

[ADR 0044](adr/0044-time-syscalls-timers-and-the-vdso.md). The personality
has the calls (`personalities/linux/src/sys/time.rs`, registered through
`sys::time::CALLS`) and the timers (`time/`); the kernel has the clocks,
the CPU-time accounting, the timer thread and the vDSO.

```text
  user mode:  __vdso_clock_gettime ── seqlock read of [vvar] ── lfence; rdtsc ──► time
                  │ Mode::Syscall, unknown clock, null pointer
                  ▼
  syscall:    clock_gettime / nanosleep / setitimer / timer_* ... (personality)
                  │ LinuxKernel: clock, set_realtime, cpu_times, block, timers_changed
                  ▼
  kernel:     time.rs (monotonic + settable wall clock) ──publish──► vdso.rs ──► [vvar]
              cputime.rs (per-slot run time, tick samples)
              linux/timers.rs: wake-ups by pid ──► linux-timers thread ──► time::fire ──► signals
```

- **Calls:** `clock_gettime`, `clock_getres`, `clock_settime`,
  `gettimeofday`, `settimeofday`, `time`, `nanosleep`, `clock_nanosleep`
  (relative and `TIMER_ABSTIME`; `EINTR` after a handler, restart after a
  stop through restart_syscall(2) with the deadline it had), `alarm`,
  `getitimer`, `setitimer` (`ITIMER_REAL`, `ITIMER_VIRTUAL`, `ITIMER_PROF`),
  `timer_create` / `settime` / `gettime` / `getoverrun` / `delete`
  (`SIGEV_SIGNAL`, `SIGEV_NONE`, `SIGEV_THREAD_ID`), `times`, `getrusage`
  (the two times), `getcpu`. Clocks: `CLOCK_REALTIME` (settable),
  `CLOCK_MONOTONIC` = `_RAW` = `CLOCK_BOOTTIME`, the coarse clocks (same
  values, the tick as resolution), `CLOCK_PROCESS_CPUTIME_ID`,
  `CLOCK_THREAD_CPUTIME_ID`.
- **Timers** live in each process's `LinuxProcess` (`time::Timers`). After
  a change the kernel records the earliest monotonic expiry (and whether a
  CPU-time timer exists) by pid, with the process's state locked; the
  `linux-timers` thread sleeps until then (a tick at a time for CPU-time
  timers) and calls `time::fire`, which queues the signals (once per
  timer; overruns counted) and re-arms periodic timers; the threads it
  names are notified like for any signal.
- **The vDSO** (`kernel/src/arch/x86_64/vdso/vdso.rs`, built by
  `kernel/build.rs` with the pinned `rustc` and `rust-lld`, embedded as
  `arch::vdso::IMAGE`): an ELF shared object, one read-execute segment, no
  relocations, exporting `__vdso_clock_gettime`, `__vdso_gettimeofday`,
  `__vdso_time`, `__vdso_clock_getres` and `__vdso_getcpu` in `LINUX_2.6`.
  It reads the `poly_vdso::Vvar` page right below its ELF header: a
  seqlock-protected snapshot of the clock's TSC conversion (the same
  `mult`/`shift` the kernel's clock applies), the wall clock's offset, the
  tick and the time zone. Where the clock does not read an invariant TSC
  the snapshot says so and the vDSO makes the system call.
- **Mapping:** exec maps the image (private, read-execute) and the data
  page below it (shared, read-only, never writable: a page-cache page every
  process maps, which the kernel writes through the direct map) and sets
  `AT_SYSINFO_EHDR`.

## NT object manager (logic only, not wired yet)

`crates/poly-ntobj` is what the NT object and synchronisation services do,
without the machine ([ADR 0030](adr/0030-nt-object-manager-and-dispatcher-in-a-crate.md)).
The kernel does not use it yet; the NT personality will call its `nt_*`
services once it decodes those system calls.

- `ObjectManager` holds every object (directories, symbolic links, events,
  mutants, semaphores, timers, and opaque sections, files, processes,
  threads, keys and devices the kernel fills in), the namespace (`\`, `\??`,
  `\GLOBAL??`, `\Device`, `\BaseNamedObjects`, `\Sessions`, `\KernelObjects`),
  one handle table per process plus a kernel table, and the blocked waits.
- Handle count and pointer count per object; temporary names go with the
  last handle, objects with the last reference. Handles are multiples of 4
  with the tag bits ignored; `-1`/`-2` are the current process and thread.
- Waits: the crate returns `Done(status)` or `Blocked(WaitId)` and queues a
  `Wake` for each blocked wait a later call satisfies; the kernel sleeps and
  wakes threads, and cancels waits on timeout, alert or APC.
- A `ParseHook` receives lookups that reach a device (or later a file system
  or the registry), so those own the names below them.
- Behaviour public documentation does not settle is marked "to be confirmed
  by the spec agent against real Windows" in the code; ADR 0030 lists it.

## Testing strategy

| Level | Where | Runs |
|---|---|---|
| Unit tests for pure logic | `crates/`, `personalities/`, `tools/` | `cargo test --workspace` (host) |
| In-kernel tests (`kernel_test!`, next to the code) | `kernel/src/ktest.rs` | every boot; `cargo xtask test` reports each test |
| User mode: hand-assembled ring-3 programs (`user/programs.rs`) making system calls, faulting, switching | `kernel_test!` in `user/tests.rs` | every boot; one boot on `-cpu max` (SMEP, SMAP, XSAVE/AVX) |
| Demand paging and exec: lazily mapped memory touched from ring 3 and by user copies, frames counted on unmap and teardown, static ELF executables assembled in the kernel (`user/test_elf.rs`) that report their initial stack | `kernel_test!` in `user/memory_tests.rs`, `user/exec_tests.rs` | every boot, as above |
| Pipes, waits and event files from ring 3: a pipe between two threads (a blocked read woken by a write, a 200 KB blocking write drained, end of file), non-blocking pipes, `FIONREAD`, `EPIPE` with `SIGPIPE` blocked and the process killed by it unblocked, `poll` / `select` / `epoll` (level- and edge-triggered, a thread woken in `epoll_wait`) on pipes and an event counter, timer expirations (blocking read, epoll, absolute times on both clocks), a signal file reading signals (also blocked in a read until another thread's `tgkill`), a blocked read ended by a handler with `EINTR` and restarted under `SA_RESTART` | `kernel_test!` in `linux/fileio_tests.rs` | every boot |
| Time: the vDSO image and its mapping, the snapshot following the wall clock; sleeps, a POSIX timer's signal through the timer thread, `ITIMER_REAL` / `ITIMER_VIRTUAL` ending a process spinning in ring 3, CPU-time clocks; `/bin/time-test` (a Rust program without a C library, `tools/xtask/programs/time_test.rs`) finding the vDSO through `AT_SYSINFO_EHDR`, resolving its symbols and comparing it with the system calls, timers with signal handlers, CPU time, once with the vDSO forced onto the TSC | `kernel_test!` in `vdso.rs`, `cputime.rs`, `linux/tests/time_tests.rs`; `/bin/time-test` also as the first process | every boot (`time-test` with the test initramfs); `cargo xtask test` |
| Linux processes: the kernel's `LinuxKernel` through real system calls from ring 3 (write to the process's files, brk, anonymous and file mmap/munmap/mprotect, arch_prctl and the FS base in ring 3, exit_group ending a spinning and a futex-waiting thread, EFAULT for bad pointers, a fault as SIGSEGV, a signal handler and rt_sigreturn), and `exec_path` of `/bin/hello` from the test initramfs | `kernel_test!` in `linux/tests.rs` | every boot; one boot on `-cpu max` (XSAVE signal frames) |
| Processes and threads from ring 3: fork with copy on write, zombies (waitid `WNOWAIT`, kill 0) and wait4's status, a thread with TLS joined through `CLONE_CHILD_CLEARTID` and a futex and its robust lock released at exit, an orphan adopted while shared anonymous memory stays shared, vfork + execve of `/bin/hello`, of a missing file and of a `#!` script, execve by a second thread (the other thread ended, close-on-exec files closed, the pid taken over); every user frame freed afterwards. `Memory::fork` itself: pages shared until written, frames freed with the last user, shared anonymous and parked pages | `kernel_test!` in `linux/task_tests.rs`, `user/memory/fork.rs` | every boot |
| Static Linux hello worlds as the first process (`poly.init`): nolibc (Rust, built with the pinned toolchain), freestanding C and static glibc (skipped without a C compiler or static C library); output and exit status 0 | `tools/xtask/src/programs.rs` | `cargo xtask test` |
| `/bin/forkexec` as init (nolibc Rust, `tools/xtask/programs/forkexec.rs`): fork + execve + wait4, copy on write, a TLS thread joined by futex, an orphan reaped by init | `tools/xtask/src/programs.rs` | `cargo xtask test` |
| Negative boot tests (faults, failures and panics must be reported, with a backtrace): unmapped access, write to text, execute stack/data, stack overflow | `poly.selftest.*` flags | `cargo xtask test` |
| The shell over the serial port: `help`, `uptime`, `lspci`, an unknown command, Tab and cursor keys, `test`, `ls` and `cat` on the initramfs, `run /bin/hello`, `reboot` | `poly.shell`, QEMU's stdin | `cargo xtask test` |
| File systems: the test initramfs (built by `cargo xtask` with `poly-cpio`, passed with `-initrd`) unpacked under `/`, its frames freed; devices, descriptors, tmpfs I/O; one boot without an initramfs | `kernel_test!` + `poly.test.initrd` | `cargo xtask test` |
| Disk file systems: ext2 on the RAM disk; the ext2 test disk (made by `cargo xtask` with `poly-ext2`, attached as `virtio2`, mounted on `/mnt`): its files, writes surviving a remount, `read` and mappings sharing a page-cache page, the writeback thread, eviction freeing frames | `kernel_test!` + `poly.test.ext2`, `poly.mount=` | `cargo xtask test` |
| Persistence: a file written through the shell on the ext2 test disk, `sync`, `reboot` (unmounted clean), `e2fsck -fn` on the image (when e2fsprogs are installed), a second boot from the same image reads it | `cargo xtask` (`ext2_persistence_test`) | `cargo xtask test` |
| Device tests against QEMU's devices: PCI config space, MSI on `edu`, the block layer on the virtio-blk test image (interrupt-driven, and in another boot polled) and the RAM disk, the watchdog on a spare virtio-blk device (and a boot that expects the disk without one must fail), the display's mode, mapping, back buffer and flush (and a boot without VGA must fail) | `kernel_test!` + `poly.test.disk`, `poly.test.msi`, `poly.test.display` | `cargo xtask test` |
| Networking: virtio-net on QEMU's user networking (interrupt-driven, and in another boot polled): ping to the gateway, 1 MiB TCP and UDP through echo servers on the host, a connection from the host through a port forward; `ifconfig`, `ping` and `nc` in the shell. Loopback TCP, UDP and ping in every boot | `kernel_test!` + `poly.test.net=<port>`; `cargo xtask` (`net.rs`) | `cargo xtask test` |
| Keyboard end to end: keys typed through the QEMU monitor (`sendkey`) decode to the expected text, US and Swiss German | `kernel_test!` + `poly.test.keyboard=<text>`, `poly.keymap` | `cargo xtask test` |
| What the screen shows: a screenshot through QEMU's QMP socket, compared pixel for pixel with the boot screen `poly-gfx` draws on the host | `cargo xtask` (`screenshot_test`) | `cargo xtask test` |
| The screen terminal: a command typed through the QEMU monitor runs in the screen's shell; screenshots compared with the output row and the status bar drawn on the host | `cargo xtask` (`terminal::boot_test`) | `cargo xtask test` |
| Conformance vs. real OS (diff runner): the `tests/linux/` corpus and spec tests on real Linux and on Poly, compared per test ([AGENT_PIPELINE.md](AGENT_PIPELINE.md)) | `tools/pipeline` | `cargo xtask pipeline diff [--target poly]`; in `cargo xtask ci`: Linux against Linux (`selfcheck`) and Poly against Linux for the tests in `tests/linux/poly-pass.txt` (`polycheck`) |
| App trace replay | recorded sessions | later |

Backtraces, symbolisation and attaching gdb/lldb are described in
[DEBUGGING.md](DEBUGGING.md).

## Memory layout (current)

The kernel page tables (`kernel/src/arch/x86_64/paging.rs`, decisions in
[ADR 0010](adr/0010-kernel-address-space.md)) replace the boot stub's
mapping early in `kernel_entry`:

| Virtual | Physical | What |
|---|---|---|
| `0x0` – `0x7fff_ffff_ffff` | – | user half: empty in the kernel PML4 (the boot identity map is gone); each user address space has its own (below) |
| `0xffff_8000_0000_0000` + phys | every region of the memory map, up to 64 TiB | direct map: RW, NX; RAM write-back, reserved ranges uncached; holes unmapped |
| `0xffff_c000_0000_0000` – `+512 GiB` | device ranges | `mm::map_mmio`: uncached (PCD+PWT), NX, a gap page after each; PCI ECAM windows and BARs. `mm::map_mmio_wc`: write-combining (PWT alone, PAT entry 1), for the framebuffer |
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
| `__nmi_stack_guard` (4 KiB) | **unmapped** | |
| NMI / machine-check IST stack (16 KiB) | read-write, no-execute | |

All kernel mappings are global and use 2 MiB pages (1 GiB where the CPU has
them) wherever alignment allows. No-execute needs EFER.NXE, which is enabled
when CPUID reports NX (without it, pages stay executable and the boot log
says `NX: no`). `arch::phys_to_virt(phys)` translates through the direct map and returns
`None` for addresses outside it; `arch::phys_range_to_virt` checks a whole
range. Before the switch it uses the boot mapping, which is why the first page
tables come from a bump allocator over RAM below 1 GiB, before the buddy
allocator exists. Every PML4 entry of the higher half is created here, at
boot (the MMIO window's as an empty PDPT), so the copies user address
spaces make of it never go stale.

User address spaces (`arch/x86_64/addrspace.rs`, decisions in
[ADR 0022](adr/0022-user-mode-address-spaces-and-system-calls.md)) have their
own PML4: the lower half is theirs, the higher half is the kernel's
(slots 256-511 copied, pointing at the kernel's own PDPTs, supervisor-only).

| User virtual | What |
|---|---|
| `0x0` – `0xffff` | never mapped (`USER_START` is 64 KiB): null pointers and small offsets from them fault |
| `0x1_0000` – `0x7fff_ffff_efff` | user mappings, 4 KiB pages, user-accessible (`USER_START` – `USER_END`); program, heap, `mmap`, stacks (laid out by `exec` and `poly-vm`, below) |
| `0x7fff_ffff_f000` – `0x7fff_ffff_ffff` | never mapped: no `syscall` can sit at the very end of the lower half, so SYSRET never sees a non-canonical RCX |
| `0x8000_0000_0000` – `0xffff_7fff_ffff_ffff` | the non-canonical hole |
| `0xffff_8000_0000_0000` – | the kernel half, as above: present in every address space, ring 0 only |

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
  wake a `WaitQueue`. `irq::count(vector)` counts the interrupts taken on
  each vector (the shell's `irq` command lists them).
- **Time:** `time::now_ns()` is nanoseconds since boot from
  `poly_irq::MonotonicClock`: the TSC when it is invariant and calibrated,
  otherwise the reference timer (under QEMU's TCG: the HPET). It never goes
  backwards, also across threads. `time::ticks()` counts tick interrupts.
  The boot log prints the APIC mode, the HPET, the TSC rate, the clock
  source and the APIC timer rate.
- **Wall clock:** `time::realtime_ns()` reads the CMOS RTC once, at its
  first use (`arch/x86_64/rtc.rs`; the registers are decoded and turned into
  Unix time by the host-tested `poly-rtc`, with the century register the
  FADT names), and adds the monotonic clock since; `time::set_realtime_ns`
  moves it (never below the monotonic clock). Linux's `CLOCK_REALTIME`
  reads it. The file systems' timestamps still count from boot.
- **CPU time** (`cputime.rs`, ADR 0044): each thread-table slot's run time,
  measured with the monotonic clock at every context switch, split into
  user and system time by tick samples (the tick records whether it
  interrupted ring 3); a reaped thread's time moves to its process.

## Threads and scheduling

Kernel threads, some of which run user code (user threads, below), on one
CPU, round robin, preempted by the timer. See
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
- **API:** `spawn(fn, arg)` (or `spawn_named(name, fn, arg)`; the boot and
  idle threads are named too), `yield_now()`, `exit(code)`, `current()`,
  `join(id)` (returns the exit code), `detach(id)`, `state(id)`, `list()`
  (every thread with its name and state, for `ps`),
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
  once the thread is also joined or detached. Its user state is dropped
  there too, in whatever thread switched (maybe inside the tick's
  handler, maybe holding sleeping locks): drop code that would sleep or
  take a lock that does not mask interrupts checks `thread::reaping()`
  and defers that part (a `user::Memory` hands its files and page-cache
  pages to the writeback thread, `fs::cache::release_later`).
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

## User mode and system calls

`kernel/src/user/` is the architecture-neutral side, `arch/x86_64/`
(`user.rs`, `syscall.rs`, `addrspace.rs`, `fpu.rs`, `usercopy.rs`) the
machine; the decisions are in
[ADR 0022](adr/0022-user-mode-address-spaces-and-system-calls.md).

- **Address spaces.** `user::AddressSpace::new()` makes a PML4 with an empty
  lower half and the kernel's higher half. `map_zeroed(virt, len, prot)`
  maps fresh zeroed frames that the space owns, `map_frames` (unsafe) maps
  frames the caller keeps owning (shared memory), `unmap` (holes skipped),
  `protect`, `translate`; `read` and `write` reach a space through the
  direct map whether or not it is loaded (a loader filling a new process).
  Ownership is a bit the CPU ignores in each leaf (`PageFlags::SOFTWARE_9`).
  Dropping the last `Arc` frees every owned frame, every page table of the
  lower half (`poly_paging::PageTables::clear`) and the PML4. All of it is
  4 KiB pages under a per-space spinlock (taken before `FRAMES`).
- **CR3 and the TLB.** The scheduler loads the address space of each user
  thread it switches to, and the kernel PML4 for kernel threads. Changing a
  loaded space invalidates the page (`INVLPG`); without PCIDs every CR3 load
  flushes the rest. With PCIDs (CPUID.01H:ECX.PCID; QEMU's TCG has none, so
  this path is only exercised on KVM or hardware) spaces take PCIDs 1-255
  round robin, a load skips the flush when the PCID is still the space's,
  and a space that changed while not loaded flushes on its next load.
  Kernel mappings are global and never flushed.
- **Threads.** `user::spawn(process, space, entry, hook)` starts a thread
  of `process` in `space` at `entry` (RIP, RSP, three arguments in RDI, RSI,
  RDX). It is a kernel thread with a 16 KiB kernel stack whose user state
  (`user::UserThread`: process, hook, address space, FPU state, FS/GS bases)
  lives in the thread table; it enters ring 3 with IRETQ and comes back on
  system calls, interrupts and exceptions. Join it like any thread.
- **Entering the kernel.** IA32_STAR/LSTAR/FMASK and EFER.SCE are set at
  boot. `syscall` lands in an assembly stub that does `swapgs` (GS base is
  the per-CPU area in ring 0, the user's GS base in ring 3), switches from
  the user stack to the thread's kernel stack (from the per-CPU area), and
  pushes the same `TrapFrame` an interrupt from ring 3 leaves. Interrupts
  and exceptions from ring 3 arrive on TSS.rsp0, the same stack; their entry
  swaps GS when the saved CS is ring 3. Both are set by the scheduler on every
  switch to a user thread. NMI and machine check have an IST stack of their
  own (they can hit the instructions before the stack switch), the double
  fault keeps its own; all gates have DPL 0, so `int n` from ring 3 is a #GP.
- **System calls.** The handler copies the frame into a
  `poly_abi::SyscallRegs`, enables interrupts and calls the thread's hook,
  `fn(&Process, &mut SyscallRegs)`: `user::DEFAULT_HOOK` routes to
  `syscall::dispatch` (the process's personality). A hook may block, sleep,
  copy user memory, end the thread (`user::exit`) or make it resume
  elsewhere (`user::set_return_address`). The return is SYSRET when the
  return address is in the lower half and IRETQ otherwise (SYSRET to a
  non-canonical address would fault in ring 0 on the user's stack); an IRETQ
  that faults on its way to ring 3 kills the thread. RFLAGS is sanitised
  (IF on; IOPL, NT, RF, VM off); RCX and R11 hold RIP and RFLAGS afterwards.
- **Faults.** A CPU exception in ring 3 never reaches the kernel's panic
  path. In a Linux process it becomes a signal (see "Linux processes"); in
  a bare process it kills the thread: one log line (`user: pid P thread T
  killed: page fault (vector 14) at rip ..., error ..., address
  ...`) and `user::killed(vector)` as its exit code, while the other
  threads go on.
- **Register frame.** While a user thread is in the kernel from ring 3,
  its `TrapFrame` sits at the top of its kernel stack whichever way it
  came in; `user::registers()` and `user::set_registers()` read and
  replace it (a signal handler's entry, `rt_sigreturn`), and a system call
  whose registers were replaced returns with IRETQ.
- **User memory.** `user::copy_from_user`, `copy_to_user` and
  `strncpy_from_user` work on the loaded address space: they refuse ranges
  that leave the user range, and run the copy in an assembly routine listed
  in an exception table, so a page fault on user memory inside it resumes at
  the routine's fixup and the call returns `Efault`. A fault on a kernel
  address there stays a kernel bug (panic). With SMAP the copies run between
  STAC and CLAC (interrupt entry clears AC), with SMEP the kernel cannot
  execute user pages; without them (QEMU's default CPU) the range checks
  still hold.
- **FPU state.** The kernel is soft-float. Each user thread has an XSAVE
  area (x87, SSE, AVX when present; FXSAVE's 512 bytes without XSAVE) on the
  heap. The registers always hold one user thread's state (the owner):
  switching to another user thread saves the owner's and restores the new
  one's, switching to kernel threads or back to the owner costs nothing; no
  #NM traps. IA32_FS_BASE and the user GS base are saved and restored per
  thread (`user::set_fs_base`, `set_gs_base`; FSGSBASE stays off).

```text
user: syscall/sysret on; PCID: no, SMEP: yes, SMAP: yes; FPU state: XSAVE, 832 bytes, XCR0 0x7
user: pid 4242 thread 5.20 killed: page fault (vector 14) at rip 0x4000c3, error 0x7, address 0x400000
```

## Memory maps, demand paging and exec

`kernel/src/user/memory.rs` and `exec.rs`; decisions in
[ADR 0036](adr/0036-demand-paging-and-exec.md). The region logic is
`poly-vm` ([ADR 0026](adr/0026-user-address-space-regions.md)), the ELF
logic `poly-elf` ([ADR 0024](adr/0024-elf-loader.md)).

- **`user::Memory`** is a process's memory (Linux's `mm`): an
  `AddressSpace` plus the `poly_vm::VmMap` of its regions behind a `Mutex`.
  `user::spawn_in(process, memory, entry, hook)` starts a thread in it, and
  its threads share it; `user::current_memory()` gives the running
  thread's. Regions are created empty: `map_anonymous`, `map_file`
  (private), `map` (any `poly_vm::MapRequest`), `unmap`, `protect`,
  `init_brk` / `brk`, `setup_stack`. Unmapped pages' frames are freed at
  once; populated pages of a region made `PROT_NONE` are parked (kept out
  of the page tables) until it is accessible again or unmapped.
- **Page faults.** A fault from ring 3 enables interrupts and calls
  `user::page_fault(addr, access)`, which asks `VmMap::fault` what the fault
  means and does it: map a zeroed frame, map a frame filled from the file,
  or upgrade the entry's rights. Success retries the instruction; anything
  else kills the thread as before, with the reason in the log line. A fault
  inside `copy_from_user` / `copy_to_user` / `strncpy_from_user` goes the
  same way before the fixup, if the copy ran with interrupts on, so a valid
  page that is not present yet never makes a copy fail. The stack region
  grows down on a fault below it, within the 8 MiB limit and above the
  guard gap. `Memory::read` / `write` do the same for the kernel's own
  accesses to a space that is not loaded.
- **Backing.** A file region maps a `FileRef`: an `Arc<dyn PageSource>`
  (`len`, `read_at`, optionally the bytes in memory; `MemoryImage` wraps
  kernel bytes, `fs::cache::FileSource` a VFS file) and a limit past which
  pages read as zero. A source on a disk mount hands out the page cache's
  pages (`PageSource::cached_page`): the memory maps the cache's frame
  itself (not owned by the address space) and holds the page while it is
  mapped, read-only in a private region and copied on the first write
  (`CopyOnWrite`), read-only then writable on the first write in a shared
  one (`MAP_SHARED`, `Memory::map_shared_file`), whose writes thereby
  reach the file. Other sources (and pages that cross a region's limit)
  get private frames, and shared mappings of them are refused (ADR 0040).
- **Frames.** `UserFrame` is a frame on its way into or out of an address
  space (`AddressSpace::map_owned`, `take`); dropping one frees it.
  `user::user_frames()` counts every frame held for user pages and
  `Memory::resident_pages()` those of one memory, which is how the tests
  check that nothing leaks.
- **Fork and copy on write** (ADR 0041, `user/memory/fork.rs`).
  `Memory::fork` copies the region map into a new address space and shares
  every resident page of private regions: the memory's own frames become
  `SharedFrame`s (a `UserFrame` behind an `Arc`, `AddressSpace::disown`),
  mapped read-only on both sides; parked frames are shared unmapped;
  shared regions are found again through the page cache or the
  `AnonShared` object behind `MAP_SHARED | MAP_ANONYMOUS`. A write to a
  shared frame with other users copies it; its last user takes it back
  (`AddressSpace::adopt`). A frame is freed with its last reference.
- **Exec.** `user::exec::load(file, &ExecParams { argv, envp, execfn,
  creds })` validates a static executable (`ET_EXEC`, or `ET_DYN` without
  `PT_INTERP`, at `0x5555_5555_4000`; `PT_INTERP` is refused), creates a
  region per planned mapping (segment file bytes through a `FileRef` window
  ending at `p_filesz`, so last pages have zero tails; `.bss` anonymous),
  starts the break after the highest segment, creates a 128 KiB stack
  region under `0x7fff_ffff_f000` and writes the psABI initial stack
  (`argc`, `argv`, `envp`, auxiliary vector with `AT_PHDR`, `AT_PHENT`,
  `AT_PHNUM`, `AT_PAGESZ`, `AT_ENTRY`, `AT_RANDOM`, IDs, `AT_SECURE`,
  `AT_EXECFN`, `AT_PLATFORM`, `AT_HWCAP`, `AT_SYSINFO_EHDR` for the vDSO
  it maps, ...). Only the stack page is
  resident when it returns. `exec::exec` (any `PageSource`) and
  `exec::exec_image` (an `Arc<[u8]>`) also start the thread.

| User virtual (after exec) | What |
|---|---|
| `0x40_0000` (`ET_EXEC`) or `0x5555_5555_4000` (static PIE) | the image's segments, then `.bss` |
| page after the highest segment | the program break (`brk`), growing up |
| below `0x7fff_f7ef_f000` (128 MiB for the stack and a 1 MiB guard gap under its top) | `mmap` area, top-down; its first mappings are the vDSO's data page and, above it, the vDSO image (`AT_SYSINFO_EHDR`) |
| `0x7fff_ffff_f000` - 128 KiB – `0x7fff_ffff_f000` | the stack region (grows to 8 MiB) |

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
- `RawMutex`: a `Mutex<()>` locked and unlocked by hand, the
  `poly_vfs::lock::RawLock` the file system crates take (ext2's volume
  lock, the page cache's inode and namespace locks).
- `SetOnce<T>`: written once at boot, read without a lock (local APIC,
  clock).

## Kernel shell

A shell built into the kernel, the debug console of the Phase 2 terminal
([ADR 0021](adr/0021-kernel-shell.md)). It never touches a device: it talks
to a `shell::Console`, so the same shell runs on the serial port and on
the screen terminal (its `ScreenConsole`, see "Screen terminal").

```
   COM1 bytes ──► IRQ 4 handler ──► ring ──► TermDecoder ──┐
   key events ─► poly_term::keys ─► bytes ─► TermDecoder ─┤
                                                           ▼
   Console::read ──Edit──► poly_line::Editor ──line──► words::split ──► COMMANDS[name](console, args)
                                  │ redraw: text + ANSI                         │ output
                                  └──────────────► Console (fmt::Write) ◄───────┘
```

- **`poly-line`** (host-tested, uses `alloc`) is the line editor: a line of
  `char`s with a cursor, edited by abstract `Edit`s (insert, Backspace,
  Delete, Left / Right, word left / right, Home / End, kill to end / start /
  word, history previous / next, complete, Enter, cancel, clear screen,
  redraw). It writes text and a few ECMA-48 sequences (cursor left / right
  by n, erase to end of line, erase screen, cursor home; SGR in the prompt),
  always moving relative to the cursor, one column per character. History
  keeps 64 lines without blank lines or a repeat of the newest; browsing it
  keeps the line being typed. Tab asks a callback for candidates, completes
  their common prefix (and a space after a unique one) and lists them in
  columns on a second Tab. `TermDecoder` turns a terminal's byte stream
  into edits (UTF-8, `CR`/`LF`/`CR LF`, `DEL`/`BS`, `ESC [` / `ESC O`
  arrows, Home, End, Delete, Ctrl-arrows, Ctrl-letters); `Edit::control`
  maps Ctrl-letters for a keyboard. `words` splits a line into arguments
  (whitespace, optional quotes, backslash escapes) and finds the closest
  word for a typo.
- **`Console`** (`kernel/src/shell/mod.rs`): `core::fmt::Write` for output
  (`"\n"` means a new line; the console adds a CR if it needs one) and
  `read() -> Option<Edit>`, which blocks; `None` ends the shell (only
  scripted consoles in tests do that). `shell::spawn(name, console)` runs
  the shell in its own kernel thread, `shell::run(console)` on the calling
  one.
- **Serial console** (`shell/serial.rs`): COM1's receive interrupt is routed
  with `arch::irq::route_isa_irq(4, ...)` (the UART raises it for each
  byte); the handler moves bytes into a 256-byte lock-free ring (one
  producer, the handler; one consumer, the console) and wakes the reader.
  Output goes through `log::write_console`, the same lock as the kernel
  log. While the shell waits for input with its prompt shown, a kernel log
  line first erases the prompt row, and the shell draws the prompt and
  line again under it (`Edit::Redraw`) within 50 ms.
- **Commands** (`shell/commands.rs`, one static table of name, usage, help
  and handler): `help`, `mem` (frames and heap), `ps` (`thread::list`),
  `lspci` (`pci::devices`, with class names from `poly_pci::ClassCode::name`),
  `disks` (`block::disks` with cache and queue counters), `ls`, `cat` and
  `mount` (through `fs::Files` and `fs::vfs()`; with arguments it mounts a
  disk), `umount`, `write` (a line into a file), `sync`, `uptime`, `irq`
  (per-vector counts), `cpu` (CPUID: vendor, brand, family / model /
  stepping, features), `test [filter]`, `echo`, `clear`, `reboot`. An unknown
  command gets the closest command name as a suggestion.
- **Network commands** (`shell/net.rs`): `ifconfig` (and `ip`, the same)
  shows `eth0` (device, link, MTU, MAC, addresses, driver and stack
  counters); `ping [-c count] <ipv4>` sends ICMP echo through a ping
  socket, one a second; `nc <ipv4> <port> [text...]` connects over TCP,
  sends the text, closes its side and prints the answer. They use the
  blocking sockets of `net::socket`, like a program would.
- **`test`** runs the kernel tests again with `ktest::run(env, filter,
  When::Runtime, console)`: the boot's `poly.selftest.*` flags are dropped,
  tests marked `#[boot_only]` (they check the state at boot, or must run on
  the boot thread) are left out, and only one run happens at a time, so a
  test that drives the shell cannot run the tests inside itself.
- **`reboot`** (`power::reboot`) unmounts the disk file systems clean
  (`fs::disk::shutdown`), writes back every disk's cache, then tries
  the FADT reset register (`acpi::Platform::reset`), the 8042's reset line
  and a triple fault (`arch::reset`), half a second each.
- The kernel tests drive the real shell with a scripted console that types
  terminal bytes; `cargo xtask test` boots with `poly.shell` and types into
  QEMU's stdin, which is the serial port.

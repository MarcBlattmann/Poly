# Debugging the kernel

Four tools, from quickest to most thorough: the serial log, the kernel
shell, the panic backtrace, and a debugger attached to QEMU.

## Serial log

Everything the kernel prints goes to COM1, which `cargo xtask run` connects
to your terminal, shared with QEMU's monitor: Ctrl-A C switches between the
two, Ctrl-A X quits. The screen opens in QEMU's own window (see "The
screen" below), where the screen terminal shows the same log (in grey)
from the last 64 KiB of the boot on. Anything after the command is passed on as the kernel
command line:

```sh
cargo xtask run                          # plain boot, into the shell
cargo xtask run poly.test=sync           # run only the in-kernel tests whose name contains "sync"
cargo xtask run poly.selftest.page_fault # boot, then fault on purpose
```

The negative boot tests use these `poly.selftest.*` flags, which fault on
purpose after the in-kernel tests: `page_fault` (unmapped address),
`write_text` (write to kernel code), `exec_stack` and `exec_data` (jump into
no-execute memory) and `stack_overflow` (run the boot stack into its guard
page). Each prints the address it is about to touch first.

To keep a copy, add `| tee serial.log`.

## The kernel shell

After the in-kernel tests, `cargo xtask run` drops into the kernel shell on
the same serial console (any boot without `poly.qemu_exit` does; the boot
tests add `poly.shell` to get it after their tests too):

```text
POLY-SELFTEST: PASS
shell: serial input on COM1, ISA IRQ 4 -> vector 0x31
boot: done; starting the shell on COM1 (type 'help')
poly> uptime
up 0:00:02.418 (2418836190 ns), 2410 ticks at 1000 Hz, 3 threads
poly> lspci
00:00.0 8086:29c0 host bridge       06.00.00
00:03.0 1af4:1042 SCSI storage      01.00.00  BAR1 mem 0xfebd5000 4 KiB  BAR4 mem 0xfe000000 16 KiB
...
9 functions
poly> hlep
unknown command 'hlep'; did you mean 'help'?
```

| Command | Shows or does |
|---|---|
| `help [command]` | every command with one line of help, or one command |
| `mem` | RAM total / free / used from the frame allocator; heap bytes in use, pages held, large runs, failed allocations; the page cache's pages, frames, dirty pages, limit, hits, misses, pages read, written and evicted |
| `ps` | every thread: id (slot.generation), state (running, runnable, blocked, exited) and name |
| `lspci` | every PCI function: address, vendor:device, class name and code, BARs |
| `disks` | every disk: description, size, interrupt vector or polled, write cache; cache buffers, dirty, hits, misses, read-ahead, write-backs, evictions; queue requests, merges, splits, interrupts, sleeps |
| `ls [path]` | a directory (default `/`) sorted by name, or one file: mode, size (or device number), name, link target |
| `cat <file>...` | the files' contents (at most 64 KiB each, so `cat /dev/zero` ends) |
| `mount [<disk> <path> [ro]]` | every mount: source (the disk for a disk mount), mount point, type, options; with arguments, mount the ext2 volume on `<disk>` (`virtio2`, `ram0`, ...) on `<path>` (made if missing) |
| `umount <path>` | unmount the disk mounted on `<path>`: written back, synced and marked clean; refused while files there are open or mapped |
| `write <file> [words...]` | write the words and a newline to `<file>` (created or truncated) |
| `sync` | write back every dirty cached page and sync every disk file system and disk |
| `uptime` | time since the clock started, ticks, threads |
| `irq` | interrupts taken per vector with their owner (timer, serial, disk, ...), and the spurious / unhandled / 8259 / APIC error counters |
| `cpu` | CPUID vendor, brand string, family / model / stepping, TSC rate, APIC ID, feature flags |
| `ifconfig`, `ip` | the network interface `eth0`: device, interrupt vector or polled, link, MTU, MAC, IPv4 and IPv6 addresses, frames and bytes in and out, drops, stack counters, interrupts and polls; and loopback |
| `ping [-c count] <ipv4>` | ICMP echo requests (default 4, one a second), each reply's round-trip time, loss and min/avg/max |
| `nc <ipv4> <port> [text...]` | a TCP connection: sends the text and a newline, closes its side, prints what comes back until the peer closes (or 5 s of silence) |
| `test [filter]` | run the in-kernel tests whose name contains `filter` (all without one), with the same output as at boot |
| `echo [words]` | print the words (quotes group words: `echo 'a  b'`) |
| `clear` | clear the screen |
| `reboot` | unmount the disk file systems clean, write back the disk caches and reset (ACPI reset register, then the 8042, then a triple fault). `cargo xtask` runs QEMU with `-no-reboot`, so QEMU exits |

Keys: Left / Right, Ctrl-Left / Ctrl-Right (or Alt-B / Alt-F) by word,
Home / End (Ctrl-A / Ctrl-E), Backspace, Delete, Ctrl-K / Ctrl-U kill to the
end / start of the line, Ctrl-W the word before the cursor, Up / Down
(Ctrl-P / Ctrl-N) recall earlier lines, Tab completes command names (and
test names after `test`; a second Tab lists the choices), Ctrl-C drops
the line, Ctrl-L clears the screen. Ctrl-A is QEMU's escape key under
`cargo xtask run` (`-serial mon:stdio`), so press it twice (Ctrl-A Ctrl-A)
to send it to the shell, or use Home.

`test` runs the tests on the shell's thread while the rest of the kernel
keeps running. It leaves out the tests marked `#[boot_only]` (they check
state from boot, such as every I/O APIC input being masked, or must run on
the boot thread) and ignores the boot's `poly.selftest.*` flags, so no test
panics on purpose. Tests that print go to the same serial console.

A kernel log line that arrives while the prompt is waiting erases the
prompt row and the shell draws it again under the log line, within 50 ms.
If a line ever looks garbled anyway, Ctrl-L redraws it. Characters wider
than one column (East Asian, emoji) and lines longer than the terminal is
wide are edited correctly but drawn wrong.

**The shell on the screen.** With a display, a second shell runs in the
screen terminal (thread `shell-screen`), typed on the guest's keyboard:
in the QEMU window, or with `sendkey` from the monitor (see "Keyboard
input"). It has the same commands and keys, its own history, and the log
behaves the same: a line arriving while you type goes above the prompt,
which is drawn again below it. Shift+Page Up / Page Down scroll back
through 1000 lines; typing scrolls back down. See "The screen" below.

The shells do not start in an automated test boot (`poly.qemu_exit`
without `poly.shell`). If the COM1 interrupt cannot be routed, the boot log
says `shell: COM1 receive interrupt not routed (...); polling`, and the
shell polls the UART every 10 ms instead.

## PCI devices and the test disk

The boot log has one `pci:` line per function (address, vendor:device,
class, BARs with their sizes), the virtio-blk bring-up and one `block:`
line per disk, then the first bytes of the root disk read through the
block layer:

```text
pci: 00:03.0 1af4:1042 class 01.00.00, BAR1 mem32 0xfebd5000 4 KiB, BAR4 mem64 pf 0xfe000000 16 KiB
pci: 00:04.0 1af4:1042 class 01.00.00, BAR1 mem32 0xfebd6000 4 KiB, BAR4 mem64 pf 0xfe004000 16 KiB
pci: 00:05.0 1234:11e8 class 00.ff.00, BAR0 mem32 0xfea00000 1 MiB
virtio-blk: 00:04.0: not started; comes up as virtio1 when asked for (a mount)
virtio-blk: 00:06.0: not started; comes up as virtio2 when asked for (a mount)
virtio-blk: 00:03.0: MSI-X with 3 vectors, table BAR1+0x0, PBA BAR1+0x800
virtio-blk: 00:03.0: 1 MiB (2048 sectors), block size 512, features 0x120000244, queue 128 entries, 126 segments per request, write cache
virtio-blk: 00:03.0: request queue on MSI-X entry 0 -> vector 0x30; interrupt-driven, waiters sleep
block: virtio0: virtio-blk 00:03.0, 2048 blocks of 512 bytes (1 MiB), write cache; queue 32 requests, cache 128 buffers (64 KiB); interrupt-driven (vector 0x30)
block: ram0: RAM disk, 512 blocks of 512 bytes (256 KiB); queue 32 requests, cache 128 buffers (64 KiB); polled
block: virtio0: block 0: 50 4f 4c 59 00 00 00 00 08 09 0a 0b 0c 0d 0e 0f  POLY............
```

00:04.0 is the spare virtio-blk device (QEMU's null block driver), used
only by the watchdog test; 00:05.0 is QEMU's `edu` device, used by the MSI
test. A device without MSI-X (or whose MSI-X cannot be set up) logs why and
is polled instead:

```text
virtio-blk: 00:03.0: no MSI-X; polling
virtio-blk: 00:03.0: polled: waiters run the queue and yield
block: virtio0: virtio-blk 00:03.0, ... cache 128 buffers (64 KiB); polled
```

`features` is what the driver negotiated (bit 32 is VERSION_1, bit 29
EVENT_IDX; the virtio-blk bits are in VIRTIO 1.2 §5.2.3). "Segments per
request" is the device's `seg_max` minus the header and status descriptors
(the driver counts them in, to be safe), capped by the queue size. To compare with
QEMU's own view, open the monitor (Ctrl-A C under `cargo xtask run`, or
`monitor <cmd>` from gdb) and use:

| Command | Shows |
|---|---|
| `info pci` | every function with its IDs, IRQ pin and BAR addresses as QEMU sees them (not its MSI or MSI-X state) |
| `info qtree` | the device tree, including the virtio-blk properties and MSI-X vector count (`vectors = 3`) |
| `info block` | the drives and the image file behind `disk0` |
| `info mtree` | the guest physical address map: where ECAM (`pcie-mmcfg-mmio`) and each BAR sit |
| `xp /4wx <table>` | an MSI-X table entry, read from the BAR at its physical address (table BAR + offset from the boot log, 16 bytes per entry): address low, high, data, vector control |

For virtio0 the table is at BAR1 + 0, so `xp /4wx 0xfebd5000` should show
`0xfee00000 0x00000000 0x00000030 0x00000000`: the boot CPU's local APIC
(destination 0), vector 0x30, unmasked (bit 0 of the last word clear).
`info irq` does not help with MSI or MSI-X: it counts only I/O APIC and
8259 inputs, and messages go straight to the local APIC. To see them,
run QEMU by hand with `-trace msix_write_config` (each change of the
enable and function-mask bits) and `-trace apic_deliver_irq` (every message
delivered, with its vector), or compare the kernel's own counters: the
`interrupts` and `sleeps` in `block::Disk::stats()`, which the tests
print. `info lapic` shows a vector stuck in the IRR or ISR.

`cargo xtask run` and the boot tests attach a fresh test image,
`target/poly-test-disk.img` (1 MiB, raw), as a modern-only virtio-blk device
(`-device virtio-blk-pci,drive=disk0,disable-legacy=on`) and pass
`poly.test.disk`, which makes the `block::*` and `virtio_blk::*` tests
require it (and the spare device, `-blockdev driver=null-co,...`). They
also attach `-device edu` and pass `poly.test.msi`, which makes
`pci::msi_without_msix_reaches_its_handler` require it. Adding
`vectors=0` to a virtio-blk device takes its MSI-X away; one boot test does
that to check the polled fallback. Every 512-byte sector starts with "POLY" and its number
(le32); the rest is a pattern of the sector and the offset
(`block::test_pattern`). The block tests run against the RAM disk too,
which they fill with the same pattern first:

```sh
od -A x -t x1z -N 16 -j $((512 * 7)) target/poly-test-disk.img   # sector 7
cargo xtask run --no-disk                                       # boot without the disk (ram0 only)
cargo xtask run poly.test=block                                 # only the block layer tests
```

The write tests change blocks 1 and 100 to 243 and put them back, and
xtask rewrites the image before every boot anyway. A failing block test
prints the disk, the operation and the error first, e.g. `block: virtio0:
sync: Io(DeviceFailed)`. To watch the device side, run QEMU by hand with
the same arguments plus QEMU trace events, e.g. `-trace 'virtio_blk_*'
-trace virtio_queue_notify -trace 'pci_cfg_*'` (printed on stderr with
the log trace backend that distribution builds use).

Errors to know (`poly_block::IoError`, wrapped in `CacheError::Io` by the
cache):

| Error | Means |
|---|---|
| `BadBuffer` | the buffer is not in the direct map (on the boot stack, say), so the device cannot be given its address; nothing was sent. Use heap memory. |
| `DeviceFailed` | the device broke the virtqueue protocol, or requests were out and none came back for the watchdog's limit (10 s on the monotonic clock): the device was reset and marked FAILED, and every later request fails the same way |
| `OutOfRange`, `BadRequest` | past the end of the disk, or not a whole number of blocks; refused before reaching the device |
| `AllPinned` | every cache buffer is pinned, so nothing can be evicted for a new block |

A thread waiting for an interrupt-driven disk is `Blocked` (on the disk's
wait queue) and wakes when the interrupt handler finishes requests, or at
the watchdog's deadline at the latest. If disk I/O hangs for the full 10
seconds and then fails with `DeviceFailed`, the device stopped answering or
its interrupts do not arrive: check the MSI-X entry and trace the
deliveries as above. `VirtioBlk::set_drop_notifications(true)` loses
requests on purpose (the watchdog test uses it on the spare device).

## Networking

`cargo xtask run` attaches a virtio-net device on QEMU's user networking
(`-netdev user,id=net0 -device virtio-net-pci,netdev=net0,disable-legacy=on`)
and passes `poly.net=qemu`, so the guest is 10.0.2.15/24 with QEMU's
virtual router (which is also the way to the host) at 10.0.2.2 and its DNS
server at 10.0.2.3. The boot log says what came up:

```text
virtio-net: 00:02.0: MSI-X with 4 vectors, table BAR1+0x0, PBA BAR1+0x800
virtio-net: 00:02.0: MAC 52:54:00:12:34:56, MTU 1500, link up, features 0x120018020, queues 128 rx / 128 tx entries
net: eth0: virtio-net 00:02.0; queues on MSI-X entry 0 -> vector 0x31; interrupt-driven
net: eth0: inet 10.0.2.15/24 gateway 10.0.2.2 dns 10.0.2.3; inet6 fe80::5054:ff:fe12:3456/64
```

`features` is what the driver negotiated: bit 5 MAC, 15 MRG_RXBUF, 16
STATUS, 29 EVENT_IDX, 32 VERSION_1 (VIRTIO 1.2 §5.1.3, §6). Without a
virtio-net device the log says `net: no network device; loopback only`,
and 127.0.0.1 still works (QEMU's own default NIC, an e1000e, which q35
adds when no `-netdev` is given, is not driven). `poly.net=off` leaves the
device alone; a malformed `poly.net=` is reported and ignored.

```text
poly> ping -c 2 10.0.2.2
PING 10.0.2.2: 56 data bytes
64 bytes from 10.0.2.2: icmp_seq=1 time=4.676 ms
64 bytes from 10.0.2.2: icmp_seq=2 time=0.511 ms
--- 10.0.2.2 ping statistics ---
2 packets transmitted, 2 received, 0% packet loss
rtt min/avg/max = 0.511/2.594/4.676 ms
```

Under user networking only pings to 10.0.2.2 are answered (QEMU's
documentation: ICMP does not work in general). TCP and UDP to 10.0.2.2
reach the host's 127.0.0.1, so a listener on the host is a quick partner:
run `nc -l 127.0.0.1 9000` (or `python3 -m http.server 9000 --bind
127.0.0.1`) on the host and `nc 10.0.2.2 9000 hello` in the guest. To
reach a port in the guest from the host, add a forward to the `-netdev`
by hand (`hostfwd=tcp:127.0.0.1:<host port>-:<guest port>`; the boot tests
use guest port 7777).

When traffic does not flow:

- `ifconfig` shows the counters on both sides of the driver. `rx` frames
  that never reach a socket show up in the stack's `ignored`, `malformed`
  or `bad checksums`; `tx busy` counts frames refused because all 128
  transmit buffers were out. `interrupts` should grow with traffic on an
  interrupt-driven device; if only `polls` grow, check the MSI-X entry as
  for the disk (`xp /4wx <table>`, see above).
- A device that breaks the virtqueue protocol is reset and the log says
  `net: eth0: the device broke the virtqueue protocol and was reset`;
  `ifconfig` then shows `link failed`.
- QEMU can record every frame: add `-object filter-dump,id=f0,netdev=net0,file=net.pcap`
  and open `net.pcap` in Wireshark or `tcpdump -r`. Trace events show the
  device side: `-trace 'virtio_net_*' -trace virtio_queue_notify`.
- The network thread is `net` in `ps`: `blocked` when idle. A socket
  call that hangs is a thread `blocked` on the readiness wait queue; it
  wakes whenever a poll moves packets or a timer fires.

`cargo xtask test` runs three network boots: one interrupt-driven and one
polled (`vectors=0`) with `poly.test.net=<port>`, where the kernel's
`net::tests` ping 10.0.2.2, push 1 MiB through a TCP echo server and
datagrams through a UDP echo server that xtask runs on the host's
`<port>`, and accept a connection that xtask makes through a host forward
once the kernel prints `net: test: listening for the host on port 7777`;
and one that types `ifconfig`, `ping` and `nc` into the shell. To run the
kernel side by hand: start an echo server on the host (`socat
TCP-LISTEN:9000,fork,bind=127.0.0.1 EXEC:cat` and the same for
`UDP-LISTEN`), then `cargo xtask run poly.test=net:: poly.test.net=9000`
(the host-forward test then waits 30 s for a connection to the guest's
port 7777 unless one is made).

## The screen

`cargo xtask run` shows the guest's screen in a QEMU window when there is a
desktop: always on macOS, on Linux when `DISPLAY` or `WAYLAND_DISPLAY` is
set. Serial output and the monitor stay in the terminal. Without a desktop,
or with `cargo xtask run --headless`, there is no window (`-display none`);
the display is still there, and screenshots still work.

The boot log says what the display driver did:

```text
mm: direct map at 0xffff800000000000: ...; write-combining (PAT): yes
pci: 00:01.0 1234:1111 class 03.00.00, BAR0 mem32 pf 0xfd000000 16 MiB, BAR2 mem32 0xfebd4000 4 KiB
display: 00:01.0 1280x800x32, 5120 bytes per line; framebuffer 0xfd000000 (16 MiB) mapped write-combining; registers through MMIO (BAR 2)
```

```sh
cargo xtask run poly.display=1920x1080   # another mode (default 1280x800; 32 bits per pixel)
cargo xtask run poly.display=off         # leave the VGA alone (text mode)
cargo xtask run poly.test=display        # only the display tests
```

A mode the device cannot show (larger than its maximum or its 16 MiB of
video memory) falls back to the default with a line saying why, e.g.
`display: 00:01.0: 9000x9000x32: the mode needs 324000000 bytes but the
framebuffer has 16777216; trying 1280x800x32`. Without the device (QEMU's
`-vga none`) the log says `display: no QEMU standard VGA or
bochs-display`. QEMU's `-device VGA,vgamem_mb=32` gives more video memory.

**The screen terminal.** After the kernel tests the display shows the
terminal (`kernel/src/terminal.rs`, logic in `poly-term`): a status bar on
the top row and a shell below it, which starts with the end of the boot
log. The boot log says what it set up:

```text
terminal: 160x49 cells in Spleen 8x16 on the display, status bar on top, 1000 lines of scrollback; a shell on the keyboard
```

The status bar reads ` Poly 0.0.1 | up 0:01:23 | 243 MiB free of 255 MiB |
us | 1 of 2 CPUs` and, on the right, `Shift+PgUp: scrollback` (or, while
scrolled back, `scrollback 48 of 300 lines`). It is refreshed every second
by the render thread `terminal`; if the uptime stops moving, that thread
is stuck (`ps` on the serial shell shows its state). Kernel log lines are
grey, the shell's output in the terminal's colours.

```sh
cargo xtask run poly.terminal=off        # keep the boot screen; no shell on the screen
cargo xtask run poly.display=1920x1200   # 12x24 font from 1920x1200 up
cargo xtask run poly.test=terminal       # only the terminal's tests
```

Log output reaches the screen up to 25 ms after the serial port. If more
than 64 KiB arrive between two looks, the oldest part is skipped and the
screen says `[N bytes of kernel log lost]`. While kernel tests run (the
`test` command from the serial shell), the screen's shell takes no keys,
since the keyboard tests read the keyboard; while a `display::` test runs,
the terminal does not draw.

The boot test `terminal::boot_test` boots at 1024x768 with only the
terminal's tests, waits for `boot: done`, types `ech`, Tab, `Polx`,
Backspace, `y-Term`, Enter through the monitor (`sendkey`) and takes a
screenshot (`target/poly-terminal-test.ppm`) every half second until a
grid row starts with `Poly-Term` exactly as `poly-gfx` draws it on the
host and the status bar starts with ` Poly <version>` in its colours (with
no other colours on that row), or the boot times out; then it says what
did not match. Open the screenshot to see why.

**Screenshots.** `cargo xtask screenshot` boots headless (with the test
devices, like `run`), waits for a line on the serial port, saves what the
screen shows and quits QEMU:

```sh
cargo xtask screenshot                                     # after the self-test's verdict, to target/poly-screen.png
cargo xtask screenshot --out shot.ppm poly.display=800x600 # PPM instead of PNG, another mode
cargo xtask screenshot --wait "idle"                       # after any line containing "idle"
```

It talks to QEMU's QMP socket (`screendump`); from the monitor of a running
QEMU (Ctrl-A C under `cargo xtask run`) the same is `screendump
/tmp/screen.ppm` or `screendump /tmp/screen.png -f png`. macOS Preview opens
both formats.

The boot test `screenshot_test` boots at 1024x768 with only the display
tests (and `poly.terminal=off`), takes a screenshot (`target/poly-screen-test.ppm`) and compares it
pixel for pixel with the boot screen drawn on the host by the same code
(`poly_gfx::boot_screen`). On a mismatch it says how many pixels differ and
where, and writes what it expected to `target/poly-screen-expected.ppm`, to
open side by side. A display test that drew on the screen and did not put
it back shows up here.

**Looking at the device.** The Bochs VBE registers are 16-bit words at
BAR 2 + 0x500 (`poly_bochs_vbe::index` has the list), so from the monitor:

```text
(qemu) xp /11hx 0xfebd4500
00000000febd4500: 0xb0c5 0x0500 0x0320 0x0020 0x0041 0x0000 0x0500 0x0ccc
00000000febd4510: 0x0000 0x0000 0x0100
```

That is the ID (0xb0c5), 1280 x 800 at 32 bits, enabled with the linear
framebuffer (0x41), line length 1280, 3276 lines of video memory, offset
(0, 0) and 16 MiB (0x100 × 64 KiB). A black screen with ENABLE at 0x41
means nothing was flushed; ENABLE at 0 means the mode was never set or was
turned off. `xp /4wx 0xfd000000` reads the top-left pixels (0x00RRGGBB; the boot screen's
background is 0x0015171c).
`info mtree` shows where `vga.vram` and the `vga.mmio` registers sit.

## In-kernel tests

Every boot runs the tests registered with `kernel_test!` (see
`kernel/src/ktest.rs`) and prints one line per test:

```text
ktest: running 6 of 6 tests
test selftest::cpu_exceptions_return ... ok
test sync::spinlock_excludes_until_guard_dropped ... ok
test result: ok. 6 passed; 0 failed; 0 filtered out
POLY-SELFTEST: PASS
```

A test is a function next to the code it checks; it returns `Ok(())` or
`Err("reason")`:

```rust
crate::kernel_test! {
    fn spinlock_excludes_until_guard_dropped() -> TestResult {
        // ...
        Ok(())
    }
}
```

Write `fn name(env: &TestEnv) -> TestResult` to get the boot info and memory
map. Put `#[boot_only]` first (before any doc comment) for a test that checks
boot-time state; the shell's `test` command then leaves it out. `cargo xtask test` fails if any test fails, if the summary line is missing
(the kernel crashed during the tests), or if the counts do not add up. Only
put tests here that need the real machine; pure logic belongs in a crate under
`crates/` with ordinary `#[test]`s.

## Panic backtraces

The kernel is built with frame pointers, so the panic handler can walk the
stack. It prints raw addresses; frame #0 is the faulting instruction for a CPU
exception (otherwise a point in the panic path), later frames are return
addresses:

```text
KERNEL PANIC: panicked at src/arch/x86_64/idt.rs:241:13:
CPU exception 14 (page fault), error 0x0, rip 0xffffffff8010776c, ...
backtrace:
  #0 0xffffffff8010776c
  #1 0xffffffff801070bc
  #2 0x00000000001000d5
```

`cargo xtask test` adds symbols as it captures the boot output. For a log you
captured yourself, or addresses copied from anywhere:

```sh
cargo xtask symbolize < serial.log          # rewrite every backtrace line in a log
cargo xtask symbolize 0xffffffff8010776c    # one or more addresses
cargo xtask symbolize --release ...         # use the release kernel
cargo xtask symbolize --elf path/to/kernel  # any other kernel ELF
```

```text
backtrace:
  #0 0xffffffff8010776c poly_kernel::selftest::trigger_page_fault+0xcc
  #1 0xffffffff801070bc kernel_entry+0x81c
  #2 0x00000000001000d5 pvh_start64+0x2b
```

A page fault's error code says what went wrong (SDM Vol. 3A §4.7): bit 0 set
means the page was present (a protection violation, not a missing page),
bit 1 a write, bit 4 an instruction fetch. So `error 0x3` is a write to a
read-only page such as kernel code, and `error 0x11` is executing no-execute
memory (then `rip` and `cr2` are equal).

A kernel stack overflow runs into the unmapped guard page below the stack.
The page fault cannot be delivered on the full stack, so it arrives as a
double fault on its own stack, and the report says which stack overflowed:

```text
CPU exception 8 (double fault), error 0x0, rip 0xffffffff8010f07b, rsp 0xffffffff80127fb0, cr2 0xffffffff80127fa8
kernel stack overflow: hit the guard page below the boot stack
```

Symbols come from the ELF that was built last, so symbolise before you rebuild.
For file and line numbers, use `addr2line -f -C -i -e
kernel/target/x86_64-unknown-none/debug/poly-kernel <addr>` (for frames after
#0, subtract 1 from the address to get the line of the call).

The walk stops at the bottom of the stack, after 32 frames, or at the first
frame pointer that is outside the kernel's stacks, misaligned or not moving up
the stack; the reason is printed when it is not the bottom. A corrupt stack
therefore gives a short backtrace, never a second fault.

A panic on a kernel thread walks that thread's own stack, which ends in the
code every thread starts in:

```text
  #3 0xffffffff8010560c poly_kernel::thread::panic_in_thread+0x1c
  #4 0xffffffff80105558 poly_kernel::thread::thread_start+0x18
  #5 0xffffffff8010d8db thread_trampoline+0x9
```

A thread that locks a `Mutex` it already holds panics with `deadlock: thread
N locks a mutex it already holds` and a backtrace of its stack; `cargo xtask
run poly.test=sync::deadlock poly.selftest.deadlock` shows what this looks
like. Blocking or yielding while holding a spinlock panics too (`... while
holding a spinlock`), as does blocking inside an interrupt handler. A thread
that waits on a `WaitQueue` nobody wakes is not detected: with interrupts on,
every thread being blocked is normal (they may all be sleeping), so the
kernel just idles. Attach a debugger and look at the threads' stacks, or
watch `monitor info lapic` to see whether interrupts still arrive.

## Interrupts and the timer

The boot log shows what was set up:

```text
irq: 8259 PICs remapped to vectors 0x20-0x2f, all masked
apic: local APIC id 0 version 0x14, xAPIC mode at 0xfee00000, 6 LVT entries, spurious vector 0xff, 1 LINT input(s) wired to NMI
apic: I/O APIC 0 at 0xfec00000: version 0x20, GSI 0-23, all masked
time: HPET at 0xfed00000: 100.000 MHz, 64-bit counter, 3 timers
time: TSC 2699.992 MHz, calibrated against the HPET (5 of 6 measurements, spread 14 ppm), invariant: no
time: clock source HPET (TSC not invariant)
time: APIC timer 62.499 MHz (input clock / 16), periodic tick 1000 Hz on vector 0xf0
irq: interrupts enabled
```

An interrupt on a vector without a handler prints `irq: interrupt on vector
0x..., which has no handler` (the first few times) and is acknowledged; an
APIC error prints its ESR. The QEMU monitor shows the controllers' state.
Run with `cargo xtask run --gdb` and use `monitor <cmd>` from the debugger, or
add `-monitor stdio` to a hand-written QEMU command line:

- `info lapic`: the local APIC of the current CPU: the LVT (the timer entry
  should be periodic, vector 0xf0, unmasked), the timer's initial and
  current count, and the ISR / IRR bitmaps. A vector stuck in the ISR means
  a handler never sent its EOI; one stuck in the IRR means interrupts are
  disabled (a spinlock held too long?) or a higher-priority vector is in
  service.
- `info pic`: the 8259s and the I/O APIC redirection table: every input
  should be masked except the ones a driver routed.
- `info irq`: how many interrupts each I/O APIC and 8259 input has raised,
  to see whether a device fires at all. MSI and MSI-X messages are not
  counted there (they go straight to the local APIC); for those, see "PCI
  devices and the test disk" above (`xp` on the MSI-X table, the
  `apic_deliver_irq` trace event).

`info registers` shows `RFLAGS` (bit 9, IF: interrupts enabled). In gdb,
`print poly_kernel::arch::x86_64::time::TICKS` gives the tick count, and a
breakpoint on `poly_kernel::arch::x86_64::irq::dispatch` stops at every
interrupt (expect one per millisecond from the tick).

## User mode

The boot log says how user mode was set up, and every user thread a CPU
exception kills leaves one line (the kernel goes on):

```text
user: syscall/sysret on; PCID: no, SMEP: no, SMAP: no; FPU state: FXSAVE, 512 bytes, XCR0 0x0
user: pid 4242 thread 2.6 killed: page fault (vector 14) at rip 0x4000be, error 0x4, address 0x50000000
user: pid 4242 thread 3.7 killed: general protection fault (vector 13) returning to rip 0x4004b7, error 0x10
```

`rip` is the user instruction (or, with "returning to", where a return
to ring 3 failed in IRETQ); `address` is CR2. The page-fault error bits are
as above, plus bit 2 (user mode): `0x4` is a user read of an unmapped page,
`0x5` a user read of a kernel page, `0x7` a user write to a read-only page,
`0x15` a user fetch from a no-execute or kernel page. The thread's exit code
(what `thread::join` returns) is `user::killed(vector)`, bit 63 plus the
vector. QEMU's default (TCG) CPU does not check addresses for canonicality:
a non-canonical access is reported as a page fault there, a #GP on hardware.

A thread with a memory map (started by `exec` or `user::spawn_in`) only
gets that line for page faults its map could not resolve, and the line ends
with why:

```text
user: pid 4343 thread 2.15 killed: page fault (vector 14) at rip 0x4000c3, error 0x7, address 0x702000 (SEGV_ACCERR)
user: pid 4343 thread 2.20 killed: page fault (vector 14) at rip 0x4000be, error 0x4, address 0x50000000 (SEGV_MAPERR)
user: pid 4343 thread 2.22 killed: page fault (vector 14) at rip 0x4000be, error 0x4, address 0x902000 (SIGBUS)
```

`SEGV_MAPERR`: no region there (and not just below the stack, within its
8 MiB limit and above the guard gap). `SEGV_ACCERR`: the region does not
allow the access: a write to a read-only region (`0x7` or, before the page
was ever touched, `0x6`), a fetch from a non-executable one (`0x14`), any
access to `PROT_NONE`. `SIGBUS`: a file page wholly past the end of the
file. Faults that are resolved leave no trace; a page that is "missing"
when the kernel itself looks (`monitor info mem`, `AddressSpace::translate`)
may simply not have been touched yet. `Memory::regions()` lists what is
mapped, and a breakpoint on `poly_kernel::user::memory::Memory::fault`
stops at every resolution (`finish` shows its result). A user copy made
with interrupts off (under a spinlock) does not fault pages in: it returns
`Efault` for a page that is not present yet, which looks like a bad user
pointer. A copy made while holding the memory map's mutex panics with
"deadlock: thread ... locks a mutex it already holds".

`cargo xtask run poly.test=user` runs only the user-mode tests (and
`poly.test=addrspace` the address-space ones, `poly.test=memory_tests` the
demand-paging ones, `poly.test=exec` the exec ones). The test programs are
assembled into the kernel (`kernel/src/user/programs.rs`) and mapped at
`0x400000` in each test address space, so a user `rip` minus `0x400000` is
the offset into `user_programs_start`. The exec tests run small static ELF
executables assembled into the kernel as well (`kernel/src/user/test_elf.rs`,
between `poly_test_elf_*_start` and `_end` in the kernel's symbols): copy
one out of the kernel image with the addresses `nm` gives and `readelf -lW`
or `objdump -D -b binary -m i386:x86-64` reads it. Each reports what it
finds on its initial stack through a test system call, so a failing exec
test usually names the first value that was wrong. SMEP, SMAP and XSAVE need a CPU model
that has them: run QEMU by hand with the command line `cargo xtask run`
uses plus `-cpu max` (QEMU's TCG has no PCIDs in any model).

In gdb, `info registers cr3` shows the address space that is loaded (bits
11:0 are the PCID when PCIDs are on), `monitor info tlb` and `monitor info
mem` its mappings (user pages have the `u` flag), and `print
poly_kernel::arch::x86_64::syscall::PERCPU` the kernel stack the next
`syscall` will use. A breakpoint on `poly_kernel::user::kill_current`
stops at every killed thread; `up` to `trap_dispatch` and `print *frame`
show its user registers.

## Keyboard input

The boot log says whether the PS/2 keyboard came up, and how:

```text
acpi: FADT SCI 9, century 0x32, PM timer port 0x608 (24-bit), 8042, reset 0xcf9=0xf
keyboard: i8042 config 0x71 -> 0x61, 0 stale byte(s) flushed; keyboard reset ok, scancode set 2 translated to set 1
keyboard: ISA IRQ 1 -> GSI 1 -> vector 0x31; layout us (US English (QWERTY)), Num Lock on
```

`, 8042` in the FADT line is the firmware's "8042 present" flag. The
config byte goes from what firmware left (read after disabling both ports)
to keyboard interrupt and translation on, mouse port off (0x61 on QEMU).
Without a keyboard the kernel says why and carries on:

| Message | Means |
|---|---|
| `no 8042 controller (ACPI FADT)` | the FADT (revision 2+) clears the 8042 flag; the ports are not touched |
| `i8042: NoController` | the status port reads 0xFF: nothing is there |
| `i8042: Timeout("...")` | the controller did not take a byte or answer within 100 ms; names the step |
| `i8042: SelfTest(x)`, `PortTest(x)` | the controller self-test (0xAA) did not answer 0x55, or the keyboard port test (0xAB) not 0x00 (1-4: clock or data line stuck) |
| `i8042: NoKeyboard`, `KeyboardSelfTest(x)` | nothing acknowledged the reset (0xFF), or the keyboard's own test failed (0xFC / 0xFD) |
| `setting the LEDs to 0b... failed: ...` | the keyboard did not acknowledge 0xED and its mask in 250 ms |

The layout is chosen on the command line, `poly.keymap=us` (the default)
or `poly.keymap=de-ch`. `keyboard::stats()` counts interrupts, bytes,
bytes dropped because nobody read them, and LED updates.

**Typing into the guest.** What you type in the terminal running `cargo
xtask run` goes to the serial port, not to the PS/2 keyboard; what you
type into QEMU's window (when there is one) goes to the keyboard. Without
a window, press PS/2 keys from QEMU's monitor (Ctrl-A C, then Ctrl-A C
again to go back): `sendkey` takes QEMU's key names, which are US
positions (`z` is the key that types y on a Swiss German keyboard), joined
with `-` for chords, and holds them for 100 ms:

```text
(qemu) sendkey shift-p
(qemu) sendkey alt_r-2          # AltGr-2: @ on de-ch
(qemu) sendkey equal            # the ^ dead key on de-ch
(qemu) sendkey ret
```

After the kernel tests, the screen terminal's shell reads the keyboard
(see "The screen"). To watch decoding by itself, run the end-to-end test
and type within 30 seconds; the kernel prints what it got:

```sh
cargo xtask run poly.test=keyboard:: poly.test.keyboard=poly poly.keymap=de-ch
```

**Headless**, from a script or another terminal: give QEMU a monitor on a
Unix socket and write `sendkey` lines to it (this is what `cargo xtask
test` does, after the kernel prints `keyboard: ready for test input`):

```sh
qemu-system-x86_64 ... -monitor unix:/tmp/poly-mon.sock,server=on,wait=off
printf 'sendkey p\nsendkey o\nsendkey l\nsendkey y\nsendkey ret\n' | nc -U -q 1 /tmp/poly-mon.sock
```

(QMP's `send-key` command does the same with JSON.) To see the device
side, run QEMU by hand with `-trace 'ps2_*' -trace 'pckbd_*'`:
`ps2_put_keycode` is each byte the keyboard sends (set 2, before
translation), `pckbd_kbd_read_data` each byte the kernel reads,
`pckbd_kbd_write_command` / `pckbd_kbd_write_data` the commands the
driver writes, `ps2_set_ledstate` the LED mask. In the monitor, `info
irq` counts IRQ 1 on the I/O APIC (one per byte) and `info pic` shows its
redirection entry: unmasked, edge-triggered, the vector from the boot log.
If keys stop arriving while `info irq` still counts, the bytes are read
but not decoded (look for a thread holding the keyboard); if the count
stands still, the controller holds a byte nobody read: a waiting reader
drains it itself within 100 ms.

The kernel tests can feed the decoder without a host: `keyboard::inject`
takes set 1 bytes as if the keyboard had sent them, and the controller's
command 0xD2 makes it deliver a byte through IRQ 1
(`keyboard::irq_delivers_controller_bytes`).

## File systems and the initramfs

`/` is a tmpfs filled from the initramfs at boot (see "File systems" in
[ARCHITECTURE.md](ARCHITECTURE.md)). `cargo xtask run` writes a small one,
`target/poly-initramfs.cpio` (`/etc/motd`, `/etc/hostname`,
`/dev/console`), and passes it with `-initrd`; `--initrd=<file>` boots
another `newc` archive instead (`find . | cpio -o -H newc > ../my.cpio`
makes one from a directory), `--no-initrd` none. The boot log shows where
QEMU put it, what was unpacked and that its pages went back to the frame
allocator:

```text
boot: initramfs at 0xffd7000-0xffd7418 (2 KiB)
fs: initramfs unpacked into /: 2 directories, 2 files (207 bytes), 0 symbolic links, 1 device nodes, 0 hard links
fs: initramfs: 1 pages (4 KiB) at 0xffd7000-0xffd8000 returned to the frame allocator
fs: devices console (5,1), tty (5,0), null (1,3), zero (1,5), full (1,7); 6 of 8 directories and nodes made
```

Then `ls /`, `ls /dev`, `cat /etc/motd` and `mount` in the shell. Without
an archive the log says `fs: no initramfs (no boot module); / is an empty
tmpfs`. A damaged archive is reported by entry (`fs: initramfs: unpacking
stopped: <name>: <errno>`, or `bad archive: ...` with the byte offset);
everything before the damage is there. In QEMU's monitor, `xp /6c
<address>` at the address from the boot log shows the archive's magic
(`070701` for `newc`).

The self-test boots use `target/poly-test-initramfs.cpio` (built by
`tools/xtask/src/initramfs.rs`; every byte is fixed, so the image is the
same on every run) with `poly.test.initrd`, which makes the `fs::` kernel
tests check its contents; `cargo xtask run poly.test=fs::
--initrd=target/poly-test-initramfs.cpio poly.test.initrd` runs them by
hand. Writes to `/dev/console` (fds 0-2 of every new process) appear in
the serial log like kernel log lines; reads from it return end of file
until the TTY layer exists.

## Disk file systems and the page cache

`cargo xtask run` and the boot tests attach a third virtio-blk device,
`target/poly-test-ext2.img` (8 MiB of ext2 with 4 KiB blocks, written by
`tools/xtask/src/ext2disk.rs` with `poly-ext2` before every boot, the same
bytes every time), and pass `poly.mount=virtio2:/mnt poly.test.ext2`. The
kernel brings the disk up when the mount asks for it and logs the mount:

```text
fs: page cache of up to 16190 pages (63 MiB); writeback every 5 s
virtio-blk: 00:06.0: MSI-X with 3 vectors, table BAR1+0x0, PBA BAR1+0x800
block: virtio2: virtio-blk 00:06.0, 16384 blocks of 512 bytes (8 MiB), write cache; queue 32 requests, cache 128 buffers (64 KiB); interrupt-driven (vector 0x32)
fs: virtio2 mounted on /mnt: ext2, 2048 blocks of 4096 bytes (1901 free), 1024 inodes (1008 free), read-write
```

A volume that was not unmounted clean adds `; was not cleanly unmounted
(run e2fsck)`. A mount that fails says why (`fs: mounting virtio2 on /mnt
failed: not an ext2 file system`, `no such disk`, an unsupported feature).
`virtioN` is the N-th virtio-blk function in PCI order (`lspci` lists them;
`virtio1` is the watchdog test's spare). In the shell:

```text
poly> write /mnt/note.txt hello disk
poly> cat /mnt/note.txt
hello disk
poly> sync
1 cached pages written back; every disk synced
poly> mount
tmpfs on / type tmpfs (rw)
virtio2 on /mnt type ext2 (rw)
poly> umount /mnt
poly> mount virtio2 /mnt ro
```

Data written to a file sits in the page cache (dirty, `mem` counts it)
until the `writeback` thread (every 5 s; `ps` shows it), `sync`, `umount`
or `reboot` writes it to ext2 and syncs the disk. `reboot` prints `fs:
/mnt (virtio2) unmounted clean` for each volume. The image survives QEMU,
so after a boot it can be checked and read on the host:

```sh
e2fsck -fn target/poly-test-ext2.img                   # check without changing anything
debugfs -R 'ls -l /' target/poly-test-ext2.img         # list the root directory
debugfs -R 'cat /note.txt' target/poly-test-ext2.img   # print a file
cargo xtask run poly.test=disk_tests                   # only the disk file system tests
```

`cargo xtask` rewrites the image before the next boot; copy it first to
keep it. The boot test `ext2_persistence_test` writes a file in one boot,
reboots, runs `e2fsck -fn` (skipped with a message when e2fsprogs are not
installed) and reads the file in a second boot from the same image.

Write errors at writeback (a full volume) lose the page's data and are
reported by the next `sync` (`sync: ENOSPC`) and counted as
`write_errors` in the cache's stats; a writeback by the thread that fails
logs `fs: writeback to /mnt (virtio2) failed: ...`. When less than 1/16 of
RAM is free, the thread evicts clean pages and logs how many. In gdb,
`poly_kernel::fs::cache::CACHE_FRAMES` and `poly_kernel::fs::cache::PASSES`
are the frames the cache holds and the writeback passes made; a thread
stuck in `poly_kernel::sync::RawMutex` waits for a volume or inode lock
(the holder is in the block layer, or deadlocked: the order is namespace,
inode, ext2 volume, disk cache).

## Attaching a debugger

```sh
cargo xtask run --gdb [kernel args...]
```

QEMU starts with the CPU stopped (`-S`) and a GDB server on localhost:1234
(`-s`). The kernel ELF has full debug info in the default (debug) build.

The first instructions run at physical addresses with paging off, and the
kernel's memory is only filled in by the loader once the CPU runs. Software
breakpoints set before that are lost, so stop in `kernel_entry` with a
**hardware** breakpoint first; after that, ordinary breakpoints work.

### gdb

```sh
gdb kernel/target/x86_64-unknown-none/debug/poly-kernel \
    -ex 'target remote :1234' -ex 'hbreak kernel_entry' -ex continue
```

```text
(gdb) break poly_kernel::selftest::trigger_page_fault
(gdb) continue
(gdb) bt
#0  poly_kernel::selftest::trigger_page_fault () at ...
#1  0xffffffff801070bc in poly_kernel::kernel_entry (start_info=<optimized out>) at src/main.rs:70
#2  0x00000000001000d5 in pvh_start64 ()
(gdb) info registers rip rsp cr2 cr3
(gdb) x/8gx $rsp
(gdb) monitor info mem
```

`next`, `step`, `finish` and `print` work as usual; with `opt-level = 1` some
locals show as `<optimized out>` and inlined code (such as `kprintln!`) shows
the line of the inlined function. `monitor <cmd>` runs a QEMU monitor command:
`monitor info mem` and `monitor info tlb` show the page tables.

### lldb

lldb needs to be told that the kernel runs at its link address:

```sh
lldb kernel/target/x86_64-unknown-none/debug/poly-kernel
(lldb) gdb-remote 1234
(lldb) target modules load --file poly-kernel --slide 0
(lldb) breakpoint set --hardware --name kernel_entry
(lldb) process continue
(lldb) breakpoint set --name poly_kernel::selftest::trigger_page_fault
(lldb) process continue
(lldb) bt
```

### Tips

- The kernel stops in `cli; hlt` after a panic. Interrupt the debugger (Ctrl-C)
  and `bt` still shows where it stopped; the panic message is in the serial log.
- `info registers` (gdb) / `register read` (lldb) also show the control
  registers (`cr0`, `cr2`, `cr3`, `cr4`), handy for page faults.
- Physical memory is visible at `0xffff800000000000 + phys` (the direct
  map), e.g. `x/4gx 0xffff800000000000 + 0x100000`. `monitor info mem`
  lists every mapping with its permissions; `monitor xp /4gx 0x100000` reads
  physical memory directly.
- To find a kernel test in the debugger, break on its function: tests are
  ordinary functions named after the test (`poly_kernel::sync::spinlock_excludes_until_guard_dropped`).

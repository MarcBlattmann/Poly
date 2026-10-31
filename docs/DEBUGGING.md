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
| `mount [<disk> <path> [ro]]` | every mount: source (the disk for a disk mount), mount point, type, options; with arguments, mount the ext2/3/4 volume on `<disk>` (`virtio2`, `ram0`, ...) on `<path>` (made if missing) |
| `umount <path>` | unmount the disk mounted on `<path>`: written back, synced and marked clean; refused while files there are open or mapped |
| `write <file> [words...]` | write the words and a newline to `<file>` (created or truncated) |
| `sync` | write back every dirty cached page and sync every disk file system and disk |
| `run [NAME=value...] <program> [args]` | start a Linux program (static or dynamically linked) from the file system, with the variables as its environment, and wait for it; its output goes to the console, then its exit status (see "Linux programs" and "Dynamically linked programs") |
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
(the kernel crashed during the tests), or if the counts do not add up. A boot
still running after 60 s is killed and reported as `timed out after 60 s
during the kernel tests (last line: "...")`: the kernel hung (or crawled)
after printing that line, which is different from `kernel test summary line
missing: the kernel stopped during the tests`, where QEMU exited early (a
triple fault, say). Only put tests here that need the real machine; pure
logic belongs in a crate under `crates/` with ordinary `#[test]`s.

Tests that involve time must hold on a slow, loaded host too (CI runners,
many QEMUs at once). There QEMU's clocks (HPET, TSC) follow the host's
wall-clock time while the emulated CPU gets only part of a host CPU, and
timer ticks that come due while the vCPU is held off merge into one: 30 ms
of clock time can hold three ticks, and a thread may get no turn for tens
of milliseconds. So count in ticks rather than clock time, check order and
eventual progress rather than "within N ms", bound waits with a generous
deadline that ends as soon as the progress is seen, and keep the failure the
test is about detectable (a tick that stops altogether must still fail or
hang). To check a change, boot the CI configuration many times in parallel
next to a few busy loops (`while :; do :; done`) and count failures per
test.

With several CPUs, "nothing else runs while interrupts are off" no longer
holds, and a thread may run on another CPU than its test. Tests that need
the old guarantees say so: `thread::pin_current()` keeps the test on its
CPU, `thread::spawn_here` starts a thread on the same one,
`smp::exclusive()` parks every other CPU (for tests that count frames or
other global totals), and `time::cpu_ticks()` counts this CPU's ticks.
Waiting for something another CPU does (reaping a thread that ran there,
a thread pinned there getting going) is a wait in time with a generous
deadline (`thread::wait_until`), not a number of yields: the other CPU may
be held off by the host for a while. Where a test must tell "late" from
"never", count the progress of what it waits for: the TLB shootdown tests
in `user/tests.rs` count the turns of the thread that must fault, which
stand still while the host does not run it. A CPU's ticks
(`percpu::cpu(i).ticks()`) also stand still then, but they are a poor
measure of one thread's progress: other threads on that CPU take ticks,
and on a busy host a tick handler that waits for QEMU (every clock read
under TCG takes its global lock) can find the next tick due as it returns,
so several ticks go by with almost nothing else done. The same goes for
the real ticks that land in a short stretch with interrupts on.

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

Kernel threads' stacks (16 KiB each) lie in their own area at
`0xffff_c080_0000_0000`, each above 16 KiB that are never mapped (ADR 0050),
so an overflow there names the thread (and its table slot):

```text
CPU exception 8 (double fault), error 0x0, rip 0xffffffff801d519b, rsp 0xffffc08000043f80, cr2 0xffffc08000043f78
kernel stack overflow in thread overflow (slot 8): hit the guard below its stack
```

`poly.selftest.stack_overflow` and `poly.selftest.thread_stack_overflow`
provoke the two on purpose (the boot tests do).

Interrupt handlers run on their CPU's own interrupt stack (16 KiB at
`0xffff_c080_0020_0000` + 32 KiB per CPU, ADR 0061), so an overflow there
reads `kernel stack overflow on CPU <n>'s interrupt stack`, and a backtrace
taken in a handler goes on from the interrupt stack into the interrupted
thread's stack (`irq_common+0x..` marks the step). An interrupt costs the
interrupted thread only its trap frame and the way out (at most 1 KiB,
`thread::INTERRUPT_STACK_BUDGET`): a thread-stack overflow with an
interrupt on top is a system call path that is too deep by itself.

### How deep do the stacks go?

Every thread and interrupt stack is painted when it is allocated, so how
deep it was ever used can be read off it. Two kernel flags help:

- `poly.debug.stack_report` prints, as a test run ends (`poly.qemu_exit`,
  also after a panic) or the machine powers off (`poweroff`), the deepest thread stack use so far and each
  interrupt stack's:

  ```text
  stacks: deepest thread stack use 12480 of 16384 bytes (thread 'user', started in 0xffffffff802ac2c0)
  stacks: CPU 0's interrupt stack: deepest use 1344 of 16384 bytes
  stacks: interrupts nested on an interrupt stack: 0
  ```

  The address is the thread's function: `cargo xtask symbolize <addr>`
  names it.

- `poly.debug.thread_stack_kib=<n>` maps only the top `n` KiB of every
  thread stack (the rest becomes guard), so a path that goes deeper than
  that overflows with a full backtrace of its deepest point instead of
  passing silently. Bisect `n` to find the deepest path of a boot.

To catch a rare overflow, boot the failing configuration many times in
parallel next to a few busy loops (see "In-kernel tests" above): a tick
that lands in the few microseconds a system call is at its deepest is what
makes it rare. For a frame that is too large, `objdump -d` the function and
look at the `sub $0x...,%rsp` (and the stack probe, `movq $0x0,(%rsp)`,
for frames over a page) in its prologue; large values passed or built by
value (`[u8; 4096]` buffers, big structs moved around at `opt-level = 1`)
are the usual cause.

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
time: clock source TSC (a virtual CPU's TSC: constant rate)
time: APIC timer 62.499 MHz (input clock / 16, calibrated against the TSC), periodic tick 1000 Hz on vector 0xf0
irq: interrupts enabled
```

Under KVM (`POLY_QEMU_ACCEL`, ADR 0077) the APIC line says `x2APIC mode`
instead: QEMU gives its CPU models x2APIC there, and the xAPIC page at
0xfee00000 then reads as all ones. A calibration that needed retries says
`, attempt N` in its line (see `CalibrationConfig::retries`); one that
failed prints the measurements of its last attempt, each as `<rate> Hz
+-<ppm>`, `(wide)` where the reads bracketing it took too long (a VM exit
on a busy host, a descheduled vCPU).

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
`print poly_kernel::arch::x86_64::time::TICKS` gives the boot CPU's tick
count, and a breakpoint on `poly_kernel::arch::x86_64::irq::dispatch`
stops at every interrupt (expect one per millisecond from each CPU's
tick).

## Several CPUs

The boot log says which CPUs came up:

```text
acpi: CPUs: 4 enabled, 0 online-capable; local APIC at 0xfee00000, 8259 PICs present
smp: 4 of 4 CPUs online (0-3)
```

`poly.smp=off` runs on the boot CPU only, `poly.cpus=<n>` on at most `n`
CPUs (`smp: 2 of 4 CPUs online (0-1), 2 left out by poly.smp / poly.cpus`):
the first thing to try when a bug might be a race between CPUs. `cargo
xtask run` uses QEMU's `-smp 2`; add `-smp <n>` to a hand-written QEMU
command line for more. The shell's `ps` shows each thread's CPU, and the
status bar the CPUs online.

**An AP that does not start** is logged as `smp: CPU <i> (APIC ID <id>) did
not start: ...` and left out; the boot goes on with the others.

- `it never reached long mode`: no answer to INIT and SIPI; the AP never ran the trampoline's first
  instructions, or died before it reported "started". Check the start-up
  area: `smp: no room below 1 MiB for the start-up code` means the memory
  map had no usable RAM there. With `-d int,cpu_reset -D qemu.log` QEMU
  logs the INIT and the SIPI's start address (the area's page) and every
  exception the AP takes on the way; a triple fault there resets only that
  CPU. In gdb, `info threads` lists one thread per vCPU: `thread 2` and
  `x/8i $pc` show where the AP is (real-mode addresses are `cs * 16 + ip`;
  break on the area's physical address with `hbreak *0x<base>`).
- `it reached the kernel but did not come online`: the trampoline worked and `ap_entry` ran, but
  the AP stopped in its own set-up (page tables, GDT/TSS, APIC, timer).
  Its own panic, if any, is in the serial log, prefixed like any panic.
  Break on `poly_kernel::arch::x86_64::smp::ap_entry` (a hardware
  breakpoint: the AP runs before the kernel's page tables are loaded there).
- An AP is up but never runs anything: `ps` shows no thread ever on it. Its
  idle loop halts until an interrupt: check its tick (`info lapic` on that
  vCPU: the timer LVT periodic, vector 0xf0) and that reschedule IPIs
  (vector 0xf1) arrive (`irq` in the shell counts every vector, over all
  CPUs).

**Debugging with several CPUs in gdb.** Each vCPU is a gdb thread:

- `info threads` lists them (thread 1 is CPU 0); `thread <n>` switches,
  then `bt`, `info registers` and `x` are that CPU's. `thread apply all bt`
  shows what every CPU is doing, the first thing to look at in a hang.
- The running CPU's per-CPU area is at its GS base: `p/x $gs_base`, then
  `p *(poly_kernel::arch::x86_64::percpu::PerCpu *)$gs_base` (CPU number,
  APIC ID, current thread, spinlocks held, ticks). On entry from ring 3
  the kernel's GS base is in `KERNEL_GS_BASE` until `swapgs`.
- Breakpoints stop the CPU that hits them; with QEMU's TCG the other vCPUs
  stop too while gdb has control. `set scheduler-locking on` keeps `step`
  and `next` on the current CPU.
- A hang with every CPU spinning in `SpinLock::lock` is a lock-order
  problem (see "Synchronisation" in ARCHITECTURE.md); one CPU waiting in
  `tlb::shoot` means a target CPU stopped serving shootdowns: look at what
  it runs with interrupts off.
- A panic stops the other CPUs with an NMI, so the backtrace in the log is
  the panicking CPU's; the others halt where they were (`thread apply all
  bt` shows where). A second panic on the CPU that is reporting one (a
  fault while it formats the report, say on a corrupt or nearly full
  stack) is printed too, once, as `KERNEL PANIC while reporting a panic:`
  with its own backtrace; before, the report just stopped mid-line.
- A kernel `ret` to a data address or to `0xffffffffffffffff`, with `rsp`
  on a CPU's interrupt stack (`0xffff_c080_002x_xxxx`), means frames on that
  stack were overwritten while still in use: code run there
  (`irqstack::run_on`: reaping in `thread::finish_switch` and
  `spawn_with`) blocked and the next interrupt reused the stack. Switching
  away while on the interrupt stack now panics at once (`thread ... blocks
  on its CPU's interrupt stack`) with the backtrace of what blocked. The
  usual culprit is a drop during reaping that reaches a sleeping file
  system lock; such drops go to the writeback thread
  (`fs::cache::release_later`), which must get the reference itself, not a
  copy (a copy it drops first leaves the original to be dropped in place).

## User mode

The boot log says how user mode was set up, and every user thread a CPU
exception kills leaves one line (the kernel goes on). That is for the
kernel tests' bare processes; in a Linux process the exception becomes a
signal instead (`linux: pid ... killed by signal 11 after page fault ...`
when nothing handles it; see "Linux programs"):

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

The layout is chosen on the command line, `poly.keymap=us` (the default),
`poly.keymap=de-ch` or `poly.keymap=de`. `keyboard::stats()` counts
interrupts, bytes, console keys dropped because nobody read them, mouse
bytes and LED updates.

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
(`keyboard::irq_delivers_controller_bytes`); 0xD3 does the same for mouse
bytes through IRQ 12 (`mouse::controller_mouse_bytes_become_events`).

**Mice, tablets and `/dev/input`** (ADR 0103). Every input device is in
the event layer and logs a line `input: event<n>: <name>`; the PS/2 mouse
adds `mouse: PS/2 IntelliMouse Explorer (id 4), ISA IRQ 12 -> ...` (id 0:
no wheel, 3: a wheel, 4: a wheel and five buttons), or `mouse: i8042
second port: NoMouse` and similar when it is missing. virtio devices come
with `-device virtio-keyboard-pci`, `virtio-mouse-pci` or
`virtio-tablet-pci` and log their name, ids, event types and interrupt.
QMP's `input-send-event` drives any of them from a script (the `input`
boot group does): keys (`{"type": "key", "data": {"down": true, "key":
{"type": "qcode", "data": "a"}}}`), relative and absolute motion (`rel` /
`abs`, axis `x` or `y`; absolute runs 0 to 32767) and buttons (`btn`,
`left`, `wheel-up`, ...); QEMU routes each to the device of its kind that
the guest enabled last. A program reads them from `/dev/input/event<n>`;
`tools/xtask/programs/evtest.rs` is a small one that prints every device
and event. If events reach the kernel but not the program, check for a
grab (`EVIOCGRAB` takes a device's events from everyone else, the screen
console included).

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
fs: devices console (5,1), tty (5,0), null (1,3), zero (1,5), full (1,7), random (1,8), urandom (1,9); 12 of 14 directories, nodes and links made
fs: proc on /proc, sysfs on /sys, tmpfs on /dev/shm; 3 of 3 mounted
```

Then `ls /`, `ls /dev`, `cat /etc/motd` and `mount` in the shell; `cat
/proc/meminfo`, `cat /proc/cpuinfo`, `ls /proc`, `ls /sys/devices/system/cpu`
show procfs and sysfs (ADR 0043). The shell is a kernel thread, so it has
no `/proc/self` (and `/proc/mounts`, a link to `self/mounts`, does not
open there): use `/proc/<pid>/...` of a running program, or `mount`. A
`/proc` file is made anew by every read; `/proc/<pid>/maps` names file
mappings by the path the file was opened by. Without
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
hand. `/dev/console` (fds 0-2 of every new process) is a terminal, see
"Terminals" below.

## Linux programs

Static Linux executables run from the root file system (ADR 0038). Put
one in an initramfs and either run it from the shell or make it the first
process:

```sh
cargo xtask pipeline initramfs ./prog --out target/prog.cpio      # ./prog as /test-binary
cargo xtask run --initrd=target/prog.cpio                          # then: run /test-binary arg1 arg2
cargo xtask run --headless --no-disk --initrd=target/prog.cpio poly.init=/test-binary poly.test.capture
cargo xtask run --headless --no-disk --initrd=target/prog.cpio poly.qemu_exit poly.init=/test-binary
```

`poly.init=<path>` runs the program as pid 1 right after the file systems
are up, before the self-test, and logs how it ended; with `poly.qemu_exit`
QEMU exits after it (status 33 whatever the program's status). With
`poly.test.capture` the program is pid 2, the child of a kernel init (pid 1,
which reaps it and orphans; ADR 0064), every system call is a
`POLY-SYSCALL` line (process and thread, number, the six argument
registers, the result in decimal, `ret=?` for `exit_group`; `clone`,
`fork`, `vfork` and `clone3` also get a `POLY-ENTER` line before the child
exists), and the bytes that reach the program's standard output and error
(the capture files it starts with on fds 1 and 2, wherever they are
duplicated) come again as hex in `POLY-OUT` lines, as the diff runner reads
them (ADR 0027):

```text
init: running /test-binary (traced)
POLY-INIT: exec /test-binary
POLY-SYSCALL: pid=2 tid=2 nr=334 args=0x4b2ca0,0x20,0x0,0x53053053,0x0,0x4a8480 ret=-38
syscall: pid 2 linux/linux #334 rseq not implemented
POLY-SYSCALL: pid=2 tid=2 nr=1 args=0x1,0x4b33d0,0xd,0x4b31b0,0x1,0x7 ret=13
POLY-SYSCALL: pid=2 tid=2 nr=231 args=0x0,0xffffffffffffffc0,0xe7,0x4b31b0,0x1,0x7 ret=?
POLY-EXIT: pid=2 code=0
init: /test-binary (pid 2) ended: exit status 0
```

A file that cannot run says why: `POLY-INIT: unsupported /bin/x: ENOENT`,
`... its interpreter (PT_INTERP) cannot be opened: ENOENT`,
`... #! scripts are not supported yet`. Every call without a handler logs
`syscall: pid P linux/linux #N name not implemented` (the first time in a
boot; `poly.strace` shows every call) and returns `-ENOSYS`: that list is
the work queue for the program. A fault that kills a Linux
process is a signal, logged with where it happened:

```text
linux: pid 1 killed by signal 11 after page fault at rip 0x401012, address 0x10
```

In a traced run every signal a thread takes is a `POLY-SIGNAL` line just
before its handler runs (or the process stops, continues, ends, or drops
it as ignored), never `SIGKILL`: what the Linux tracer records as
`--- SIGx ---` (ADR 0062). A traced process keeps an ignored signal until
a thread takes it, so `SIGCHLD` shows up after a child ends. A call a
signal interrupted is recorded with `ret=-512` (or -513, -514, -516: the
restart numbers a tracer sees) and appears again if it was restarted.

The diff runner does all of this per corpus test:
`cargo xtask pipeline diff --target poly [--tests brk,mmap]` boots one
kernel per test (the debug kernel; `--build-kernel` rebuilds it), compares
it with real Linux and keeps each boot's serial log under
`target/pipeline/runs/latest/poly/<test>.serial.log` next to the traces.
`cargo xtask pipeline polycheck` (part of `cargo xtask ci`) does the same
and fails if a test listed in `tests/linux/poly-pass.txt` no longer
passes; it names tests that newly pass, to be added there.

The in-kernel tests of the Linux services run with `poly.test=linux::`;
they make real system calls from ring 3 (`kernel/src/linux/tests.rs`).
`poly.test=task_tests` runs those of fork, clone, execve and wait4
(`kernel/src/linux/task_tests.rs`): each program exits with 0 or the
number of its first failed check (90 and up, next to the check in the
assembly), which the failing test prints. `poly.test=memory::fork` runs
the copy-on-write tests of `Memory::fork`. A process tree that looks
wrong: `getppid` 0 means the kernel adopted the process (no live init);
a parent that never reaps keeps its children as zombies (their records
stay until then, in `linux::task`'s `ZOMBIES`).
`poly.test=cred_tests` runs the credential tests (ADR 0045,
`kernel/src/linux/cred_tests.rs`), numbered the same way; their exec
tests run a small report program whose exit status is a bit set (1:
effective uid 0, 2: real uid 1000, 4: saved uid 0, 8: some effective
capability). Every process the kernel starts runs as root with every
capability; an unexpected `EACCES` or `EPERM` from a program that changed
its ids usually means a capability rule (capabilities(7)) took a
capability away: `capget` shows the sets, `getresuid` the ids.
The hello worlds the boot tests use are built into
`target/xtask-programs/` (`hello`: Rust, no C library; `forkexec`: the
nolibc process and thread test, `poly.init=/bin/forkexec`, which prints
`forkexec: ok` or what failed; `corpus/`: the C corpus with `hello` and
the static glibc `libc-hello`).

## Dynamically linked programs

A program with `PT_INTERP` starts in its interpreter, glibc's own
`ld.so`, which exec maps next to it (ADR 0049); everything after that
(finding, mapping and relocating the shared libraries) is the loader's
work, done with ordinary system calls. The boot tests' programs and an
image with them, the host's `ld.so` and the libraries they need come from
one command (nothing is run on the host; the C library is only copied):

```sh
cargo xtask dynamic-initramfs                 # prints target/poly-dynamic-initramfs.cpio
cargo xtask run --initrd=target/poly-dynamic-initramfs.cpio
cargo xtask run --headless --no-disk --initrd=target/poly-dynamic-initramfs.cpio \
    poly.qemu_exit poly.init=/bin/dyn-threads poly.test.capture
```

`/bin/dyn-hello` (and `dyn-hello-nopie`, the same as `ET_EXEC`),
`/bin/dyn-dlopen` (its plugin is `/lib/poly/libpolyplugin.so`),
`/bin/dyn-threads` and `/bin/dyn-cxx` print `<name>: ok` (the hello: `dyn-hello:
hello, world`) or `<name>: FAIL <what>`. The sources are in
`tools/xtask/programs/dynamic/`, the binaries in
`target/xtask-programs/dynamic/`. The libraries sit where the host has
them (`/lib/x86_64-linux-gnu/...` on Debian and Ubuntu), with links from
`/lib`, `/usr/lib`, `/lib64` and `/usr/lib64`; there is no
`/etc/ld.so.cache`.

When a dynamically linked program fails:

- `POLY-INIT: unsupported ...: its interpreter (PT_INTERP) cannot be
  opened: ENOENT` (or `execve` failing with `ENOENT` although the program
  exists): the image lacks the interpreter the program names
  (`readelf -l` shows it). `ELIBBAD`: the interpreter is not an ELF file,
  or names an interpreter itself.
- The loader's own messages (`error while loading shared libraries:
  libfoo.so.1: cannot open shared object file`) mean it searched and did
  not find the library: put it in one of the directories above.
- glibc's loader is unchanged, so its debugging switches (ld.so(8)) work.
  `run` takes environment variables before the program:

  ```text
  poly> run LD_DEBUG=libs /bin/dyn-hello
           1:	find library=libc.so.6 [0]; searching
           1:	 search cache=/etc/ld.so.cache
           1:	 search path=/lib/x86_64-linux-gnu:/usr/lib/x86_64-linux-gnu:/lib:/usr/lib		(system search path)
           1:	  trying file=/lib/x86_64-linux-gnu/libc.so.6
  ...
  ```

  `LD_DEBUG=help` lists the categories (`files`, `symbols`, `bindings`,
  `reloc`, `all`, ...); `LD_LIBRARY_PATH`, `LD_PRELOAD` and `LD_BIND_NOW`
  work too. The loader can also be run as a program: `run
  /lib64/ld-linux-x86-64.so.2 /bin/dyn-hello` (then it maps the program
  itself), or with ld.so(8)'s options (`--list`, `--library-path`).
- `poly.test.capture` shows every call the loader makes: `openat` of each
  library it tries (`ret=-2` for the misses), `mmap` of a reservation and
  of each segment over it (`MAP_FIXED`), `mprotect` of the RELRO pages.
  The only call without a handler should be `rseq` (`ENOSYS` on purpose,
  ADR 0049); any other "not implemented" line is what to build next.
- A crash inside a library: the `killed by signal` line gives `rip`, and
  `run LD_DEBUG=files <program>` prints where the loader put each
  library (`base: 0x00007ffff7cae000` for `libc.so.6`), so `rip` minus
  the base of the library it falls in is the address in that file, for
  `objdump -d` or `addr2line -e` on the host's copy. The interpreter is
  the first mapping of the `mmap` area (just under the stack's reserve);
  `AT_BASE` says where. A running program's `/proc/self/maps` names the
  file of every mapping.

The kernel side: `poly.test=exec_tests` runs the exec tests, among them a
PIE and an `ET_EXEC` program naming a tiny interpreter of our own
(`user/test_elf.rs`, `TestElf::Ld`), which reports its load address,
`AT_BASE`, `AT_ENTRY` and `AT_PHDR` and jumps to the program;
`poly.test=advice` the `madvise` tests of `Memory::advise`.

## NT processes (PE executables)

A PE executable runs as an NT process (ADR 0063): the kernel maps its image
section, sets up the PEB, the first TEB and the stack, and starts the entry
point; its system calls go to `nt::syscall`. The NT state lives in
`kernel/src/nt` behind one lock, keyed by pid.

- **The service numbers are Poly's own** (`personalities/nt/src/services.rs`),
  not Windows'. A number in `eax` at a ring-3 `syscall` is `table << 12 |
  index` (table 0 executive, 3 Poly's); a call Poly does not implement logs
  `syscall: pid N nt/nt #X not implemented` and returns
  `STATUS_NOT_IMPLEMENTED` (0xC0000002), the seed of the API tracer.
- **A test program can report through `PolyTrace`** (service `0x3000`): it
  logs `nt: pid N PolyTrace a b c` with its three argument registers. The
  self-built NT test PE uses it to say which step failed and with which
  `NTSTATUS` before it ends with `NtTerminateProcess`.
- **Threads start through a per-process trampoline** (ADR 0065): a new
  thread (and the first one) begins at the trampoline, which calls its start
  routine and, on return, ends the thread with `NtTerminateThread`; the last
  thread ending the process gives the process its exit code. A thread's own
  TEB, stack and thread object are in `kernel/src/nt` + `NtProcess`.
- **Blocking waits** park on the generic process wait queue
  (`kernel/src/nt/sched.rs`): a blocked `NtWaitFor*` registers a per-thread
  control block, and every signalling call drains the dispatcher's wakes
  (`drain_wakes`) and unparks the waiters. A wait stuck forever usually means
  a `Wake` was queued but its thread had no stored flags, or a signalling
  call did not run `drain_wakes` (it runs after every NT system call in
  `nt::syscall`). `NtSuspendThread` parks the target in `return_to_user` with
  its context saved for `NtGetContextThread`.
- **User APCs** are delivered through the APC trampoline: the thread runs the
  routine and re-enters the kernel with `PolyResumeApc` (service `0x3001`) to
  restore the interrupted context. An APC that never returns to the wait
  usually means the saved context (`ThreadCtl::apc_saved`) was lost.
- **An unhandled fault** in an NT process is logged like any user thread
  (`user: pid N thread T killed: ...`); structured exception handling (SEH)
  is a later milestone, so a fault that is not a guard page kills the thread.
  A guard-page fault that grows a thread stack is resolved silently
  (`user::page_fault` → `nt::resolve_fault`) and the instruction retried.
- **The ring-3 NT tests** are `kernel/src/nt/test_pe.rs` (the object manager,
  memory and sections) and `kernel/src/nt/test_threads.rs` (threads,
  synchronisation, APCs, a timer and a suspended thread's context), tiny
  position-independent PEs Poly assembles itself (no Microsoft toolchain).
  Each ends with exit code `0x5A` on success; `kernel_test!
  nt_test_program_runs_from_ring3` and `nt_threads_and_sync_from_ring3` build
  them, run them and check that code. A different exit code means a step
  failed — the preceding `nt: ... PolyTrace 0xbadbad <step> <status>` line
  names which step and the `NTSTATUS` it saw.
- **The registry** (ADR 0066) logs where its hives live at the first NT
  use: `nt: registry hives in /mnt/root/poly/nt/registry`, or `nt: registry
  in memory (no disk with a /poly directory)` (then nothing is saved; make
  a `/poly` directory on a writable disk, as `cargo xtask root-disk` does).
  A hive file that does not decode logs `nt: registry: hive SOFTWARE is
  corrupt (...); using the defaults` and is replaced at the next save; a
  failed save logs `writing hive ... failed` and `NtFlushKey` returns
  `STATUS_REGISTRY_IO_FAILED` (0xC000014D). The files are Poly's own format
  (`POLYHIVE`, `crates/poly-registry/src/hive.rs`): to look inside one, decode
  it on the host with `poly_registry::hive::decode`. A notification whose
  `IO_STATUS_BLOCK` never changes in another process is waiting for that
  process's next system call (completions are written there). The ring-3
  test is `kernel/src/nt/test_registry.rs` (`cargo xtask run
  poly.test=nt_registry`); its `PolyTrace 0xbadbad <step> <status>` names
  the failed step as the module documentation numbers them.
- **The object, memory, thread and synchronisation logic is host-tested** in
  `poly-ntobj` (the dispatcher against a randomized model) and
  `personalities/nt` against a fake kernel (`cargo test -p poly-ntobj -p
  poly-personality-nt`): that is where to reproduce a status, layout or
  dispatcher bug without booting, since the kernel side only maps, locks,
  blocks and wakes.

## NT file I/O (ADR 0067)

A PE executable reaches the file system through file and device objects in
the object namespace (`personalities/nt/src/io`, carried out by
`kernel/src/nt/io.rs`):

- **The namespace.** Each VFS mount is a device `\Device\PolyFs<N>` with a
  parse procedure; drive letters are symbolic links in `\??`: `\??\C:` →
  `\Device\PolyFs1` (the VFS's `/nt/c`, the NT system root), `\??\Z:` →
  `\Device\PolyFs0` (`/`, the whole VFS), and one device per other mount.
  So `\??\C:\Windows\win.ini` becomes `/nt/c/Windows/win.ini`. `NtCreateFile`
  of a path Poly has not attached a volume for is `STATUS_OBJECT_PATH_NOT_FOUND`
  (the layer is attached lazily the first time an NT process starts — before
  `fs::init` there is nothing). Dump the devices from the shell's object
  listing or the namespace lookups in the tests.
- **Case.** Lookup is case-insensitive and case-preserving over the
  case-sensitive VFS (`io::walk`): the exact name wins, else the entry equal
  without case whose bytes are smallest (so `README` beats `Readme`), so the
  choice is stable. `FileNameInformation` reports the on-disk case. A VFS
  name that is not UTF-8 is shown with U+FFFD and cannot be opened by that
  name.
- **Share modes** (`io::share`) follow `CreateFile`'s documented table; a
  refused open is `STATUS_SHARING_VIOLATION` and nothing on disk changes (the
  truncation of an overwrite happens only after every check passes). An open
  that asks for no read/write/delete access (attributes only) is not checked
  or counted.
- **Asynchronous I/O** (`io::irp`). A handle without `FILE_SYNCHRONOUS_IO_*`
  is overlapped: `NtReadFile`/`NtWriteFile` make an I/O request packet queued
  on the volume's **worker thread** (`ps` lists `nt-io` once one starts) and
  return `STATUS_PENDING`; the worker runs the transfer with no NT lock held
  and the request completes through the `IO_STATUS_BLOCK`, the `Event` (reset
  when the request is made) or the file object, and the `ApcRoutine`. A
  synchronous handle keeps a file position and the call waits on the file
  object. A completion that never arrives usually means the worker was not
  kicked (`io::after_call` runs after every NT system call and drains
  completions) or the requesting process's memory was not registered
  (`io::register`, keyed by pid). The completion writes the caller's memory
  **before** signalling, so a waiter always sees the status block filled in.
- **Cancellation.** `NtCancelIoFile` (the thread's requests on a handle) and
  `NtCancelIoFileEx` (the process's, all or one) complete queued or waiting
  requests with `STATUS_CANCELLED`; a request already running on the worker
  finishes on its own. A thread's requests are all cancelled when it exits,
  without their APCs. `\Device\PolyPipe\<name>` is a stand-in slow device: a
  read with no data waits until a write to the same name feeds it, which is
  what the cancellation test exercises.
- **Byte-range locks** (`io::locks`) are mandatory and never merged: a read
  overlapping another owner's exclusive lock, or a write overlapping any lock
  that is not the owner's own exclusive one, is `STATUS_FILE_LOCK_CONFLICT`;
  a lock that cannot be granted is `STATUS_LOCK_NOT_GRANTED` (fail-immediately)
  or waits. They are not the POSIX `poly_vfs::locks` table, so a Linux
  process's `fcntl` locks and an NT process's locks on one file do not see
  each other yet.
- **Attributes** (`io::info`): `FILE_ATTRIBUTE_READONLY` is the absence of
  write permission (enforced by NT, since the VFS lets root write anything),
  `DIRECTORY` and `REPARSE_POINT` come from the node type, and `HIDDEN`,
  `SYSTEM`, `ARCHIVE` and the birth time live in the extended attribute
  `user.poly.ntattr` (ADR 0060; a file system without xattrs keeps none).
  Times are 100 ns since 1601.
- **Delete** is deferred: `FileDispositionInformation` or a
  `FILE_DELETE_ON_CLOSE` handle marks the open node, and the name goes when
  the last file object on it closes (`IoState::file_closed`); new opens
  meanwhile are `STATUS_DELETE_PENDING`. `NtDeleteFile` opens with `DELETE`,
  marks and closes.
- **The ring-3 file I/O test** is `kernel/src/nt/test_io.rs` (`kernel_test!
  nt_file_io_from_ring3`): a self-assembled PE that creates/writes/reads a
  file under `C:\`, opens it in another case, hits a sharing violation,
  enumerates a directory with `*.txt`, completes overlapped reads through an
  event and an APC, cancels a pending pipe read, hits a lock conflict and a
  delete-on-close. It ends `0x5A` on success; a `PolyTrace 0xbadbad <step>
  <status>` line names the failing step. The path parsing, case folding,
  share table, locks, wildcards and info-class layouts are host-tested in
  `cargo test -p poly-personality-nt io`.

## Pipes, waits and event files

Pipes, `poll` / `select`, `epoll` and the event files (ADR 0042) leave
nothing in the log while they work. What to look at when a thread hangs
in one:

- A thread blocked in a pipe read, `poll` or `epoll_wait` is parked
  (`process::park`) with a waker registered on the file's `WaitQueue`:
  `OpenFile::wait_queue()` and `WaitQueue::len()` say who waits (the
  kernel tests wait for exactly that before they write or signal). A
  file whose readiness never changes has no queue; `poll` on it only
  sees a change when something else wakes the thread.
- A wake-up that never comes is usually a notification that was not
  made: every change of a file object's readiness must call `notify`
  (pipes on every read and write and on the last close of an end).
  Spurious wake-ups are fine; every wait checks again.
- Timer files are woken by the `timers` kernel thread (`ps` lists it
  once the first timer was armed); `fs::poll::pending_timers()` counts
  the callbacks it waits for. A timer read always counts expirations
  from the clock, so a late callback delays a waiter but loses nothing.
- Signal files read the reading thread's pending signals: a signal the
  thread does not block is delivered normally first (and ends the read
  with `EINTR`). `rt_sigpending` shows what is pending.
- A blocked read interrupted by a signal returns
  `RESTART_IF_SA_RESTART` inside the kernel: the call is restarted under
  `SA_RESTART` (`rip` back at the `syscall`) or fails with `EINTR`;
  `poll` / `select` are never restarted after a handler, `epoll_wait`
  never at all.

The kernel tests run with `poly.test=linux::fileio_tests`; the host tests
are `cargo test -p poly-vfs objects` and `cargo test -p
poly-personality-linux fileio`.

## Sockets

Sockets (ADR 0048) are open files on anonymous `S_IFSOCK` inodes: a
thread blocked in `accept`, `recv`, `connect` or a
blocking `read` / `write` on one is parked on that socket's `WaitQueue`,
like a pipe reader (see the previous section). What to check when one
hangs:

- An Internet socket's queue is woken by `net::notify` after a poll of
  the stack found the socket changed (`Stack::take_changed`). The socket
  file must be watched: `net::notify::watched()` counts them (every
  `InetSocket` registers itself, and unregisters when dropped). A change
  the stack does not see as one (readiness and queued input the same
  as before) wakes nobody; every waiter retries after a wake-up anyway.
- `ifconfig` shows whether packets moved at all; loopback traffic is the
  stack's `loop` count.
- Unix sockets wake each other directly (a send notifies the receiver, a
  receive the sender or the blocked datagram senders, `accept` the
  connectors waiting for a backlog slot).
- Descriptors in flight (`SCM_RIGHTS`) live in receive queues until
  received or dropped with the queue; `linux::net::in_flight()` counts
  them, and the collector for unreachable cycles runs after every
  `close` while any are in flight.
- A call a signal ended returns `RESTART_IF_SA_RESTART` inside the
  kernel unless the socket has a timeout (`SO_RCVTIMEO` for accept and
  receives, `SO_SNDTIMEO` for connect and sends), as for pipes.

`/bin/socket-test` (`tools/xtask/programs/socket_test.rs`, in the test
initramfs) prints one line per check (`socket-test: ok <name>` or
`socket-test: FAIL <name>: <value>`). Run it from the shell (`run
/bin/socket-test` with the test initramfs), or as the first process:

```sh
cargo xtask pipeline initramfs target/xtask-programs/socket-test --out target/socket-test.cpio
cargo xtask run --headless --no-disk --initrd=target/socket-test.cpio poly.qemu_exit poly.init=/test-binary
```

With `/etc/socket-test.conf` (`<address> <port>`) in the image it also
talks to TCP and UDP echo servers there; the network boot test of `cargo
xtask test` does that with the host's servers at 10.0.2.2. The kernel
tests run with `poly.test=socket_tests`; the host tests are `cargo test
-p poly-unix`, `cargo test -p poly-net syscalls` and `cargo test -p
poly-personality-linux socket` / `netlink`.

## Terminals

The terminals (ADR 0046) log one line at boot:

```text
tty: ttyS0 (4,64) on COM1, tty1 (4,1) on the screen, /dev/console is ttyS0, ptys on /dev/pts
```

`/dev/console` is the terminal `console=` names: `ttyS0` (the default)
or `tty1`, the screen terminal, which then comes up before the first
program. A program started with `poly.init` gets `/dev/console` on fds
0-2 but no controlling terminal (as on Linux): a shell started there has
no job control, and `^C` signals nobody, until it takes the terminal with
`ioctl(0, TIOCSCTTY, 0)` (BusyBox: `setsid cttyhack sh`). A program
started with the shell's `run` gets the shell's own terminal and leads a
session on it, so its job control works.

```sh
cargo xtask run --initrd=prog.cpio poly.init=/bin/sh            # init on the serial terminal
cargo xtask run --initrd=prog.cpio poly.init=/bin/sh console=tty1   # init on the screen's terminal
```

**Who gets typed input.** While any program has a terminal open, its
device's input is the terminal's (threads `tty-serial` and `tty-keys` in
`ps`); otherwise the kernel shell on that device reads it. If the serial
shell seems deaf after a `run`, something the program started still has
`/dev/ttyS0` open (a background child): it gets the input. Program output
on COM1 is written as it comes (`log::write_tty`), erasing a shown shell
prompt first; it does not appear on the screen.

**A read that does not return.** In canonical mode (the default) a read
waits for a whole line: `Enter` (CR, which `ICRNL` turns into NL),
`Ctrl-D` at the start of a line for end of file. In raw mode `MIN` and
`TIME` decide. A process in a background group of its terminal's session
that reads is stopped by `SIGTTIN` (`linux: pid N stopped by signal 21`
in the log with `poly.strace`); writing with `TOSTOP` set, or changing the terminal, stops it
with `SIGTTOU` (22). `stty -a` (or `TCGETS`) shows the attributes;
`stty sane` puts them back.

**Signals.** `^C` (`SIGINT`), `^\` (`SIGQUIT`) and `^Z` (`SIGTSTP`) go to
the terminal's foreground process group, and only when the terminal is a
session's controlling terminal. Init only gets the ones it handles. When a
session leader ends, its terminal's foreground group gets `SIGHUP`; a pty
whose master closes sends `SIGHUP` to the session leader and `SIGHUP` and
`SIGCONT` to the foreground group, and its slave reads end of file from
then on.

**Pseudo-terminals.** `/dev/ptmx` makes a pair; `/dev/pts` lists one node
per open master (`ls /dev/pts` in the shell). A slave cannot be opened
(`EIO`) until the master unlocks it (`TIOCSPTLCK` with 0, what
unlockpt(3) does). With every slave closed, master reads fail with `EIO`.

The tests: `cargo test -p poly-tty` (the line discipline, terminals and
ptys on the host), `cargo test -p poly-personality-linux jobs` (orphaned
process groups), `cargo xtask run poly.test=tty::` (from ring 3), and the
boot test that types into `/bin/ttyinit` as init on the serial port
(`cargo xtask test`; its output shows the echo of what was typed).

## BusyBox

BusyBox as init with a shell on the console (ADR 0051):

```sh
cargo xtask run --busybox                  # init and a shell on the serial console (this terminal)
cargo xtask run --busybox console=tty1     # the shell on the screen terminal (QEMU's window)
cargo xtask busybox-initramfs              # just write target/poly-busybox-initramfs.cpio
cargo xtask busybox-test                   # the boot test alone
```

The BusyBox is the host's static one (`apt install busybox-static`), or
`POLY_BUSYBOX=<file>`, or BusyBox 1.36.1 built once into
`target/busybox/` (`POLY_BUSYBOX=build` forces that; the build log is
`target/busybox/build.log`). The boot shows `rcS: Linux 6.1.0-poly up,
file systems mounted`, the shell's banner and `root@poly:~# `. `poweroff`
ends QEMU (ACPI S5: `poweroff: ACPI S5 (SLP_TYP 0/0 to PM1a 0x604)`),
`reboot` resets (QEMU exits too, with `-no-reboot`), Ctrl-A X always
works. `/dev/vda`, `/dev/vdb`, `/dev/vdc` are the virtio-blk disks; with
the test disks attached (the default) `/mnt` has the ext2 test disk
(`poly.mount=virtio2:/mnt`), and `mount -t ext2 /dev/vdc /m` mounts it
where you like once it is unmounted.

**An applet that fails.** First look for `syscall: pid N linux/linux #NR
name not implemented` in the log (each call is logged once per boot).
Then trace it: `poly.strace` writes every system call of every Linux
process, with string arguments, as it returns:

```text
strace: pid 20 openat(0xffffff9c, "/dev/tty", 0x2, 0x0, ..) = -6 ENXIO
strace: pid 20 ioctl(0x0, 0x541e, 0x7fffffffeb08, 0x0, ..) = -25 ENOTTY
```

`poly.strace=<name>` keeps the lines of processes whose `comm` is
`<name>` (the applet's name when the shell ran it through its link,
`busybox` for `/sbin/init`). Tracing is slow over the serial line; trace
one applet, not a whole session. Many applets run inside the shell
without a new process (`sleep` and `echo`, for instance), so their calls show up
under the shell's pid. With `poly.strace` the kernel also logs every stop,
continue and kill of a process (`linux: pid 13 killed by signal 15`);
without it only kills by signals that dump core (`SIGSEGV`, `SIGABRT`,
`SIGQUIT`, ...).

**Init.** BusyBox's init blocks its signals and takes them with
`rt_sigtimedwait`; `poweroff` sends it `SIGUSR2` (`kill(0x1, 0xc, ..)`
in a trace). If `poweroff` or `reboot` does nothing, trace pid 1. A shell
that says `can't access tty; job control turned off` did not get a
controlling terminal: `cttyhack` finds the console's terminal with
`TIOCGSERIAL` (serial) or `VT_GETSTATE` (screen) and opens `/dev/ttyS0` or
`/dev/tty1`.

**The boot test** types into the shell through QEMU's standard input
and waits for each prompt (90 s per step); when a step fails it names the
step and the output, and writes the whole session (escape sequences
removed) to `target/poly-busybox-boot.log`. It is skipped with the reason
when no BusyBox could be found or built.

## The root disk: bash, coreutils, Python, gcc

The host's bash, coreutils, grep, sed, Python 3 and gcc on an ext4 disk,
booted through BusyBox's init and a chroot (ADR 0059):

```sh
cargo xtask root-disk                    # write target/poly-root-disk.img afresh (prints its path)
cargo xtask run --disk-root              # boot it: bash on the serial console (the image is kept)
cargo xtask run --disk-root --fresh-disk # make the image again first
cargo xtask root-disk-test               # the boot test alone (its own image, poly-root-disk-test.img)
```

The image is made from the host's files (nothing is run on the host to
find them); a program the host lacks is left out with a `note:` line and
the boot test skips the steps that need it. The boot shows `rootdisk:
/dev/vda mounted on /mnt/root` and the disk's bash prompt `root@poly:~# `.
Inside, `poweroff` and `reboot` are scripts in `/usr/local/sbin` that
signal init (pid 1, outside the chroot); Ctrl-A X always works. Mounts in
`/proc/mounts` show the system's paths (`/mnt/root`, ...), not the
chroot's. Without e2fsprogs (or with `POLY_ROOTDISK_FORMAT=poly-ext2`) the
image is written by `poly-ext2` instead of `mke2fs -d`.

After a boot the image can be read on the host: `debugfs -R 'ls -l /root'
target/poly-root-disk.img`, `e2fsck -fn target/poly-root-disk.img` (the
image must have been unmounted: `poweroff`, not Ctrl-A X).

**The boot test** types one command line per step into the disk's bash
and compares the output exactly; it reports every failing step at the end
(`==> FAILED: <step>: <output>` as it goes) and writes the session to
`target/poly-rootdisk-boot.log` (`poly-rootdisk-boot2.log` for the second
boot). Two switches help with one program:

```sh
POLY_ROOTDISK_STEPS=poly-test-sys cargo xtask root-disk-test   # only steps whose name has this text
POLY_ROOTDISK_APPEND=poly.strace=python3 cargo xtask root-disk-test   # more kernel arguments
```

`strace:` lines (and the kernel's `syscall:`, `fs:` and `linux:` lines) are
taken out of each step's output, so a traced run still checks it; the
trace is in the log. Any `not implemented` line but `rseq` fails the test.
What earlier failures looked like, and where to look:

- `bash: /usr/bin/python3: Cannot allocate memory` or gcc's `cannot execute
  '.../cc1': execv: Cannot allocate memory`: exec could not get kernel
  memory for the file (it reads only the ELF headers now,
  `Elf::parse_prefix`); `ENOMEM` from `mmap` with plenty of RAM free is a
  region map limit (`VmMap::remap`, `place`).
- `OSError: [Errno 38] Function not implemented` from Python: a call
  without a handler; the `syscall: ... not implemented` line names it.
- A program that hangs in a lock: `F_SETLKW` and blocking `flock` wait for
  `fs::locks::generation()` to move; locks go on every close of a
  descriptor of the file by the owning process and when a description is
  released (`fs::locks::closed`). A lock that never goes usually means a
  descriptor is still open somewhere (a child that inherited it).
- `dd` or a reader that sees short reads of a regular file or
  `/dev/zero`: reads of those return all that was asked
  (`sys::io::read_into`); pipes and terminals give what is there.

## Time, timers and the vDSO

The first exec sets the vDSO up and says how `clock_gettime` works in
user mode (ADR 0044):

```text
vdso: 3616 bytes, mapped into every Linux process; clock_gettime reads the TSC
```

"reads the TSC" is the normal case, under QEMU too: QEMU's CPU models do
not report an invariant TSC but set the hypervisor bit, and a virtual
CPU's TSC counts as constant rate (ADR 0072). On a real CPU without an
invariant TSC the clock reads the HPET and the line says "makes system
calls (the clock does not read a constant-rate TSC)".
The image is `kernel/target/x86_64-unknown-none/<profile>/build/poly-kernel-*/out/vdso.so`:
`readelf -lrdW --dyn-syms -V` shows its one `PT_LOAD`, the five
`__vdso_*@@LINUX_2.6` symbols and no relocations (`build.rs` refuses the
build otherwise), `objdump -d -M intel` its code. In a process the image
sits right above its data page, both near the top of the `mmap` area;
`AT_SYSINFO_EHDR` gives the image's address.

`cargo xtask run poly.test=time` runs the time tests (sleeps, a POSIX
timer, interval timers ending a spinning process, CPU-time clocks) and
`poly.test=vdso` the vDSO ones. `/bin/time-test` prints one line per check
(`time-test: ok <name>` or `time-test: FAIL <name>: <value>`); run it from
the shell (`run /bin/time-test` with the test initramfs) or as the first
process:

```sh
cargo xtask pipeline initramfs target/xtask-programs/time-test --out target/time-test.cpio
cargo xtask run --headless --no-disk --initrd=target/time-test.cpio poly.qemu_exit poly.init=/test-binary
```

A timer that does not fire: the `linux-timers` thread must exist (`ps`;
it starts with the first timer anyone arms), and a signal with the default
action never reaches process 1 (kill(2)'s rule for init, which timers
follow too). CPU-time clocks only move while the thread runs; their user
and system split follows the TSC cycles spent in each mode, measured at
every entry from and return to ring 3 (ADR 0070). Under a loaded host a
running thread is charged for the time QEMU's vCPU is held off, mostly as
system time: under TCG every clock read is an HPET access that waits for
QEMU's global lock, so a system call can take tens of milliseconds.
Tests of CPU time therefore enclose CPU-clock readings in clock-time
readings and compare against what passed between them.

## Disk file systems and the page cache

`cargo xtask run` and the boot tests attach a third virtio-blk device,
`target/poly-test-ext2.img` (8 MiB of ext2 with 4 KiB blocks, written by
`tools/xtask/src/ext2disk.rs` with `poly-ext2` before every boot, the same
bytes every time), and a fourth, `target/poly-test-ext4.img` (16 MiB of
ext4 with a 4 MiB journal, `tools/xtask/src/ext4disk.rs`, likewise), and
pass `poly.mount=virtio2:/mnt,virtio3:/ext4 poly.test.ext2
poly.test.ext4`. The kernel brings the disks up when the mount asks for
them and logs each mount:

```text
fs: page cache of up to 16190 pages (63 MiB); writeback every 5 s
virtio-blk: 00:06.0: MSI-X with 3 vectors, table BAR1+0x0, PBA BAR1+0x800
block: virtio2: virtio-blk 00:06.0, 16384 blocks of 512 bytes (8 MiB), write cache; queue 32 requests, cache 128 buffers (64 KiB); interrupt-driven (vector 0x32)
fs: virtio2 mounted on /mnt: ext2, 2048 blocks of 4096 bytes (1901 free), 1024 inodes (1008 free), read-write
```

An ext2 volume (or ext4 without a journal) that was not unmounted clean
adds `; was not cleanly unmounted (run e2fsck)`. A journaled one is
consistent once its journal is replayed, which mounting does first and
reports:

```text
fs: virtio3: journal replayed (2 transactions), 1 orphans processed
```

A mount that fails says why (`fs: mounting virtio2 on /mnt failed: not an
ext2/3/4 file system`, `no such disk`, `unsupported incompatible features
0x10` (here `meta_bg`), `corrupt superblock checksum`, `the journal needs
recovery but the device is read-only`). An unknown read-only-compatible
feature (quota, ...) or `inline_data` mounts read-only.
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
dumpe2fs -h target/poly-test-ext4.img                  # features, journal, needs_recovery, orphans
debugfs -R 'logdump -a' target/poly-test-ext4.img      # the journal's transactions (after a crash)
debugfs -R 'ex /dir/fragmented.bin' target/poly-test-ext4.img   # an extent tree, level by level
```

`e2fsck -fn` on an image whose journal needs replaying reports problems
without replaying it (`-n` changes nothing); replay first, on a copy: `cp
target/poly-test-ext4.img /tmp/x.img && e2fsck -fy /tmp/x.img`, or mount
it with the driver. A file system that refuses to mount with `EBADMSG`
errors in the log has a checksum mismatch: `e2fsck -fn` names the
structure. `poly_ext2::Ext2::stats()` counts commits (`commits`), what
mounting replayed (`replayed`) and orphans processed (`orphans`); with gdb,
a thread stuck in `poly_kernel::sync::RawMutex` under `commit` waits for
the disk's flush.

`cargo xtask` rewrites the image before the next boot; copy it first to
keep it. The boot test `ext2_persistence_test` writes a file in one boot,
reboots, runs `e2fsck -fn` (skipped with a message when e2fsprogs are not
installed) and reads the file in a second boot from the same image;
`ext4disk::boot_tests` does the same on `/ext4`, then boots with
`poly.test.ext4.crash` (a test writes and syncs `/ext4/committed.txt`, the
boot ends without unmounting), checks on the host that the image needs a
journal replay, and boots again: the mount must log the replay and the
file must be there.

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

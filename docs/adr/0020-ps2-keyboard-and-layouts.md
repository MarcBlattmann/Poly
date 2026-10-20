# 0020: PS/2 keyboard: decoding and layouts in `poly-input`, an i8042 driver on IRQ 1 with a byte ring

**Status:** Accepted. Builds on [ADR 0009](0009-drivers-behind-access-traits.md)
(drivers behind access traits) and [ADR 0016](0016-interrupt-dispatch-tick-and-kernel-locks.md)
(`route_isa_irq`, `WaitQueue`, handlers that never block).

## Context

Phase 2 builds a terminal into the kernel. It needs a keyboard: on QEMU
(and most PCs, often through firmware emulation of USB keyboards) that is
a PS/2 keyboard behind an i8042-compatible controller on ports 0x60/0x64,
raising ISA IRQ 1. The owner types on a Swiss German keyboard, so besides
US the first layouts include de-CH, with AltGr and dead keys. USB HID and
virtio-input come much later (Phase 6, Phase 11) and must reuse the key
and layout model, not the PS/2 decoder.

## Decision

**One host-tested crate, `poly-input`, for everything but port access.**
It holds the key model, the scancode decoder, the modifier state, the
layouts, the composer, a lock-free byte ring and the i8042 bring-up
sequence behind an `Io` trait (status, data, command, a clock), tested
against a simulated controller and keyboard that can be absent, mute,
fail its self-tests, ask for resends or refuse commands. The kernel
supplies the ports (`arch/x86_64/i8042.rs`) and the interrupt plumbing
(`keyboard.rs`).

**Keys are physical positions** (`Key`, named after the W3C UI Events
`code` values: `Key::Y` is the key a US keyboard labels Y). Decoders turn
bytes into `(Key, pressed)`; layouts turn keys into characters. A future
USB or virtio-input decoder produces the same `Key`s.

**Scancode set 1 with the controller translating.** Bring-up asks the
keyboard for set 2 (its default after reset) and turns the controller's
translation on, so the driver sees set 1, which every controller produces
and QEMU uses by default. Only a set 1 decoder exists. It handles the E0
and E1 prefixes, drops the 101-key keyboard's "fake shifts", decodes Print
Screen (also SysRq) and Pause (also Ctrl-Break, as E0 46), ignores
replies and unknown codes, and resynchronises after a broken sequence;
it never panics on any byte stream (exhaustive and randomized tests).

**State and output are separate steps.** `KeyState` tracks the keys down
(a press of a key already down is a typematic repeat, flagged on the
event), the modifier bits (either key of a pair) and the three locks
(toggled on non-repeated presses). `Composer` applies a `Layout` to a
`KeyEvent` and returns `Output`: nothing, one character, two characters
(an accent that did not combine, then the character), a pending dead key,
or a `Special` key for the line editor (Enter, arrows, Home, F-keys...).
`Keyboard` chains decoder, state and composer for the driver.

**Layouts are data.** A `Layout` is a table of six levels per key (base,
Shift, Caps Lock, Caps Lock + Shift, AltGr, AltGr + Shift), each a
character, a dead key or nothing, built at compile time by `Layout::new`
from `(Key, KeyDef)` entries and small helpers (`letter`, `chars`,
`.alt_gr()`, `.caps()`). The Caps Lock levels exist for de-CH, where Shift
on ü/ö/ä types è/é/à and Caps Lock types Ü/Ö/Ä. Keypad behaviour (Num Lock
xor Shift), Ctrl-letter control characters and dead-key composition are
code shared by all layouts; dead-key compositions are one table per accent.
Behaviour follows the public Windows layout descriptions: a dead key then
Space types the accent, then a letter it does not combine with types both,
then another dead key types both accents, then a special key drops it.
AltGr is the right Alt key on layouts that have it (`has_alt_gr`), and a
second Alt key on those that do not; Ctrl+Alt is not treated as AltGr.

**The interrupt handler only moves bytes.** IRQ 1 is routed with
`arch::irq::route_isa_irq` after the polled bring-up; then the keyboard
interrupt is enabled in the controller's configuration byte and the
controller is drained once, because a byte that arrived before that raised
no edge. The handler reads every waiting byte (bounded), pushes it into a
256-byte single-producer single-consumer `ByteRing` (atomics only, no
lock, no `unsafe`) and wakes a `WaitQueue`. Decoding runs in the thread
that calls `next_event` / `try_next_event`, under a sleeping `Mutex` held
only while bytes are decoded, never while waiting for input. A full ring
drops and counts bytes.

**Keyboard commands with the interrupt on.** After bring-up only the
handler reads the data port. To set the LEDs (0xED and a mask, each
acknowledged), the thread raises a "reply awaited" flag, writes the byte,
and sleeps on a second `WaitQueue`; the handler hands an ACK or RESEND to
it instead of the ring (neither is a set 1 key code). The LEDs follow the
locks whenever an event changes them.

**Lost interrupts are survived.** A waiting reader wakes every 100 ms and
drains the controller itself with interrupts off, as the handler would,
so a lost edge never wedges the keyboard.

**A missing controller is not an error.** If the ACPI FADT (revision 2 or
later) says there is no 8042, the kernel does not touch the ports.
Otherwise `i8042::init` probes: a status register reading 0xFF means
nothing is there, and every wait is bounded by time (100 ms for the
controller, 250 ms for an acknowledgement, 1 s for the keyboard's
self-test) and by a poll count. Any failure is logged and the kernel runs
on without a keyboard; `next_event` then only returns injected bytes.

**Tests at three levels.** Host: the decoder against a hand-written table
of every key's make and break bytes, the layouts key by key, dead keys,
the i8042 bring-up against the simulator, randomized byte streams. Kernel:
injected scancodes decoded through the ring (US and de-CH), modifiers on
events, the I/O APIC route, bytes the controller fakes with its "write
keyboard output buffer" command (0xD2) arriving through IRQ 1, and Caps
Lock LED updates acknowledged through the handler. End to end: `cargo
xtask test` boots with `poly.test.keyboard=<text>` and a QEMU monitor on
a Unix socket, waits for the kernel to say it is ready and types with
`sendkey`, once with the US layout and once with `poly.keymap=de-ch`
(AltGr, dead keys, ü/è); the kernel prints what it typed and fails the
test on a mismatch or after 30 s.

## Consequences

- Adding a layout is a table and an entry in `LAYOUTS`; adding a keyboard
  type (USB HID, virtio-input) is a decoder producing `ScanEvent`s, with
  `KeyState`, layouts and the composer reused.
- No set 2 or set 3 decoder: a controller that cannot translate is not
  supported. The mouse port stays disabled until a mouse driver exists.
- Bytes lost to a full ring can leave a key "down" until it is pressed
  again (`Keyboard::resync` forgets every key; nothing calls it yet).
- Keyboard hot-plug, keyboards on the second port and firmware that
  reports no 8042 while having one (the FADT is trusted) are not handled.
- The terminal gets characters and special keys, not a TTY: line editing,
  key repeat timing and input methods live above this layer.
- SMP: the ring has one producer, so IRQ 1 must stay on one CPU (it targets
  the boot CPU today) and injection masks interrupts on that CPU.

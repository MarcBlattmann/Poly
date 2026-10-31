# 0103: Input: one event layer in evdev's vocabulary, `/dev/input/event*`, the PS/2 mouse and virtio-input

**Status:** Accepted. Builds on [ADR 0020](0020-ps2-keyboard-and-layouts.md)
(PS/2 keyboard, layouts, `poly-input`), [ADR 0009](0009-drivers-behind-access-traits.md)
(drivers behind access traits), [ADR 0015](0015-pci-and-virtio-in-the-kernel.md)
(PCI and virtio) and [ADR 0042](0042-readiness-pipes-poll-epoll-and-event-files.md)
(wait queues and `poll`). Roadmap Phase 6: "Input: virtio-input / PS/2 keyboard and
mouse, input event layer, keymaps".

## Context

Until now the only input device was the PS/2 keyboard, and the only reader
the kernel's own terminal: bytes went from IRQ 1 into a byte ring and were
decoded by whoever asked for a key. A desktop needs more: mice (relative
motion, wheels, buttons), tablets (QEMU's `virtio-tablet-pci` gives an
absolute pointer, which is what a VM wants), virtio keyboards, and an
interface a compositor and libinput can use unchanged. On Linux that
interface is the event device, `/dev/input/event<n>`: `read` returns
`struct input_event`s, `poll` says when there are some, and the `EVIOC*`
ioctls describe the device and grab it. Its types and codes
(`linux/input-event-codes.h`) are also what virtio-input devices send
(VIRTIO 1.2 §5.8), so they are the natural vocabulary for every driver.

## Decision

**One event layer, host-tested, in evdev's vocabulary.** `poly_input::hub`
holds every device: its description (`DeviceInfo`: name, ids, a bitmap per
event type, absolute axis ranges, repeat parameters), its live state (keys
down, LEDs, axis values), the frame it is building, and its readers.
Drivers report `(type, code, value)` events; `SYN_REPORT` ends a frame,
which goes to every reader at once. The layer follows the semantics the
kernel's input documentation (`input.rst`, `event-codes.rst`) gives the
event interface, so `/dev/input/event*` is a thin shell over it:
undeclared events and events that change nothing are dropped (a key press
of a key that is down is the device's own repeat, value 2), empty frames
are not delivered, a reader whose queue cannot take a frame loses its
queue and gets `SYN_DROPPED`, `EVIOCGRAB` gives one reader every event.
It never allocates or sleeps while reporting (queues are made at `open`,
frames have a fixed capacity), so drivers report from interrupt handlers,
with the hub under an interrupt-safe spinlock.

**Keys as evdev key codes, `Key`s as physical positions.** `keycode` maps
the 105 `Key`s to `KEY_*` codes and back (the PC Application key is
`KEY_COMPOSE`, `KEY_MENU` maps to it too). The scancode decoders produce
`Key`s (set 1, as the controller translates, and now set 2, for keyboards
spoken to untranslated); virtio keyboards produce codes. Either way the
layer sees codes.

**The console is a reader of every keyboard.** Devices marked as console
keyboards (the PS/2 one; a virtio device with the letter keys) also feed
one console queue, which the screen terminal reads through `keyboard.rs`
as before (`next_event`, `try_next_event`, ...), with `Keyboard::apply`
turning key events into text. A grab takes a keyboard's keys away from the
console as well, as Linux's console behaves. The console's lock state
lights the lock LEDs of every console keyboard (`set_console_leds`),
through a hook per device: `0xED` for PS/2, `EV_LED` events on the status
queue for virtio.

**Repeat.** A device either repeats itself (PS/2: the keyboard's typematic
repeat arrives as more make codes, reported as value 2, so the screen
terminal behaves exactly as before) or asks the layer to (`soft_repeat`:
virtio keyboards that declare `EV_REP`): the last key pressed repeats after
the delay (250 ms) every period (33 ms), `EVIOCSREP` changes both. A
kernel thread, `input`, sleeps until the next repeat is due
(`Hub::next_deadline`) and makes it (`Hub::tick`).

**Decoding moves into the interrupt handler.** The PS/2 driver decodes set
1 bytes as they arrive (cheap and allocation-free) and reports key events
with the time of the interrupt, instead of buffering bytes for a reader.
Timestamps are then right, the event layer sees keys whether or not
anybody reads the console, and repeat timing cannot be confused by a slow
reader. The keyboard's command replies (`ACK`/`RESEND` of an LED update)
are still handed to the waiting thread.

**Keymaps are data.** Layouts stay tables of six levels per key (ADR
0020); German (`de`, T1 as Windows has it: dead ^ ´ \`, AltGr for @ € µ ² ³
{ [ ] } \\ | ~, keypad comma) joins US and Swiss German, selected with
`poly.keymap=`. `keysym` gives a key's X11 keysym (`X11/keysymdef.h`) on
the level the modifiers select, which a compositor's keymap needs; text
still comes from the `Composer` (dead keys).

**The PS/2 mouse** shares the i8042 with the keyboard. `i8042::init_aux`
(after the keyboard, polled, keyboard port disabled meanwhile) tests and
enables the second port, resets the mouse and knocks the IntelliMouse
sample-rate sequences (200, 100, 80: a wheel, ID 3; then 200, 200, 80:
five buttons, ID 4), then sets 100 reports/s, 8 counts/mm, 1:1 scaling and
reporting on. IRQ 12 is routed to the same handler as IRQ 1 (the controller
says whose byte it holds). `ps2mouse::MouseDecoder` assembles 3- or 4-byte
packets, finds the packet start again by bit 3 of the first byte and by
time gaps, drops overflowed counts; `MouseState` reports `REL_X`, `REL_Y`
(negated: PS/2 counts up, evdev down), `REL_WHEEL` (positive up, away
from the user) and button changes. QEMU's mouse answers ID 4.

**virtio-input** is a crate of its own, `poly-virtio-input`, like
virtio-blk and virtio-net. `read_info` queries the configuration (§5.8.4,
`select`/`subsel`: name, serial, ids, properties, `EV_BITS` per type,
`ABS_INFO` per axis) straight into a `DeviceInfo`; the event queue is kept
full of 8-byte buffers and the events go to the layer unchanged; the status
queue carries LED events to the device. Features: none of its own;
VERSION_1, EVENT_IDX and ACCESS_PLATFORM when offered. The kernel brings
up every function (MSI-X entry 0 for both queues and configuration
changes, masked until the handler is attached; polled by the input thread
every 10 ms without MSI-X).

**`/dev/input/event<n>`** is character device 13, 64 + n (the Linux device
list's "input core" entries), mode 0660 root:root (key presses are
private; Linux uses group `input`). Each `open` is a client of the hub
with a queue of 256 events. `read` returns whole events (24 bytes on
x86-64), `EINVAL` for a smaller buffer, `EAGAIN` when empty; blocking reads
and `poll` wait on a `poly_vfs` wait queue per device, which only the input
thread notifies (interrupt handlers must not take its lock). `write`
injects whole events into the device (an `EV_LED` lights the LED, every
reader sees it). The ioctls: `EVIOCGVERSION`, `EVIOCGID`, `EVIOCGREP` /
`EVIOCSREP`, `EVIOCGNAME` / `EVIOCGPHYS` / `EVIOCGUNIQ` / `EVIOCGPROP`,
`EVIOCGKEY` / `EVIOCGLED` / `EVIOCGSND` / `EVIOCGSW`, `EVIOCGBIT` (bitmaps
in whole 64-bit words), `EVIOCGABS` / `EVIOCSABS`, `EVIOCGRAB` (argument by
value; a second grab is `EBUSY`), `EVIOCREVOKE`, `EVIOCSCLOCKID`
(realtime by default, monotonic, boottime), `EVIOCGEFFECTS` (0). Keycode
remapping, multitouch slots, event masks and force feedback are `EINVAL`.
The request numbers are decoded by direction, size, type and number, as
`asm-generic/ioctl.h` encodes them.

**Sources.** VIRTIO 1.2 §5.8 (input device); IBM's PS/2 Hardware
Interface Technical Reference and the public IntelliMouse protocol notes
(OSDev wiki "PS/2 Mouse", "Mouse Input") for the mouse; Microsoft's
Keyboard Scan Code Specification (Appendix C, set 1 and set 2 columns);
the uapi headers `linux/input.h`, `linux/input-event-codes.h`,
`asm-generic/ioctl.h` and the kernel's input documentation for the event
interface (ABI facts only, CLEAN_ROOM.md); Microsoft's keyboard layout
descriptions for German; `X11/keysymdef.h` for keysyms; QEMU's QMP
reference for the test's `input-send-event`. No driver source (Linux,
QEMU or other) was read.

## Tests

- Host: `poly-input` (scancode set 2 round trips, fake shifts, Pause,
  resynchronisation; key code table; German layout; keysyms; the hub's
  frames, filtering, grabs, overruns with `SYN_DROPPED`, soft repeat, LEDs,
  revocation, long frames; the evdev bytes, request numbers against the C
  values, every ioctl; the mouse decoder and its event conversion; the
  mouse bring-up against the simulated controller, now with a mouse) and
  `poly-virtio-input` (against a simulated device: configuration, event
  delivery with buffers recycled, the status queue, interrupts, short
  buffers, a broken device reset, bring-up failures).
- Kernel (`input::*`, `mouse::*`, `keyboard::*`): event nodes exist with
  the right numbers; an event file reads injected keys and its readers are
  woken by the input thread; a grab keeps keys from the console; holding a
  key soft-repeats through the input thread; mouse bytes faked by the
  controller (0xD3) arrive through IRQ 12 as motion, wheel and a click;
  the existing keyboard tests, now through the event layer, plus German.
- Boot group `input`: `/bin/evtest` (no C library) as init describes every
  device with the ioctls, checks a grab and an LED write, then reads what
  QMP's `input-send-event` sends: through the PS/2 keyboard and mouse
  (keys, motion, the wheel, a click), and with `virtio-keyboard-pci`,
  `virtio-mouse-pci` and `virtio-tablet-pci` (keys on the virtio keyboard,
  motion on the virtio mouse, an absolute position on the tablet); then the
  event layer's kernel tests with the three virtio devices attached.

## Consequences

- A compositor can open `/dev/input/event*` directly. libinput also wants
  udev to find devices (`/run/udev`, sysfs `input` class); that is not
  provided yet, nor `/dev/input/mice`, `/proc/bus/input/devices`, hotplug,
  multitouch or force feedback.
- Kernel-side readers other than the console would be clients of the hub
  like the event files (the NT personality's raw input, later).
- The PS/2 keyboard is still read with translation on; set 2 decoding is
  ready for controllers without translation.

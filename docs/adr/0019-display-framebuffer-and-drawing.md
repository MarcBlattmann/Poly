# 0019: The display: Bochs VBE mode setting, a write-combining framebuffer behind a RAM back buffer, and a drawing library over caller memory

**Status:** Accepted. Amends [0010](0010-kernel-address-space.md): the
kernel now reprograms one entry of the PAT, for write-combining mappings.

## Context

Phase 2 builds a terminal into the kernel: a debug console on screen until
user space and the desktop exist. It needs a display driver, drawing
primitives, a font and a text screen model, in a way that the terminal
(and later a panic screen) can use from any context, that the boot tests
can check without a window, and that stays useful once virtio-gpu
(Phase 6) replaces the device underneath.

QEMU's `q35` machine comes with the standard VGA (PCI 1234:1111), which
implements the Bochs VBE ("DISPI") interface: a handful of 16-bit
registers that set a linear framebuffer mode. It needs no firmware calls,
which a PVH boot does not have anyway. Its framebuffer is a PCI BAR. The
kernel maps device memory uncached only ([ADR 0010](0010-kernel-address-space.md)),
which makes every framebuffer write a separate bus transaction on real
hardware and makes reading pixels back (to scroll) slow everywhere.

## Decision

**Bochs VBE first, behind a host-tested crate.** `poly-bochs-vbe` holds the
protocol (register indices, probing, the off / set / on sequence for a mode,
reading it back and checking it) behind a `Registers` trait, tested against
a simulated device, including ones that refuse a mode or never switch on
([ADR 0009](0009-drivers-behind-access-traits.md)). The kernel implements
the trait over the device's MMIO BAR (registers at 0x500) and, for VGA
devices without it, over I/O ports 0x1ce/0x1cf (`arch/x86_64/dispi.rs`).
The mode comes from `poly.display=<width>x<height>`, else 1280x800, always
32 bits per pixel (XRGB8888); a mode the device refuses falls back to the
default. `poly.display=off` leaves the device alone.

**Write-combining through PAT entry 1.** `paging::init` loads IA32_PAT with
the power-on table except entry 1 (PWT alone), which becomes WC instead of
WT, before any mapping uses that entry. `mm::map_mmio_wc` maps with PWT
alone; it refuses ranges the direct map covers (RAM, and reserved ranges,
which the direct map maps uncached), so no page has two memory types. The
framebuffer BAR is mapped this way, once, and never through
`Device::map_bar`. Without PAT (CPUID) it is mapped uncached. Application
processors must load the same PAT when SMP arrives.

**All drawing goes to a back buffer in RAM.** The back buffer is
`width × height × 4` bytes of physically contiguous frames
(`frames::allocate_contiguous`, which strings 4 MiB blocks together for
modes past 1280x800, since a heap allocation is at most 4 MiB), reached
through the write-back direct map. Drawing, scrolling (moving pixels
within it) and reading pixels back cost what RAM costs. `display::flush`
copies a rectangle to the framebuffer, row by row: the framebuffer is only
ever written, in long runs, which is where write-combining pays. The
display lives behind a spinlock, so any context may draw; `display::draw`
draws and flushes under one lock.

**`poly-gfx`: drawing over caller memory, allocation-free.** A `Canvas` is
width, height, stride and a `PixelFormat` (XRGB8888, XBGR8888, RGB888,
RGB565) over a `&mut [u8]`; it fills and clips rectangles, draws lines,
blits (converting formats), copies within itself (memmove semantics) and
draws text. Like every shared crate it allocates nothing: the kernel gives
it the back buffer, and `TextGrid` gets its cells as a slice whose size
decides the scrollback.

**A permissively licensed font, converted by a checked tool.** Spleen
(BSD 2-Clause) 8x16 and 12x24, as BDF files in `third_party/spleen/` with
their licence, listed in `THIRD_PARTY.md`. `cargo xtask font` converts them
to Rust tables (code point ranges and bitmaps as hex strings decoded at
compile time); the tables are checked in, and `cargo xtask ci` fails if they
no longer match the BDFs. Characters a font lacks draw as a box.

**The text screen tracks what changed.** `TextGrid` keeps, per visible row,
the span of columns written since the last render, how many lines the screen
scrolled, and where it drew the cursor. `render` first moves the pixels of
scrolled lines (`copy_within`), then draws only the dirty cells, and
returns the rectangle it touched for `flush`. It handles the escape
sequences a shell's output uses (SGR colours and attributes, cursor
movement, erasing, insert and delete) and ignores the rest. A randomized
test feeds two grids the same input, renders one incrementally and the
other from scratch, and requires identical pixels.

**The boot screen is shared with the boot test.** `poly_gfx::boot_screen`
draws "Poly", the kernel version and the mode. A boot test asks for a
non-default mode, takes a screenshot through QEMU's QMP socket
(`screendump`, which works with `-display none`) after the display tests,
and compares it pixel for pixel with the same picture rendered on the host.

## Consequences

- The driver is QEMU-only (Bochs and VirtualBox's old adapter also have
  DISPI; real hardware does not). Real hardware gets a framebuffer from
  UEFI GOP (Phase 11) or a GPU driver; both only need to provide a mapping,
  a stride and a format, and the rest (back buffer, flush, everything in
  `poly-gfx`) stays.
- Under TCG at the dev profile's `opt-level = 1`, a full redraw of a
  1280x800 screen of 8x16 text takes about 40 ms, a full flush about 20 ms
  and one changed cell well under a millisecond. A terminal should render
  once per batch of output, not per line: a render after many scrolled
  lines costs one full redraw.
- The back buffer costs `width × height × 4` bytes of RAM (4000 KiB at
  1280x800), and modes over 4 MiB round up to whole 4 MiB blocks.
- Drawing holds a spinlock with interrupts off; a full redraw delays the
  tick by its duration. Keep work under the lock to what changed.
- The kernel's PAT differs from the power-on one in entry 1 only; any code
  that sets PWT without PCD now gets write-combining, not write-through.

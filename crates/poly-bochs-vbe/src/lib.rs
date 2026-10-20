//! The Bochs VBE display interface ("DISPI"), as QEMU's standard VGA
//! (`-device VGA`, the default display of the `q35` and `pc` machines) and
//! `bochs-display` devices offer it: probing and setting a linear
//! framebuffer mode.
//!
//! The interface is a bank of 16-bit registers selected by index
//! ([`index`]). The guest turns the display off ([`enable`] = 0), writes
//! the resolution and colour depth, and turns it on again with the linear
//! framebuffer flag; the device then shows `width × height` pixels from the
//! start of its framebuffer (PCI BAR 0), `VIRT_WIDTH` pixels per line.
//! Reading XRES, YRES and BPP while the GETCAPS flag is set gives the
//! largest values the device accepts; VIDEO_MEMORY_64K gives the
//! framebuffer's size.
//!
//! The registers are reached in one of two ways, which the kernel
//! implements behind [`Registers`]: an index port and a data port (I/O
//! ports 0x1ce and 0x1cf, VGA devices only), or memory-mapped, register `i`
//! at offset `0x500 + 2 * i` of the device's MMIO BAR (BAR 2), which both
//! devices have.
//!
//! Sources: QEMU's documentation of its standard VGA device
//! (`docs/specs/standard-vga.rst`: the MMIO BAR layout, the PCI IDs) and the
//! public description of the Bochs VBE extensions (the OSDev wiki, "Bochs
//! VBE Extensions": register indices, enable flags, interface IDs). Like
//! the other driver crates ([ADR 0009](../../docs/adr/0009-drivers-behind-access-traits.md)),
//! everything read from the device is checked, nothing panics on it, and
//! nothing allocates.

#![no_std]

use core::fmt;

/// PCI vendor and device ID of QEMU's standard VGA and `bochs-display`.
pub const PCI_VENDOR: u16 = 0x1234;
pub const PCI_DEVICE: u16 = 0x1111;

/// Offset of the DISPI registers in the MMIO BAR: register `i` is the
/// 16-bit word at `MMIO_DISPI + 2 * i`.
pub const MMIO_DISPI: usize = 0x500;
/// Offset of the VGA I/O ports 0x3c0-0x3df in the MMIO BAR of the standard
/// VGA, one byte per port.
pub const MMIO_VGA_PORTS: usize = 0x400;
/// QEMU's extended registers in the MMIO BAR (QEMU 2.2 and later): the size
/// of this block (32 bits), then the framebuffer byte order.
pub const MMIO_QEMU_EXT: usize = 0x600;
/// Framebuffer byte order register, relative to [`MMIO_QEMU_EXT`].
pub const QEMU_EXT_ENDIANNESS: usize = 0x4;
/// [`QEMU_EXT_ENDIANNESS`] value for little-endian pixels.
pub const QEMU_EXT_LITTLE_ENDIAN: u32 = 0x1e1e_1e1e;

/// The I/O ports of the index / data interface.
pub const PORT_INDEX: u16 = 0x1ce;
pub const PORT_DATA: u16 = 0x1cf;

/// Register indices.
pub mod index {
    /// Interface version: [`ID_MIN`](super::ID_MIN) and up.
    pub const ID: u16 = 0x0;
    pub const XRES: u16 = 0x1;
    pub const YRES: u16 = 0x2;
    /// Bits per pixel: 4, 8, 15, 16, 24 or 32.
    pub const BPP: u16 = 0x3;
    /// [`enable`](super::enable) flags.
    pub const ENABLE: u16 = 0x4;
    /// The 64 KiB window of the framebuffer the legacy VGA aperture shows.
    pub const BANK: u16 = 0x5;
    /// Pixels per framebuffer line, and lines in the framebuffer.
    pub const VIRT_WIDTH: u16 = 0x6;
    pub const VIRT_HEIGHT: u16 = 0x7;
    /// Where in the virtual screen the display starts.
    pub const X_OFFSET: u16 = 0x8;
    pub const Y_OFFSET: u16 = 0x9;
    /// Framebuffer size in 64 KiB units (interface version 4 and up).
    pub const VIDEO_MEMORY_64K: u16 = 0xa;
}

/// Bits of the ENABLE register.
pub mod enable {
    pub const ENABLED: u16 = 0x01;
    /// While set, XRES, YRES and BPP read as the largest values supported.
    pub const GETCAPS: u16 = 0x02;
    pub const DAC_8BIT: u16 = 0x20;
    /// Show the linear framebuffer (rather than the banked window).
    pub const LFB: u16 = 0x40;
    /// Keep the framebuffer's contents when the mode is set.
    pub const NO_CLEAR_MEM: u16 = 0x80;
}

/// The lowest interface ID; a device reports `ID_MIN + version`.
pub const ID_MIN: u16 = 0xb0c0;
/// The highest ID accepted as this interface.
pub const ID_MAX: u16 = 0xb0cf;
/// Version 2 brought 32 bits per pixel and the linear framebuffer, 3
/// GETCAPS, 4 VIDEO_MEMORY_64K.
const VERSION_LFB: u16 = 2;
const VERSION_GETCAPS: u16 = 3;
const VERSION_VIDEO_MEMORY: u16 = 4;

/// Access to the DISPI registers by index.
pub trait Registers {
    fn read(&self, index: u16) -> u16;
    fn write(&self, index: u16, value: u16);
}

impl<R: Registers + ?Sized> Registers for &R {
    fn read(&self, index: u16) -> u16 {
        (**self).read(index)
    }

    fn write(&self, index: u16, value: u16) {
        (**self).write(index, value)
    }
}

/// What a device can do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    /// The interface version (ID minus [`ID_MIN`]).
    pub version: u16,
    /// Largest width, height and bits per pixel, if the device says.
    pub max: Option<Mode>,
    /// Framebuffer size in bytes, if the device says.
    pub vram: Option<u64>,
}

/// A resolution and colour depth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mode {
    pub width: u16,
    pub height: u16,
    pub bpp: u16,
}

impl Mode {
    /// `width` × `height` at 32 bits per pixel.
    pub const fn new(width: u16, height: u16) -> Mode {
        Mode { width, height, bpp: 32 }
    }

    /// A mode written as `<width>x<height>` (as on a kernel command line),
    /// at 32 bits per pixel. Both must be at least 1.
    pub fn parse(s: &str) -> Option<Mode> {
        let (w, h) = s.split_once(['x', 'X'])?;
        let num = |t: &str| -> Option<u16> {
            let n: u16 = t.parse().ok()?;
            (n > 0 && t.bytes().all(|b| b.is_ascii_digit())).then_some(n)
        };
        Some(Mode::new(num(w)?, num(h)?))
    }

    /// Bytes the mode needs: `width × height` pixels.
    pub const fn bytes(&self) -> u64 {
        self.width as u64 * self.height as u64 * self.bpp.div_ceil(8) as u64
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x{}x{}", self.width, self.height, self.bpp)
    }
}

/// The mode the device shows, as its registers say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModeInfo {
    pub mode: Mode,
    /// Pixels per framebuffer line.
    pub virt_width: u16,
    /// Lines the framebuffer holds at this line length.
    pub virt_height: u16,
    /// Where the display starts in the framebuffer, in pixels and lines.
    pub x_offset: u16,
    pub y_offset: u16,
}

impl ModeInfo {
    /// Bytes from the start of one line to the start of the next.
    pub const fn stride(&self) -> usize {
        self.virt_width as usize * self.mode.bpp.div_ceil(8) as usize
    }

    /// Byte offset in the framebuffer of the display's top-left pixel.
    pub const fn offset(&self) -> usize {
        self.y_offset as usize * self.stride() + self.x_offset as usize * self.mode.bpp.div_ceil(8) as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The ID register does not hold a DISPI ID: no such device there.
    NotPresent { id: u16 },
    /// Older than version 2: no linear framebuffer at 32 bits per pixel.
    TooOld { version: u16 },
    /// Larger, or deeper, than the device supports.
    Unsupported { asked: Mode, max: Mode },
    /// The mode does not fit the framebuffer.
    TooLarge { needed: u64, vram: u64 },
    /// The device did not take the mode: what it shows instead, if anything.
    Rejected { asked: Mode, got: Option<ModeInfo> },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotPresent { id } => write!(f, "no Bochs VBE interface (ID register reads {id:#06x})"),
            Error::TooOld { version } => write!(f, "Bochs VBE version {version} has no 32-bit linear framebuffer"),
            Error::Unsupported { asked, max } => write!(f, "{asked} is beyond the device's maximum of {max}"),
            Error::TooLarge { needed, vram } => {
                write!(f, "the mode needs {needed} bytes but the framebuffer has {vram}")
            }
            Error::Rejected { asked, got: Some(got) } => {
                write!(f, "asked for {asked}, the device shows {}", got.mode)
            }
            Error::Rejected { asked, got: None } => write!(f, "asked for {asked}, the device left the display off"),
        }
    }
}

/// Check the interface ID and read what the device can do. Reading the
/// maximum mode turns the display off, so probe before [`set_mode`].
pub fn probe(r: &impl Registers) -> Result<Capabilities, Error> {
    let id = r.read(index::ID);
    if !(ID_MIN..=ID_MAX).contains(&id) {
        return Err(Error::NotPresent { id });
    }
    let version = id - ID_MIN;
    if version < VERSION_LFB {
        return Err(Error::TooOld { version });
    }
    let max = (version >= VERSION_GETCAPS).then(|| {
        r.write(index::ENABLE, enable::GETCAPS);
        let max = Mode { width: r.read(index::XRES), height: r.read(index::YRES), bpp: r.read(index::BPP) };
        r.write(index::ENABLE, 0);
        max
    });
    let vram = (version >= VERSION_VIDEO_MEMORY).then(|| r.read(index::VIDEO_MEMORY_64K) as u64 * 0x1_0000);
    // A device that reports zeros does not know (or say) its limits.
    let max = max.filter(|m| m.width > 0 && m.height > 0 && m.bpp > 0);
    Ok(Capabilities { version, max, vram: vram.filter(|&v| v > 0) })
}

/// The mode shown now, if the display is on with the linear framebuffer.
pub fn current(r: &impl Registers) -> Option<ModeInfo> {
    let on = enable::ENABLED | enable::LFB;
    if r.read(index::ENABLE) & on != on {
        return None;
    }
    Some(ModeInfo {
        mode: Mode { width: r.read(index::XRES), height: r.read(index::YRES), bpp: r.read(index::BPP) },
        virt_width: r.read(index::VIRT_WIDTH),
        virt_height: r.read(index::VIRT_HEIGHT),
        x_offset: r.read(index::X_OFFSET),
        y_offset: r.read(index::Y_OFFSET),
    })
}

/// Switch to `mode` with the linear framebuffer, the display starting at
/// the beginning of the framebuffer, and check that the device took it.
/// `vram` is the framebuffer's size as the caller knows it (the BAR); the
/// smaller of it and the device's own figure counts. The device clears the
/// framebuffer.
pub fn set_mode(r: &impl Registers, caps: &Capabilities, mode: Mode, vram: u64) -> Result<ModeInfo, Error> {
    if let Some(max) = caps.max
        && (mode.width > max.width || mode.height > max.height || mode.bpp > max.bpp)
    {
        return Err(Error::Unsupported { asked: mode, max });
    }
    let vram = caps.vram.map_or(vram, |v| v.min(vram));
    if mode.bytes() > vram || mode.width == 0 || mode.height == 0 {
        return Err(Error::TooLarge { needed: mode.bytes(), vram });
    }
    // The device sets a mode when the display goes from off to on.
    r.write(index::ENABLE, 0);
    r.write(index::XRES, mode.width);
    r.write(index::YRES, mode.height);
    r.write(index::BPP, mode.bpp);
    r.write(index::ENABLE, enable::ENABLED | enable::LFB);
    r.write(index::X_OFFSET, 0);
    r.write(index::Y_OFFSET, 0);
    let got = current(r);
    match got {
        Some(info)
            if info.mode == mode
                && info.virt_width >= mode.width
                && info.offset() == 0
                && (info.stride() as u64) * mode.height as u64 <= vram =>
        {
            Ok(info)
        }
        _ => Err(Error::Rejected { asked: mode, got }),
    }
}

/// Turn the display off (back to VGA text or legacy modes).
pub fn disable(r: &impl Registers) {
    r.write(index::ENABLE, 0);
}

#[cfg(test)]
mod tests;

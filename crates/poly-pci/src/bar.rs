//! Base address registers (PCI 3.0 §6.2.5.1).
//!
//! Bit 0 tells memory (0) from I/O (1). A memory BAR's bits 2:1 give its
//! type: 00 is a 32-bit BAR, 10 a 64-bit BAR whose upper half is the next
//! register, 01 and 11 are reserved. Bit 3 is the prefetchable flag. The size
//! is never stored: software writes all ones, reads back which address bits
//! stuck (the size is the lowest one) and restores the original value.

use crate::{Bdf, ConfigSpace, Header, HeaderType, command, header, reg};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BarKind {
    Memory32,
    /// Occupies this register and the next.
    Memory64,
    Io,
}

/// A decoded and sized BAR.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Bar {
    pub kind: BarKind,
    /// Bus address (memory) or port number (I/O) currently programmed.
    pub address: u64,
    /// A power of two.
    pub size: u64,
    /// Always false for I/O.
    pub prefetchable: bool,
}

impl Bar {
    pub const fn is_memory(&self) -> bool {
        !matches!(self.kind, BarKind::Io)
    }

    /// Registers this BAR takes up.
    pub const fn slots(&self) -> usize {
        match self.kind {
            BarKind::Memory64 => 2,
            _ => 1,
        }
    }

    /// Does `[offset, offset + len)` lie inside the BAR? Used to check
    /// device-provided offsets (MSI-X table, virtio structures) before
    /// touching them.
    pub const fn contains(&self, offset: u64, len: u64) -> bool {
        match offset.checked_add(len) {
            Some(end) => end <= self.size,
            None => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarError {
    /// The header type has fewer BARs than that.
    NoSuchBar,
    /// Memory type 01 or 11.
    ReservedType,
    /// A 64-bit BAR in the last register, with no upper half.
    Truncated,
}

/// Decode and size BAR `index` of a function.
///
/// Memory and I/O decoding are switched off in the command register while
/// the BAR holds all ones, so the function never claims a bogus range, and
/// restored afterwards. Nothing else may touch the function meanwhile, and
/// the driver must not be using it. Returns `Ok(None)` for an unimplemented
/// BAR (no writable address bits).
pub fn probe_bar<C: ConfigSpace + ?Sized>(
    cfg: &C,
    bdf: Bdf,
    header_type: HeaderType,
    index: usize,
) -> Result<Option<Bar>, BarError> {
    if index >= header_type.bar_count() {
        return Err(BarError::NoSuchBar);
    }
    let off = reg::BAR0 + 4 * index as u16;
    let orig = cfg.read32(bdf, off);
    let kind = if orig & 1 != 0 {
        BarKind::Io
    } else {
        match (orig >> 1) & 3 {
            0b00 => BarKind::Memory32,
            0b10 => BarKind::Memory64,
            _ => return Err(BarError::ReservedType),
        }
    };
    if kind == BarKind::Memory64 && index + 1 >= header_type.bar_count() {
        return Err(BarError::Truncated);
    }

    let cmd = header::read_command(cfg, bdf);
    header::write_command(cfg, bdf, cmd & !(command::IO_SPACE | command::MEMORY_SPACE));
    let lo = size_register(cfg, bdf, off, orig);
    let (orig_hi, hi) = if kind == BarKind::Memory64 {
        let orig_hi = cfg.read32(bdf, off + 4);
        (orig_hi, size_register(cfg, bdf, off + 4, orig_hi))
    } else {
        (0, 0)
    };
    header::write_command(cfg, bdf, cmd);

    let (mask, address) = match kind {
        BarKind::Io => ((lo & !0x3) as u64, (orig & !0x3) as u64),
        BarKind::Memory32 => ((lo & !0xf) as u64, (orig & !0xf) as u64),
        BarKind::Memory64 => {
            (((hi as u64) << 32) | (lo & !0xf) as u64, ((orig_hi as u64) << 32) | (orig & !0xf) as u64)
        }
    };
    if mask == 0 {
        return Ok(None);
    }
    // The size is the lowest writable address bit. Using it rather than
    // `!mask + 1` also copes with I/O BARs that only decode 16 bits and
    // read back zero above them.
    let size = mask & mask.wrapping_neg();
    let prefetchable = kind != BarKind::Io && orig & 0x8 != 0;
    Ok(Some(Bar { kind, address, size, prefetchable }))
}

/// Write all ones, read back, restore.
fn size_register<C: ConfigSpace + ?Sized>(cfg: &C, bdf: Bdf, off: u16, orig: u32) -> u32 {
    cfg.write32(bdf, off, u32::MAX);
    let v = cfg.read32(bdf, off);
    cfg.write32(bdf, off, orig);
    v
}

/// Probe every BAR of a function, indexed by register: a 64-bit BAR sits at
/// its first register and the next one reads `None`. BARs that fail to
/// decode are `None` as well.
pub fn probe_bars<C: ConfigSpace + ?Sized>(cfg: &C, header: &Header) -> [Option<Bar>; 6] {
    let mut bars = [None; 6];
    let mut i = 0;
    while i < header.header_type.bar_count() {
        match probe_bar(cfg, header.bdf, header.header_type, i) {
            Ok(Some(bar)) => {
                bars[i] = Some(bar);
                i += bar.slots();
            }
            _ => i += 1,
        }
    }
    bars
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{Function, Sim};

    fn dev(f: Function) -> (Sim, Header) {
        let s = Sim::new();
        let b = Bdf::new(0, 4, 0);
        s.add(b, f);
        let h = Header::read(&s, b).unwrap();
        (s, h)
    }

    #[test]
    fn sizes_all_kinds() {
        let (s, h) = dev(Function::endpoint(1, 1, [1, 0, 0])
            .bar32(0, 0xfebf_0000, 0x1000, false, false)
            .bar32(1, 0xc000, 0x40, true, false)
            .bar64(2, 0x8_0000_0000, 0x4000, true)
            .bar64(4, 0x4_0000_0000, 0x2_0000_0000, false)
            .with16(reg::COMMAND, command::MEMORY_SPACE | command::IO_SPACE));
        let bars = probe_bars(&s, &h);
        assert_eq!(
            bars[0],
            Some(Bar { kind: BarKind::Memory32, address: 0xfebf_0000, size: 0x1000, prefetchable: false })
        );
        assert_eq!(bars[1], Some(Bar { kind: BarKind::Io, address: 0xc000, size: 0x40, prefetchable: false }));
        assert_eq!(
            bars[2],
            Some(Bar { kind: BarKind::Memory64, address: 0x8_0000_0000, size: 0x4000, prefetchable: true })
        );
        assert_eq!(bars[3], None);
        // A BAR larger than 4 GiB: every lower address bit reads back zero.
        assert_eq!(
            bars[4],
            Some(Bar { kind: BarKind::Memory64, address: 0x4_0000_0000, size: 0x2_0000_0000, prefetchable: false })
        );
        assert_eq!(bars[5], None);
    }

    #[test]
    fn probing_restores_everything() {
        let cmd = command::MEMORY_SPACE | command::IO_SPACE | command::BUS_MASTER;
        let (s, h) = dev(Function::endpoint(1, 1, [1, 0, 0])
            .bar32(0, 0xfebf_0000, 0x1000, false, false)
            .bar64(2, 0x8_0000_0000, 0x4000, true)
            .with16(reg::COMMAND, cmd));
        let before: [u32; 6] = core::array::from_fn(|i| s.get32(h.bdf, reg::BAR0 + 4 * i as u16));
        probe_bars(&s, &h);
        let after: [u32; 6] = core::array::from_fn(|i| s.get32(h.bdf, reg::BAR0 + 4 * i as u16));
        assert_eq!(before, after);
        assert_eq!(s.get16(h.bdf, reg::COMMAND), cmd);
        // Decode was off while the BARs held all ones (the simulator asserts
        // that on every such write) and the command register was written as
        // a 16-bit register.
        assert!(s.writes.borrow().iter().any(|w| w.1 == reg::COMMAND && w.2 == 2 && w.3 == 0x4));
    }

    #[test]
    fn unimplemented_reserved_and_truncated() {
        let (s, h) = dev(Function::endpoint(1, 1, [1, 0, 0])
            .with32(reg::BAR0 + 4, 0b010)
            .with32(reg::BAR0 + 20, 0b100)
            .writable32(reg::BAR0 + 20, 0xffff_f000));
        assert_eq!(probe_bar(&s, h.bdf, h.header_type, 0), Ok(None));
        assert_eq!(probe_bar(&s, h.bdf, h.header_type, 1), Err(BarError::ReservedType));
        assert_eq!(probe_bar(&s, h.bdf, h.header_type, 5), Err(BarError::Truncated));
        assert_eq!(probe_bar(&s, h.bdf, h.header_type, 6), Err(BarError::NoSuchBar));
        assert_eq!(probe_bars(&s, &h), [None; 6]);
    }

    #[test]
    fn bridge_has_two_bars() {
        let s = Sim::new();
        let b = Bdf::new(0, 1, 0);
        s.add(b, Function::bridge(0, 1, 1).bar32(0, 0xfe00_0000, 0x100, false, false));
        let h = Header::read(&s, b).unwrap();
        assert_eq!(probe_bar(&s, b, h.header_type, 2), Err(BarError::NoSuchBar));
        assert_eq!(probe_bars(&s, &h)[0].map(|b| b.size), Some(0x100));
    }

    #[test]
    fn contains_checks_overflow() {
        let bar = Bar { kind: BarKind::Memory32, address: 0, size: 0x1000, prefetchable: false };
        assert!(bar.contains(0, 0x1000));
        assert!(bar.contains(0xff0, 0x10));
        assert!(!bar.contains(0xff0, 0x11));
        assert!(!bar.contains(u64::MAX, 2));
        assert_eq!(bar.slots(), 1);
        assert!(bar.is_memory());
    }
}

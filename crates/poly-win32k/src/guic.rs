//! The third GUI slice's shared definitions (ADR 0172): DPI awareness
//! (`winuser.h` `DPI_AWARENESS_CONTEXT`, `windef.h` `DPI_AWARENESS`) and the
//! window state kept besides the first two slices'.

/// Poly's internal awareness values (one per context).
pub const AWARE_UNAWARE: u8 = 0;
pub const AWARE_SYSTEM: u8 = 1;
pub const AWARE_PER_MONITOR: u8 = 2;
pub const AWARE_PER_MONITOR_V2: u8 = 3;
pub const AWARE_GDI_SCALED: u8 = 4;

/// `ERROR_INVALID_STATE` (`winerror.h`): `SetParent` across DPI kinds.
pub const ERROR_INVALID_STATE: u32 = 5023;

/// Which windows may be parent and child: unaware and GDI-scaled together,
/// the per-monitor kinds together, system-aware alone.
pub fn kind(a: u8) -> u8 {
    match a {
        AWARE_UNAWARE | AWARE_GDI_SCALED => 0,
        AWARE_SYSTEM => 1,
        _ => 2,
    }
}

/// `DPI_AWARENESS` (`windef.h`) of an awareness.
pub fn dpi_awareness(a: u8) -> i32 {
    match a {
        AWARE_SYSTEM => 1,
        AWARE_PER_MONITOR | AWARE_PER_MONITOR_V2 => 2,
        _ => 0,
    }
}

/// The awareness a `DPI_AWARENESS_CONTEXT` value names: the pseudo handles
/// `-1..=-5` and the values the real handles have (`GetThreadDpiAwarenessContext.yaml`).
pub fn context_awareness(h: i64) -> Option<u8> {
    // Real handles carry flag bits above the context: bit 31 is ignored.
    let h = if !(-5..0).contains(&h) { i64::from((h as u32) & 0x7FFF_FFFF) } else { h };
    Some(match h {
        -1 | 0x6010 => AWARE_UNAWARE,
        -2 | 0x6011 => AWARE_SYSTEM,
        -3 | 0x12 => AWARE_PER_MONITOR,
        -4 | 0x22 => AWARE_PER_MONITOR_V2,
        -5 | 0x4000_6010 => AWARE_GDI_SCALED,
        _ => return None,
    })
}

/// The real handle value of an awareness.
pub fn context_handle(a: u8) -> i64 {
    match a {
        AWARE_SYSTEM => 0x6011,
        AWARE_PER_MONITOR => 0x12,
        AWARE_PER_MONITOR_V2 => 0x22,
        AWARE_GDI_SCALED => 0x4000_6010,
        _ => 0x6010,
    }
}

/// `WS_EX_LAYERED` (`winuser.h`).
pub const WS_EX_LAYERED: u32 = 0x0008_0000;

/// A window's layered attributes (`SetLayeredWindowAttributes.yaml`):
/// what the last call stored, whether any call has (`attrs`), and whether
/// `UpdateLayeredWindow` has been used (`update`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Layered {
    pub key: u32,
    pub alpha: u8,
    pub flags: u32,
    pub attrs: bool,
    pub update: bool,
}

/// The extended style a `SetWindowLong` leaves: the layered bit is refused
/// for a child window, and removing it forgets the attributes.
pub fn filter_ex_style(style: u32, old: u32, new: u32, lay: &mut Layered) -> u32 {
    let mut new = new;
    if style & 0x4000_0000 != 0 {
        new &= !WS_EX_LAYERED;
    }
    if (old ^ new) & WS_EX_LAYERED != 0 {
        *lay = Layered::default();
    }
    new
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contexts_map_both_ways() {
        for h in [-1i64, -2, -3, -4, -5] {
            let a = context_awareness(h).unwrap();
            assert_eq!(context_awareness(context_handle(a)), Some(a));
        }
        assert_eq!(context_awareness(0xFFFF_FFFF_8000_6010u64 as i64), Some(AWARE_UNAWARE));
        for bad in [0i64, 1, 2, 3, -6, -7, -16, 16, 0x6012, 0x6013, 0x1234, 0x7FFF_FFFF, 0x4000_6011] {
            assert_eq!(context_awareness(bad), None, "{bad:#x}");
        }
        assert_eq!(dpi_awareness(AWARE_PER_MONITOR_V2), 2);
        assert_eq!(kind(AWARE_GDI_SCALED), kind(AWARE_UNAWARE));
        assert_ne!(kind(AWARE_SYSTEM), kind(AWARE_PER_MONITOR));
    }
}

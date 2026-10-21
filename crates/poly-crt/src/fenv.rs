//! The floating-point control and status words of `_controlfp`,
//! `_controlfp_s`, `_control87`, `_clearfp` and `_statusfp` (`float.h`'s
//! bits) on x64, where they live in the SSE `MXCSR` register (Intel SDM
//! vol. 1, 10.2.3: flags in bits 0–5, DAZ in 6, masks in 7–12, rounding in
//! 13–14, FTZ in 15). The x87 precision and infinity controls (`_MCW_PC`,
//! `_MCW_IC`) do not exist on x64 (Learn: "_MCW_PC ... not supported on x64
//! platforms"): `_controlfp` ignores them, `_controlfp_s` refuses them.

pub const EM_INEXACT: u32 = 0x0000_0001;
pub const EM_UNDERFLOW: u32 = 0x0000_0002;
pub const EM_OVERFLOW: u32 = 0x0000_0004;
pub const EM_ZERODIVIDE: u32 = 0x0000_0008;
pub const EM_INVALID: u32 = 0x0000_0010;
pub const EM_DENORMAL: u32 = 0x0008_0000;
pub const MCW_EM: u32 = 0x0008_001F;
pub const RC_NEAR: u32 = 0x0000_0000;
pub const RC_DOWN: u32 = 0x0000_0100;
pub const RC_UP: u32 = 0x0000_0200;
pub const RC_CHOP: u32 = 0x0000_0300;
pub const MCW_RC: u32 = 0x0000_0300;
pub const DN_SAVE: u32 = 0x0000_0000;
pub const DN_FLUSH: u32 = 0x0100_0000;
pub const DN_FLUSH_RESULTS: u32 = 0x0200_0000;
pub const DN_FLUSH_OPERANDS: u32 = 0x0300_0000;
pub const MCW_DN: u32 = 0x0300_0000;
pub const MCW_PC: u32 = 0x0003_0000;
pub const MCW_IC: u32 = 0x0004_0000;

pub const SW_INEXACT: u32 = 0x0000_0001;
pub const SW_UNDERFLOW: u32 = 0x0000_0002;
pub const SW_OVERFLOW: u32 = 0x0000_0004;
pub const SW_ZERODIVIDE: u32 = 0x0000_0008;
pub const SW_INVALID: u32 = 0x0000_0010;
pub const SW_DENORMAL: u32 = 0x0008_0000;

/// `MXCSR` at reset (all exceptions masked, round to nearest).
pub const MXCSR_DEFAULT: u32 = 0x1F80;

/// `float.h` exception bits ↔ `MXCSR` bit positions (flag; the mask is 7
/// bits higher).
const EXCEPTIONS: [(u32, u32); 6] =
    [(EM_INVALID, 0), (EM_DENORMAL, 1), (EM_ZERODIVIDE, 2), (EM_OVERFLOW, 3), (EM_UNDERFLOW, 4), (EM_INEXACT, 5)];

/// The control word an `MXCSR` value stands for.
pub fn control(mxcsr: u32) -> u32 {
    let mut cw = 0;
    for (bit, pos) in EXCEPTIONS {
        if mxcsr & (1 << (pos + 7)) != 0 {
            cw |= bit;
        }
    }
    cw |= match (mxcsr >> 13) & 3 {
        0 => RC_NEAR,
        1 => RC_DOWN,
        2 => RC_UP,
        _ => RC_CHOP,
    };
    cw |= match (mxcsr & (1 << 15) != 0, mxcsr & (1 << 6) != 0) {
        (false, false) => DN_SAVE,
        (true, true) => DN_FLUSH,
        (true, false) => DN_FLUSH_RESULTS,
        (false, true) => DN_FLUSH_OPERANDS,
    };
    cw
}

/// `MXCSR` after `_controlfp(new, mask)`: the bits of `mask` taken from
/// `new` (the x87-only fields ignored).
pub fn apply(mxcsr: u32, new: u32, mask: u32) -> u32 {
    let mask = mask & (MCW_EM | MCW_RC | MCW_DN);
    let cw = (control(mxcsr) & !mask) | (new & mask);
    let mut m = mxcsr & !(0x3F << 7) & !(3 << 13) & !(1 << 15) & !(1 << 6);
    for (bit, pos) in EXCEPTIONS {
        if cw & bit != 0 {
            m |= 1 << (pos + 7);
        }
    }
    m |= match cw & MCW_RC {
        RC_NEAR => 0,
        RC_DOWN => 1 << 13,
        RC_UP => 2 << 13,
        _ => 3 << 13,
    };
    m |= match cw & MCW_DN {
        DN_SAVE => 0,
        DN_FLUSH => (1 << 15) | (1 << 6),
        DN_FLUSH_RESULTS => 1 << 15,
        _ => 1 << 6,
    };
    m
}

/// `_statusfp`: the sticky flags.
pub fn status(mxcsr: u32) -> u32 {
    let mut sw = 0;
    for (bit, pos) in EXCEPTIONS {
        if mxcsr & (1 << pos) != 0 {
            // The SW_ bits equal the EM_ bits.
            sw |= bit;
        }
    }
    sw
}

/// `_clearfp`: `MXCSR` without its flags.
pub fn clear(mxcsr: u32) -> u32 {
    mxcsr & !0x3F
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_and_changes() {
        assert_eq!(control(MXCSR_DEFAULT), MCW_EM);
        let m = apply(MXCSR_DEFAULT, RC_CHOP, MCW_RC);
        assert_eq!(m, MXCSR_DEFAULT | (3 << 13));
        assert_eq!(control(m), MCW_EM | RC_CHOP);
        let m = apply(MXCSR_DEFAULT, 0, EM_ZERODIVIDE);
        assert_eq!(m & (1 << 9), 0);
        assert_eq!(control(m), MCW_EM & !EM_ZERODIVIDE);
        let m = apply(MXCSR_DEFAULT, DN_FLUSH, MCW_DN);
        assert_eq!(control(m) & MCW_DN, DN_FLUSH);
        assert_eq!(m & 0x8040, 0x8040);
        assert_eq!(apply(MXCSR_DEFAULT, 0, MCW_PC), MXCSR_DEFAULT, "x87 precision is ignored");
        let flagged = MXCSR_DEFAULT | 0b100100;
        assert_eq!(status(flagged), SW_ZERODIVIDE | SW_INEXACT);
        assert_eq!(clear(flagged), MXCSR_DEFAULT);
        assert_eq!(status(MXCSR_DEFAULT | 2), SW_DENORMAL);
        assert_eq!(apply(flagged, RC_UP, MCW_RC) & 0x3F, 0b100100, "flags survive");
        let _ = (SW_INVALID, SW_OVERFLOW, SW_UNDERFLOW, MCW_IC, RC_DOWN);
    }
}

//! `AlphaBlend`'s arithmetic (`specs/windows/msimg32.dll/AlphaBlend.yaml`): the blend of one byte
//! with a constant alpha, the blend of a premultiplied pixel, the checks of the
//! `BLENDFUNCTION`, and which source column or row a destination one takes when the rectangles
//! differ in size.

/// `AC_SRC_OVER`, `AC_SRC_ALPHA` (`wingdi.h`).
pub const AC_SRC_OVER: u8 = 0;
pub const AC_SRC_ALPHA: u8 = 1;

/// A `BLENDFUNCTION` passed by value: `BlendOp`, `BlendFlags`, `SourceConstantAlpha`,
/// `AlphaFormat` in its four bytes, lowest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlendFunction {
    pub op: u8,
    pub flags: u8,
    pub constant: u8,
    pub format: u8,
}

impl BlendFunction {
    pub fn from_u32(v: u32) -> BlendFunction {
        let b = v.to_le_bytes();
        BlendFunction { op: b[0], flags: b[1], constant: b[2], format: b[3] }
    }

    /// Whether the call takes it (`errors` `blendfunction`): `AC_SRC_OVER`, the flags 0, 1, 128
    /// or 129, the format 0 or `AC_SRC_ALPHA`.
    pub fn valid(&self) -> bool {
        self.op == AC_SRC_OVER && matches!(self.flags, 0 | 1 | 128 | 129) && self.format <= AC_SRC_ALPHA
    }
}

/// `(x * y + 127) / 255` for bytes.
fn mul(x: u32, y: u32) -> u32 {
    (x * y + 127) / 255
}

/// `constant_alpha`: byte `s` over byte `d` at constant alpha `a`.
pub fn blend_constant(s: u8, d: u8, a: u8) -> u8 {
    let (s, d, a) = (u32::from(s), u32::from(d), u32::from(a));
    ((s * a + d * (255 - a) + 127) / 255) as u8
}

/// `premultiplied`: the colour bytes `s` (with the source alpha byte `sa`) over the destination's
/// `d` (with its alpha byte `da`), scaled by the constant alpha `a`; the colours and the alpha
/// byte of the result.
///
/// The scaled source and the scaled destination are added as one 32-bit pixel (blue, green, red,
/// alpha from the lowest byte up): a byte whose sum passes 255 keeps the low eight bits and
/// carries one into the next byte. For premultiplied pixels no sum passes 255; for a colour byte
/// above its alpha byte this gives what the sheet's `non_premultiplied` observed: a sum of 355
/// or 256 reads 100 or 1 when the byte below also passed 255 (the sum less 255), and the alpha
/// byte one more than the formula (`s = 255, sa = 1, a = 255`: alpha 2).
pub fn blend_premultiplied(s: [u8; 3], sa: u8, d: [u8; 3], da: u8, a: u8) -> ([u8; 3], u8) {
    // A pixel of alpha 0 leaves the destination alone, whatever its colour.
    if sa == 0 {
        return (d, da);
    }
    let a32 = u32::from(a);
    let a2 = if sa == 255 { a32 } else { mul(u32::from(sa), a32) };
    let mut sums = [0u32; 4];
    for i in 0..3 {
        let (sv, dv) = (u32::from(s[i]), u32::from(d[i]));
        sums[i] = if sa == 255 { (sv * a32 + dv * (255 - a32) + 127) / 255 } else { mul(sv, a32) + mul(dv, 255 - a2) };
    }
    sums[3] = a2 + mul(u32::from(da), 255 - a2);
    let mut out = [0u8; 4];
    let mut carry = 0;
    for i in 0..4 {
        let v = sums[i] + carry;
        out[i] = (v & 0xFF) as u8;
        carry = v >> 8;
    }
    ([out[0], out[1], out[2]], out[3])
}

/// `stretching`: the source column (or row) destination column `dx` takes when `sw` source
/// pixels are drawn over `dw` destination ones.
pub fn source_index(dx: u32, sw: u32, dw: u32) -> u32 {
    let k = if dw == sw || dw + 1 == sw {
        0
    } else if dw == sw + 1 {
        sw - 1
    } else {
        sw / 2
    };
    ((u64::from(dx) * u64::from(sw) + u64::from(k)) / u64::from(dw)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    use std::vec::Vec;

    #[test]
    fn constant_alpha_examples() {
        assert_eq!(blend_constant(200, 50, 0), 50);
        assert_eq!(blend_constant(200, 50, 255), 200);
        // Red 255 at 128 over black, again and again.
        let mut d = 0u8;
        let seen: Vec<u8> = (0..6)
            .map(|_| {
                d = blend_constant(255, d, 128);
                d
            })
            .collect();
        assert_eq!(seen, [128, 192, 224, 240, 248, 252]);
    }

    #[test]
    fn premultiplied_examples() {
        // Opaque source pixel: the constant-alpha rule, alpha = a + da*(255-a)/255.
        assert_eq!(blend_premultiplied([10, 20, 30], 255, [100, 100, 100], 50, 255), ([10, 20, 30], 255));
        // A transparent black pixel leaves the destination.
        assert_eq!(blend_premultiplied([0; 3], 0, [37, 37, 37], 77, 200), ([37, 37, 37], 77));
        // non_premultiplied: white of alpha 1 over 100: blue 355 keeps 99 and carries, so green
        // and red read 100 and the alpha byte 2 (one more than the formula's 1).
        assert_eq!(blend_premultiplied([255, 255, 255], 1, [100, 100, 100], 0, 255), ([99, 100, 100], 2));
        // 256 gives 1 above a byte that carried.
        assert_eq!(blend_premultiplied([2, 2, 2], 1, [255, 255, 255], 0, 255), ([0, 1, 1], 2));
        // Half-transparent white at full constant alpha over black.
        let (c, a) = blend_premultiplied([128, 128, 128], 128, [0, 0, 0], 0, 255);
        assert_eq!((c, a), ([128, 128, 128], 128));
    }

    #[test]
    fn stretching_examples() {
        let cols = |sw: u32, dw: u32| (0..dw).map(|d| source_index(d, sw, dw)).collect::<Vec<_>>();
        assert_eq!(cols(4, 8), [0, 0, 1, 1, 2, 2, 3, 3]);
        assert_eq!(cols(4, 2), [1, 3]);
        assert_eq!(cols(4, 3), [0, 1, 2]);
        assert_eq!(cols(4, 5), [0, 1, 2, 3, 3]);
        assert_eq!(cols(4, 6), [0, 1, 1, 2, 3, 3]);
        assert_eq!(cols(5, 13), [0, 0, 0, 1, 1, 2, 2, 2, 3, 3, 4, 4, 4]);
    }

    #[test]
    fn blend_function_checks() {
        assert!(BlendFunction::from_u32(0x01FF_0000).valid());
        assert!(BlendFunction::from_u32(0x0080_8100).valid());
        assert!(!BlendFunction::from_u32(0x0080_0001).valid());
        assert!(!BlendFunction::from_u32(0x0280_0000).valid());
        assert!(!BlendFunction::from_u32(0x0080_0200).valid());
    }
}

//! What a clear stores (ID3D11DeviceContext.CopyResource.yaml `clear`,
//! ID3D11DeviceContext.ClearDepthStencilView.yaml).

use crate::format;

/// A float colour channel as UNORM8: clamped to [0, 1], NaN as 0, rounded to nearest.
pub fn unorm8(v: f32) -> u8 {
    let v = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
    round_half_even((v * 255.0) as f64) as u8
}

/// Round to nearest, ties to even, of a product computed in single precision (as the device does:
/// 0.3 * 0xFFFFFF stores 0x4CCCCC).
fn round_half_even(x: f64) -> u64 {
    let f = x as u64;
    let d = x - f as f64;
    if d > 0.5 || (d == 0.5 && f % 2 == 1) { f + 1 } else { f }
}

/// The linear value `v` (0..1) as an sRGB-encoded UNORM8 (Learn, "Gamma correction"; IEC 61966-2-1).
pub fn srgb8(v: f32) -> u8 {
    let v = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) } as f64;
    let e = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * libm_pow(v, 1.0 / 2.4) - 0.055 };
    round_half_even(e * 255.0) as u8
}

/// sRGB-encoded UNORM8 to linear.
pub fn srgb_to_linear(c: u8) -> f64 {
    let c = c as f64 / 255.0;
    if c <= 0.040_45 { c / 12.92 } else { libm_pow((c + 0.055) / 1.055, 2.4) }
}

/// `x^y` for x > 0 by exp(y ln x), in core only.
fn libm_pow(x: f64, y: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    exp(y * ln(x))
}

fn ln(x: f64) -> f64 {
    // x = m * 2^e with m in [1, 2): ln x = e ln 2 + ln m, ln m by atanh series.
    let bits = x.to_bits();
    let e = ((bits >> 52) & 0x7ff) as i64 - 1023;
    let m = f64::from_bits((bits & !(0x7ff << 52)) | (1023 << 52));
    let t = (m - 1.0) / (m + 1.0);
    let t2 = t * t;
    let mut sum = 0.0;
    let mut term = t;
    let mut k = 1.0;
    for _ in 0..30 {
        sum += term / k;
        term *= t2;
        k += 2.0;
    }
    e as f64 * core::f64::consts::LN_2 + 2.0 * sum
}

fn floor(x: f64) -> f64 {
    let t = x as i64 as f64;
    if t > x { t - 1.0 } else { t }
}

fn exp(x: f64) -> f64 {
    // e^x = 2^k e^r, |r| <= ln2/2.
    let k = floor(x / core::f64::consts::LN_2 + 0.5);
    let r = x - k * core::f64::consts::LN_2;
    let mut sum = 1.0;
    let mut term = 1.0;
    for n in 1..30 {
        term *= r / n as f64;
        sum += term;
    }
    sum * f64::from_bits(((k as i64 + 1023) as u64) << 52)
}

/// The bytes one texel of `format` holds after `ClearRenderTargetView` with `rgba`, for the 8-bit
/// colour formats (R8G8B8A8 / B8G8R8A8 / B8G8R8X8 and their sRGB forms); `None` for others.
pub fn color_texel(format: u32, rgba: [f32; 4]) -> Option<alloc::vec::Vec<u8>> {
    let enc = |v: f32| if format::is_srgb(format) { srgb8(v) } else { unorm8(v) };
    let (r, g, b) = (enc(rgba[0]), enc(rgba[1]), enc(rgba[2]));
    let a = unorm8(rgba[3]);
    Some(match format {
        28..=32 => alloc::vec![r, g, b, a],
        87 | 91 => alloc::vec![b, g, r, a],
        88 | 93 => alloc::vec![b, g, r, 0xFF],
        2 => [rgba[0], rgba[1], rgba[2], rgba[3]].iter().flat_map(|v| v.to_le_bytes()).collect(),
        10 => [rgba[0], rgba[1], rgba[2], rgba[3]].iter().flat_map(|v| f16(*v).to_le_bytes()).collect(),
        61 => alloc::vec![r],
        65 => alloc::vec![a],
        _ => return None,
    })
}

/// A float as IEEE half (round to nearest even), for R16G16B16A16_FLOAT clears.
pub fn f16(v: f32) -> u16 {
    let b = v.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xff) as i32;
    let man = b & 0x7f_ffff;
    if exp == 0xff {
        return sign | 0x7c00 | if man != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 0x1f {
        return sign | 0x7c00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        let m = (man | 0x80_0000) >> (1 - e);
        let half = m >> 13;
        let rest = m & 0x1fff;
        let r = if rest > 0x1000 || (rest == 0x1000 && half & 1 == 1) { half + 1 } else { half };
        return sign | r as u16;
    }
    let half = (e as u32) << 10 | man >> 13;
    let rest = man & 0x1fff;
    let r = if rest > 0x1000 || (rest == 0x1000 && half & 1 == 1) { half + 1 } else { half };
    sign | r as u16
}

/// A depth value as the format stores it: values outside [0, 1] and NaN clear to 1.0
/// (ClearDepthStencilView.yaml `d24_s8`, `d32`, `d16`); UNORM with round half to even.
pub fn depth_value(depth: f32) -> f32 {
    if depth.is_nan() || !(0.0..=1.0).contains(&depth) { 1.0 } else { depth }
}

/// The 24-bit UNORM of a depth value.
pub fn depth24(depth: f32) -> u32 {
    round_half_even((depth_value(depth) * 16_777_215.0) as f64) as u32
}

/// The 16-bit UNORM of a depth value.
pub fn depth16(depth: f32) -> u16 {
    round_half_even((depth_value(depth) * 65535.0) as f64) as u16
}

/// `D3D11_CLEAR_FLAG` (d3d11.h).
pub const CLEAR_DEPTH: u32 = 1;
pub const CLEAR_STENCIL: u32 = 2;

/// Clear one texel of depth `format` in place (`texel` is its bytes) with `flags`, `depth`,
/// `stencil`: the planes the flags do not name are kept.
pub fn clear_depth_texel(format: u32, texel: &mut [u8], flags: u32, depth: f32, stencil: u8) {
    let d = flags & CLEAR_DEPTH != 0;
    let s = flags & CLEAR_STENCIL != 0;
    match format {
        45 | 44 => {
            let mut v = u32::from_le_bytes([texel[0], texel[1], texel[2], texel[3]]);
            if d {
                v = (v & 0xFF00_0000) | depth24(depth);
            }
            if s {
                v = (v & 0x00FF_FFFF) | (stencil as u32) << 24;
            }
            texel[..4].copy_from_slice(&v.to_le_bytes());
        }
        40 | 39 => {
            if d {
                texel[..4].copy_from_slice(&depth_value(depth).to_bits().to_le_bytes());
            }
        }
        55 | 53 => {
            if d {
                texel[..2].copy_from_slice(&depth16(depth).to_le_bytes());
            }
        }
        20 | 19 => {
            if d {
                texel[..4].copy_from_slice(&depth_value(depth).to_bits().to_le_bytes());
            }
            if s {
                texel[4..8].copy_from_slice(&(stencil as u32).to_le_bytes());
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bgra(rgba: [f32; 4]) -> u32 {
        let t = color_texel(87, rgba).unwrap();
        u32::from_le_bytes([t[0], t[1], t[2], t[3]])
    }

    #[test]
    fn colour_clears_of_the_sheet() {
        assert_eq!(bgra([0.25, 0.5, 0.75, 1.0]), 0xFF4080BF);
        assert_eq!(bgra([0.2, 0.4, 0.6, 0.8]), 0xCC336699);
        assert_eq!(bgra([2.0, -1.0, 1.5, 0.5]), 0x80FF00FF);
        assert_eq!(bgra([f32::NAN, 0.0, 0.0, 1.0]), 0xFF000000);
        assert_eq!(bgra([0.001, 0.002, 0.003, 0.004]), 0x01000101);
        assert_eq!(bgra([0.5 / 255.0, 1.5 / 255.0, 2.5 / 255.0, 127.5 / 255.0]), 0x80000202);
        let t = color_texel(28, [1.0, 0.5, 0.25, 0.75]).unwrap();
        assert_eq!(t, [0xFF, 0x80, 0x40, 0xBF]);
    }

    #[test]
    fn depth_clears_of_the_sheet() {
        assert_eq!(depth24(0.25), 0x400000);
        assert_eq!(depth24(0.3), 0x4CCCCC);
        assert_eq!(depth24(0.999), 0xFFBE76);
        assert_eq!(depth24(1e-7), 2);
        assert_eq!(depth24(0.5), 0x800000);
        for v in [2.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(depth24(v), 0xFFFFFF);
        }
        assert_eq!(depth16(0.5), 0x8000);
        assert_eq!(depth16(0.3), 0x4CCC);
        assert_eq!(depth16(0.999), 0xFFBD);
        let mut t = [0u8; 4];
        clear_depth_texel(45, &mut t, 3, 0.0, 0xFF);
        assert_eq!(u32::from_le_bytes(t), 0xFF000000);
        clear_depth_texel(45, &mut t, 1, 0.5, 7);
        assert_eq!(u32::from_le_bytes(t), 0xFF800000);
        clear_depth_texel(45, &mut t, 2, 0.25, 0x55);
        assert_eq!(u32::from_le_bytes(t), 0x55800000);
    }

    #[test]
    fn srgb() {
        assert_eq!(srgb8(0.5), 0xBC);
        assert_eq!(srgb8(1.0), 0xFF);
        assert!((srgb_to_linear(0xBC) - 0.5).abs() < 0.01);
        assert_eq!(f16(1.0), 0x3C00);
        assert_eq!(f16(0.5), 0x3800);
    }
}

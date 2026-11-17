//! State objects (ID3D11Device.CreateBlendState / CreateDepthStencilState / CreateRasterizerState /
//! CreateSamplerState .yaml): which descriptions are valid and the normalised description that
//! `GetDesc` returns. Two descriptions that normalise equal are the same object (the DLL caches by
//! the normalised words).

use crate::hr::*;

/// The most distinct blend, depth-stencil and rasterizer objects alive at once (4096 for samplers).
pub const MAX_STATES: usize = 4095;
pub const MAX_SAMPLERS: usize = 4096;

/// `D3D11_BLEND_DESC1` as words: AlphaToCoverage, Independent, then 8 targets of 10 words
/// (BlendEnable, LogicOpEnable, Src, Dest, Op, SrcAlpha, DestAlpha, OpAlpha, LogicOp, WriteMask).
pub type Blend1 = [u32; 82];

/// The default blend description (Learn, "D3D11_BLEND_DESC": blending off, ONE / ZERO / ADD,
/// write mask ALL, logic op NOOP).
pub fn default_blend() -> Blend1 {
    let mut b = [0u32; 82];
    for t in 0..8 {
        b[2 + t * 10..12 + t * 10].copy_from_slice(&[0, 0, 2, 1, 1, 2, 1, 1, 4, 0xF]);
    }
    b
}

/// `D3D11_BLEND_DESC` (2 + 8 x 8 words) as a `Blend1` (no logic op).
pub fn blend_from_desc(d: &[u32; 66]) -> Blend1 {
    let mut b = [0u32; 82];
    b[0] = d[0];
    b[1] = d[1];
    for t in 0..8 {
        let s = &d[2 + t * 8..10 + t * 8];
        b[2 + t * 10..12 + t * 10].copy_from_slice(&[s[0], 0, s[1], s[2], s[3], s[4], s[5], s[6], 4, s[7]]);
    }
    b
}

/// A `Blend1` as `D3D11_BLEND_DESC`.
pub fn blend_to_desc(b: &Blend1) -> [u32; 66] {
    let mut d = [0u32; 66];
    d[0] = b[0];
    d[1] = b[1];
    for t in 0..8 {
        let s = &b[2 + t * 10..12 + t * 10];
        d[2 + t * 8..10 + t * 8].copy_from_slice(&[s[0], s[2], s[3], s[4], s[5], s[6], s[7], s[9]]);
    }
    d
}

fn blend_factor_ok(f: u32, alpha: bool) -> bool {
    match f {
        1..=11 | 14..=19 => !(alpha && matches!(f, 3 | 4 | 9 | 10 | 16 | 17)),
        _ => false,
    }
}

/// Check and normalise a blend description.
pub fn blend(d: &Blend1) -> Result<Blend1, HRESULT> {
    let bad = Err(E_INVALIDARG);
    let mut n = [0u32; 82];
    n[0] = u32::from(d[0] != 0);
    n[1] = u32::from(d[1] != 0);
    let independent = n[1] != 0;
    let targets = if independent { 8 } else { 1 };
    for t in 0..targets {
        let s = &d[2 + t * 10..12 + t * 10];
        let enable = s[0] != 0;
        let logic = s[1] != 0;
        let mask = s[9] & 0xFF;
        if mask > 0xF {
            return bad;
        }
        if logic && (enable || independent || s[8] > 15) {
            return bad;
        }
        let o = &mut n[2 + t * 10..12 + t * 10];
        if enable {
            let ops_ok = (1..=5).contains(&s[4]) && (1..=5).contains(&s[7]);
            if !blend_factor_ok(s[2], false)
                || !blend_factor_ok(s[3], false)
                || !blend_factor_ok(s[5], true)
                || !blend_factor_ok(s[6], true)
                || !ops_ok
            {
                return bad;
            }
            o.copy_from_slice(&[1, 0, s[2], s[3], s[4], s[5], s[6], s[7], 4, mask]);
        } else {
            o.copy_from_slice(&[0, u32::from(logic), 2, 1, 1, 2, 1, 1, if logic { s[8] } else { 4 }, mask]);
        }
    }
    if !independent {
        let first: [u32; 10] = n[2..12].try_into().expect("ten words");
        for t in 1..8 {
            n[2 + t * 10..12 + t * 10].copy_from_slice(&first);
        }
    }
    Ok(n)
}

/// `D3D11_DEPTH_STENCIL_DESC` as 13 words: DepthEnable, DepthWriteMask, DepthFunc, StencilEnable,
/// the two UINT8 masks in one word (read low byte, write next), FrontFace (4), BackFace (4).
pub type Depth = [u32; 13];

pub fn depth(d: &Depth) -> Result<Depth, HRESULT> {
    let bad = Err(E_INVALIDARG);
    let mut n = *d;
    n[0] = u32::from(d[0] != 0);
    n[3] = u32::from(d[3] != 0);
    if n[0] != 0 {
        if d[1] > 1 || !(1..=8).contains(&d[2]) {
            return bad;
        }
    } else {
        n[1] = 1;
        n[2] = 2;
    }
    if n[3] != 0 {
        if d[5..13].iter().any(|v| !(1..=8).contains(v)) {
            return bad;
        }
        n[4] = d[4] & 0xFFFF;
    } else {
        n[4] = 0xFFFF;
        n[5..13].copy_from_slice(&[1, 1, 1, 8, 1, 1, 1, 8]);
    }
    Ok(n)
}

/// `D3D11_RASTERIZER_DESC2` as 12 words: FillMode, CullMode, FrontCounterClockwise, DepthBias,
/// DepthBiasClamp, SlopeScaledDepthBias, DepthClipEnable, ScissorEnable, MultisampleEnable,
/// AntialiasedLineEnable, ForcedSampleCount, ConservativeRaster.
pub type Raster = [u32; 12];

pub fn raster(d: &Raster) -> Result<Raster, HRESULT> {
    let bad = Err(E_INVALIDARG);
    if !(2..=3).contains(&d[0]) || !(1..=3).contains(&d[1]) {
        return bad;
    }
    if f32::from_bits(d[4]).is_nan() || f32::from_bits(d[5]).is_nan() {
        return bad;
    }
    if !matches!(d[10], 0 | 1 | 4 | 8) || d[11] > 1 {
        return bad;
    }
    let mut n = *d;
    for i in [2, 6, 7, 8, 9] {
        n[i] = u32::from(d[i] != 0);
    }
    Ok(n)
}

/// `D3D11_SAMPLER_DESC` as 13 words: Filter, AddressU, V, W, MipLODBias, MaxAnisotropy,
/// ComparisonFunc, BorderColor (4), MinLOD, MaxLOD.
pub type Sampler = [u32; 13];

fn filter_ok(f: u32) -> bool {
    matches!(f & !0x180, 0 | 1 | 4 | 5 | 0x10 | 0x11 | 0x14 | 0x15 | 0x55) && f <= 0x1FF
}

pub fn sampler(d: &Sampler) -> Result<Sampler, HRESULT> {
    let bad = Err(E_INVALIDARG);
    if !filter_ok(d[0]) || d[1..4].iter().any(|a| !(1..=5).contains(a)) {
        return bad;
    }
    let bias = f32::from_bits(d[4]);
    if bias.is_nan() || !(-16.0..16.0).contains(&bias) {
        return bad;
    }
    if d[5] > 16 {
        return bad;
    }
    let comparison = (d[0] >> 7) & 3 == 1;
    if comparison && !(1..=8).contains(&d[6]) {
        return bad;
    }
    if f32::from_bits(d[11]).is_nan() || f32::from_bits(d[12]).is_nan() {
        return bad;
    }
    let mut n = *d;
    if !comparison {
        n[6] = 1;
    }
    if d[0] & 0x40 == 0 {
        n[5] = 0;
    }
    if !d[1..4].contains(&4) {
        n[7..11].copy_from_slice(&[0; 4]);
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_normalisation() {
        let d = default_blend();
        assert_eq!(blend(&d), Ok(d));
        let mut b = d;
        b[0] = 2;
        b[2] = 7;
        b[4] = 99; // SrcBlend invalid but blending off on target 0? no: enabled
        assert_eq!(blend(&b), Err(E_INVALIDARG));
        let mut b = d;
        b[4] = 99; // SrcBlend 99 with BlendEnable FALSE
        assert_eq!(blend(&b), Ok(d));
        let mut b = d;
        b[2 + 5 * 10] = 1; // blend on target 5, independent off: the default
        assert_eq!(blend(&b), Ok(d));
        let mut b = d;
        b[11] = 0x10;
        assert_eq!(blend(&b), Err(E_INVALIDARG));
        b[11] = 0x100;
        assert_eq!(blend(&b).unwrap()[11], 0);
        let mut b = d;
        b[3] = 1;
        b[10] = 16;
        assert_eq!(blend(&b), Err(E_INVALIDARG));
        b[10] = 3;
        assert_eq!(blend(&b).unwrap()[23], 1);
        let mut b = d;
        b[2] = 1;
        b[5] = 16;
        b[7] = 3;
        assert_eq!(blend(&b), Err(E_INVALIDARG));
    }

    #[test]
    fn depth_normalisation() {
        let def: Depth = [1, 1, 2, 0, 0xFFFF, 1, 1, 1, 8, 1, 1, 1, 8];
        assert_eq!(depth(&def), Ok(def));
        let mut d = def;
        d[0] = 0;
        d[1] = 0;
        d[2] = 99;
        assert_eq!(depth(&d).unwrap()[1..3], [1, 2]);
        let mut d = def;
        d[2] = 9;
        assert_eq!(depth(&d), Err(E_INVALIDARG));
        let mut d = def;
        d[3] = 7;
        d[4] = 0xF00F;
        d[5] = 9;
        assert_eq!(depth(&d), Err(E_INVALIDARG));
        let mut d = def;
        d[4] = 0;
        d[5..13].copy_from_slice(&[3; 8]);
        assert_eq!(depth(&d).unwrap(), def);
    }

    #[test]
    fn raster_and_sampler() {
        let r: Raster = [3, 3, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0];
        assert!(raster(&r).is_ok());
        let mut x = r;
        x[0] = 1;
        assert_eq!(raster(&x), Err(E_INVALIDARG));
        let mut x = r;
        x[2] = 5;
        assert_eq!(raster(&x).unwrap()[2], 1);
        x[4] = f32::NAN.to_bits();
        assert_eq!(raster(&x), Err(E_INVALIDARG));
        let s: Sampler = [0x15, 3, 3, 3, 0, 1, 0, 0, 0, 0, 0, 0xFF7F_FFFF, 0x7F7F_FFFF];
        let n = sampler(&s).unwrap();
        assert_eq!((n[5], n[6]), (0, 1));
        for bad in [2, 3, 0x40, 0x56, 0x1FF, 0x200, u32::MAX] {
            let mut x = s;
            x[0] = bad;
            assert_eq!(sampler(&x), Err(E_INVALIDARG), "{bad:#x}");
        }
        let mut x = s;
        x[4] = 16.0f32.to_bits();
        assert_eq!(sampler(&x), Err(E_INVALIDARG));
        x[4] = (-16.0f32).to_bits();
        assert!(sampler(&x).is_ok());
        let mut x = s;
        x[0] = 0x95;
        x[6] = 9;
        assert_eq!(sampler(&x), Err(E_INVALIDARG));
    }
}

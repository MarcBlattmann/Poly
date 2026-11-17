//! Swap chain descriptions (IDXGIFactory2.CreateSwapChainForHwnd.yaml,
//! IDXGIFactory2.CreateSwapChainForComposition.yaml): which `DXGI_SWAP_CHAIN_DESC1` a window or a
//! composition swap chain accepts.

use crate::hr::*;

/// `DXGI_SWAP_CHAIN_DESC1` as 12 words: Width, Height, Format, Stereo, SampleDesc (Count,
/// Quality), BufferUsage, BufferCount, Scaling, SwapEffect, AlphaMode, Flags.
pub type Desc1 = [u32; 12];

/// `DXGI_SWAP_EFFECT` (dxgi.h).
pub mod effect {
    pub const DISCARD: u32 = 0;
    pub const SEQUENTIAL: u32 = 1;
    pub const FLIP_SEQUENTIAL: u32 = 3;
    pub const FLIP_DISCARD: u32 = 4;
}

pub fn is_flip(effect: u32) -> bool {
    effect == effect::FLIP_SEQUENTIAL || effect == effect::FLIP_DISCARD
}

/// Check a description for a window (`composition` false) or for DirectComposition.
pub fn check(d: &Desc1, composition: bool) -> Result<(), HRESULT> {
    let call = Err(DXGI_ERROR_INVALID_CALL);
    let arg = Err(E_INVALIDARG);
    let [w, h, format, stereo, count, quality, usage, buffers, scaling, effect, alpha, flags] = *d;
    let flip = is_flip(effect);
    if !matches!(effect, 0 | 1 | 3 | 4) || composition && !flip {
        return call;
    }
    let (lo, hi) = if flip { (2, 16) } else { (1, 16) };
    if !(lo..=hi).contains(&buffers) {
        return call;
    }
    if w == u32::MAX || h == u32::MAX {
        return arg;
    }
    if composition && (w == 0 || h == 0) {
        return call;
    }
    let flip_formats = [10, 24, 28, 87];
    if flip {
        if !flip_formats.contains(&format) {
            return call;
        }
    } else if !matches!(format, 10 | 24 | 28 | 29 | 87 | 89 | 91) {
        return arg;
    }
    let scaling_ok = if flip && !composition { scaling <= 1 } else { scaling == 0 };
    if !scaling_ok {
        return call;
    }
    let alpha_ok = if composition { matches!(alpha, 0 | 1 | 3) } else { matches!(alpha, 0 | 3) };
    if !alpha_ok {
        return call;
    }
    if flip {
        if (count, quality) != (1, 0) {
            return call;
        }
    } else if !matches!(count, 1 | 2 | 4 | 8 | 16) || quality != 0 {
        return arg;
    }
    if stereo != 0 && !flip {
        return call;
    }
    if flip {
        if !matches!(usage, 0 | 0x10 | 0x20 | 0x30 | 0x60) {
            return call;
        }
    } else if usage == u32::MAX {
        return arg;
    }
    // DXGI_SWAP_CHAIN_FLAG (dxgi.h): the ones a window or composition swap chain takes.
    let mut allowed = 0x2 | 0x4 | 0x8 | 0x20 | 0x1000 | 0x2000;
    if flip {
        allowed |= 0x40 | 0x800;
    } else {
        allowed |= 0x10;
    }
    if flip && flags & 0x10 != 0 {
        return arg;
    }
    if flags & !allowed != 0 {
        return call;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(effect: u32, buffers: u32) -> Desc1 {
        [64, 32, 87, 0, 1, 0, 0x20, buffers, 0, effect, 0, 0]
    }

    #[test]
    fn window_swap_chains() {
        assert_eq!(check(&d(4, 2), false), Ok(()));
        assert_eq!(check(&d(0, 1), false), Ok(()));
        assert_eq!(check(&d(4, 1), false), Err(DXGI_ERROR_INVALID_CALL));
        assert_eq!(check(&d(0, 17), false), Err(DXGI_ERROR_INVALID_CALL));
        assert_eq!(check(&d(2, 2), false), Err(DXGI_ERROR_INVALID_CALL));
        let mut x = d(0, 1);
        x[2] = 88;
        assert_eq!(check(&x, false), Err(E_INVALIDARG));
        let mut x = d(4, 2);
        x[2] = 29;
        assert_eq!(check(&x, false), Err(DXGI_ERROR_INVALID_CALL));
        let mut x = d(4, 2);
        x[10] = 1;
        assert_eq!(check(&x, false), Err(DXGI_ERROR_INVALID_CALL));
        let mut x = d(0, 1);
        x[4] = 3;
        assert_eq!(check(&x, false), Err(E_INVALIDARG));
        let mut x = d(4, 2);
        x[11] = 0x40 | 0x800 | 2;
        assert_eq!(check(&x, false), Ok(()));
        x[11] = 0x10;
        assert_eq!(check(&x, false), Err(E_INVALIDARG));
        let mut x = d(0, 1);
        x[11] = 0x40;
        assert_eq!(check(&x, false), Err(DXGI_ERROR_INVALID_CALL));
        x[11] = 0x2000 | 0x10;
        assert_eq!(check(&x, false), Ok(()));
        let mut x = d(0, 1);
        x[0] = u32::MAX;
        assert_eq!(check(&x, false), Err(E_INVALIDARG));
    }

    #[test]
    fn composition_swap_chains() {
        assert_eq!(check(&d(4, 2), true), Ok(()));
        assert_eq!(check(&d(0, 2), true), Err(DXGI_ERROR_INVALID_CALL));
        let mut x = d(4, 2);
        x[0] = 0;
        assert_eq!(check(&x, true), Err(DXGI_ERROR_INVALID_CALL));
        x = d(4, 2);
        x[10] = 1;
        assert_eq!(check(&x, true), Ok(()));
        x[10] = 2;
        assert_eq!(check(&x, true), Err(DXGI_ERROR_INVALID_CALL));
        x = d(4, 2);
        x[8] = 1;
        assert_eq!(check(&x, true), Err(DXGI_ERROR_INVALID_CALL));
        x = d(4, 2);
        x[6] = 0x220;
        assert_eq!(check(&x, true), Err(DXGI_ERROR_INVALID_CALL));
    }
}

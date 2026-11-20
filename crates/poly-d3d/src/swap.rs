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

/// A rectangle: left, top, right, bottom (a `RECT`).
pub type Rect = [i32; 4];

/// What a flip-model back buffer of `width` x `height` still has to take from the last presented
/// frame when it is presented: the area `owed` (where it holds an older frame) less the `drawn`
/// rectangles (the present's dirty rectangles, which the app redrew whole: Present1.yaml; none
/// means all of it). Rectangles are clipped to the buffer; the result does not overlap.
///
/// The buffer is filled at its own present and not right after the one before (ADR 0224
/// addendum), so a frame the app redraws whole (Chrome's scrolled page) is not copied at all.
pub fn carry(owed: &[Rect], drawn: &[Rect], width: u32, height: u32) -> alloc::vec::Vec<Rect> {
    use alloc::vec::Vec;
    let (w, h) = (width.min(i32::MAX as u32) as i32, height.min(i32::MAX as u32) as i32);
    let clip = |r: &Rect| [r[0].clamp(0, w), r[1].clamp(0, h), r[2].clamp(0, w), r[3].clamp(0, h)];
    let owed: Vec<Rect> = owed.iter().map(clip).filter(|r| r[0] < r[2] && r[1] < r[3]).collect();
    let drawn: Vec<Rect> = drawn.iter().map(clip).filter(|r| r[0] < r[2] && r[1] < r[3]).collect();
    let mut out = Vec::new();
    if owed.is_empty() || drawn.is_empty() {
        return out;
    }
    // Bands of rows between the rectangles' edges: in each, the owed spans less the drawn ones.
    let mut ys: Vec<i32> = owed.iter().chain(&drawn).flat_map(|r| [r[1], r[3]]).collect();
    ys.sort_unstable();
    ys.dedup();
    let spans = |rects: &[Rect], y0: i32, y1: i32| -> Vec<(i32, i32)> {
        let mut s: Vec<(i32, i32)> = rects.iter().filter(|r| r[1] <= y0 && r[3] >= y1).map(|r| (r[0], r[2])).collect();
        s.sort_unstable();
        let mut merged: Vec<(i32, i32)> = Vec::new();
        for (a, b) in s {
            match merged.last_mut() {
                Some(last) if a <= last.1 => last.1 = last.1.max(b),
                _ => merged.push((a, b)),
            }
        }
        merged
    };
    for band in ys.windows(2) {
        let (y0, y1) = (band[0], band[1]);
        let cut = spans(&drawn, y0, y1);
        for (mut a, b) in spans(&owed, y0, y1) {
            for &(ca, cb) in &cut {
                if cb <= a || ca >= b {
                    continue;
                }
                if ca > a {
                    out.push([a, y0, ca, y1]);
                }
                a = a.max(cb);
                if a >= b {
                    break;
                }
            }
            if a < b {
                out.push([a, y0, b, y1]);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(rs: &[Rect]) -> i64 {
        rs.iter().map(|r| i64::from(r[2] - r[0]) * i64::from(r[3] - r[1])).sum()
    }

    #[test]
    fn carry_leaves_what_the_app_redrew() {
        let all = [0, 0, 100, 50];
        // Redrawn whole (no dirty rectangles) or nothing owed: nothing to take.
        assert!(carry(&[all], &[], 100, 50).is_empty());
        assert!(carry(&[], &[all], 100, 50).is_empty());
        // The same area owed and drawn (a scrolled page): nothing.
        assert!(carry(&[[0, 10, 100, 50]], &[[0, 10, 100, 50]], 100, 50).is_empty());
        // Owed all, a 10x10 drawn in the middle: the rest, 4900 pixels, none of them drawn.
        let r = carry(&[all], &[[40, 20, 50, 30]], 100, 50);
        assert_eq!(area(&r), 5000 - 100);
        for p in &r {
            assert!(p[2] <= 40 || p[0] >= 50 || p[3] <= 20 || p[1] >= 30, "{p:?} overlaps the drawn square");
        }
        // Owed a small square, drawn elsewhere: the square.
        assert_eq!(carry(&[[1, 1, 3, 3]], &[[50, 40, 60, 45]], 100, 50), alloc::vec![[1, 1, 3, 3]]);
        // Overlapping owed rectangles are taken once; parts outside the buffer are clipped.
        let r = carry(&[[0, 0, 20, 20], [10, 10, 30, 30], [90, 40, 200, 90]], &[[0, 0, 1, 1]], 100, 50);
        assert_eq!(area(&r), 400 + 400 - 100 - 1 + 10 * 10);
        // Two drawn spans cut one owed span into three.
        let r = carry(&[[0, 0, 100, 1]], &[[10, 0, 20, 1], [30, 0, 40, 1]], 100, 50);
        assert_eq!(r, alloc::vec![[0, 0, 10, 1], [20, 0, 30, 1], [40, 0, 100, 1]]);
        // Empty and reversed rectangles count for nothing.
        assert!(carry(&[[5, 5, 5, 9], [9, 9, 2, 2]], &[[0, 0, 1, 1]], 100, 50).is_empty());
    }

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

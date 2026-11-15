//! World transformations and stretching blits (Learn: `SetGraphicsMode`,
//! `SetWorldTransform`, `ModifyWorldTransform`, `BitBlt`, `StretchBlt`,
//! `StretchDIBits`, `SetDIBitsToDevice`).

use super::{dib32, rows};
use crate::bitmap::{Bitmap, Storage};
use crate::gdi::*;
use crate::region::Rect;
use crate::xform::*;

const SRCCOPY: u32 = 0x00CC_0020;
const PATCOPY: u32 = 0x00F0_0021;

#[test]
fn the_world_transformation_needs_the_advanced_mode() {
    let mut g = Gdi::new();
    let (dc, _px) = dib32(&mut g, 4, 4);
    assert_eq!(g.graphics_mode(dc), GM_COMPATIBLE);
    assert_eq!(g.world_transform(dc), Some(Xform::IDENTITY));
    // GM_COMPATIBLE: neither call changes it.
    assert!(!g.set_world_transform(dc, Xform::translate(1.0, 2.0)));
    assert!(!g.modify_world_transform(dc, Xform::IDENTITY, MWT_IDENTITY));
    assert_eq!(g.set_graphics_mode(dc, 3), (0, ERROR_INVALID_PARAMETER));
    assert_eq!(g.set_graphics_mode(dc, GM_ADVANCED), (GM_COMPATIBLE, 0));
    assert!(g.set_world_transform(dc, Xform::translate(1.0, 2.0)));
    assert_eq!(g.world_transform(dc), Some(Xform::translate(1.0, 2.0)));
    // A transformation that cannot be inverted is refused.
    assert!(!g.set_world_transform(dc, Xform::scale(0.0, 1.0)));
    // Back to GM_COMPATIBLE only with the identity.
    assert_eq!(g.set_graphics_mode(dc, GM_COMPATIBLE), (0, 0));
    assert!(g.modify_world_transform(dc, Xform::IDENTITY, MWT_IDENTITY));
    assert_eq!(g.set_graphics_mode(dc, GM_COMPATIBLE), (GM_ADVANCED, 0));
    assert_eq!(g.set_graphics_mode(0x1234, GM_ADVANCED), (0, ERROR_INVALID_HANDLE));
    assert_eq!(g.graphics_mode(0x1234), 0);
}

#[test]
fn save_and_restore_keep_the_transformation() {
    let mut g = Gdi::new();
    let (dc, _px) = dib32(&mut g, 4, 4);
    g.set_graphics_mode(dc, GM_ADVANCED);
    g.set_world_transform(dc, Xform::scale(2.0, 2.0));
    let level = g.save_dc(dc);
    assert!(g.modify_world_transform(dc, Xform::translate(1.0, 0.0), MWT_RIGHTMULTIPLY));
    assert_eq!(g.world_transform(dc).unwrap().apply(1, 1), (3, 2));
    assert!(g.restore_dc(dc, level));
    assert_eq!(g.world_transform(dc), Some(Xform::scale(2.0, 2.0)));
}

#[test]
fn drawing_follows_the_world_transformation() {
    let mut g = Gdi::new();
    let (dc, mut px) = dib32(&mut g, 8, 4);
    px.fill(0x00FF_FFFF);
    let red = g.create_solid_brush(0x0000_00FF);
    g.select_object(dc, u64::from(red)).unwrap();
    g.set_graphics_mode(dc, GM_ADVANCED);
    // Scale by 2, then move right by 1: a 2x1 rectangle at (1,1) covers
    // (3,2)..(7,4).
    g.set_world_transform(dc, Xform { m11: 2.0, m22: 2.0, dx: 1.0, ..Xform::IDENTITY });
    g.pat_blt(dc, 1, 1, 2, 1, PATCOPY);
    let key = [(0x00FF_FFFF, '.'), (0x00FF_0000, 'R')];
    assert_eq!(rows(&px, 8, &key), ["........", "........", "...RRRR.", "...RRRR."]);
    // The clip box is in logical coordinates (8x4 device = 3.5x2 world,
    // shifted by the offset).
    let (_, b) = g.clip_box(dc);
    assert_eq!(b, Rect::new(-1, 0, 4, 2));
    // A pixel goes through it too.
    assert_eq!(g.set_pixel(dc, 0, 0, 0x0000_FF00), 0x0000_FF00);
    assert_eq!(px[1], 0x0000_FF00);
}

/// A 4x2 source of distinct colours `0x00000a`..`0x000011` in a memory DC.
fn source(g: &mut Gdi) -> (u64, alloc::vec::Vec<u32>) {
    let (src, mut s) = dib32(g, 4, 2);
    for (i, v) in s.iter_mut().enumerate() {
        *v = 0x0a + i as u32;
    }
    (src, s)
}

fn hex(px: &[u32], w: usize) -> alloc::vec::Vec<alloc::vec::Vec<u32>> {
    px.chunks(w).map(|r| r.to_vec()).collect()
}

#[test]
fn bitblt_moves_with_a_translation_and_stretches_with_a_scale() {
    let mut g = Gdi::new();
    let (src, _s) = source(&mut g);
    let (dst, d) = dib32(&mut g, 8, 4);
    g.set_graphics_mode(dst, GM_ADVANCED);
    g.set_world_transform(dst, Xform::translate(2.0, 1.0));
    assert_eq!(g.bit_blt(dst, 0, 0, 2, 1, src, 1, 1, SRCCOPY), (true, 0));
    assert_eq!(&d[8 + 2..8 + 4], &[0x0f, 0x10]);
    // Scaled by 2: each source pixel becomes 2x2.
    let (dst2, d2) = dib32(&mut g, 8, 4);
    g.set_graphics_mode(dst2, GM_ADVANCED);
    g.set_world_transform(dst2, Xform::scale(2.0, 2.0));
    g.bit_blt(dst2, 0, 0, 4, 2, src, 0, 0, SRCCOPY);
    assert_eq!(
        hex(&d2, 8),
        [
            [0x0a, 0x0a, 0x0b, 0x0b, 0x0c, 0x0c, 0x0d, 0x0d],
            [0x0a, 0x0a, 0x0b, 0x0b, 0x0c, 0x0c, 0x0d, 0x0d],
            [0x0e, 0x0e, 0x0f, 0x0f, 0x10, 0x10, 0x11, 0x11],
            [0x0e, 0x0e, 0x0f, 0x0f, 0x10, 0x10, 0x11, 0x11],
        ]
    );
    // A rotation in the source DC fails.
    g.set_graphics_mode(src, GM_ADVANCED);
    g.set_world_transform(src, Xform { m11: 0.0, m12: 1.0, m21: -1.0, m22: 0.0, dx: 0.0, dy: 0.0 });
    assert!(!g.bit_blt(dst, 0, 0, 1, 1, src, 0, 0, SRCCOPY).0);
}

#[test]
fn stretchblt_compresses_and_mirrors() {
    let mut g = Gdi::new();
    let (src, _s) = source(&mut g);
    let (dst, d) = dib32(&mut g, 4, 2);
    // 4x2 into 2x1: the source pixel under each destination pixel's
    // centre (columns 1 and 3 of row 1).
    assert_eq!(g.stretch_blt(dst, (0, 0, 2, 1), src, (0, 0, 4, 2), SRCCOPY), (true, 0));
    assert_eq!(&d[0..2], &[0x0f, 0x11]);
    // A negative destination width mirrors.
    g.stretch_blt(dst, (4, 0, -4, 2), src, (0, 0, 4, 2), SRCCOPY);
    assert_eq!(hex(&d, 4), [[0x0d, 0x0c, 0x0b, 0x0a], [0x11, 0x10, 0x0f, 0x0e]]);
    // Both negative: no mirror.
    g.stretch_blt(dst, (4, 2, -4, -2), src, (4, 2, -4, -2), SRCCOPY);
    assert_eq!(hex(&d, 4), [[0x0a, 0x0b, 0x0c, 0x0d], [0x0e, 0x0f, 0x10, 0x11]]);
    assert_eq!(g.set_stretch_mode(dst, HALFTONE), BLACKONWHITE);
    assert_eq!(g.stretch_mode(dst), HALFTONE);
    assert_eq!(g.set_stretch_mode(dst, 9), 0);
}

#[test]
fn a_dib_reaches_a_dc_bottom_row_last() {
    let mut g = Gdi::new();
    let (dst, mut d) = dib32(&mut g, 4, 2);
    d.fill(0x00FF_FFFF);
    // A bottom-up 4x2 DIB: memory row 0 is the bottom row.
    let bits: alloc::vec::Vec<u8> = [1u32, 2, 3, 4, 5, 6, 7, 8].iter().flat_map(|v| v.to_le_bytes()).collect();
    let bmp = |h: i32, bits: alloc::vec::Vec<u8>| Bitmap {
        width: 4,
        height: h,
        bpp: 32,
        stride: 16,
        top_down: false,
        dib: true,
        colors: alloc::vec::Vec::new(),
        storage: Storage::Owned(bits),
    };
    assert!(g.dib_to_device(dst, (0, 0, 4, 2), &bmp(2, bits.clone()), Rect::new(0, 0, 4, 2), (false, false), SRCCOPY));
    assert_eq!(hex(&d, 4), [[5, 6, 7, 8], [1, 2, 3, 4]]);
    // Mirrored vertically.
    g.dib_to_device(dst, (0, 0, 4, 2), &bmp(2, bits.clone()), Rect::new(0, 0, 4, 2), (false, true), SRCCOPY);
    assert_eq!(hex(&d, 4), [[1, 2, 3, 4], [5, 6, 7, 8]]);
    d.fill(0x00FF_FFFF);
    // Only one row of the image given (its top row): the rest stays.
    g.dib_to_device(dst, (0, 0, 4, 2), &bmp(1, bits[16..].to_vec()), Rect::new(0, 0, 4, 2), (false, false), SRCCOPY);
    assert_eq!(hex(&d, 4), [[5, 6, 7, 8], [0xFF_FFFF; 4]]);
}

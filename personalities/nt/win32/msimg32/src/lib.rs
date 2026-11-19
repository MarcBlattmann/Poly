//! Our `msimg32.dll`. Learn documents `AlphaBlend`, `GradientFill` and `TransparentBlt` in
//! `wingdi.h` with "DLL: Msimg32.dll"; `gdi32.dll` exports the same functions as `GdiAlphaBlend`,
//! `GdiGradientFill` and `GdiTransparentBlt` (export names), and this DLL passes `AlphaBlend` on to
//! `gdi32`, where the drawing is.
//!
//! It used to be one of the generated fail-only DLLs (ADR 0155, step 10), which exist for 64-bit
//! programs only. Google's 32-bit `updater.exe` imports `AlphaBlend` and did not load without a
//! 32-bit `msimg32.dll` (`0xC0000135`), so this crate is built for both widths (ADR 0186's
//! `cargo xtask wow64`).
//!
//! `GradientFill` and `TransparentBlt` are not implemented yet: they fail (FALSE and
//! `ERROR_CALL_NOT_IMPLEMENTED`) with their full signatures, so that a 32-bit caller's stack is
//! left right.

#![no_std]
#![allow(clippy::too_many_arguments)]

use poly_win32_rt as _;

#[cfg_attr(target_arch = "x86", link(name = "gdi32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "gdi32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GdiAlphaBlend(
        dest: usize,
        x_dest: i32,
        y_dest: i32,
        w_dest: i32,
        h_dest: i32,
        src: usize,
        x_src: i32,
        y_src: i32,
        w_src: i32,
        h_src: i32,
        blend: u32,
    ) -> i32;
}

/// `AlphaBlend(hdcDest, xoriginDest, yoriginDest, wDest, hDest, hdcSrc, xoriginSrc, yoriginSrc,
/// wSrc, hSrc, ftn)`: `gdi32`'s `GdiAlphaBlend`. The `BLENDFUNCTION` is four bytes passed by value.
#[unsafe(no_mangle)]
pub extern "system" fn AlphaBlend(
    dest: usize,
    x_dest: i32,
    y_dest: i32,
    w_dest: i32,
    h_dest: i32,
    src: usize,
    x_src: i32,
    y_src: i32,
    w_src: i32,
    h_src: i32,
    blend: u32,
) -> i32 {
    // SAFETY: the caller's arguments, passed on as they are; gdi32 checks the handles.
    unsafe { GdiAlphaBlend(dest, x_dest, y_dest, w_dest, h_dest, src, x_src, y_src, w_src, h_src, blend) }
}

/// `GradientFill(hdc, pVertex, nVertex, pMesh, nMesh, ulMode)`: not implemented yet.
#[unsafe(no_mangle)]
pub extern "system" fn GradientFill(
    _hdc: usize,
    _vertices: *const u8,
    _vertex_count: u32,
    _mesh: *const u8,
    _mesh_count: u32,
    _mode: u32,
) -> i32 {
    poly_win32_rt::unimplemented_export("stub: msimg32!GradientFill", true, 0) as i32
}

/// `TransparentBlt(hdcDest, xoriginDest, yoriginDest, wDest, hDest, hdcSrc, xoriginSrc,
/// yoriginSrc, wSrc, hSrc, crTransparent)`: not implemented yet.
#[unsafe(no_mangle)]
pub extern "system" fn TransparentBlt(
    _dest: usize,
    _x_dest: i32,
    _y_dest: i32,
    _w_dest: i32,
    _h_dest: i32,
    _src: usize,
    _x_src: i32,
    _y_src: i32,
    _w_src: i32,
    _h_src: i32,
    _transparent: u32,
) -> i32 {
    poly_win32_rt::unimplemented_export("stub: msimg32!TransparentBlt", true, 0) as i32
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn msimg32_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}

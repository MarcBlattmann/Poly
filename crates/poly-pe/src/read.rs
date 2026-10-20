//! Bounds-checked little-endian reads. Every function returns `None` instead
//! of reading past the end of its slice or overflowing an offset.

use crate::{Error, MAX_NAME_LEN};

pub(crate) fn bytes(b: &[u8], off: usize, len: usize) -> Option<&[u8]> {
    b.get(off..off.checked_add(len)?)
}

pub(crate) fn u8_at(b: &[u8], off: usize) -> Option<u8> {
    b.get(off).copied()
}

pub(crate) fn u16_at(b: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes(b, off, 2)?.try_into().ok()?))
}

pub(crate) fn u32_at(b: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes(b, off, 4)?.try_into().ok()?))
}

pub(crate) fn u64_at(b: &[u8], off: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes(b, off, 8)?.try_into().ok()?))
}

/// A pointer-sized field: 8 bytes when `wide`, else 4 (zero-extended).
pub(crate) fn ptr_at(b: &[u8], off: usize, wide: bool) -> Option<u64> {
    if wide { u64_at(b, off) } else { u32_at(b, off).map(u64::from) }
}

/// The bytes before the first NUL at `off`. The NUL must come within
/// [`MAX_NAME_LEN`] bytes and before the end of `b`.
pub(crate) fn cstr(b: &[u8], off: usize) -> Result<&[u8], Error> {
    let rest = b.get(off..).ok_or(Error::Truncated)?;
    let window = &rest[..rest.len().min(MAX_NAME_LEN + 1)];
    match window.iter().position(|&c| c == 0) {
        Some(n) => Ok(&window[..n]),
        None => Err(Error::BadString),
    }
}

pub(crate) fn put_u16(b: &mut [u8], off: usize, v: u16) -> Option<()> {
    b.get_mut(off..off.checked_add(2)?)?.copy_from_slice(&v.to_le_bytes());
    Some(())
}

pub(crate) fn put_u32(b: &mut [u8], off: usize, v: u32) -> Option<()> {
    b.get_mut(off..off.checked_add(4)?)?.copy_from_slice(&v.to_le_bytes());
    Some(())
}

pub(crate) fn put_u64(b: &mut [u8], off: usize, v: u64) -> Option<()> {
    b.get_mut(off..off.checked_add(8)?)?.copy_from_slice(&v.to_le_bytes());
    Some(())
}

/// Rounds `v` up to a multiple of `align` (a power of two, or zero for "no
/// alignment"). `None` on overflow.
pub(crate) fn align_up(v: u32, align: u32) -> Option<u32> {
    if align <= 1 {
        return Some(v);
    }
    Some(v.checked_add(align - 1)? & !(align - 1))
}

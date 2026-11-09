//! `HSTRING`s (Learn: "HSTRING", "WindowsCreateString",
//! "WindowsCreateStringReference", "WindowsDeleteString" and the other
//! `winstring.h` functions).
//!
//! An `HSTRING` is an opaque handle; the empty string is `NULL`. Ours
//! points at a [`Header`] of 24 bytes (the size of `HSTRING_HEADER` on
//! x64): for a string made with `WindowsCreateString` the header begins a
//! heap block that owns the characters and a reference count; for a
//! *fast-pass* string (`WindowsCreateStringReference`) the header is the
//! caller's own `HSTRING_HEADER` and the characters stay the caller's, so
//! deleting it does nothing and duplicating it makes a heap copy.

use crate::hresult::{E_INVALIDARG, E_OUTOFMEMORY, E_POINTER, Hresult, S_OK};
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

/// `E_BOUNDS` (`winerror.h`): `WindowsSubstring`'s index is out of range.
pub const E_BOUNDS: Hresult = 0x8000_000B_u32 as i32;

const FLAG_REFERENCE: u32 = 1;
/// A buffer made by `WindowsPreallocateStringBuffer`, not yet promoted.
const FLAG_BUFFER: u32 = 2;

/// 24 bytes, `sizeof(HSTRING_HEADER)` on x64 (`winstring.h`).
#[derive(Debug)]
#[repr(C)]
pub struct Header {
    flags: u32,
    len: u32,
    buf: *const u16,
    refs: AtomicUsize,
}

/// An `HSTRING`: a pointer to a [`Header`], or null for the empty string.
pub type HString = *mut Header;

/// What a heap string is: the header first, so the handle is the block.
#[repr(C)]
struct Heap {
    hdr: Header,
    data: Box<[u16]>,
}

/// The characters of a string with no characters: a lone NUL.
static EMPTY: [u16; 1] = [0];

fn heap_string(chars: &[u16], flags: u32) -> Result<HString, Hresult> {
    let mut v: Vec<u16> = Vec::new();
    v.try_reserve_exact(chars.len() + 1).map_err(|_| E_OUTOFMEMORY)?;
    v.extend_from_slice(chars);
    v.push(0);
    let data = v.into_boxed_slice();
    let buf = data.as_ptr();
    let heap = Box::new(Heap { hdr: Header { flags, len: chars.len() as u32, buf, refs: AtomicUsize::new(1) }, data });
    Ok(Box::into_raw(heap).cast::<Header>())
}

/// `WindowsCreateString`: a copy of `len` characters at `src`; the empty
/// string (`len` 0) is `NULL`.
///
/// # Safety
/// `src` is null or readable for `len` characters.
pub unsafe fn create(src: *const u16, len: u32) -> Result<HString, Hresult> {
    if len == 0 {
        return Ok(core::ptr::null_mut());
    }
    if src.is_null() {
        return Err(E_POINTER);
    }
    // SAFETY: `len` readable characters.
    let chars = unsafe { core::slice::from_raw_parts(src, len as usize) };
    heap_string(chars, 0)
}

/// `WindowsCreateStringReference`: a fast-pass string over the caller's
/// characters (`src[len]` is NUL by the function's contract) whose header
/// is `header`'s 24 bytes.
///
/// # Safety
/// `header` is null or writable for 24 bytes (8-aligned) that outlive the
/// string; `src` outlives it too.
pub unsafe fn create_reference(src: *const u16, len: u32, header: *mut Header) -> Result<HString, Hresult> {
    if header.is_null() {
        return Err(E_POINTER);
    }
    if len == 0 {
        return Ok(core::ptr::null_mut());
    }
    if src.is_null() {
        return Err(E_POINTER);
    }
    // SAFETY: the caller's header storage.
    unsafe {
        header.write(Header { flags: FLAG_REFERENCE, len, buf: src, refs: AtomicUsize::new(1) });
    }
    Ok(header)
}

/// `WindowsDeleteString`: drop one reference; a fast-pass string is left
/// alone.
///
/// # Safety
/// `h` is null or a handle from this module that is still valid.
pub unsafe fn delete(h: HString) -> Hresult {
    if h.is_null() {
        return S_OK;
    }
    // SAFETY: a valid handle.
    let (flags, last) = unsafe { ((*h).flags, (*h).refs.fetch_sub(1, Ordering::AcqRel) == 1) };
    if flags & FLAG_REFERENCE == 0 && last {
        // SAFETY: the last reference of a block `heap_string` made.
        drop(unsafe { Box::from_raw(h.cast::<Heap>()) });
    }
    S_OK
}

/// `WindowsDuplicateString`: the same string (one more reference), or a
/// heap copy of a fast-pass string.
///
/// # Safety
/// `h` is null or a valid handle.
pub unsafe fn duplicate(h: HString) -> Result<HString, Hresult> {
    if h.is_null() {
        return Ok(h);
    }
    // SAFETY: a valid handle.
    unsafe {
        if (*h).flags & FLAG_REFERENCE != 0 {
            let chars = core::slice::from_raw_parts((*h).buf, (*h).len as usize);
            heap_string(chars, 0)
        } else {
            (*h).refs.fetch_add(1, Ordering::AcqRel);
            Ok(h)
        }
    }
}

/// `WindowsGetStringRawBuffer`: the NUL-terminated characters and their
/// number (a lone NUL for the empty string).
///
/// # Safety
/// `h` is null or a valid handle; the characters live as long as it does.
pub unsafe fn raw_buffer(h: HString) -> (*const u16, u32) {
    if h.is_null() {
        return (EMPTY.as_ptr(), 0);
    }
    // SAFETY: a valid handle.
    unsafe { ((*h).buf, (*h).len) }
}

unsafe fn chars<'a>(h: HString) -> &'a [u16] {
    // SAFETY: the caller's valid handle.
    unsafe {
        let (p, n) = raw_buffer(h);
        core::slice::from_raw_parts(p, n as usize)
    }
}

/// `WindowsGetStringLen`.
///
/// # Safety
/// `h` is null or a valid handle.
pub unsafe fn len(h: HString) -> u32 {
    // SAFETY: as documented.
    unsafe { raw_buffer(h).1 }
}

/// `WindowsIsStringEmpty`.
///
/// # Safety
/// `h` is null or a valid handle.
pub unsafe fn is_empty(h: HString) -> bool {
    // SAFETY: as documented.
    unsafe { len(h) == 0 }
}

/// `WindowsStringHasEmbeddedNull`.
///
/// # Safety
/// `h` is null or a valid handle.
pub unsafe fn has_embedded_null(h: HString) -> bool {
    // SAFETY: as documented.
    unsafe { chars(h).contains(&0) }
}

/// `WindowsCompareStringOrdinal`: -1, 0 or 1 by UTF-16 code unit.
///
/// # Safety
/// Both are null or valid handles.
pub unsafe fn compare_ordinal(a: HString, b: HString) -> i32 {
    // SAFETY: as documented.
    let (x, y) = unsafe { (chars(a), chars(b)) };
    match x.cmp(y) {
        core::cmp::Ordering::Less => -1,
        core::cmp::Ordering::Equal => 0,
        core::cmp::Ordering::Greater => 1,
    }
}

/// `WindowsConcatString`.
///
/// # Safety
/// Both are null or valid handles.
pub unsafe fn concat(a: HString, b: HString) -> Result<HString, Hresult> {
    // SAFETY: as documented.
    unsafe {
        if is_empty(a) {
            return duplicate(b);
        }
        if is_empty(b) {
            return duplicate(a);
        }
        let mut v: Vec<u16> = Vec::new();
        v.try_reserve_exact(chars(a).len() + chars(b).len()).map_err(|_| E_OUTOFMEMORY)?;
        v.extend_from_slice(chars(a));
        v.extend_from_slice(chars(b));
        heap_string(&v, 0)
    }
}

/// `WindowsSubstring`: from `start` to the end (`E_BOUNDS` past the end).
///
/// # Safety
/// `h` is null or a valid handle.
pub unsafe fn substring(h: HString, start: u32) -> Result<HString, Hresult> {
    // SAFETY: as documented.
    unsafe {
        let all = chars(h);
        if start as usize > all.len() {
            return Err(E_BOUNDS);
        }
        if start as usize == all.len() {
            return Ok(core::ptr::null_mut());
        }
        if start == 0 {
            return duplicate(h);
        }
        heap_string(&all[start as usize..], 0)
    }
}

/// `WindowsPreallocateStringBuffer`: a zeroed buffer of `len` characters
/// (and its NUL) to fill in; returns the buffer handle and the characters.
/// `len` 0 is `E_INVALIDARG` (there is nothing to promote).
pub fn preallocate(len: u32) -> Result<(HString, *mut u16), Hresult> {
    if len == 0 {
        return Err(E_INVALIDARG);
    }
    let zeros: Vec<u16> = alloc::vec![0; len as usize];
    let h = heap_string(&zeros, FLAG_BUFFER)?;
    // SAFETY: the block just made.
    let p = unsafe { (*h).buf as *mut u16 };
    Ok((h, p))
}

/// `WindowsPromoteStringBuffer`: the filled-in buffer becomes an `HSTRING`.
///
/// # Safety
/// `buffer` is a handle from [`preallocate`] not yet promoted or deleted.
pub unsafe fn promote(buffer: HString) -> Result<HString, Hresult> {
    if buffer.is_null() {
        return Err(E_POINTER);
    }
    // SAFETY: a valid buffer handle.
    unsafe {
        if (*buffer).flags & FLAG_BUFFER == 0 {
            return Err(E_INVALIDARG);
        }
        (*buffer).flags &= !FLAG_BUFFER;
    }
    Ok(buffer)
}

/// `WindowsDeleteStringBuffer`: drop a buffer that was not promoted.
///
/// # Safety
/// `buffer` is null or a handle from [`preallocate`] not yet promoted.
pub unsafe fn delete_buffer(buffer: HString) -> Hresult {
    if buffer.is_null() {
        return S_OK;
    }
    // SAFETY: a valid buffer handle.
    unsafe {
        if (*buffer).flags & FLAG_BUFFER == 0 {
            return E_INVALIDARG;
        }
        delete(buffer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn header_is_24_bytes() {
        assert_eq!(core::mem::size_of::<Header>(), 24);
    }

    #[test]
    fn create_copies_and_delete_frees() {
        let src = w("Windows.Foundation.Uri");
        // SAFETY: valid handles throughout.
        unsafe {
            let h = create(src.as_ptr(), src.len() as u32).unwrap();
            assert!(!h.is_null());
            let (p, n) = raw_buffer(h);
            assert_eq!(n as usize, src.len());
            assert_eq!(core::slice::from_raw_parts(p, n as usize + 1)[n as usize], 0);
            assert_ne!(p, src.as_ptr());
            assert_eq!(delete(h), S_OK);
        }
    }

    #[test]
    fn empty_is_null() {
        // SAFETY: valid handles throughout.
        unsafe {
            assert!(create(core::ptr::null(), 0).unwrap().is_null());
            assert_eq!(create(core::ptr::null(), 3), Err(E_POINTER));
            let (p, n) = raw_buffer(core::ptr::null_mut());
            assert_eq!((n, *p), (0, 0));
            assert!(is_empty(core::ptr::null_mut()));
            assert_eq!(delete(core::ptr::null_mut()), S_OK);
        }
    }

    #[test]
    fn reference_uses_the_callers_header_and_characters() {
        let src = w("abc\0");
        let mut hdr = core::mem::MaybeUninit::<Header>::uninit();
        // SAFETY: the header and characters outlive the string.
        unsafe {
            let h = create_reference(src.as_ptr(), 3, hdr.as_mut_ptr()).unwrap();
            assert_eq!(h, hdr.as_mut_ptr());
            assert_eq!(raw_buffer(h), (src.as_ptr(), 3));
            // Deleting a fast-pass string is a no-op; duplicating copies it.
            let d = duplicate(h).unwrap();
            assert_ne!(d, h);
            assert_eq!(compare_ordinal(h, d), 0);
            assert_eq!(delete(h), S_OK);
            assert_eq!(delete(d), S_OK);
        }
    }

    #[test]
    fn duplicate_shares_until_the_last_delete() {
        let src = w("x");
        // SAFETY: valid handles throughout.
        unsafe {
            let h = create(src.as_ptr(), 1).unwrap();
            let d = duplicate(h).unwrap();
            assert_eq!(d, h);
            delete(h);
            assert_eq!(chars(d), &w("x")[..]);
            delete(d);
        }
    }

    #[test]
    fn compare_concat_substring() {
        let (a, b) = (w("abc"), w("abd"));
        // SAFETY: valid handles throughout.
        unsafe {
            let ha = create(a.as_ptr(), 3).unwrap();
            let hb = create(b.as_ptr(), 3).unwrap();
            assert_eq!(compare_ordinal(ha, hb), -1);
            assert_eq!(compare_ordinal(hb, ha), 1);
            assert_eq!(compare_ordinal(ha, ha), 0);
            assert_eq!(compare_ordinal(core::ptr::null_mut(), ha), -1);
            let c = concat(ha, hb).unwrap();
            assert_eq!(chars(c), &w("abcabd")[..]);
            let s = substring(c, 3).unwrap();
            assert_eq!(chars(s), &w("abd")[..]);
            assert!(substring(c, 6).unwrap().is_null());
            assert_eq!(substring(c, 7), Err(E_BOUNDS));
            assert!(!has_embedded_null(c));
            for h in [ha, hb, c, s] {
                delete(h);
            }
            let z = [97u16, 0, 98];
            let hz = create(z.as_ptr(), 3).unwrap();
            assert!(has_embedded_null(hz));
            delete(hz);
        }
    }

    #[test]
    fn preallocated_buffers_promote_once() {
        let (buf, p) = preallocate(3).unwrap();
        // SAFETY: the buffer was just made, three characters long.
        unsafe {
            core::slice::from_raw_parts_mut(p, 3).copy_from_slice(&w("hey"));
            let h = promote(buf).unwrap();
            assert_eq!(chars(h), &w("hey")[..]);
            assert_eq!(promote(h), Err(E_INVALIDARG));
            assert_eq!(delete_buffer(h), E_INVALIDARG);
            delete(h);
            let (b2, _) = preallocate(1).unwrap();
            assert_eq!(delete_buffer(b2), S_OK);
        }
        assert_eq!(preallocate(0).err(), Some(E_INVALIDARG));
    }
}

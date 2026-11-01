//! Our `oleaut32.dll` (ADR 0087): `BSTR`s (`SysAllocString`,
//! `SysAllocStringLen`, `SysReAllocString`, `SysReAllocStringLen`,
//! `SysFreeString`, `SysStringLen`) and `VARIANT`s (`VariantInit`,
//! `VariantClear`, `VariantCopy`), the Automation basics console programs
//! such as 7-Zip's command line use, exported at the ordinals their
//! import libraries bind (`build.rs`). The decisions are
//! `poly_kernelbase::oleaut`'s; the memory is the process heap.

#![no_std]
#![allow(clippy::missing_safety_doc)]
// rustc exports every `#[no_mangle]` function by name (its `.def` file);
// `build.rs` exports them again with their ordinals, which lld-link keeps
// (the first specification), warning that the name came twice.
#![allow(linker_messages)]

use poly_kernelbase::oleaut::{self, *};
use poly_win32_rt as _;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetProcessHeap() -> *mut u8;
    fn HeapAlloc(heap: *mut u8, flags: u32, bytes: usize) -> *mut u8;
    fn HeapFree(heap: *mut u8, flags: u32, p: *mut u8) -> i32;
    fn SetLastError(error: u32);
}

/// `poly-kernelbase` links `alloc`: the process heap (16-byte aligned).
struct ProcessHeap;

// SAFETY: HeapAlloc returns 16-byte aligned blocks; stricter alignments
// are refused.
unsafe impl core::alloc::GlobalAlloc for ProcessHeap {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        if layout.align() > 16 {
            return core::ptr::null_mut();
        }
        // SAFETY: the process heap.
        unsafe { HeapAlloc(GetProcessHeap(), 0, layout.size().max(1)) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, _: core::alloc::Layout) {
        // SAFETY: a block from `alloc`.
        unsafe { HeapFree(GetProcessHeap(), 0, ptr) };
    }
}

#[global_allocator]
static ALLOC: ProcessHeap = ProcessHeap;

type Bstr = *mut u16;

// ----- BSTR ---------------------------------------------------------------------------

/// A new `BSTR` of `chars` characters copied from `src` (or zeros), or
/// NULL.
///
/// # Safety
/// `src` is null or readable for `chars` characters.
unsafe fn alloc_bstr(src: *const u16, chars: u32) -> Bstr {
    let Some(size) = bstr_alloc_size(chars) else { return core::ptr::null_mut() };
    // SAFETY: the process heap; HEAP_ZERO_MEMORY (8) when there is no source.
    let block = unsafe { HeapAlloc(GetProcessHeap(), if src.is_null() { 8 } else { 0 }, size) };
    if block.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: `block` holds the prefix, `chars` characters and the NUL.
    unsafe {
        block.cast::<u32>().write(chars * 2);
        let s = block.add(BSTR_PREFIX).cast::<u16>();
        if !src.is_null() {
            core::ptr::copy(src, s, chars as usize);
        }
        s.add(chars as usize).write(0);
        s
    }
}

/// The length of a NUL-terminated string in characters.
///
/// # Safety
/// `s` is a NUL-terminated string.
unsafe fn wcslen(s: *const u16) -> u32 {
    let mut n = 0usize;
    // SAFETY: the caller's contract.
    while unsafe { *s.add(n) } != 0 {
        n += 1;
    }
    n as u32
}

/// `SysAllocString`: Learn: NULL for a NULL string.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SysAllocString(s: *const u16) -> Bstr {
    if s.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's string.
    cleared(unsafe { alloc_bstr(s, wcslen(s)) })
}

/// `SysAllocStringLen`: `chars` characters (embedded NULs allowed) and a
/// NUL; with a NULL source the characters are zero (Learn lets them be
/// anything).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SysAllocStringLen(s: *const u16, chars: u32) -> Bstr {
    // SAFETY: the caller's string of `chars` characters.
    cleared(unsafe { alloc_bstr(s, chars) })
}

/// A new string from `SysAllocString` / `SysAllocStringLen`, with the last
/// error set to `ERROR_SUCCESS` when there is one: ADR 0110's trace of
/// 7-Zip records Windows doing so (Learn is silent; ADR 0114, to be
/// confirmed by the spec agent).
fn cleared(s: Bstr) -> Bstr {
    if !s.is_null() {
        // SAFETY: a kernel32 call without pointers.
        unsafe { SetLastError(0) };
    }
    s
}

/// `SysFreeString`: Learn: NULL is ignored.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SysFreeString(s: Bstr) {
    if !s.is_null() {
        // SAFETY: a BSTR from `alloc_bstr`: its block starts at the prefix.
        unsafe { HeapFree(GetProcessHeap(), 0, s.cast::<u8>().sub(BSTR_PREFIX)) };
    }
}

/// `SysStringLen`: the prefix's byte count in characters; 0 for NULL.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SysStringLen(s: Bstr) -> u32 {
    if s.is_null() {
        return 0;
    }
    // SAFETY: a BSTR's prefix precedes it.
    unsafe { s.cast::<u8>().sub(BSTR_PREFIX).cast::<u32>().read() / 2 }
}

/// `SysReAllocStringLen`: replace `*old` with a new `BSTR` of `chars`
/// characters from `s` (which may be the old string itself); TRUE on
/// success, FALSE (and `*old` unchanged) without memory.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SysReAllocStringLen(old: *mut Bstr, s: *const u16, chars: u32) -> i32 {
    if old.is_null() {
        return 0;
    }
    // SAFETY: the caller's string; the new one is made before the old is
    // freed, so `s` may point into it.
    unsafe {
        let new = alloc_bstr(s, chars);
        if new.is_null() {
            return 0;
        }
        SysFreeString(*old);
        *old = new;
    }
    1
}

/// `SysReAllocString`: as `SysReAllocStringLen` with the string's length.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SysReAllocString(old: *mut Bstr, s: *const u16) -> i32 {
    // SAFETY: the caller's string.
    let chars = if s.is_null() { 0 } else { unsafe { wcslen(s) } };
    // SAFETY: as above.
    unsafe { SysReAllocStringLen(old, s, chars) }
}

// ----- VARIANT ------------------------------------------------------------------------------

/// `VariantInit`: `vt` is `VT_EMPTY` (Learn: the reserved fields too; the
/// value is left as it is).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn VariantInit(v: *mut u8) {
    // SAFETY: the caller's VARIANT: vt and the three reserved WORDs.
    unsafe { core::ptr::write_bytes(v, 0, VARIANT_VALUE) };
}

/// Call `IUnknown` method `slot` (1: `AddRef`, 2: `Release`) of `object`.
///
/// # Safety
/// `object` is null or a COM object.
unsafe fn unknown_call(object: *mut u8, slot: usize) {
    if object.is_null() {
        return;
    }
    type Method = unsafe extern "system" fn(*mut u8) -> u32;
    // SAFETY: a COM object starts with its vtable pointer; slots 1 and 2
    // are `AddRef` and `Release` (`unknwn.h`).
    unsafe {
        let vtable = object.cast::<*const usize>().read();
        core::mem::transmute::<usize, Method>(vtable.add(slot).read())(object);
    }
}

/// `VariantClear`: release what the variant owns, then `VT_EMPTY`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn VariantClear(v: *mut u8) -> u32 {
    if v.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's VARIANT (24 bytes).
    unsafe {
        let vt = v.cast::<u16>().read_unaligned();
        let value = v.add(VARIANT_VALUE).cast::<*mut u8>().read_unaligned();
        match oleaut::held(vt) {
            Held::Bad => return DISP_E_BADVARTYPE,
            Held::Unsupported => return E_NOTIMPL,
            Held::Bstr => SysFreeString(value.cast()),
            Held::Interface => unknown_call(value, 2),
            Held::Bits => {}
        }
        VariantInit(v);
    }
    S_OK
}

/// `VariantCopy`: clear the destination, then copy the source; a `BSTR` is
/// duplicated, an interface gets `AddRef` (Learn).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn VariantCopy(dest: *mut u8, src: *const u8) -> u32 {
    if dest.is_null() || src.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's VARIANTs.
    unsafe {
        let vt = src.cast::<u16>().read_unaligned();
        let held = oleaut::held(vt);
        match held {
            Held::Bad => return DISP_E_BADVARTYPE,
            Held::Unsupported => return E_NOTIMPL,
            _ => {}
        }
        if core::ptr::eq(dest, src) {
            return S_OK;
        }
        let r = VariantClear(dest);
        if r != S_OK {
            return r;
        }
        let mut copy = [0u8; VARIANT_SIZE];
        core::ptr::copy_nonoverlapping(src, copy.as_mut_ptr(), VARIANT_SIZE);
        let value = src.add(VARIANT_VALUE).cast::<*mut u8>().read_unaligned();
        match held {
            Held::Bstr if !value.is_null() => {
                let dup = SysAllocStringLen(value.cast(), SysStringLen(value.cast()));
                if dup.is_null() {
                    return E_OUTOFMEMORY;
                }
                copy[VARIANT_VALUE..VARIANT_VALUE + 8].copy_from_slice(&(dup as u64).to_le_bytes());
            }
            Held::Interface => unknown_call(value, 1),
            _ => {}
        }
        core::ptr::copy_nonoverlapping(copy.as_ptr(), dest, VARIANT_SIZE);
    }
    S_OK
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn oleaut32_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}

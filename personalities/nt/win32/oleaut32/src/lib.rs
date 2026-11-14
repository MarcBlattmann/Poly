//! Our `oleaut32.dll` (ADR 0087): `BSTR`s (`SysAllocString`,
//! `SysAllocStringLen`, `SysReAllocString`, `SysReAllocStringLen`,
//! `SysFreeString`, `SysStringLen`) and `VARIANT`s (`VariantInit`,
//! `VariantClear`, `VariantCopy`), the Automation basics console programs
//! such as 7-Zip's command line use, exported at the ordinals their
//! import libraries bind (`build.rs`). The decisions are
//! `poly_kernelbase::oleaut`'s; the memory is the process heap.
//!
//! Since ADR 0155 step 9 also what Chrome's `chrome.dll` delay-loads by ordinal: safe arrays
//! (`SafeArrayCreateVector` and nine more; `poly_kernelbase::safearray`), `SysStringByteLen`,
//! `SysAllocStringByteLen`, `VarBstrCmp` and `LoadRegTypeLib` (no type library is registered).

#![no_std]
#![allow(clippy::missing_safety_doc)]
// rustc exports every `#[no_mangle]` function by name (its `.def` file);
// `build.rs` exports them again with their ordinals, which lld-link keeps
// (the first specification), warning that the name came twice.
#![allow(linker_messages)]

extern crate alloc;

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

// ----- byte-length BSTRs, VarBstrCmp, LoadRegTypeLib (ADR 0155, step 9) -----------------------

/// `SysStringByteLen`: the length of a `BSTR` in bytes (its prefix); 0 for NULL.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SysStringByteLen(s: Bstr) -> u32 {
    if s.is_null() {
        return 0;
    }
    // SAFETY: a BSTR's prefix precedes it.
    unsafe { s.cast::<u8>().sub(BSTR_PREFIX).cast::<u32>().read() }
}

/// `SysAllocStringByteLen(psz, len)`: a `BSTR` of `len` bytes copied from `psz` (zeros when it is
/// NULL), followed by a NUL (Learn: "allocates a new string of len bytes, copies len bytes from
/// the passed string into it, and then appends a null character"); the prefix is `len`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SysAllocStringByteLen(s: *const u8, len: u32) -> Bstr {
    let Some(size) = (len as usize).checked_add(BSTR_PREFIX + 2) else { return core::ptr::null_mut() };
    // SAFETY: the process heap, zeroed (HEAP_ZERO_MEMORY, 8): the NUL and, without a source, the
    // bytes.
    let block = unsafe { HeapAlloc(GetProcessHeap(), 8, size) };
    if block.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: `block` holds the prefix, `len` bytes and the NUL; `s` is readable for `len` bytes.
    unsafe {
        block.cast::<u32>().write(len);
        let data = block.add(BSTR_PREFIX);
        if !s.is_null() {
            core::ptr::copy(s, data, len as usize);
        }
        data.cast()
    }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CompareStringW(locale: u32, flags: u32, a: *const u16, a_len: i32, b: *const u16, b_len: i32) -> i32;
}

/// `VarBstrCmp(bstrLeft, bstrRight, lcid, dwFlags)`: `VARCMP_LT`, `VARCMP_EQ` or `VARCMP_GT`, as
/// `CompareStringW` orders the two strings for the locale and flags; a NULL `BSTR` is an empty
/// string.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn VarBstrCmp(left: Bstr, right: Bstr, lcid: u32, flags: u32) -> u32 {
    let empty = [0u16; 1];
    // SAFETY: the callers' BSTRs.
    let (ln, rn) = unsafe { (SysStringLen(left), SysStringLen(right)) };
    let l = if left.is_null() { empty.as_ptr() } else { left.cast_const() };
    let r = if right.is_null() { empty.as_ptr() } else { right.cast_const() };
    // SAFETY: two strings of the stated lengths.
    poly_kernelbase::safearray::varcmp(unsafe { CompareStringW(lcid, flags, l, ln as i32, r, rn as i32) })
}

/// `LoadRegTypeLib(rguid, wVerMajor, wVerMinor, lcid, pptlib)`: Poly's registry has no type
/// libraries, so none is registered under any id.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LoadRegTypeLib(
    _guid: *const u8,
    _major: u16,
    _minor: u16,
    _lcid: u32,
    lib: *mut *mut u8,
) -> u32 {
    if lib.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's out-pointer.
    unsafe { lib.write(core::ptr::null_mut()) };
    poly_kernelbase::safearray::TYPE_E_LIBNOTREGISTERED
}

// ----- safe arrays (ADR 0155, step 9) ------------------------------------------------------------

use poly_kernelbase::safearray::{self as sa, Element};

/// A field of a `SAFEARRAY`.
///
/// # Safety
/// `psa` is a `SAFEARRAY` descriptor.
unsafe fn sa_read<T: Copy>(psa: *const u8, at: usize) -> T {
    // SAFETY: the caller's descriptor; fields may be unaligned in a packed container.
    unsafe { psa.add(at).cast::<T>().read_unaligned() }
}

/// The bounds of a descriptor as `(cElements, lLbound)`, one per dimension.
///
/// # Safety
/// `psa` is a `SAFEARRAY` descriptor with `cDims` bounds.
unsafe fn sa_bounds(psa: *const u8) -> alloc::vec::Vec<(u32, i32)> {
    // SAFETY: the caller's descriptor.
    unsafe {
        let dims = usize::from(sa_read::<u16>(psa, sa::SA_DIMS));
        (0..dims)
            .map(|d| (sa_read::<u32>(psa, sa::SA_BOUNDS + 8 * d), sa_read::<i32>(psa, sa::SA_BOUNDS + 8 * d + 4)))
            .collect()
    }
}

/// `SafeArrayCreateVector(vt, lLbound, cElements)`: a one-dimensional array of zeroed elements, or
/// NULL (a type that cannot be an element, no memory). The descriptor and the data are two heap
/// blocks; the type is kept in front of the descriptor.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SafeArrayCreateVector(vt: u16, lower: i32, count: u32) -> *mut u8 {
    let Some((size, _)) = sa::element(vt) else { return core::ptr::null_mut() };
    let Some(bytes) = sa::data_size(size, count) else { return core::ptr::null_mut() };
    // SAFETY: the process heap, zeroed.
    unsafe {
        let block = HeapAlloc(GetProcessHeap(), 8, sa::SA_HIDDEN + sa::SA_VECTOR_SIZE);
        let data = HeapAlloc(GetProcessHeap(), 8, bytes.max(1));
        if block.is_null() || data.is_null() {
            HeapFree(GetProcessHeap(), 0, block);
            HeapFree(GetProcessHeap(), 0, data);
            return core::ptr::null_mut();
        }
        block.cast::<u32>().write(u32::from(vt));
        let psa = block.add(sa::SA_HIDDEN);
        psa.add(sa::SA_DIMS).cast::<u16>().write(1);
        psa.add(sa::SA_FEATURES).cast::<u16>().write(sa::features(vt));
        psa.add(sa::SA_ELEMENT_SIZE).cast::<u32>().write(size);
        psa.add(sa::SA_DATA).cast::<*mut u8>().write(data);
        psa.add(sa::SA_BOUNDS).cast::<u32>().write(count);
        psa.add(sa::SA_BOUNDS + 4).cast::<i32>().write(lower);
        psa
    }
}

/// Release what element `at` of the data holds.
///
/// # Safety
/// `at` is an element of an array whose elements are `kind`.
unsafe fn sa_release(kind: Element, at: *mut u8) {
    // SAFETY: the caller's element.
    unsafe {
        match kind {
            Element::Bits => {}
            Element::Bstr => SysFreeString(at.cast::<Bstr>().read_unaligned()),
            Element::Interface => unknown_call(at.cast::<*mut u8>().read_unaligned(), 2),
            Element::Variant => {
                VariantClear(at);
            }
        }
    }
}

/// `SafeArrayDestroy(psa)`: the elements are released (strings freed, interfaces released,
/// variants cleared), then the data and the descriptor. Learn: `DISP_E_ARRAYISLOCKED` for an
/// array that is locked. NULL is nothing to destroy. Only an array `oleaut32` made (it records
/// its type) has its blocks freed; another module's descriptor is left to that module.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SafeArrayDestroy(psa: *mut u8) -> u32 {
    if psa.is_null() {
        return S_OK;
    }
    // SAFETY: the caller's descriptor.
    unsafe {
        if sa_read::<u32>(psa, sa::SA_LOCKS) != 0 {
            return sa::DISP_E_ARRAYISLOCKED;
        }
        let features = sa_read::<u16>(psa, sa::SA_FEATURES);
        let size = sa_read::<u32>(psa, sa::SA_ELEMENT_SIZE) as usize;
        let data = sa_read::<*mut u8>(psa, sa::SA_DATA);
        let count: u64 = sa_bounds(psa).iter().map(|b| u64::from(b.0)).product();
        let kind = sa::element_of(features);
        if !data.is_null() && kind != Element::Bits {
            for i in 0..count as usize {
                sa_release(kind, data.add(i * size));
            }
        }
        if features & sa::FADF_HAVEVARTYPE != 0 && sa_read::<u16>(psa, sa::SA_DIMS) == 1 {
            HeapFree(GetProcessHeap(), 0, data);
            HeapFree(GetProcessHeap(), 0, psa.sub(sa::SA_HIDDEN));
        }
    }
    S_OK
}

/// `SafeArrayGetDim(psa)`: the number of dimensions; 0 for NULL.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SafeArrayGetDim(psa: *const u8) -> u32 {
    if psa.is_null() {
        return 0;
    }
    // SAFETY: the caller's descriptor.
    u32::from(unsafe { sa_read::<u16>(psa, sa::SA_DIMS) })
}

/// The lower (`upper` false) or upper bound of dimension `dim`.
///
/// # Safety
/// `psa` is null or a descriptor; `out` is null or a `LONG`.
unsafe fn sa_bound(psa: *const u8, dim: u32, out: *mut i32, upper: bool) -> u32 {
    if psa.is_null() || out.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's descriptor and out-pointer.
    match sa::bound(&unsafe { sa_bounds(psa) }, dim) {
        Ok((lower, high)) => {
            // SAFETY: as above.
            unsafe { out.write_unaligned(if upper { high } else { lower }) };
            S_OK
        }
        Err(e) => e,
    }
}

/// `SafeArrayGetUBound(psa, nDim, plUbound)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SafeArrayGetUBound(psa: *const u8, dim: u32, out: *mut i32) -> u32 {
    // SAFETY: the caller's arguments.
    unsafe { sa_bound(psa, dim, out, true) }
}

/// `SafeArrayGetLBound(psa, nDim, plLbound)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SafeArrayGetLBound(psa: *const u8, dim: u32, out: *mut i32) -> u32 {
    // SAFETY: the caller's arguments.
    unsafe { sa_bound(psa, dim, out, false) }
}

/// `SafeArrayAccessData(psa, ppvData)`: "Increments the lock count of an array, and retrieves a
/// pointer to the array data" (Learn).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SafeArrayAccessData(psa: *mut u8, data: *mut *mut u8) -> u32 {
    if psa.is_null() || data.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's descriptor and out-pointer.
    unsafe {
        let locks = sa_read::<u32>(psa, sa::SA_LOCKS);
        if locks == u32::MAX {
            data.write(core::ptr::null_mut());
            return sa::E_UNEXPECTED;
        }
        psa.add(sa::SA_LOCKS).cast::<u32>().write_unaligned(locks + 1);
        data.write(sa_read::<*mut u8>(psa, sa::SA_DATA));
    }
    S_OK
}

/// `SafeArrayUnaccessData(psa)`: the lock count goes down; `E_UNEXPECTED` for an array that is
/// not locked (Learn).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SafeArrayUnaccessData(psa: *mut u8) -> u32 {
    if psa.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's descriptor.
    unsafe {
        let locks = sa_read::<u32>(psa, sa::SA_LOCKS);
        if locks == 0 {
            return sa::E_UNEXPECTED;
        }
        psa.add(sa::SA_LOCKS).cast::<u32>().write_unaligned(locks - 1);
    }
    S_OK
}

/// Where the element `indices` names is, with the element kind and size. Poly's element
/// functions take one-dimensional arrays (what `SafeArrayCreateVector` makes); an array of more
/// dimensions is `E_INVALIDARG`.
///
/// # Safety
/// `psa` is a descriptor, `indices` its indexes.
unsafe fn sa_element(psa: *const u8, indices: *const i32) -> Result<(*mut u8, Element, usize), u32> {
    // SAFETY: the caller's descriptor and index vector (one index).
    unsafe {
        let bounds = sa_bounds(psa);
        if bounds.len() != 1 {
            return Err(E_INVALIDARG);
        }
        let at = sa::position(bounds[0], indices.read_unaligned())?;
        let size = sa_read::<u32>(psa, sa::SA_ELEMENT_SIZE) as usize;
        let data = sa_read::<*mut u8>(psa, sa::SA_DATA);
        if data.is_null() {
            return Err(E_INVALIDARG);
        }
        Ok((data.add(at * size), sa::element_of(sa_read::<u16>(psa, sa::SA_FEATURES)), size))
    }
}

/// `SafeArrayGetElement(psa, rgIndices, pv)`: a copy of the element in `*pv`: a string is
/// duplicated, an interface gets `AddRef`, a variant is copied with `VariantCopy` (Learn).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SafeArrayGetElement(psa: *const u8, indices: *const i32, out: *mut u8) -> u32 {
    if psa.is_null() || indices.is_null() || out.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's array, indexes and a buffer of one element.
    unsafe {
        let (at, kind, size) = match sa_element(psa, indices) {
            Ok(e) => e,
            Err(e) => return e,
        };
        match kind {
            Element::Bits => core::ptr::copy_nonoverlapping(at, out, size),
            Element::Bstr => {
                let s = at.cast::<Bstr>().read_unaligned();
                let dup = if s.is_null() {
                    core::ptr::null_mut()
                } else {
                    let d = SysAllocStringByteLen(s.cast(), SysStringByteLen(s));
                    if d.is_null() {
                        return E_OUTOFMEMORY;
                    }
                    d
                };
                out.cast::<Bstr>().write_unaligned(dup);
            }
            Element::Interface => {
                let object = at.cast::<*mut u8>().read_unaligned();
                unknown_call(object, 1);
                out.cast::<*mut u8>().write_unaligned(object);
            }
            Element::Variant => {
                VariantInit(out);
                return VariantCopy(out, at);
            }
        }
    }
    S_OK
}

/// `SafeArrayPutElement(psa, rgIndices, pv)`: the element is replaced by a copy of the value;
/// what it held is released first. Learn: `VT_DISPATCH`, `VT_UNKNOWN` and `VT_BSTR` "are
/// pointers, and do not require another level of indirection": `pv` is the string or the
/// interface itself.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SafeArrayPutElement(psa: *mut u8, indices: *const i32, value: *mut u8) -> u32 {
    if psa.is_null() || indices.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's array, indexes and value.
    unsafe {
        let (at, kind, size) = match sa_element(psa, indices) {
            Ok(e) => e,
            Err(e) => return e,
        };
        match kind {
            Element::Bits => {
                if value.is_null() {
                    return E_INVALIDARG;
                }
                core::ptr::copy(value, at, size);
            }
            Element::Bstr => {
                let s: Bstr = value.cast();
                let dup = if s.is_null() {
                    core::ptr::null_mut()
                } else {
                    let d = SysAllocStringByteLen(s.cast(), SysStringByteLen(s));
                    if d.is_null() {
                        return E_OUTOFMEMORY;
                    }
                    d
                };
                sa_release(kind, at);
                at.cast::<Bstr>().write_unaligned(dup);
            }
            Element::Interface => {
                unknown_call(value, 1);
                sa_release(kind, at);
                at.cast::<*mut u8>().write_unaligned(value);
            }
            Element::Variant => {
                if value.is_null() {
                    return E_INVALIDARG;
                }
                return VariantCopy(at, value);
            }
        }
    }
    S_OK
}

/// `SafeArrayGetVartype(psa, pvt)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SafeArrayGetVartype(psa: *const u8, vt: *mut u16) -> u32 {
    if psa.is_null() || vt.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's descriptor; an array with FADF_HAVEVARTYPE is one oleaut32 made, with
    // its type in front of the descriptor.
    unsafe {
        let features = sa_read::<u16>(psa, sa::SA_FEATURES);
        let stored =
            if features & sa::FADF_HAVEVARTYPE != 0 { psa.sub(sa::SA_HIDDEN).cast::<u32>().read() as u16 } else { 0 };
        match sa::vartype(features, stored) {
            Ok(t) => {
                vt.write_unaligned(t);
                S_OK
            }
            Err(e) => e,
        }
    }
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn oleaut32_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}

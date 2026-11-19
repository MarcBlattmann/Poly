//! More of the Automation types (ADR 0220, B2b): arrays of several dimensions
//! (`SafeArrayCreate`), copying arrays (`SafeArrayCopy`), `VariantCopyInd` and the conversions of
//! `VariantChangeType` the marshaller and `ITypeInfo::Invoke` need (numbers, booleans, strings).
//! Learn: "SafeArrayCreate", "SafeArrayCopy", "VariantCopyInd", "VariantChangeType".

use crate::com::*;
use crate::{GetProcessHeap, HeapAlloc, HeapFree, SysAllocStringLen, SysFreeString, SysStringLen, VariantClear, VariantCopy, VariantInit};
use alloc::string::String;
use alloc::vec::Vec;
use poly_kernelbase::oleaut::{VARIANT_SIZE, VARIANT_VALUE, VT_ARRAY, VT_BYREF};
use poly_kernelbase::safearray::{self as sa, Element};
use poly_com::typelib::*;

pub(crate) const DISP_E_TYPEMISMATCH: Hresult = 0x8002_0005_u32 as i32;
pub(crate) const DISP_E_OVERFLOW: Hresult = 0x8002_000A_u32 as i32;
pub(crate) const DISP_E_BADVARTYPE: Hresult = 0x8002_0008_u32 as i32;

/// A `VARIANT` as bytes (24 on x64).
pub(crate) type Variant = [u8; VARIANT_SIZE];

pub(crate) fn vt_of(v: *const u8) -> u16 {
    // SAFETY: the caller's VARIANT.
    unsafe { v.cast::<u16>().read_unaligned() }
}

pub(crate) fn value_of(v: *const u8) -> u64 {
    // SAFETY: the caller's VARIANT: 8 bytes of value at offset 8.
    unsafe { v.add(VARIANT_VALUE).cast::<u64>().read_unaligned() }
}

pub(crate) fn set(v: *mut u8, vt: u16, value: u64) {
    // SAFETY: the caller's VARIANT.
    unsafe {
        core::ptr::write_bytes(v, 0, VARIANT_SIZE);
        v.cast::<u16>().write_unaligned(vt);
        v.add(VARIANT_VALUE).cast::<u64>().write_unaligned(value);
    }
}

// ----- arrays --------------------------------------------------------------------------------------

/// `SafeArrayCreate(vt, cDims, rgsabound)`: an array of zeroed elements; the bounds are kept in
/// the order given (dimension 1 first, as `SafeArrayGetLBound` reads them).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SafeArrayCreate(vt: u16, dims: u32, bounds: *const [u32; 2]) -> *mut u8 {
    if dims == 0 || bounds.is_null() || dims > 64 {
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's `dims` bounds.
    let b: Vec<(u32, i32)> = (0..dims as usize).map(|i| unsafe { let x = *bounds.add(i); (x[0], x[1] as i32) }).collect();
    create(vt, &b)
}

/// An array of type `vt` with the bounds `(cElements, lLbound)` per dimension (NULL on failure).
pub(crate) fn create(vt: u16, bounds: &[(u32, i32)]) -> *mut u8 {
    let Some((size, _)) = sa::element(vt) else { return core::ptr::null_mut() };
    let count: u64 = bounds.iter().map(|b| u64::from(b.0)).product();
    let Some(bytes) = u32::try_from(count).ok().and_then(|c| sa::data_size(size, c)) else {
        return core::ptr::null_mut();
    };
    // SAFETY: the process heap, zeroed; the descriptor is written in full below.
    unsafe {
        let block = HeapAlloc(GetProcessHeap(), 8, sa::SA_HIDDEN + sa::SA_BOUNDS + 8 * bounds.len());
        let data = HeapAlloc(GetProcessHeap(), 8, bytes.max(1));
        if block.is_null() || data.is_null() {
            HeapFree(GetProcessHeap(), 0, block);
            HeapFree(GetProcessHeap(), 0, data);
            return core::ptr::null_mut();
        }
        block.cast::<u32>().write(u32::from(vt));
        let psa = block.add(sa::SA_HIDDEN);
        psa.add(sa::SA_DIMS).cast::<u16>().write(bounds.len() as u16);
        psa.add(sa::SA_FEATURES).cast::<u16>().write(sa::features(vt));
        psa.add(sa::SA_ELEMENT_SIZE).cast::<u32>().write(size);
        psa.add(sa::SA_DATA).cast::<*mut u8>().write(data);
        for (i, (n, lo)) in bounds.iter().enumerate() {
            psa.add(sa::SA_BOUNDS + 8 * i).cast::<u32>().write(*n);
            psa.add(sa::SA_BOUNDS + 8 * i + 4).cast::<i32>().write(*lo);
        }
        psa
    }
}

/// What an array is: its element type, bounds, element size and data.
pub(crate) struct ArrayInfo {
    pub vt: u16,
    pub bounds: Vec<(u32, i32)>,
    pub size: u32,
    pub data: *mut u8,
}

impl ArrayInfo {
    pub(crate) fn count(&self) -> usize {
        self.bounds.iter().map(|b| b.0 as usize).product()
    }
}

/// Read an array's descriptor.
///
/// # Safety
/// `psa` is a `SAFEARRAY`.
pub(crate) unsafe fn info(psa: *const u8) -> Option<ArrayInfo> {
    if psa.is_null() {
        return None;
    }
    let mut vt = 0u16;
    // SAFETY: the caller's descriptor.
    if unsafe { crate::SafeArrayGetVartype(psa, &mut vt) } != 0 {
        return None;
    }
    // SAFETY: as above.
    unsafe {
        let dims = psa.add(sa::SA_DIMS).cast::<u16>().read_unaligned() as usize;
        let bounds = (0..dims)
            .map(|d| {
                (
                    psa.add(sa::SA_BOUNDS + 8 * d).cast::<u32>().read_unaligned(),
                    psa.add(sa::SA_BOUNDS + 8 * d + 4).cast::<i32>().read_unaligned(),
                )
            })
            .collect();
        Some(ArrayInfo {
            vt,
            bounds,
            size: psa.add(sa::SA_ELEMENT_SIZE).cast::<u32>().read_unaligned(),
            data: psa.add(sa::SA_DATA).cast::<*mut u8>().read_unaligned(),
        })
    }
}

/// `SafeArrayCopy(psa, ppsaOut)`: a new array with copies of the elements.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SafeArrayCopy(psa: *const u8, out: *mut *mut u8) -> Hresult {
    if out.is_null() {
        return E_INVALIDARG;
    }
    put(out, core::ptr::null_mut());
    if psa.is_null() {
        return S_OK;
    }
    // SAFETY: the caller's descriptor.
    let Some(a) = (unsafe { info(psa) }) else { return E_INVALIDARG };
    let n = create(a.vt, &a.bounds);
    if n.is_null() {
        return E_OUTOFMEMORY;
    }
    // SAFETY: the new array's descriptor (same shape).
    let b = unsafe { info(n) }.unwrap_or_else(|| unreachable!());
    let kind = sa::element(a.vt).map_or(Element::Bits, |e| e.1);
    for i in 0..a.count() {
        // SAFETY: element `i` of both arrays.
        unsafe {
            let s = a.data.add(i * a.size as usize);
            let d = b.data.add(i * b.size as usize);
            match kind {
                Element::Bits => core::ptr::copy_nonoverlapping(s, d, a.size as usize),
                Element::Bstr => {
                    let x = s.cast::<*mut u16>().read_unaligned();
                    let c = if x.is_null() { x } else { SysAllocStringLen(x, SysStringLen(x)) };
                    d.cast::<*mut u16>().write_unaligned(c);
                }
                Element::Interface => {
                    let x = s.cast::<*mut core::ffi::c_void>().read_unaligned();
                    add_ref(x);
                    d.cast::<*mut core::ffi::c_void>().write_unaligned(x);
                }
                Element::Variant => {
                    VariantCopy(d, s);
                }
            }
        }
    }
    put(out, n);
    S_OK
}

// ----- variants ------------------------------------------------------------------------------------

/// `VariantCopyInd(dest, src)`: like `VariantCopy`, but a by-reference variant is copied as the
/// value it points to.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn VariantCopyInd(dest: *mut u8, src: *const u8) -> Hresult {
    if dest.is_null() || src.is_null() {
        return E_INVALIDARG;
    }
    let vt = vt_of(src);
    if vt & VT_BYREF == 0 {
        // SAFETY: the caller's variants.
        return unsafe { copy(dest, src) };
    }
    let base = vt & !VT_BYREF;
    let p = value_of(src) as *const u8;
    if p.is_null() {
        return E_INVALIDARG;
    }
    let mut tmp: Variant = [0; VARIANT_SIZE];
    if base == VT_VARIANT {
        // SAFETY: a pointer to a VARIANT.
        return unsafe { VariantCopyInd(dest, p) };
    }
    // SAFETY: a pointer to a value of `base`.
    let bits = unsafe {
        match base & !VT_ARRAY {
            _ if base & VT_ARRAY != 0 => p.cast::<u64>().read_unaligned(),
            VT_I1 | VT_UI1 => u64::from(p.read()),
            VT_I2 | VT_UI2 | VT_BOOL => u64::from(p.cast::<u16>().read_unaligned()),
            VT_I4 | VT_UI4 | VT_R4 | VT_ERROR | VT_INT | VT_UINT => u64::from(p.cast::<u32>().read_unaligned()),
            VT_DECIMAL => return E_NOTIMPL,
            _ => p.cast::<u64>().read_unaligned(),
        }
    };
    set(tmp.as_mut_ptr(), base, bits);
    // SAFETY: `tmp` borrows what it points to; `copy` duplicates it.
    unsafe { copy(dest, tmp.as_ptr()) }
}

/// `VariantCopy` that also copies arrays.
///
/// # Safety
/// The caller's variants.
pub(crate) unsafe fn copy(dest: *mut u8, src: *const u8) -> Hresult {
    let vt = vt_of(src);
    if vt & VT_ARRAY != 0 && vt & VT_BYREF == 0 {
        if core::ptr::eq(dest, src) {
            return S_OK;
        }
        let mut n: *mut u8 = core::ptr::null_mut();
        // SAFETY: the array the variant holds.
        let hr = unsafe { SafeArrayCopy(value_of(src) as *const u8, &mut n) };
        if hr < 0 {
            return hr;
        }
        // SAFETY: the caller's destination.
        unsafe { VariantClear(dest) };
        set(dest, vt, n as u64);
        return S_OK;
    }
    // SAFETY: the caller's variants.
    unsafe { VariantCopy(dest, src) as Hresult }
}

/// A number held by a variant of a numeric type, as `f64` and, when integral, `i128`.
fn number(vt: u16, bits: u64) -> Option<(f64, Option<i128>)> {
    let i = |v: i128| Some((v as f64, Some(v)));
    match vt {
        VT_I1 => i(i128::from(bits as u8 as i8)),
        VT_UI1 => i(i128::from(bits as u8)),
        VT_I2 => i(i128::from(bits as u16 as i16)),
        VT_UI2 => i(i128::from(bits as u16)),
        VT_I4 | VT_INT | VT_ERROR => i(i128::from(bits as u32 as i32)),
        VT_UI4 | VT_UINT => i(i128::from(bits as u32)),
        VT_I8 => i(i128::from(bits as i64)),
        VT_UI8 => i(i128::from(bits)),
        VT_BOOL => i(if bits as u16 == 0 { 0 } else { -1 }),
        VT_R4 => {
            let f = f64::from(f32::from_bits(bits as u32));
            Some((f, None))
        }
        VT_R8 | VT_DATE => Some((f64::from_bits(bits), None)),
        VT_CY => Some(((bits as i64) as f64 / 10000.0, None)),
        VT_EMPTY => i(0),
        _ => None,
    }
}

/// Round half to even (Learn: conversions to integers round "to the nearest even integer").
fn round_even(f: f64) -> f64 {
    let r = libm_round(f);
    if (f - trunc(f)).abs() == 0.5 && r % 2.0 != 0.0 { r - f.signum() } else { r }
}

fn trunc(f: f64) -> f64 {
    if f.abs() >= 4_503_599_627_370_496.0 { f } else { (f as i64) as f64 }
}

fn libm_round(f: f64) -> f64 {
    let t = trunc(f);
    if (f - t).abs() >= 0.5 { t + f.signum() } else { t }
}

/// The text of a number as `VariantChangeType` writes it (plain decimal).
fn number_text(vt: u16, bits: u64) -> Option<String> {
    let (f, i) = number(vt, bits)?;
    if vt == VT_BOOL {
        return Some(String::from(if i == Some(0) { "False" } else { "True" }));
    }
    Some(match i {
        Some(v) => alloc::format!("{v}"),
        None => {
            if f == trunc(f) && f.abs() < 1e15 {
                alloc::format!("{}", f as i64)
            } else {
                alloc::format!("{f}")
            }
        }
    })
}

/// A string's number (decimal integers and simple decimals; blanks around are allowed).
fn parse_number(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    let (neg, digits) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let (ip, fp) = digits.split_once('.').unwrap_or((digits, ""));
    if (ip.is_empty() && fp.is_empty()) || !ip.bytes().all(|c| c.is_ascii_digit()) || !fp.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let mut v = 0f64;
    for c in ip.bytes() {
        v = v * 10.0 + f64::from(c - b'0');
    }
    let mut scale = 0.1;
    for c in fp.bytes() {
        v += f64::from(c - b'0') * scale;
        scale /= 10.0;
    }
    Some(if neg { -v } else { v })
}

/// The bits of a `to` value for the number `(f, i)`; `DISP_E_OVERFLOW` out of range.
fn to_number(to: u16, f: f64, i: Option<i128>) -> Result<u64, Hresult> {
    let int = |lo: i128, hi: i128| -> Result<i128, Hresult> {
        let v = i.unwrap_or_else(|| round_even(f) as i128);
        if v < lo || v > hi { Err(DISP_E_OVERFLOW) } else { Ok(v) }
    };
    Ok(match to {
        VT_I1 => int(-128, 127)? as i8 as u8 as u64,
        VT_UI1 => int(0, 255)? as u64,
        VT_I2 => int(-32768, 32767)? as i16 as u16 as u64,
        VT_UI2 => int(0, 65535)? as u64,
        VT_I4 | VT_INT => int(i128::from(i32::MIN), i128::from(i32::MAX))? as i32 as u32 as u64,
        VT_UI4 | VT_UINT => int(0, i128::from(u32::MAX))? as u64,
        VT_I8 => int(i128::from(i64::MIN), i128::from(i64::MAX))? as i64 as u64,
        VT_UI8 => int(0, i128::from(u64::MAX))? as u64,
        VT_R4 => u64::from((f as f32).to_bits()),
        VT_R8 | VT_DATE => f.to_bits(),
        VT_CY => ((f * 10000.0) as i64) as u64,
        VT_BOOL => {
            if f != 0.0 {
                0xFFFF
            } else {
                0
            }
        }
        _ => return Err(DISP_E_TYPEMISMATCH),
    })
}

/// Convert `src` to type `to` into `dest` (a fresh variant).
///
/// # Safety
/// `src` and `dest` are variants.
pub(crate) unsafe fn change_type(dest: *mut u8, src: *const u8, to: u16) -> Hresult {
    let mut vt = vt_of(src);
    let mut bits = value_of(src);
    let mut deref: Variant = [0; VARIANT_SIZE];
    if vt & VT_BYREF != 0 {
        // SAFETY: the caller's variant; `deref` holds the value it points to.
        let hr = unsafe { VariantCopyInd(deref.as_mut_ptr(), src) };
        if hr < 0 {
            return hr;
        }
        vt = vt_of(deref.as_ptr());
        bits = value_of(deref.as_ptr());
    }
    let src_v = if vt_of(deref.as_ptr()) != 0 { deref.as_ptr() } else { src };
    let r = if vt == to || to == VT_VARIANT {
        // SAFETY: the variants.
        unsafe { copy(dest, src_v) }
    } else if to == VT_BSTR {
        let text = if vt == VT_EMPTY { Some(String::new()) } else { number_text(vt, bits) };
        match text {
            Some(t) => {
                set(dest, VT_BSTR, bstr(&t) as u64);
                S_OK
            }
            None => DISP_E_TYPEMISMATCH,
        }
    } else if vt == VT_BSTR {
        // SAFETY: the BSTR the variant holds.
        let s = unsafe { bstr_text(bits as *const u16) }.unwrap_or_default();
        let n = if to == VT_BOOL && (s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("false")) {
            Some(if s.eq_ignore_ascii_case("true") { -1.0 } else { 0.0 })
        } else {
            parse_number(&s)
        };
        match n {
            Some(f) => match to_number(to, f, if f == trunc(f) { Some(f as i128) } else { None }) {
                Ok(b) => {
                    set(dest, to, b);
                    S_OK
                }
                Err(hr) => hr,
            },
            None => DISP_E_TYPEMISMATCH,
        }
    } else if to == VT_EMPTY {
        set(dest, VT_EMPTY, 0);
        S_OK
    } else {
        match number(vt, bits) {
            Some((f, i)) => match to_number(to, f, i) {
                Ok(b) => {
                    set(dest, to, b);
                    S_OK
                }
                Err(hr) => hr,
            },
            None => DISP_E_TYPEMISMATCH,
        }
    };
    if vt_of(deref.as_ptr()) != 0 {
        // SAFETY: our copy.
        unsafe { VariantClear(deref.as_mut_ptr()) };
    }
    r
}

/// `VariantChangeType(dest, src, flags, vt)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn VariantChangeType(dest: *mut u8, src: *const u8, _flags: u16, vt: u16) -> Hresult {
    if dest.is_null() || src.is_null() {
        return E_INVALIDARG;
    }
    let mut tmp: Variant = [0; VARIANT_SIZE];
    // SAFETY: the caller's variants; the result goes through `tmp` (dest may be src).
    unsafe {
        VariantInit(tmp.as_mut_ptr());
        let hr = change_type(tmp.as_mut_ptr(), src, vt);
        if hr < 0 {
            return hr;
        }
        if !core::ptr::eq(dest as *const u8, src) {
            VariantClear(dest);
        } else {
            VariantClear(dest);
        }
        core::ptr::copy_nonoverlapping(tmp.as_ptr(), dest, VARIANT_SIZE);
    }
    S_OK
}

/// `VariantChangeTypeEx(dest, src, lcid, flags, vt)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn VariantChangeTypeEx(dest: *mut u8, src: *const u8, _lcid: u32, flags: u16, vt: u16) -> Hresult {
    // SAFETY: as above.
    unsafe { VariantChangeType(dest, src, flags, vt) }
}

/// Free what a BSTR-holding slot owns (used by the marshaller's temporaries).
pub(crate) fn free_bstr(b: *mut u16) {
    // SAFETY: a BSTR or NULL.
    unsafe { SysFreeString(b) };
}

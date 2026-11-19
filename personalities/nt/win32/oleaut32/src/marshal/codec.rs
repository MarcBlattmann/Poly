//! The values a marshalled call carries (Poly's own encoding; nothing of it is visible to
//! programs): numbers little-endian, `BSTR`s with their byte length (an odd length is kept),
//! `LPWSTR`s with their length in characters, `VARIANT`s as their type and value, `SAFEARRAY`s as
//! their element type, bounds and elements, interface pointers as the `OBJREF` that
//! `CoMarshalInterface` writes. A value is read from and written to memory laid out as the
//! parameter's type is (Learn: "VARIANT structure", "SAFEARRAY structure", "BSTR").

use crate::com::*;
use crate::variant::{self, DISP_E_BADVARTYPE};
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::ffi::c_void;
use poly_com::typelib::*;
use poly_kernelbase::oleaut::VARIANT_SIZE;

/// `RPC_X_NULL_REF_POINTER` as an `HRESULT` (sheet `CoUnmarshalInterface`, rule `lpwstr`).
pub(crate) const RPC_X_NULL_REF_POINTER: Hresult = 0x8007_06F4_u32 as i32;
/// `RPC_E_INVALID_DATA`: a reply or request that does not decode.
pub(crate) const RPC_E_INVALID_DATA: Hresult = 0x8001_010F_u32 as i32;

/// How a value travels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Ty {
    /// An integer of 1, 2, 4 or 8 bytes.
    Int(u8),
    /// A floating-point number of 4 or 8 bytes.
    Float(u8),
    Bstr,
    Lpwstr,
    Variant,
    /// A `SAFEARRAY*` of elements of this variant type.
    Array(u16),
    /// An interface pointer of this interface.
    Itf(Guid),
    /// A pointer to a value of this type.
    Ptr(Box<Ty>),
    /// Nothing Poly marshals.
    Bad,
}

impl Ty {
    /// The bytes a value takes in memory.
    pub(crate) fn size(&self) -> usize {
        match self {
            Ty::Int(n) | Ty::Float(n) => usize::from(*n),
            Ty::Variant => VARIANT_SIZE,
            _ => core::mem::size_of::<usize>(),
        }
    }
}

/// The encoding of a variant type `vt` (without `VT_BYREF`/`VT_ARRAY`).
pub(crate) fn of_vt(vt: u16) -> Ty {
    match vt {
        VT_I1 | VT_UI1 => Ty::Int(1),
        VT_I2 | VT_UI2 | VT_BOOL => Ty::Int(2),
        VT_I4 | VT_UI4 | VT_INT | VT_UINT | VT_ERROR | VT_HRESULT => Ty::Int(4),
        VT_I8 | VT_UI8 | VT_CY => Ty::Int(8),
        VT_R4 => Ty::Float(4),
        VT_R8 | VT_DATE => Ty::Float(8),
        VT_BSTR => Ty::Bstr,
        VT_LPWSTR => Ty::Lpwstr,
        VT_VARIANT => Ty::Variant,
        VT_UNKNOWN => Ty::Itf(IID_IUNKNOWN),
        VT_DISPATCH => Ty::Itf(IID_IDISPATCH),
        _ => Ty::Bad,
    }
}

/// Whether a `VARIANT` may have the type `vt` (sheet `CoUnmarshalInterface`, rule `variants`:
/// others are `DISP_E_BADVARTYPE` in the client).
pub(crate) fn variant_ok(vt: u16) -> bool {
    let base = vt & 0x0FFF;
    if vt & 0x9000 != 0 {
        return false;
    }
    let byref = vt & VT_BYREF != 0;
    let array = vt & VT_ARRAY != 0;
    match base {
        VT_EMPTY | VT_NULL => !byref && !array,
        VT_VARIANT => byref || array,
        VT_DECIMAL => !array,
        2..=11 | 13 | 16..=23 => true,
        _ => false,
    }
}

/// Bytes being written.
#[derive(Default)]
pub(crate) struct W(pub Vec<u8>);

impl W {
    pub(crate) fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    pub(crate) fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub(crate) fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub(crate) fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub(crate) fn bytes(&mut self, b: &[u8]) {
        self.u32(b.len() as u32);
        self.0.extend_from_slice(b);
    }
    pub(crate) fn guid(&mut self, g: &Guid) {
        self.u32(g.data1);
        self.u16(g.data2);
        self.u16(g.data3);
        self.0.extend_from_slice(&g.data4);
    }
    pub(crate) fn wstr(&mut self, s: &[u16]) {
        self.u32(s.len() as u32);
        for u in s {
            self.u16(*u);
        }
    }
}

/// Bytes being read.
pub(crate) struct R<'a>(pub &'a [u8]);

impl R<'_> {
    pub(crate) fn take(&mut self, n: usize) -> Result<&[u8], Hresult> {
        if self.0.len() < n {
            return Err(RPC_E_INVALID_DATA);
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    pub(crate) fn u8(&mut self) -> Result<u8, Hresult> {
        Ok(self.take(1)?[0])
    }
    pub(crate) fn u16(&mut self) -> Result<u16, Hresult> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    pub(crate) fn u32(&mut self) -> Result<u32, Hresult> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    pub(crate) fn u64(&mut self) -> Result<u64, Hresult> {
        Ok(u64::from(self.u32()?) | (u64::from(self.u32()?) << 32))
    }
    pub(crate) fn bytes(&mut self) -> Result<Vec<u8>, Hresult> {
        let n = self.u32()? as usize;
        Ok(self.take(n)?.to_vec())
    }
    pub(crate) fn guid(&mut self) -> Result<Guid, Hresult> {
        let d1 = self.u32()?;
        let d2 = self.u16()?;
        let d3 = self.u16()?;
        let d4 = self.take(8)?;
        Ok(Guid { data1: d1, data2: d2, data3: d3, data4: [d4[0], d4[1], d4[2], d4[3], d4[4], d4[5], d4[6], d4[7]] })
    }
    pub(crate) fn wstr(&mut self) -> Result<Vec<u16>, Hresult> {
        let n = self.u32()? as usize;
        let b = self.take(n * 2)?;
        Ok(b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect())
    }
}

// ----- interface pointers through streams --------------------------------------------------------

type CreateStream = unsafe extern "system" fn(*mut c_void, i32, *mut *mut c_void) -> Hresult;
type Marshal = unsafe extern "system" fn(*mut c_void, *const Guid, *mut c_void, u32, *mut c_void, u32) -> Hresult;
type Unmarshal = unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> Hresult;

fn new_stream() -> Result<*mut c_void, Hresult> {
    let f = ole32(CREATE_STREAM_ON_HGLOBAL);
    if f == 0 {
        return Err(E_UNEXPECTED);
    }
    let mut s: *mut c_void = core::ptr::null_mut();
    // SAFETY: ole32's CreateStreamOnHGlobal.
    let hr = unsafe { core::mem::transmute::<usize, CreateStream>(f)(core::ptr::null_mut(), 1, &mut s) };
    if hr < 0 || s.is_null() { Err(if hr < 0 { hr } else { E_OUTOFMEMORY }) } else { Ok(s) }
}

/// `IStream::Seek(0, origin)`; the new position.
fn seek(s: *mut c_void, origin: u32) -> u64 {
    let mut pos = 0u64;
    vcall!(s, 5, (i64, u32, *mut u64) -> Hresult, 0, origin, &mut pos);
    pos
}

/// The `OBJREF` for the interface `iid` of `p`, marshalled for another process (`NORMAL`).
pub(crate) fn marshal_itf(iid: &Guid, p: *mut c_void) -> Result<Vec<u8>, Hresult> {
    let f = ole32(CO_MARSHAL_INTERFACE);
    if f == 0 {
        return Err(E_UNEXPECTED);
    }
    let s = new_stream()?;
    // SAFETY: ole32's CoMarshalInterface with our stream and the caller's object; MSHCTX_LOCAL, NORMAL.
    let hr = unsafe { core::mem::transmute::<usize, Marshal>(f)(s, iid, p, 0, core::ptr::null_mut(), 0) };
    if hr < 0 {
        release(s);
        return Err(hr);
    }
    let n = seek(s, 1) as usize;
    seek(s, 0);
    let mut v = alloc::vec![0u8; n];
    let mut got = 0u32;
    vcall!(s, 3, (*mut u8, u32, *mut u32) -> Hresult, v.as_mut_ptr(), n as u32, &mut got);
    release(s);
    v.truncate(got as usize);
    Ok(v)
}

/// The interface `iid` from an `OBJREF`.
pub(crate) fn unmarshal_itf(iid: &Guid, data: &[u8]) -> Result<*mut c_void, Hresult> {
    let f = ole32(CO_UNMARSHAL_INTERFACE);
    if f == 0 {
        return Err(E_UNEXPECTED);
    }
    let s = new_stream()?;
    let mut put_n = 0u32;
    vcall!(s, 4, (*const u8, u32, *mut u32) -> Hresult, data.as_ptr(), data.len() as u32, &mut put_n);
    seek(s, 0);
    let mut out: *mut c_void = core::ptr::null_mut();
    // SAFETY: ole32's CoUnmarshalInterface with our stream.
    let hr = unsafe { core::mem::transmute::<usize, Unmarshal>(f)(s, iid, &mut out) };
    release(s);
    if hr < 0 { Err(hr) } else { Ok(out) }
}

// ----- values ------------------------------------------------------------------------------------

fn rd<T: Copy>(p: *const u8) -> T {
    // SAFETY: the caller's value of `T` (maybe unaligned).
    unsafe { p.cast::<T>().read_unaligned() }
}

fn wr<T>(p: *mut u8, v: T) {
    // SAFETY: the caller's storage for a `T`.
    unsafe { p.cast::<T>().write_unaligned(v) }
}

/// Encode the value of type `ty` at `p`.
pub(crate) fn enc(w: &mut W, ty: &Ty, p: *const u8) -> Result<(), Hresult> {
    match ty {
        Ty::Int(n) | Ty::Float(n) => {
            // SAFETY: `n` bytes at `p`.
            w.0.extend_from_slice(unsafe { core::slice::from_raw_parts(p, usize::from(*n)) });
        }
        Ty::Bstr => {
            let b: *mut u16 = rd(p);
            if b.is_null() {
                w.u8(0);
            } else {
                // SAFETY: a BSTR: its byte length before it.
                let n = unsafe { crate::SysStringByteLen(b) } as usize;
                w.u8(1);
                // SAFETY: `n` bytes of the string.
                w.bytes(unsafe { core::slice::from_raw_parts(b.cast::<u8>(), n) });
            }
        }
        Ty::Lpwstr => {
            let s: *const u16 = rd(p);
            if s.is_null() {
                return Err(RPC_X_NULL_REF_POINTER);
            }
            let mut n = 0;
            // SAFETY: a NUL-terminated string.
            while unsafe { *s.add(n) } != 0 {
                n += 1;
            }
            w.u8(1);
            // SAFETY: `n` characters.
            w.wstr(unsafe { core::slice::from_raw_parts(s, n) });
        }
        Ty::Variant => enc_variant(w, p)?,
        Ty::Array(_) => enc_array(w, rd(p))?,
        Ty::Itf(iid) => {
            let x: *mut c_void = rd(p);
            if x.is_null() {
                w.u8(0);
            } else {
                let o = marshal_itf(iid, x)?;
                w.u8(1);
                w.bytes(&o);
            }
        }
        Ty::Ptr(inner) => {
            let q: *const u8 = rd(p);
            if q.is_null() {
                w.u8(0);
            } else {
                w.u8(1);
                enc(w, inner, q)?;
            }
        }
        Ty::Bad => return Err(DISP_E_BADVARTYPE),
    }
    Ok(())
}

/// Decode a value of type `ty` into the storage `p` (`ty.size()` bytes, which it then owns).
pub(crate) fn dec(r: &mut R, ty: &Ty, p: *mut u8) -> Result<(), Hresult> {
    match ty {
        Ty::Int(n) | Ty::Float(n) => {
            let b = r.take(usize::from(*n))?;
            // SAFETY: storage of `n` bytes.
            unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), p, b.len()) };
        }
        Ty::Bstr => {
            let b = if r.u8()? == 0 {
                core::ptr::null_mut()
            } else {
                let bytes = r.bytes()?;
                // SAFETY: `bytes.len()` bytes.
                let b = unsafe { crate::SysAllocStringByteLen(bytes.as_ptr(), bytes.len() as u32) };
                if b.is_null() {
                    return Err(E_OUTOFMEMORY);
                }
                b
            };
            wr(p, b);
        }
        Ty::Lpwstr => {
            let s = if r.u8()? == 0 {
                core::ptr::null_mut()
            } else {
                let units = r.wstr()?;
                let m = task_alloc((units.len() + 1) * 2) as *mut u16;
                if m.is_null() {
                    return Err(E_OUTOFMEMORY);
                }
                // SAFETY: room for the characters and the NUL.
                unsafe {
                    core::ptr::copy_nonoverlapping(units.as_ptr(), m, units.len());
                    *m.add(units.len()) = 0;
                }
                m
            };
            wr(p, s);
        }
        Ty::Variant => dec_variant(r, p)?,
        Ty::Array(_) => wr(p, dec_array(r)?),
        Ty::Itf(iid) => {
            let x = if r.u8()? == 0 { core::ptr::null_mut() } else { unmarshal_itf(iid, &r.bytes()?)? };
            wr(p, x);
        }
        Ty::Ptr(inner) => {
            let q = if r.u8()? == 0 {
                core::ptr::null_mut()
            } else {
                let q = task_alloc(inner.size().max(8)) as *mut u8;
                if q.is_null() {
                    return Err(E_OUTOFMEMORY);
                }
                // SAFETY: fresh storage.
                unsafe { core::ptr::write_bytes(q, 0, inner.size().max(8)) };
                dec(r, inner, q)?;
                q
            };
            wr(p, q);
        }
        Ty::Bad => return Err(DISP_E_BADVARTYPE),
    }
    Ok(())
}

/// Release what the value of type `ty` at `p` owns (the storage itself stays).
pub(crate) fn free(ty: &Ty, p: *mut u8) {
    match ty {
        Ty::Bstr => crate::variant::free_bstr(rd(p)),
        Ty::Lpwstr => task_free(rd(p)),
        Ty::Variant => {
            // SAFETY: a VARIANT; a by-reference one's target is ours (it was decoded).
            unsafe { free_variant(p) };
        }
        Ty::Array(_) => {
            // SAFETY: an array we decoded (or NULL).
            unsafe { crate::SafeArrayDestroy(rd(p)) };
        }
        Ty::Itf(_) => {
            release(rd(p));
        }
        Ty::Ptr(inner) => {
            let q: *mut u8 = rd(p);
            if !q.is_null() {
                free(inner, q);
                task_free(q.cast());
            }
        }
        _ => {}
    }
    // The slot is cleared so a second free does nothing.
    // SAFETY: the storage of the value.
    unsafe { core::ptr::write_bytes(p, 0, ty.size().min(8).max(1)) };
}

/// Clear a decoded variant: a by-reference variant's target was allocated by [`dec_variant`].
///
/// # Safety
/// `p` is a VARIANT this module decoded.
pub(crate) unsafe fn free_variant(p: *mut u8) {
    let vt = variant::vt_of(p);
    if vt & VT_BYREF != 0 {
        let t = vt & !VT_BYREF;
        let q = variant::value_of(p) as *mut u8;
        let inner = if t & VT_ARRAY != 0 { Ty::Array(t & !VT_ARRAY) } else { of_vt(t) };
        if !q.is_null() {
            free(&inner, q);
            task_free(q.cast());
        }
        variant::set(p, VT_EMPTY, 0);
    } else {
        // SAFETY: an owned variant.
        unsafe { crate::VariantClear(p) };
    }
}

fn value_ty(vt: u16) -> Ty {
    if vt & VT_ARRAY != 0 { Ty::Array(vt & !VT_ARRAY) } else { of_vt(vt) }
}

fn enc_variant(w: &mut W, p: *const u8) -> Result<(), Hresult> {
    let vt = variant::vt_of(p);
    if !variant_ok(vt) {
        return Err(DISP_E_BADVARTYPE);
    }
    w.u16(vt);
    let base = vt & !VT_BYREF;
    if base == VT_EMPTY || base == VT_NULL {
        return Ok(());
    }
    if vt == VT_DECIMAL {
        // SAFETY: the whole 16 bytes of a DECIMAL (it overlays the variant).
        w.0.extend_from_slice(unsafe { core::slice::from_raw_parts(p, 16) });
        return Ok(());
    }
    let value = p.wrapping_add(8);
    if vt & VT_BYREF != 0 {
        let q: *const u8 = rd(value);
        if q.is_null() {
            return Err(RPC_X_NULL_REF_POINTER);
        }
        if base == VT_VARIANT {
            return enc_variant(w, q);
        }
        if base == VT_DECIMAL {
            // SAFETY: a DECIMAL at `q`.
            w.0.extend_from_slice(unsafe { core::slice::from_raw_parts(q, 16) });
            return Ok(());
        }
        return enc(w, &value_ty(base), q);
    }
    if vt == VT_BSTR && rd::<*mut u16>(value).is_null() {
        // A NULL string in a VARIANT arrives as an empty one (sheet `CoUnmarshalInterface`, rule
        // `variants`); a BSTR parameter keeps its NULL.
        w.u8(1);
        w.bytes(&[]);
        return Ok(());
    }
    enc(w, &value_ty(vt), value)
}

fn dec_variant(r: &mut R, p: *mut u8) -> Result<(), Hresult> {
    let vt = r.u16()?;
    if !variant_ok(vt) {
        return Err(DISP_E_BADVARTYPE);
    }
    variant::set(p, VT_EMPTY, 0);
    let base = vt & !VT_BYREF;
    if base == VT_EMPTY || base == VT_NULL {
        variant::set(p, vt, 0);
        return Ok(());
    }
    if vt == VT_DECIMAL {
        let b = r.take(16)?;
        // SAFETY: the variant's 16 bytes.
        unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), p, 16) };
        return Ok(());
    }
    if vt & VT_BYREF != 0 {
        let (size, ty) = match base {
            VT_VARIANT => (VARIANT_SIZE, None),
            VT_DECIMAL => (16, None),
            _ => {
                let t = value_ty(base);
                (t.size(), Some(t))
            }
        };
        let q = task_alloc(size.max(8)) as *mut u8;
        if q.is_null() {
            return Err(E_OUTOFMEMORY);
        }
        // SAFETY: fresh storage.
        unsafe { core::ptr::write_bytes(q, 0, size.max(8)) };
        match (base, ty) {
            (VT_VARIANT, _) => dec_variant(r, q)?,
            (_, Some(t)) => dec(r, &t, q)?,
            _ => {
                let b = r.take(16)?;
                // SAFETY: 16 bytes of storage.
                unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), q, 16) };
            }
        }
        variant::set(p, vt, q as u64);
        return Ok(());
    }
    let t = value_ty(vt);
    let mut slot = [0u8; 8];
    dec(r, &t, slot.as_mut_ptr())?;
    variant::set(p, vt, u64::from_le_bytes(slot));
    Ok(())
}

fn enc_array(w: &mut W, psa: *const u8) -> Result<(), Hresult> {
    if psa.is_null() {
        w.u8(0);
        return Ok(());
    }
    // SAFETY: the caller's array.
    let Some(a) = (unsafe { variant::info(psa) }) else { return Err(DISP_E_BADVARTYPE) };
    let et = of_vt(a.vt);
    if et == Ty::Bad {
        return Err(DISP_E_BADVARTYPE);
    }
    w.u8(1);
    w.u16(a.vt);
    w.u16(a.bounds.len() as u16);
    for (n, lo) in &a.bounds {
        w.u32(*n);
        w.u32(*lo as u32);
    }
    for i in 0..a.count() {
        enc(w, &et, a.data.wrapping_add(i * a.size as usize))?;
    }
    Ok(())
}

fn dec_array(r: &mut R) -> Result<*mut u8, Hresult> {
    if r.u8()? == 0 {
        return Ok(core::ptr::null_mut());
    }
    let vt = r.u16()?;
    let dims = usize::from(r.u16()?);
    if dims == 0 || dims > 64 {
        return Err(RPC_E_INVALID_DATA);
    }
    let mut bounds = Vec::with_capacity(dims);
    for _ in 0..dims {
        bounds.push((r.u32()?, r.u32()? as i32));
    }
    let et = of_vt(vt);
    if et == Ty::Bad {
        return Err(DISP_E_BADVARTYPE);
    }
    let count: usize = bounds.iter().map(|b| b.0 as usize).product();
    if count > r.0.len() {
        return Err(RPC_E_INVALID_DATA);
    }
    let psa = variant::create(vt, &bounds);
    if psa.is_null() {
        return Err(E_OUTOFMEMORY);
    }
    // SAFETY: the array just made.
    let a = unsafe { variant::info(psa) }.ok_or(E_OUTOFMEMORY)?;
    for i in 0..count {
        dec(r, &et, a.data.wrapping_add(i * a.size as usize))?;
    }
    Ok(psa)
}

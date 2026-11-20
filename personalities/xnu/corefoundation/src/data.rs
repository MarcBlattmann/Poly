//! CFData and CFMutableData (`<CoreFoundation/CFData.h>`).

use crate::base::{self, CFIndex, Header, Ref, id};
use crate::string::CFRange;
use alloc::vec::Vec;
use core::ffi::c_void;

#[repr(C)]
pub struct Data {
    h: Header,
    bytes: Vec<u8>,
    /// Bytes the caller keeps (`CFDataCreateWithBytesNoCopy`): pointer,
    /// length and the allocator that frees them.
    outside: Option<(*const u8, usize, Ref)>,
    /// A mutable data's fixed capacity, 0 for none.
    capacity: usize,
}

impl Drop for Data {
    fn drop(&mut self) {
        if let Some((p, _, dealloc)) = self.outside
            && dealloc != crate::allocator::null_allocator()
        {
            crate::allocator::deallocate(crate::allocator::resolve(dealloc), p as *mut c_void);
        }
    }
}

fn get<'a>(cf: Ref) -> &'a Data {
    if base::type_of(cf) != id::DATA {
        base::crash("not a CFData");
    }
    // SAFETY: a data object.
    unsafe { &*(cf as *const Data) }
}

fn get_mut<'a>(cf: Ref) -> &'a mut Data {
    if !base::is_mutable(cf) {
        base::raise("NSInternalInconsistencyException", "mutating method sent to immutable object");
    }
    // SAFETY: a mutable data object; one thread.
    unsafe { &mut *(cf as *mut Data) }
}

pub fn bytes<'a>(cf: Ref) -> &'a [u8] {
    let d = get(cf);
    match d.outside {
        // SAFETY: the caller's bytes, alive as long as the object.
        Some((p, n, _)) => unsafe { core::slice::from_raw_parts(p, n) },
        None => &d.bytes,
    }
}

pub fn capacity(cf: Ref) -> usize {
    let d = get(cf);
    if d.capacity > 0 { d.capacity } else { bytes(cf).len() }
}

pub fn from_vec(alloc: Ref, v: Vec<u8>, mutable: bool) -> Ref {
    base::make(alloc, id::DATA, mutable, |h| Data { h, bytes: v, outside: None, capacity: 0 }) as Ref
}

/// # Safety
/// The count reached zero.
pub unsafe fn destroy(cf: Ref) {
    // SAFETY: a data object made here.
    unsafe { base::destroy::<Data>(cf) }
}

/// Make a mutable data own its bytes (after a no-copy start).
fn own(d: &mut Data) {
    if let Some((p, n, _)) = d.outside {
        // SAFETY: the caller's bytes.
        d.bytes = unsafe { core::slice::from_raw_parts(p, n) }.to_vec();
        d.outside = None;
    }
}

fn grow_check(d: &Data, new_len: usize) {
    if d.capacity > 0 && new_len > d.capacity {
        base::crash("CFData: past its fixed capacity");
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFDataGetTypeID() -> usize {
    id::DATA
}

/// # Safety
/// `p` holds `n` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFDataCreate(alloc: Ref, p: *const u8, n: CFIndex) -> Ref {
    let v = if n <= 0 {
        Vec::new()
    } else {
        if p.is_null() {
            base::crash("CFDataCreate: NULL bytes");
        }
        // SAFETY: the caller's contract.
        unsafe { core::slice::from_raw_parts(p, n as usize) }.to_vec()
    };
    from_vec(alloc, v, false)
}

/// # Safety
/// `p` holds `n` bytes that stay valid until the data is freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFDataCreateWithBytesNoCopy(alloc: Ref, p: *const u8, n: CFIndex, dealloc: Ref) -> Ref {
    let n = n.max(0) as usize;
    base::make(alloc, id::DATA, false, |h| Data { h, bytes: Vec::new(), outside: Some((p, n, dealloc)), capacity: 0 })
        as Ref
}

#[unsafe(no_mangle)]
pub extern "C" fn CFDataCreateCopy(alloc: Ref, d: Ref) -> Ref {
    from_vec(alloc, bytes(d).to_vec(), false)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFDataCreateMutable(alloc: Ref, capacity: CFIndex) -> Ref {
    base::make(alloc, id::DATA, true, |h| Data {
        h,
        bytes: Vec::new(),
        outside: None,
        capacity: capacity.max(0) as usize,
    }) as Ref
}

#[unsafe(no_mangle)]
pub extern "C" fn CFDataCreateMutableCopy(alloc: Ref, capacity: CFIndex, d: Ref) -> Ref {
    let v = bytes(d).to_vec();
    base::make(alloc, id::DATA, true, |h| Data { h, bytes: v, outside: None, capacity: capacity.max(0) as usize })
        as Ref
}

#[unsafe(no_mangle)]
pub extern "C" fn CFDataGetLength(d: Ref) -> CFIndex {
    bytes(d).len() as CFIndex
}

#[unsafe(no_mangle)]
pub extern "C" fn CFDataGetBytePtr(d: Ref) -> *const u8 {
    let b = bytes(d);
    if b.is_empty() { core::ptr::null() } else { b.as_ptr() }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFDataGetMutableBytePtr(d: Ref) -> *mut u8 {
    let m = get_mut(d);
    own(m);
    if m.bytes.is_empty() { core::ptr::null_mut() } else { m.bytes.as_mut_ptr() }
}

/// # Safety
/// `buf` has room for `r.length` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFDataGetBytes(d: Ref, r: CFRange, buf: *mut u8) {
    let b = bytes(d);
    if r.location < 0 || r.length < 0 || (r.location + r.length) as usize > b.len() {
        base::raise("NSRangeException", "range out of bounds");
    }
    // SAFETY: the caller's contract.
    unsafe { core::ptr::copy_nonoverlapping(b[r.location as usize..].as_ptr(), buf, r.length as usize) };
}

/// # Safety
/// `p` holds `n` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFDataAppendBytes(d: Ref, p: *const u8, n: CFIndex) {
    let m = get_mut(d);
    own(m);
    if n > 0 {
        grow_check(m, m.bytes.len() + n as usize);
        // SAFETY: the caller's contract.
        m.bytes.extend_from_slice(unsafe { core::slice::from_raw_parts(p, n as usize) });
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFDataSetLength(d: Ref, n: CFIndex) {
    let m = get_mut(d);
    own(m);
    grow_check(m, n.max(0) as usize);
    m.bytes.resize(n.max(0) as usize, 0);
}

#[unsafe(no_mangle)]
pub extern "C" fn CFDataIncreaseLength(d: Ref, n: CFIndex) {
    let len = bytes(d).len() as CFIndex;
    CFDataSetLength(d, len + n.max(0));
}

/// # Safety
/// `p` holds `n` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFDataReplaceBytes(d: Ref, r: CFRange, p: *const u8, n: CFIndex) {
    let m = get_mut(d);
    own(m);
    let len = m.bytes.len();
    if r.location < 0 || r.length < 0 || (r.location + r.length) as usize > len {
        base::raise("NSRangeException", "range out of bounds");
    }
    let new: &[u8] = if n > 0 {
        // SAFETY: the caller's contract.
        unsafe { core::slice::from_raw_parts(p, n as usize) }
    } else {
        &[]
    };
    grow_check(m, len - r.length as usize + new.len());
    let at = r.location as usize;
    m.bytes.splice(at..at + r.length as usize, new.iter().copied());
}

#[unsafe(no_mangle)]
pub extern "C" fn CFDataDeleteBytes(d: Ref, r: CFRange) {
    // SAFETY: nothing is read for an empty replacement.
    unsafe { CFDataReplaceBytes(d, r, core::ptr::null(), 0) }
}

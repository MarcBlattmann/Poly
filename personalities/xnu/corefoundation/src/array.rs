//! CFArray and CFMutableArray (`<CoreFoundation/CFArray.h>`): pointers in
//! order, kept alive and compared through the callback table the array was
//! made with.

use crate::base::{self, Boolean, CFIndex, Header, Ref, id};
use crate::string::CFRange;
use alloc::vec::Vec;
use core::ffi::c_void;

pub type RetainCb = Option<unsafe extern "C" fn(Ref, Ref) -> Ref>;
pub type ReleaseCb = Option<unsafe extern "C" fn(Ref, Ref)>;
pub type DescCb = Option<unsafe extern "C" fn(Ref) -> Ref>;
pub type EqualCb = Option<unsafe extern "C" fn(Ref, Ref) -> Boolean>;
pub type HashCb = Option<unsafe extern "C" fn(Ref) -> usize>;

/// `CFArrayCallBacks` (`<CoreFoundation/CFArray.h>`, 40 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ArrayCallBacks {
    pub version: CFIndex,
    pub retain: RetainCb,
    pub release: ReleaseCb,
    pub copy_description: DescCb,
    pub equal: EqualCb,
}

/// The retain of the standard tables: `CFRetain`.
pub unsafe extern "C" fn type_retain(_alloc: Ref, v: Ref) -> Ref {
    base::retain(v)
}

/// The release of the standard tables: `CFRelease`.
pub unsafe extern "C" fn type_release(_alloc: Ref, v: Ref) {
    base::release(v)
}

pub const TYPE_CALLBACKS: ArrayCallBacks = ArrayCallBacks {
    version: 0,
    retain: Some(type_retain),
    release: Some(type_release),
    copy_description: Some(base::CFCopyDescription),
    equal: Some(base::CFEqual),
};

pub const NO_CALLBACKS: ArrayCallBacks =
    ArrayCallBacks { version: 0, retain: None, release: None, copy_description: None, equal: None };

#[unsafe(no_mangle)]
pub static kCFTypeArrayCallBacks: ArrayCallBacks = TYPE_CALLBACKS;

#[repr(C)]
pub struct Array {
    h: Header,
    cb: ArrayCallBacks,
    items: Vec<Ref>,
}

fn get<'a>(cf: Ref) -> &'a Array {
    if base::type_of(cf) != id::ARRAY {
        base::crash("not a CFArray");
    }
    // SAFETY: an array object.
    unsafe { &*(cf as *const Array) }
}

fn get_mut<'a>(cf: Ref) -> &'a mut Array {
    get(cf);
    if !base::is_mutable(cf) {
        base::raise("NSInternalInconsistencyException", "mutating method sent to immutable object");
    }
    // SAFETY: a mutable array; one thread.
    unsafe { &mut *(cf as *mut Array) }
}

pub fn items<'a>(cf: Ref) -> &'a [Ref] {
    &get(cf).items
}

pub fn count(cf: Ref) -> usize {
    get(cf).items.len()
}

fn alloc_of(cf: Ref) -> Ref {
    // SAFETY: an object made here.
    unsafe { base::header(cf) }.alloc
}

fn keep(a: &Array, alloc: Ref, v: Ref) -> Ref {
    match a.cb.retain {
        Some(r) => {
            if v.is_null() {
                base::raise("NSInvalidArgumentException", "attempt to insert nil object");
            }
            // SAFETY: the table's own function.
            unsafe { r(alloc, v) }
        }
        None => v,
    }
}

fn drop_value(cb: &ArrayCallBacks, alloc: Ref, v: Ref) {
    if let Some(r) = cb.release {
        // SAFETY: the table's own function.
        unsafe { r(alloc, v) };
    }
}

fn eq(cb: &ArrayCallBacks, a: Ref, b: Ref) -> bool {
    match cb.equal {
        // SAFETY: the table's own function.
        Some(e) => a == b || unsafe { e(a, b) } != 0,
        None => a == b,
    }
}

/// A new array of `values`, retained through `cb`.
pub fn from_refs(alloc: Ref, values: &[Ref], cb: &ArrayCallBacks, mutable: bool) -> Ref {
    let p = base::make(alloc, id::ARRAY, mutable, |h| Array { h, cb: *cb, items: Vec::with_capacity(values.len()) });
    // SAFETY: just made.
    let a = unsafe { &mut *p };
    let al = a.h.alloc;
    for &v in values {
        let k = keep(a, al, v);
        a.items.push(k);
    }
    p as Ref
}

/// # Safety
/// The count reached zero.
pub unsafe fn destroy(cf: Ref) {
    let a = get(cf);
    let al = a.h.alloc;
    for &v in &a.items {
        drop_value(&a.cb, al, v);
    }
    // SAFETY: an array made here.
    unsafe { base::destroy::<Array>(cf) }
}

pub fn equal(x: Ref, y: Ref) -> bool {
    let (a, b) = (get(x), get(y));
    a.items.len() == b.items.len() && a.items.iter().zip(&b.items).all(|(&p, &q)| eq(&a.cb, p, q))
}

/// `(\n    a,\n    b\n)`.
pub fn describe(cf: Ref) -> Vec<u16> {
    let a = get(cf);
    let mut out = base::units_of("(\n");
    for (i, &v) in a.items.iter().enumerate() {
        out.extend(base::units_of("    "));
        let text = if a.cb.copy_description.is_some() && !v.is_null() {
            base::formatting(v)
        } else {
            base::units_of(&alloc::format!("{v:p}"))
        };
        out.extend(poly_cf::text::indent_inner(&text));
        if i + 1 < a.items.len() {
            out.push(u16::from(b','));
        }
        out.push(u16::from(b'\n'));
    }
    out.push(u16::from(b')'));
    out
}

fn check_range(a: &Array, r: CFRange) -> core::ops::Range<usize> {
    if r.location < 0 || r.length < 0 || (r.location + r.length) as usize > a.items.len() {
        base::raise("NSRangeException", "range out of bounds");
    }
    r.location as usize..(r.location + r.length) as usize
}

fn callbacks(cb: *const ArrayCallBacks) -> ArrayCallBacks {
    // SAFETY: a callback table or null (the API's contract).
    if cb.is_null() { NO_CALLBACKS } else { unsafe { *cb } }
}

// ----- exports -----

#[unsafe(no_mangle)]
pub extern "C" fn CFArrayGetTypeID() -> usize {
    id::ARRAY
}

/// # Safety
/// `values` holds `n` pointers; `cb` is a table or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFArrayCreate(alloc: Ref, values: *const Ref, n: CFIndex, cb: *const ArrayCallBacks) -> Ref {
    let cb = callbacks(cb);
    let v: &[Ref] = if n <= 0 {
        &[]
    } else if values.is_null() {
        if cb.retain.is_some() {
            base::raise("NSInvalidArgumentException", "attempt to insert nil object");
        }
        base::crash("CFArrayCreate: NULL values");
    } else {
        // SAFETY: the caller's contract.
        unsafe { core::slice::from_raw_parts(values, n as usize) }
    };
    from_refs(alloc, v, &cb, false)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFArrayCreateCopy(alloc: Ref, a: Ref) -> Ref {
    let src = get(a);
    from_refs(alloc, &src.items, &src.cb, false)
}

/// # Safety
/// `cb` is a table or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFArrayCreateMutable(alloc: Ref, _capacity: CFIndex, cb: *const ArrayCallBacks) -> Ref {
    from_refs(alloc, &[], &callbacks(cb), true)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFArrayCreateMutableCopy(alloc: Ref, _capacity: CFIndex, a: Ref) -> Ref {
    let src = get(a);
    from_refs(alloc, &src.items, &src.cb, true)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFArrayGetCount(a: Ref) -> CFIndex {
    count(a) as CFIndex
}

#[unsafe(no_mangle)]
pub extern "C" fn CFArrayGetValueAtIndex(a: Ref, i: CFIndex) -> Ref {
    match usize::try_from(i).ok().and_then(|i| get(a).items.get(i)) {
        Some(&v) => v,
        None => base::raise("NSRangeException", "index beyond bounds"),
    }
}

/// # Safety
/// `buf` has room for `r.length` pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFArrayGetValues(a: Ref, r: CFRange, buf: *mut Ref) {
    let arr = get(a);
    let range = check_range(arr, r);
    // SAFETY: the caller's contract.
    unsafe { core::ptr::copy_nonoverlapping(arr.items[range].as_ptr(), buf, r.length as usize) };
}

#[unsafe(no_mangle)]
pub extern "C" fn CFArrayGetFirstIndexOfValue(a: Ref, r: CFRange, v: Ref) -> CFIndex {
    let arr = get(a);
    let range = check_range(arr, r);
    range.clone().find(|&i| eq(&arr.cb, arr.items[i], v)).map_or(-1, |i| i as CFIndex)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFArrayGetLastIndexOfValue(a: Ref, r: CFRange, v: Ref) -> CFIndex {
    let arr = get(a);
    let range = check_range(arr, r);
    range.rev().find(|&i| eq(&arr.cb, arr.items[i], v)).map_or(-1, |i| i as CFIndex)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFArrayContainsValue(a: Ref, r: CFRange, v: Ref) -> Boolean {
    (CFArrayGetFirstIndexOfValue(a, r, v) >= 0) as Boolean
}

#[unsafe(no_mangle)]
pub extern "C" fn CFArrayGetCountOfValue(a: Ref, r: CFRange, v: Ref) -> CFIndex {
    let arr = get(a);
    let range = check_range(arr, r);
    range.filter(|&i| eq(&arr.cb, arr.items[i], v)).count() as CFIndex
}

/// # Safety
/// `f` is a function the program gave.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFArrayApplyFunction(
    a: Ref,
    r: CFRange,
    f: Option<unsafe extern "C" fn(Ref, *mut c_void)>,
    ctx: *mut c_void,
) {
    let arr = get(a);
    let range = check_range(arr, r);
    let values: Vec<Ref> = arr.items[range].to_vec();
    if let Some(f) = f {
        for v in values {
            // SAFETY: the program's function.
            unsafe { f(v, ctx) };
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFArrayAppendValue(a: Ref, v: Ref) {
    let al = alloc_of(a);
    let m = get_mut(a);
    let k = keep(m, al, v);
    m.items.push(k);
}

#[unsafe(no_mangle)]
pub extern "C" fn CFArrayInsertValueAtIndex(a: Ref, i: CFIndex, v: Ref) {
    let al = alloc_of(a);
    let m = get_mut(a);
    if i < 0 || i as usize > m.items.len() {
        base::raise("NSRangeException", "index beyond bounds");
    }
    let k = keep(m, al, v);
    m.items.insert(i as usize, k);
}

#[unsafe(no_mangle)]
pub extern "C" fn CFArraySetValueAtIndex(a: Ref, i: CFIndex, v: Ref) {
    let al = alloc_of(a);
    let m = get_mut(a);
    if i < 0 || i as usize > m.items.len() {
        base::raise("NSRangeException", "index beyond bounds");
    }
    let k = keep(m, al, v);
    if i as usize == m.items.len() {
        m.items.push(k);
    } else {
        let old = core::mem::replace(&mut m.items[i as usize], k);
        let cb = m.cb;
        drop_value(&cb, al, old);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFArrayRemoveValueAtIndex(a: Ref, i: CFIndex) {
    let al = alloc_of(a);
    let m = get_mut(a);
    if i < 0 || i as usize >= m.items.len() {
        base::raise("NSRangeException", "index beyond bounds");
    }
    let old = m.items.remove(i as usize);
    let cb = m.cb;
    drop_value(&cb, al, old);
}

#[unsafe(no_mangle)]
pub extern "C" fn CFArrayRemoveAllValues(a: Ref) {
    let al = alloc_of(a);
    let m = get_mut(a);
    let old = core::mem::take(&mut m.items);
    let cb = m.cb;
    for v in old {
        drop_value(&cb, al, v);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFArrayExchangeValuesAtIndices(a: Ref, i: CFIndex, j: CFIndex) {
    let m = get_mut(a);
    let n = m.items.len();
    if i < 0 || j < 0 || i as usize >= n || j as usize >= n {
        base::raise("NSRangeException", "index beyond bounds");
    }
    m.items.swap(i as usize, j as usize);
}

#[unsafe(no_mangle)]
pub extern "C" fn CFArrayAppendArray(a: Ref, other: Ref, r: CFRange) {
    let src = get(other);
    let range = check_range(src, r);
    let values: Vec<Ref> = src.items[range].to_vec();
    for v in values {
        CFArrayAppendValue(a, v);
    }
}

/// # Safety
/// `values` holds `n` pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFArrayReplaceValues(a: Ref, r: CFRange, values: *const Ref, n: CFIndex) {
    let al = alloc_of(a);
    let m = get_mut(a);
    let range = check_range(m, r);
    let new: &[Ref] = if n > 0 {
        // SAFETY: the caller's contract.
        unsafe { core::slice::from_raw_parts(values, n as usize) }
    } else {
        &[]
    };
    let kept: Vec<Ref> = new.iter().map(|&v| keep(m, al, v)).collect();
    let old: Vec<Ref> = m.items.splice(range, kept).collect();
    let cb = m.cb;
    for v in old {
        drop_value(&cb, al, v);
    }
}

/// # Safety
/// `cmp` is a function the program gave.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFArraySortValues(
    a: Ref,
    r: CFRange,
    cmp: Option<unsafe extern "C" fn(Ref, Ref, *mut c_void) -> CFIndex>,
    ctx: *mut c_void,
) {
    let m = get_mut(a);
    let range = check_range(m, r);
    let Some(cmp) = cmp else { return };
    let mut part: Vec<Ref> = m.items[range.clone()].to_vec();
    // SAFETY: the program's comparator.
    part.sort_by(|&x, &y| unsafe { cmp(x, y, ctx) }.cmp(&0));
    m.items[range].copy_from_slice(&part);
}

/// # Safety
/// `cmp` is a function the program gave.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFArrayBSearchValues(
    a: Ref,
    r: CFRange,
    v: Ref,
    cmp: Option<unsafe extern "C" fn(Ref, Ref, *mut c_void) -> CFIndex>,
    ctx: *mut c_void,
) -> CFIndex {
    let arr = get(a);
    let range = check_range(arr, r);
    let Some(cmp) = cmp else { return r.location };
    let (mut lo, mut hi) = (range.start, range.end);
    while lo < hi {
        let mid = (lo + hi) / 2;
        // SAFETY: the program's comparator.
        if unsafe { cmp(arr.items[mid], v, ctx) } < 0 {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo as CFIndex
}

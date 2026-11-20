//! CFDictionary and CFSet, mutable and not (`<CoreFoundation/CFDictionary.h>`,
//! `<CoreFoundation/CFSet.h>`). One table serves both: a set is a table of
//! keys whose values are the keys themselves. Keys are told apart by the
//! key callbacks' hash and equal functions (by pointer without them).

use crate::array::{DescCb, EqualCb, HashCb, ReleaseCb, RetainCb, type_release, type_retain};
use crate::base::{self, Boolean, CFIndex, Header, Ref, id};
use alloc::vec::Vec;
use core::ffi::c_void;

/// `CFDictionaryKeyCallBacks` and `CFSetCallBacks` (48 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct KeyCallBacks {
    pub version: CFIndex,
    pub retain: RetainCb,
    pub release: ReleaseCb,
    pub copy_description: DescCb,
    pub equal: EqualCb,
    pub hash: HashCb,
}

/// `CFDictionaryValueCallBacks` (40 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ValueCallBacks {
    pub version: CFIndex,
    pub retain: RetainCb,
    pub release: ReleaseCb,
    pub copy_description: DescCb,
    pub equal: EqualCb,
}

/// The retain of `kCFCopyStringDictionaryKeyCallBacks`: an immutable copy.
unsafe extern "C" fn copy_string_retain(alloc: Ref, v: Ref) -> Ref {
    crate::string::CFStringCreateCopy(alloc, v)
}

pub const TYPE_KEYS: KeyCallBacks = KeyCallBacks {
    version: 0,
    retain: Some(type_retain),
    release: Some(type_release),
    copy_description: Some(base::CFCopyDescription),
    equal: Some(base::CFEqual),
    hash: Some(base::CFHash),
};
pub const TYPE_VALUES: ValueCallBacks = ValueCallBacks {
    version: 0,
    retain: Some(type_retain),
    release: Some(type_release),
    copy_description: Some(base::CFCopyDescription),
    equal: Some(base::CFEqual),
};
const COPY_STRING_KEYS: KeyCallBacks = KeyCallBacks { retain: Some(copy_string_retain), ..TYPE_KEYS };
const NO_KEYS: KeyCallBacks =
    KeyCallBacks { version: 0, retain: None, release: None, copy_description: None, equal: None, hash: None };
const NO_VALUES: ValueCallBacks =
    ValueCallBacks { version: 0, retain: None, release: None, copy_description: None, equal: None };

#[unsafe(no_mangle)]
pub static kCFTypeDictionaryKeyCallBacks: KeyCallBacks = TYPE_KEYS;
#[unsafe(no_mangle)]
pub static kCFCopyStringDictionaryKeyCallBacks: KeyCallBacks = COPY_STRING_KEYS;
#[unsafe(no_mangle)]
pub static kCFTypeDictionaryValueCallBacks: ValueCallBacks = TYPE_VALUES;
#[unsafe(no_mangle)]
pub static kCFTypeSetCallBacks: KeyCallBacks = TYPE_KEYS;
#[unsafe(no_mangle)]
pub static kCFCopyStringSetCallBacks: KeyCallBacks = COPY_STRING_KEYS;

#[repr(C)]
pub struct Table {
    h: Header,
    kcb: KeyCallBacks,
    vcb: ValueCallBacks,
    keys: Vec<Ref>,
    values: Vec<Ref>,
    hashes: Vec<u64>,
}

fn is_table(cf: Ref) -> bool {
    matches!(base::type_of(cf), id::DICTIONARY | id::SET)
}

fn get<'a>(cf: Ref) -> &'a Table {
    if !is_table(cf) {
        base::crash("not a CFDictionary or CFSet");
    }
    // SAFETY: a table object.
    unsafe { &*(cf as *const Table) }
}

fn get_mut<'a>(cf: Ref) -> &'a mut Table {
    get(cf);
    if !base::is_mutable(cf) {
        base::raise("NSInternalInconsistencyException", "mutating method sent to immutable object");
    }
    // SAFETY: a mutable table; one thread.
    unsafe { &mut *(cf as *mut Table) }
}

pub fn count(cf: Ref) -> usize {
    get(cf).keys.len()
}

fn is_set(t: &Table) -> bool {
    t.h.info & 0xffff == id::SET as u64
}

impl Table {
    fn hash_of(&self, k: Ref) -> u64 {
        match self.kcb.hash {
            // SAFETY: the table's own function.
            Some(h) => (unsafe { h(k) }) as u64,
            None => k as u64,
        }
    }

    fn find(&self, k: Ref) -> Option<usize> {
        let h = self.hash_of(k);
        (0..self.keys.len()).find(|&i| {
            self.hashes[i] == h
                && (self.keys[i] == k
                    // SAFETY: the table's own function.
                    || self.kcb.equal.is_some_and(|e| unsafe { e(self.keys[i], k) } != 0))
        })
    }

    fn values_equal(&self, a: Ref, b: Ref) -> bool {
        a == b
            // SAFETY: the table's own function.
            || self.vcb.equal.is_some_and(|e| unsafe { e(a, b) } != 0)
    }

    fn alloc(&self) -> Ref {
        self.h.alloc
    }

    fn keep_key(&self, k: Ref) -> Ref {
        match self.kcb.retain {
            Some(r) => {
                if k.is_null() {
                    base::raise("NSInvalidArgumentException", "attempt to insert nil key");
                }
                // SAFETY: the table's own function.
                unsafe { r(self.alloc(), k) }
            }
            None => k,
        }
    }

    fn keep_value(&self, v: Ref) -> Ref {
        if is_set(self) {
            return v;
        }
        match self.vcb.retain {
            Some(r) => {
                if v.is_null() {
                    base::raise("NSInvalidArgumentException", "attempt to insert nil value");
                }
                // SAFETY: the table's own function.
                unsafe { r(self.alloc(), v) }
            }
            None => v,
        }
    }

    fn drop_key(&self, k: Ref) {
        if let Some(r) = self.kcb.release {
            // SAFETY: the table's own function.
            unsafe { r(self.alloc(), k) };
        }
    }

    fn drop_value(&self, v: Ref) {
        if is_set(self) {
            return;
        }
        if let Some(r) = self.vcb.release {
            // SAFETY: the table's own function.
            unsafe { r(self.alloc(), v) };
        }
    }

    /// Add a new pair (the key is not there).
    fn insert(&mut self, k: Ref, v: Ref) {
        let h = self.hash_of(k);
        let kk = self.keep_key(k);
        let vv = if is_set(self) { kk } else { self.keep_value(v) };
        self.keys.push(kk);
        self.values.push(vv);
        self.hashes.push(h);
    }

    fn replace_value(&mut self, i: usize, v: Ref) {
        if is_set(self) {
            // A set's SetValue replaces the member itself.
            let kk = self.keep_key(v);
            let old = core::mem::replace(&mut self.keys[i], kk);
            self.values[i] = kk;
            self.hashes[i] = self.hash_of(kk);
            self.drop_key(old);
            return;
        }
        let vv = self.keep_value(v);
        let old = core::mem::replace(&mut self.values[i], vv);
        self.drop_value(old);
    }

    fn remove_at(&mut self, i: usize) {
        let k = self.keys.remove(i);
        let v = self.values.remove(i);
        self.hashes.remove(i);
        self.drop_value(v);
        self.drop_key(k);
    }
}

fn make(alloc: Ref, t: usize, mutable: bool, kcb: KeyCallBacks, vcb: ValueCallBacks) -> *mut Table {
    base::make(alloc, t, mutable, |h| Table { h, kcb, vcb, keys: Vec::new(), values: Vec::new(), hashes: Vec::new() })
}

/// # Safety
/// The count reached zero.
pub unsafe fn destroy(cf: Ref) {
    let t = get(cf);
    for i in 0..t.keys.len() {
        t.drop_value(t.values[i]);
        t.drop_key(t.keys[i]);
    }
    // SAFETY: a table made here.
    unsafe { base::destroy::<Table>(cf) }
}

pub fn equal(x: Ref, y: Ref) -> bool {
    let (a, b) = (get(x), get(y));
    if a.keys.len() != b.keys.len() {
        return false;
    }
    (0..a.keys.len()).all(|i| match b.find(a.keys[i]) {
        Some(j) => is_set(a) || a.values_equal(a.values[i], b.values[j]),
        None => false,
    })
}

fn text_of(desc: DescCb, v: Ref) -> Vec<u16> {
    if desc.is_some() && !v.is_null() { base::formatting(v) } else { base::units_of(&alloc::format!("{v:p}")) }
}

/// `{\n    key = value;\n}`.
pub fn describe(cf: Ref) -> Vec<u16> {
    let t = get(cf);
    let mut out = base::units_of("{\n");
    for i in 0..t.keys.len() {
        out.extend(base::units_of("    "));
        out.extend(poly_cf::text::indent_inner(&text_of(t.kcb.copy_description, t.keys[i])));
        out.extend(base::units_of(" = "));
        out.extend(poly_cf::text::indent_inner(&text_of(t.vcb.copy_description, t.values[i])));
        out.extend(base::units_of(";\n"));
    }
    out.push(u16::from(b'}'));
    out
}

/// `{(\n    a,\n    b\n)}`.
pub fn describe_set(cf: Ref) -> Vec<u16> {
    let t = get(cf);
    let mut out = base::units_of("{(\n");
    for (i, &k) in t.keys.iter().enumerate() {
        out.extend(base::units_of("    "));
        out.extend(poly_cf::text::indent_inner(&text_of(t.kcb.copy_description, k)));
        if i + 1 < t.keys.len() {
            out.push(u16::from(b','));
        }
        out.push(u16::from(b'\n'));
    }
    out.extend(base::units_of(")}"));
    out
}

fn key_callbacks(cb: *const KeyCallBacks) -> KeyCallBacks {
    // SAFETY: a table or null (the API's contract).
    if cb.is_null() { NO_KEYS } else { unsafe { *cb } }
}

fn value_callbacks(cb: *const ValueCallBacks) -> ValueCallBacks {
    // SAFETY: a table or null.
    if cb.is_null() { NO_VALUES } else { unsafe { *cb } }
}

/// A new dictionary of the pairs (the first of equal keys wins).
pub fn dictionary(
    alloc: Ref,
    keys: &[Ref],
    values: &[Ref],
    kcb: &KeyCallBacks,
    vcb: &ValueCallBacks,
    mutable: bool,
) -> Ref {
    let p = make(alloc, id::DICTIONARY, mutable, *kcb, *vcb);
    // SAFETY: just made.
    let t = unsafe { &mut *p };
    for (&k, &v) in keys.iter().zip(values) {
        if t.find(k).is_none() {
            t.insert(k, v);
        }
    }
    p as Ref
}

pub fn pairs(cf: Ref) -> (Vec<Ref>, Vec<Ref>) {
    let t = get(cf);
    (t.keys.clone(), t.values.clone())
}

pub fn lookup(cf: Ref, k: Ref) -> Option<Ref> {
    let t = get(cf);
    t.find(k).map(|i| t.values[i])
}

/// # Safety
/// `p` holds `n` pointers when `n` > 0.
unsafe fn list<'a>(p: *const Ref, n: CFIndex) -> &'a [Ref] {
    if n <= 0 {
        &[]
    } else if p.is_null() {
        base::crash("NULL list with a count");
    } else {
        // SAFETY: the caller's contract.
        unsafe { core::slice::from_raw_parts(p, n as usize) }
    }
}

// ----- CFDictionary -----

#[unsafe(no_mangle)]
pub extern "C" fn CFDictionaryGetTypeID() -> usize {
    id::DICTIONARY
}

/// # Safety
/// `keys` and `values` hold `n` pointers; the tables are tables or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFDictionaryCreate(
    alloc: Ref,
    keys: *const Ref,
    values: *const Ref,
    n: CFIndex,
    kcb: *const KeyCallBacks,
    vcb: *const ValueCallBacks,
) -> Ref {
    // SAFETY: the caller's contract.
    let (k, v) = unsafe { (list(keys, n), list(values, n)) };
    dictionary(alloc, k, v, &key_callbacks(kcb), &value_callbacks(vcb), false)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFDictionaryCreateCopy(alloc: Ref, d: Ref) -> Ref {
    let t = get(d);
    dictionary(alloc, &t.keys, &t.values, &t.kcb, &t.vcb, false)
}

/// # Safety
/// The tables are tables or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFDictionaryCreateMutable(
    alloc: Ref,
    _capacity: CFIndex,
    kcb: *const KeyCallBacks,
    vcb: *const ValueCallBacks,
) -> Ref {
    dictionary(alloc, &[], &[], &key_callbacks(kcb), &value_callbacks(vcb), true)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFDictionaryCreateMutableCopy(alloc: Ref, _capacity: CFIndex, d: Ref) -> Ref {
    let t = get(d);
    dictionary(alloc, &t.keys, &t.values, &t.kcb, &t.vcb, true)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFDictionaryGetCount(d: Ref) -> CFIndex {
    count(d) as CFIndex
}

#[unsafe(no_mangle)]
pub extern "C" fn CFDictionaryGetCountOfKey(d: Ref, k: Ref) -> CFIndex {
    get(d).find(k).is_some() as CFIndex
}

#[unsafe(no_mangle)]
pub extern "C" fn CFDictionaryGetCountOfValue(d: Ref, v: Ref) -> CFIndex {
    let t = get(d);
    t.values.iter().filter(|&&x| t.values_equal(x, v)).count() as CFIndex
}

#[unsafe(no_mangle)]
pub extern "C" fn CFDictionaryContainsKey(d: Ref, k: Ref) -> Boolean {
    get(d).find(k).is_some() as Boolean
}

#[unsafe(no_mangle)]
pub extern "C" fn CFDictionaryContainsValue(d: Ref, v: Ref) -> Boolean {
    (CFDictionaryGetCountOfValue(d, v) > 0) as Boolean
}

#[unsafe(no_mangle)]
pub extern "C" fn CFDictionaryGetValue(d: Ref, k: Ref) -> Ref {
    lookup(d, k).unwrap_or_default()
}

/// # Safety
/// `out` is null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFDictionaryGetValueIfPresent(d: Ref, k: Ref, out: *mut Ref) -> Boolean {
    match lookup(d, k) {
        Some(v) => {
            if !out.is_null() {
                // SAFETY: the caller's contract.
                unsafe { *out = v };
            }
            1
        }
        None => 0,
    }
}

/// # Safety
/// `keys` and `values` are null or have room for every pair.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFDictionaryGetKeysAndValues(d: Ref, keys: *mut Ref, values: *mut Ref) {
    let t = get(d);
    // SAFETY: the caller's contract.
    unsafe {
        if !keys.is_null() {
            core::ptr::copy_nonoverlapping(t.keys.as_ptr(), keys, t.keys.len());
        }
        if !values.is_null() {
            core::ptr::copy_nonoverlapping(t.values.as_ptr(), values, t.values.len());
        }
    }
}

/// # Safety
/// `f` is a function the program gave.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFDictionaryApplyFunction(
    d: Ref,
    f: Option<unsafe extern "C" fn(Ref, Ref, *mut c_void)>,
    ctx: *mut c_void,
) {
    let (k, v) = pairs(d);
    if let Some(f) = f {
        for (k, v) in k.into_iter().zip(v) {
            // SAFETY: the program's function.
            unsafe { f(k, v, ctx) };
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFDictionarySetValue(d: Ref, k: Ref, v: Ref) {
    let t = get_mut(d);
    match t.find(k) {
        Some(i) => t.replace_value(i, v),
        None => t.insert(k, v),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFDictionaryAddValue(d: Ref, k: Ref, v: Ref) {
    let t = get_mut(d);
    if t.find(k).is_none() {
        t.insert(k, v);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFDictionaryReplaceValue(d: Ref, k: Ref, v: Ref) {
    let t = get_mut(d);
    if let Some(i) = t.find(k) {
        t.replace_value(i, v);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFDictionaryRemoveValue(d: Ref, k: Ref) {
    let t = get_mut(d);
    if let Some(i) = t.find(k) {
        t.remove_at(i);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFDictionaryRemoveAllValues(d: Ref) {
    let t = get_mut(d);
    while !t.keys.is_empty() {
        t.remove_at(t.keys.len() - 1);
    }
}

// ----- CFSet -----

/// A new set of the distinct values.
pub fn set(alloc: Ref, values: &[Ref], cb: &KeyCallBacks, mutable: bool) -> Ref {
    let p = make(alloc, id::SET, mutable, *cb, NO_VALUES);
    // SAFETY: just made.
    let t = unsafe { &mut *p };
    for &v in values {
        if t.find(v).is_none() {
            t.insert(v, v);
        }
    }
    p as Ref
}

#[unsafe(no_mangle)]
pub extern "C" fn CFSetGetTypeID() -> usize {
    id::SET
}

/// # Safety
/// `values` holds `n` pointers; `cb` is a table or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFSetCreate(alloc: Ref, values: *const Ref, n: CFIndex, cb: *const KeyCallBacks) -> Ref {
    // SAFETY: the caller's contract.
    let v = unsafe { list(values, n) };
    set(alloc, v, &key_callbacks(cb), false)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFSetCreateCopy(alloc: Ref, s: Ref) -> Ref {
    let t = get(s);
    set(alloc, &t.keys, &t.kcb, false)
}

/// # Safety
/// `cb` is a table or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFSetCreateMutable(alloc: Ref, _capacity: CFIndex, cb: *const KeyCallBacks) -> Ref {
    set(alloc, &[], &key_callbacks(cb), true)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFSetCreateMutableCopy(alloc: Ref, _capacity: CFIndex, s: Ref) -> Ref {
    let t = get(s);
    set(alloc, &t.keys, &t.kcb, true)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFSetGetCount(s: Ref) -> CFIndex {
    count(s) as CFIndex
}

#[unsafe(no_mangle)]
pub extern "C" fn CFSetContainsValue(s: Ref, v: Ref) -> Boolean {
    get(s).find(v).is_some() as Boolean
}

#[unsafe(no_mangle)]
pub extern "C" fn CFSetGetCountOfValue(s: Ref, v: Ref) -> CFIndex {
    get(s).find(v).is_some() as CFIndex
}

#[unsafe(no_mangle)]
pub extern "C" fn CFSetGetValue(s: Ref, v: Ref) -> Ref {
    let t = get(s);
    t.find(v).map_or(core::ptr::null(), |i| t.keys[i])
}

/// # Safety
/// `out` is null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFSetGetValueIfPresent(s: Ref, v: Ref, out: *mut Ref) -> Boolean {
    let t = get(s);
    match t.find(v) {
        Some(i) => {
            if !out.is_null() {
                // SAFETY: the caller's contract.
                unsafe { *out = t.keys[i] };
            }
            1
        }
        None => 0,
    }
}

/// # Safety
/// `buf` has room for every member.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFSetGetValues(s: Ref, buf: *mut Ref) {
    let t = get(s);
    // SAFETY: the caller's contract.
    unsafe { core::ptr::copy_nonoverlapping(t.keys.as_ptr(), buf, t.keys.len()) };
}

/// # Safety
/// `f` is a function the program gave.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFSetApplyFunction(
    s: Ref,
    f: Option<unsafe extern "C" fn(Ref, *mut c_void)>,
    ctx: *mut c_void,
) {
    let members = get(s).keys.clone();
    if let Some(f) = f {
        for m in members {
            // SAFETY: the program's function.
            unsafe { f(m, ctx) };
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFSetAddValue(s: Ref, v: Ref) {
    let t = get_mut(s);
    if t.find(v).is_none() {
        t.insert(v, v);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFSetSetValue(s: Ref, v: Ref) {
    let t = get_mut(s);
    match t.find(v) {
        Some(i) => t.replace_value(i, v),
        None => t.insert(v, v),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFSetReplaceValue(s: Ref, v: Ref) {
    let t = get_mut(s);
    if let Some(i) = t.find(v) {
        t.replace_value(i, v);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFSetRemoveValue(s: Ref, v: Ref) {
    let t = get_mut(s);
    if let Some(i) = t.find(v) {
        t.remove_at(i);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFSetRemoveAllValues(s: Ref) {
    let t = get_mut(s);
    while !t.keys.is_empty() {
        t.remove_at(t.keys.len() - 1);
    }
}

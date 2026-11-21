//! The class, method, selector, protocol and object functions of
//! `<objc/runtime.h>` (the class_*, method_*, sel_*, object_*, objc_*
//! sheets), and the allocation entry points the compiler emits.

use crate::memory::release_obj;
use crate::msgsend::{_objc_msgForward, initialize, resolve, send0, send1, send1_bool};
use crate::{Raw, lock, sel, sys};
use alloc::vec::Vec;
use core::ffi::c_char;
use poly_objc::Mem;
use poly_objc::abi;
use poly_objc::arc::policy;

fn cstr(p: *const c_char) -> Vec<u8> {
    Raw.cstr(p as u64)
}

/// The class to `+initialize` for a lookup in `cls` (a metaclass's class).
fn init_target(cls: u64) -> u64 {
    let g = lock();
    match g.rt.class(cls) {
        Some(c) if c.meta => g.rt.class_for_meta(cls).unwrap_or(0),
        Some(_) => cls,
        None => 0,
    }
}

/// Whether instances of `cls` answer `s` (the resolver asked once).
pub(crate) fn responds(cls: u64, s: u64) -> bool {
    if cls == 0 || s == 0 {
        return false;
    }
    if lock().rt.find_method(cls, s).is_some() {
        return true;
    }
    resolve(cls, s) && lock().rt.find_method(cls, s).is_some()
}

// ---- classes -------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn objc_getClass(name: *const c_char) -> u64 {
    if name.is_null() {
        return 0;
    }
    lock().rt.class_named(&cstr(name)).unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_lookUpClass(name: *const c_char) -> u64 {
    objc_getClass(name)
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_getRequiredClass(name: *const c_char) -> u64 {
    let c = objc_getClass(name);
    if c == 0 {
        let mut m = b"objc: link error: class '".to_vec();
        m.extend_from_slice(&cstr(name));
        m.extend_from_slice(b"' not linked into application\n");
        crate::fatal(&m);
    }
    c
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_getMetaClass(name: *const c_char) -> u64 {
    let c = objc_getClass(name);
    if c == 0 { 0 } else { Raw.u64(c) }
}

/// `objc_getClassList`: the number of registered classes; up to `count`
/// of them into `buf`.
#[unsafe(no_mangle)]
pub extern "C" fn objc_getClassList(buf: *mut u64, count: i32) -> i32 {
    let list = lock().rt.class_list();
    if !buf.is_null() {
        for (i, c) in list.iter().take(count.max(0) as usize).enumerate() {
            // SAFETY: the caller's buffer holds `count` classes.
            unsafe { *buf.add(i) = *c };
        }
    }
    list.len() as i32
}

static NIL_NAME: &[u8] = b"nil\0";
static NULL_SELECTOR: &[u8] = b"<null selector>\0";

#[unsafe(no_mangle)]
pub extern "C" fn class_getName(cls: u64) -> *const c_char {
    match lock().rt.class(cls) {
        Some(c) => c.name as *const c_char,
        None => NIL_NAME.as_ptr().cast(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn class_getSuperclass(cls: u64) -> u64 {
    lock().rt.class(cls).map_or(0, |c| c.superclass)
}

#[unsafe(no_mangle)]
pub extern "C" fn class_isMetaClass(cls: u64) -> i8 {
    i8::from(lock().rt.class(cls).is_some_and(|c| c.meta))
}

#[unsafe(no_mangle)]
pub extern "C" fn class_getInstanceSize(cls: u64) -> usize {
    lock().rt.class(cls).map_or(0, |c| c.instance_size as usize)
}

#[unsafe(no_mangle)]
pub extern "C" fn class_getImageName(cls: u64) -> *const c_char {
    let g = lock();
    let Some(i) = g.rt.class(cls).and_then(|c| c.image) else { return core::ptr::null() };
    g.rt.images().get(i).map_or(core::ptr::null(), |im| im.path as *const c_char)
}

#[unsafe(no_mangle)]
pub extern "C" fn class_getInstanceMethod(cls: u64, s: u64) -> u64 {
    if cls == 0 || s == 0 {
        return 0;
    }
    if let Some(m) = lock().rt.find_method(cls, s) {
        return m;
    }
    if resolve(cls, s) {
        return lock().rt.find_method(cls, s).unwrap_or(0);
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn class_getClassMethod(cls: u64, s: u64) -> u64 {
    if cls == 0 {
        return 0;
    }
    let meta = lock().rt.class(cls).map_or(0, |c| if c.meta { cls } else { c.isa });
    class_getInstanceMethod(meta, s)
}

#[unsafe(no_mangle)]
pub extern "C" fn class_getMethodImplementation(cls: u64, s: u64) -> u64 {
    if cls == 0 || s == 0 {
        return 0;
    }
    initialize(init_target(cls));
    if let Some(imp) = lock().rt.lookup(cls, s) {
        return imp;
    }
    if resolve(cls, s)
        && let Some(imp) = lock().rt.lookup(cls, s)
    {
        return imp;
    }
    _objc_msgForward as *const () as u64
}

#[unsafe(no_mangle)]
pub extern "C" fn class_getMethodImplementation_stret(cls: u64, s: u64) -> u64 {
    let imp = class_getMethodImplementation(cls, s);
    if imp == _objc_msgForward as *const () as u64 {
        unsafe extern "C" {
            fn _objc_msgForward_stret();
        }
        return _objc_msgForward_stret as *const () as u64;
    }
    imp
}

#[unsafe(no_mangle)]
pub extern "C" fn class_respondsToSelector(cls: u64, s: u64) -> i8 {
    i8::from(responds(cls, s))
}

fn types_of(t: *const c_char, buf: &mut Vec<u8>) -> Option<&[u8]> {
    if t.is_null() {
        return None;
    }
    *buf = cstr(t);
    Some(buf)
}

#[unsafe(no_mangle)]
pub extern "C" fn class_addMethod(cls: u64, s: u64, imp: u64, types: *const c_char) -> i8 {
    if cls == 0 || s == 0 {
        return 0;
    }
    let mut buf = Vec::new();
    let t = types_of(types, &mut buf);
    i8::from(lock().rt.add_method(&mut Raw, cls, s, imp, t))
}

#[unsafe(no_mangle)]
pub extern "C" fn class_replaceMethod(cls: u64, s: u64, imp: u64, types: *const c_char) -> u64 {
    if cls == 0 || s == 0 {
        return 0;
    }
    let mut buf = Vec::new();
    let t = types_of(types, &mut buf);
    lock().rt.replace_method(&mut Raw, cls, s, imp, t)
}

/// `class_copyMethodList`: the class's own methods (categories first), in
/// a malloc'ed array the caller frees; NULL when there are none.
#[unsafe(no_mangle)]
pub extern "C" fn class_copyMethodList(cls: u64, out_count: *mut u32) -> *mut u64 {
    let list: Vec<u64> = lock().rt.class(cls).map(|c| c.methods.clone()).unwrap_or_default();
    if !out_count.is_null() {
        // SAFETY: the caller's count variable.
        unsafe { *out_count = list.len() as u32 };
    }
    if list.is_empty() {
        return core::ptr::null_mut();
    }
    // SAFETY: room for the methods and a terminating NULL.
    unsafe {
        let p: *mut u64 = sys::calloc(list.len() + 1, 8).cast();
        core::ptr::copy_nonoverlapping(list.as_ptr(), p, list.len());
        p
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn class_conformsToProtocol(cls: u64, proto: u64) -> i8 {
    i8::from(cls != 0 && proto != 0 && lock().rt.conforms(&Raw, cls, proto))
}

#[unsafe(no_mangle)]
pub extern "C" fn class_addIvar(cls: u64, name: *const c_char, size: usize, align: u8, types: *const c_char) -> i8 {
    if cls == 0 || name.is_null() {
        return 0;
    }
    let mut g = lock();
    let n = cstr(name);
    if g.rt.class(cls).is_some_and(|c| c.ivars.iter().any(|i| Raw.cstr(i.name) == n)) {
        return 0;
    }
    let mut z = n;
    z.push(0);
    let name = Raw.alloc(&z);
    let mut t = cstr(types);
    t.push(0);
    let types = Raw.alloc(&t);
    i8::from(g.rt.add_ivar(cls, name, types, size as u64, align))
}

/// `class_createInstance`: a zeroed instance of `cls` with `extra` bytes,
/// its first word the class, its count 1.
#[unsafe(no_mangle)]
pub extern "C" fn class_createInstance(cls: u64, extra: usize) -> u64 {
    if cls == 0 {
        return 0;
    }
    let size = lock().rt.class(cls).map_or(8, |c| c.instance_size).max(8) as usize + extra;
    // SAFETY: a fresh allocation of at least 8 bytes.
    unsafe {
        let p: *mut u64 = sys::calloc(1, size).cast();
        if p.is_null() {
            return 0;
        }
        *p = cls;
        p as u64
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_allocateClassPair(superclass: u64, name: *const c_char, extra: usize) -> u64 {
    let _ = extra;
    let mut z = cstr(name);
    z.push(0);
    // SAFETY: fresh zeroed class objects.
    let (cls, meta) =
        unsafe { (sys::calloc(1, abi::CLASS_SIZE as usize) as u64, sys::calloc(1, abi::CLASS_SIZE as usize) as u64) };
    let mut g = lock();
    let n = Raw.alloc(&z);
    if !g.rt.allocate_class_pair(&mut Raw, superclass, n, cls, meta) {
        drop(g);
        // SAFETY: allocated above, never published.
        unsafe {
            sys::free(cls as *mut _);
            sys::free(meta as *mut _);
        }
        return 0;
    }
    let cache = &raw const crate::nsobject::EMPTY_CACHE.0 as u64;
    Raw.write_u64(cls + abi::CLASS_CACHE, cache);
    Raw.write_u64(meta + abi::CLASS_CACHE, cache);
    cls
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_registerClassPair(cls: u64) {
    lock().rt.register_class_pair(&Raw, cls);
}

// ---- objects --------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn object_getClass(obj: u64) -> u64 {
    if obj == 0 { 0 } else { Raw.u64(obj) }
}

#[unsafe(no_mangle)]
pub extern "C" fn object_setClass(obj: u64, cls: u64) -> u64 {
    if obj == 0 {
        return 0;
    }
    let old = Raw.u64(obj);
    Raw.write_u64(obj, cls);
    old
}

#[unsafe(no_mangle)]
pub extern "C" fn object_isClass(obj: u64) -> i8 {
    i8::from(obj != 0 && lock().rt.class(obj).is_some())
}

#[unsafe(no_mangle)]
pub extern "C" fn object_getClassName(obj: u64) -> *const c_char {
    class_getName(object_getClass(obj))
}

/// `objc_destructInstance`: the `.cxx_destruct` methods of the class chain
/// (subclass first), the weak references cleared, the associations
/// released. The memory stays.
#[unsafe(no_mangle)]
pub extern "C" fn objc_destructInstance(obj: u64) -> u64 {
    if obj == 0 {
        return 0;
    }
    let destruct = sel(b".cxx_destruct");
    let mut c = Raw.u64(obj);
    let mut guard = 0;
    while c != 0 && guard < 4096 {
        let (imp, sup) = {
            let g = lock();
            let imp = g.rt.own_method(c, destruct).and_then(|h| g.rt.method(h)).map(|m| m.imp);
            (imp, g.rt.class(c).map_or(0, |k| k.superclass))
        };
        if let Some(imp) = imp.filter(|&i| i != 0) {
            // SAFETY: -.cxx_destruct is (self, _cmd).
            let f: extern "C" fn(u64, u64) = unsafe { core::mem::transmute(imp as usize) };
            f(obj, destruct);
        }
        c = sup;
        guard += 1;
    }
    let (locations, assocs) = {
        let mut g = lock();
        (g.weak.clear(obj), g.assoc.remove_all(obj))
    };
    for l in locations {
        Raw.write_u64(l, 0);
    }
    for a in assocs {
        if policy::owns(a.policy) {
            release_obj_msg(a.value);
        }
    }
    obj
}

fn release_obj_msg(obj: u64) {
    crate::memory::objc_release(obj);
}

/// `object_dispose` (and the end of NSObject's `-dealloc`).
#[unsafe(no_mangle)]
pub extern "C" fn object_dispose(obj: u64) -> u64 {
    if obj == 0 {
        return 0;
    }
    objc_destructInstance(obj);
    lock().refs.freed(obj);
    // SAFETY: an instance made by class_createInstance (calloc).
    unsafe { sys::free(obj as *mut _) };
    0
}

// ---- selectors and methods ------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn sel_registerName(name: *const c_char) -> u64 {
    if name.is_null() {
        return 0;
    }
    sel(&cstr(name))
}

#[unsafe(no_mangle)]
pub extern "C" fn sel_getUid(name: *const c_char) -> u64 {
    sel_registerName(name)
}

#[unsafe(no_mangle)]
pub extern "C" fn sel_getName(s: u64) -> *const c_char {
    if s == 0 { NULL_SELECTOR.as_ptr().cast() } else { s as *const c_char }
}

#[unsafe(no_mangle)]
pub extern "C" fn sel_isEqual(a: u64, b: u64) -> i8 {
    i8::from(a == b)
}

#[unsafe(no_mangle)]
pub extern "C" fn method_getName(m: u64) -> u64 {
    lock().rt.method(m).map_or(0, |r| r.sel)
}

#[unsafe(no_mangle)]
pub extern "C" fn method_getImplementation(m: u64) -> u64 {
    lock().rt.method(m).map_or(0, |r| r.imp)
}

static EMPTY: &[u8] = b"\0";

#[unsafe(no_mangle)]
pub extern "C" fn method_getTypeEncoding(m: u64) -> *const c_char {
    match lock().rt.method(m) {
        Some(r) if r.types != 0 => r.types as *const c_char,
        Some(_) => EMPTY.as_ptr().cast(),
        None => core::ptr::null(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn method_setImplementation(m: u64, imp: u64) -> u64 {
    lock().rt.set_imp(m, imp)
}

#[unsafe(no_mangle)]
pub extern "C" fn method_exchangeImplementations(a: u64, b: u64) {
    lock().rt.exchange(a, b);
}

// ---- instance variables ---------------------------------------------------------

/// `class_getInstanceVariable`: an `Ivar` is a record of name, type and
/// offset (three words) the runtime makes on the first query and keeps.
#[unsafe(no_mangle)]
pub extern "C" fn class_getInstanceVariable(cls: u64, name: *const c_char) -> u64 {
    if cls == 0 || name.is_null() {
        return 0;
    }
    let n = cstr(name);
    let mut g = lock();
    let Some(iv) = g.rt.find_ivar(&Raw, cls, &n) else { return 0 };
    if let Some(&h) = g.ivars.get(&(cls, iv.name)) {
        return h;
    }
    let mut rec = [0u8; 24];
    rec[0..8].copy_from_slice(&iv.name.to_le_bytes());
    rec[8..16].copy_from_slice(&iv.types.to_le_bytes());
    rec[16..24].copy_from_slice(&iv.offset.to_le_bytes());
    let h = Raw.alloc(&rec);
    g.ivars.insert((cls, iv.name), h);
    h
}

#[unsafe(no_mangle)]
pub extern "C" fn ivar_getName(iv: u64) -> *const c_char {
    if iv == 0 { core::ptr::null() } else { Raw.u64(iv) as *const c_char }
}

#[unsafe(no_mangle)]
pub extern "C" fn ivar_getTypeEncoding(iv: u64) -> *const c_char {
    if iv == 0 { core::ptr::null() } else { Raw.u64(iv + 8) as *const c_char }
}

#[unsafe(no_mangle)]
pub extern "C" fn ivar_getOffset(iv: u64) -> isize {
    if iv == 0 { 0 } else { Raw.u64(iv + 16) as isize }
}

// ---- images ------------------------------------------------------------------------

/// `objc_copyClassNamesForImage`: the names of the classes the image at
/// `path` defines, in its class list's order, in a malloc'ed array.
#[unsafe(no_mangle)]
pub extern "C" fn objc_copyClassNamesForImage(path: *const c_char, out_count: *mut u32) -> *mut u64 {
    let want = cstr(path);
    let names: Vec<u64> = {
        let g = lock();
        let image = g.rt.images().iter().position(|im| Raw.cstr(im.path) == want);
        image
            .map(|i| g.rt.classes_of_image(i).iter().filter_map(|&c| g.rt.class(c).map(|k| k.name)).collect())
            .unwrap_or_default()
    };
    if !out_count.is_null() {
        // SAFETY: the caller's count variable.
        unsafe { *out_count = names.len() as u32 };
    }
    if names.is_empty() {
        return core::ptr::null_mut();
    }
    // SAFETY: room for the names and a terminating NULL.
    unsafe {
        let p: *mut u64 = sys::calloc(names.len() + 1, 8).cast();
        core::ptr::copy_nonoverlapping(names.as_ptr(), p, names.len());
        p
    }
}

// ---- protocols --------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn objc_getProtocol(name: *const c_char) -> u64 {
    if name.is_null() {
        return 0;
    }
    lock().rt.protocol_named(&cstr(name)).unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn protocol_conformsToProtocol(p: u64, other: u64) -> i8 {
    i8::from(p != 0 && other != 0 && lock().rt.protocol_includes(&Raw, p, other))
}

#[unsafe(no_mangle)]
pub extern "C" fn protocol_getName(p: u64) -> *const c_char {
    if p == 0 { core::ptr::null() } else { Raw.u64(p + abi::PROTO_NAME) as *const c_char }
}

// ---- allocation the compiler emits ---------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn objc_alloc(cls: u64) -> u64 {
    if cls == 0 { 0 } else { send0(cls, sel(b"alloc")) }
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_allocWithZone(cls: u64) -> u64 {
    if cls == 0 { 0 } else { send1(cls, sel(b"allocWithZone:"), 0) }
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_alloc_init(cls: u64) -> u64 {
    let o = objc_alloc(cls);
    if o == 0 { 0 } else { send0(o, sel(b"init")) }
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_opt_new(cls: u64) -> u64 {
    if cls == 0 { 0 } else { send0(cls, sel(b"new")) }
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_opt_class(obj: u64) -> u64 {
    if obj == 0 { 0 } else { send0(obj, sel(b"class")) }
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_opt_self(obj: u64) -> u64 {
    obj
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_opt_isKindOfClass(obj: u64, cls: u64) -> i8 {
    i8::from(obj != 0 && send1_bool(obj, sel(b"isKindOfClass:"), cls))
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_opt_respondsToSelector(obj: u64, s: u64) -> i8 {
    i8::from(obj != 0 && send1_bool(obj, sel(b"respondsToSelector:"), s))
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_constructInstance(cls: u64, bytes: *mut u64) -> u64 {
    if cls == 0 || bytes.is_null() {
        return 0;
    }
    // SAFETY: the caller's memory, large enough for an instance.
    unsafe { *bytes = cls };
    bytes as u64
}

/// The object used by `release_obj` for its import (keeps the module
/// references in one place).
#[allow(dead_code)]
fn _uses() {
    let _ = release_obj;
}

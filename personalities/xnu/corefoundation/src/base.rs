//! CoreFoundation's objects: the header every object starts with, type
//! identifiers, reference counting, and the polymorphic functions
//! (`CFGetTypeID`, `CFEqual`, `CFHash`, `CFCopyDescription`, ...).
//!
//! Layout of an object made here:
//!
//! ```text
//!   +0   class word   a class record of this framework (the Objective-C
//!                     runtime may later put a bridged class here)
//!   +8   info         bits 0..16 the CFTypeID, bit 16 immortal, bit 17 mutable
//!   +16  retain count
//!   +24  allocator    the CFAllocator it was made with (retained), or null
//!   +32  the type's own fields
//! ```
//!
//! A `CFSTR` constant is laid out by the compiler instead (spec sheet
//! `__CFConstantStringClassReference`): its class word is the address of
//! `__CFConstantStringClassReference`, and that is how it is recognised.

use crate::string;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicU64, Ordering};

pub type Ref = *const c_void;
pub type CFIndex = isize;
pub type CFTypeID = usize;
pub type CFHashCode = usize;
pub type Boolean = u8;

/// Type identifiers (the values the spec sheet `CFGetTypeID` saw, which it
/// calls informative; any distinct, stable values would do).
pub mod id {
    pub const ALLOCATOR: usize = 2;
    pub const STRING: usize = 7;
    pub const NULL: usize = 16;
    pub const SET: usize = 17;
    pub const DICTIONARY: usize = 18;
    pub const ARRAY: usize = 19;
    pub const DATA: usize = 20;
    pub const BOOLEAN: usize = 21;
    pub const NUMBER: usize = 22;
    pub const CHARACTER_SET: usize = 25;
    pub const ERROR: usize = 27;
    pub const URL: usize = 29;
    pub const BUNDLE: usize = 31;
    pub const UUID: usize = 34;
    pub const DATE: usize = 42;
    pub const RUN_LOOP: usize = 43;
    pub const OBSERVER: usize = 45;
    pub const SOURCE: usize = 46;
    pub const TIMER: usize = 47;
    pub const LOCALE: usize = 55;
}

/// The class names `CFCopyTypeIDDescription` gives.
const NAMES: &[(usize, &str)] = &[
    (id::ALLOCATOR, "CFAllocator"),
    (id::STRING, "CFString"),
    (id::NULL, "CFNull"),
    (id::SET, "CFSet"),
    (id::DICTIONARY, "CFDictionary"),
    (id::ARRAY, "CFArray"),
    (id::DATA, "CFData"),
    (id::BOOLEAN, "CFBoolean"),
    (id::NUMBER, "CFNumber"),
    (id::CHARACTER_SET, "CFCharacterSet"),
    (id::ERROR, "CFError"),
    (id::URL, "CFURL"),
    (id::BUNDLE, "CFBundle"),
    (id::UUID, "CFUUID"),
    (id::DATE, "CFDate"),
    (id::RUN_LOOP, "CFRunLoop"),
    (id::OBSERVER, "CFRunLoopObserver"),
    (id::SOURCE, "CFRunLoopSource"),
    (id::TIMER, "CFRunLoopTimer"),
    (id::LOCALE, "CFLocale"),
];

pub fn type_name(t: usize) -> Option<&'static str> {
    NAMES.iter().find(|(i, _)| *i == t).map(|(_, n)| *n)
}

/// A class record: room for an Objective-C class structure (so the runtime
/// can make these classes later), then the type it stands for.
#[repr(C)]
pub struct Class {
    pub objc: [usize; 10],
    pub type_id: usize,
}

impl Class {
    pub const fn new(type_id: usize) -> Class {
        Class { objc: [0; 10], type_id }
    }
}

macro_rules! classes {
    ($($name:ident = $id:expr;)*) => {
        $(pub static $name: Class = Class::new($id);)*
        /// The class record of a type.
        pub fn class_of(t: usize) -> *const Class {
            match t {
                $(x if x == $id => &$name,)*
                _ => &OTHER_CLASS,
            }
        }
    };
}

classes! {
    ALLOCATOR_CLASS = id::ALLOCATOR;
    STRING_CLASS = id::STRING;
    NULL_CLASS = id::NULL;
    SET_CLASS = id::SET;
    DICTIONARY_CLASS = id::DICTIONARY;
    ARRAY_CLASS = id::ARRAY;
    DATA_CLASS = id::DATA;
    BOOLEAN_CLASS = id::BOOLEAN;
    NUMBER_CLASS = id::NUMBER;
    URL_CLASS = id::URL;
    BUNDLE_CLASS = id::BUNDLE;
    DATE_CLASS = id::DATE;
    RUN_LOOP_CLASS = id::RUN_LOOP;
    OBSERVER_CLASS = id::OBSERVER;
    SOURCE_CLASS = id::SOURCE;
    TIMER_CLASS = id::TIMER;
}
static OTHER_CLASS: Class = Class::new(0);

/// The class every `CFSTR` constant points to (exported data; the compiler
/// emits a reference to it in each constant string).
#[unsafe(no_mangle)]
pub static __CFConstantStringClassReference: Class = Class::new(id::STRING);

pub const IMMORTAL: u64 = 1 << 16;
pub const MUTABLE: u64 = 1 << 17;
/// What `CFGetRetainCount` says of an object that is never freed.
pub const IMMORTAL_COUNT: CFIndex = 0x0FFF_FFFF_FFFF_FFFF;

/// The header of every object made here.
#[repr(C)]
pub struct Header {
    pub isa: *const Class,
    pub info: u64,
    pub rc: AtomicU64,
    pub alloc: Ref,
}

// SAFETY: statics of these are immortal and immutable but for the atomic
// count, which they never use.
unsafe impl Sync for Header {}

impl Header {
    /// The header of a static object that is never counted or freed.
    pub const fn immortal(class: &'static Class, t: usize) -> Header {
        Header { isa: class, info: t as u64 | IMMORTAL, rc: AtomicU64::new(1), alloc: core::ptr::null() }
    }
}

/// A raw pointer that may live in a static.
#[repr(transparent)]
pub struct Shared<T>(pub *const T);
// SAFETY: the pointees are immortal objects or tables, never written.
unsafe impl<T> Sync for Shared<T> {}

/// Print a message and stop the process the way CoreFoundation's checks
/// do on x86-64 (an invalid instruction, SIGILL).
pub fn crash(msg: &str) -> ! {
    crate::sys::stderr(b"CoreFoundation: ");
    crate::sys::stderr(msg.as_bytes());
    crate::sys::stderr(b"\n");
    // SAFETY: `ud2` raises #UD; the process ends with SIGILL.
    unsafe { core::arch::asm!("ud2", options(noreturn)) }
}

/// What the spec sheets call raising an Objective-C exception: the reason
/// on stderr, then the process ends (there is no runtime to catch it yet).
pub fn raise(name: &str, reason: &str) -> ! {
    crate::sys::stderr(b"*** Terminating app due to uncaught exception '");
    crate::sys::stderr(name.as_bytes());
    crate::sys::stderr(b"', reason: '");
    crate::sys::stderr(reason.as_bytes());
    crate::sys::stderr(b"'\n");
    crash("uncaught exception")
}

pub fn is_constant_string(cf: Ref) -> bool {
    // SAFETY: every CF object starts with its class word.
    !cf.is_null() && unsafe { *(cf as *const *const Class) } == &raw const __CFConstantStringClassReference
}

/// The header of an object made here (not a `CFSTR` constant).
///
/// # Safety
/// `cf` is a live object of this framework.
pub unsafe fn header<'a>(cf: Ref) -> &'a Header {
    // SAFETY: the caller's contract.
    unsafe { &*(cf as *const Header) }
}

/// The type of an object.
pub fn type_of(cf: Ref) -> usize {
    if cf.is_null() {
        crash("CFGetTypeID: NULL");
    }
    if is_constant_string(cf) {
        return id::STRING;
    }
    // SAFETY: a CF object (the API's contract).
    (unsafe { header(cf) }.info & 0xffff) as usize
}

pub fn is_mutable(cf: Ref) -> bool {
    // SAFETY: a CF object made here.
    !is_constant_string(cf) && unsafe { header(cf) }.info & MUTABLE != 0
}

/// Make an object of `T` (whose first field is its [`Header`]) with
/// `alloc` (null: the default allocator); `None` if the allocator gave no
/// memory.
pub fn make<T>(alloc: Ref, t: usize, mutable: bool, build: impl FnOnce(Header) -> T) -> *mut T {
    let alloc = crate::allocator::resolve(alloc);
    let p = crate::allocator::allocate(alloc, core::mem::size_of::<T>()) as *mut T;
    if p.is_null() {
        crash("out of memory");
    }
    let h = Header {
        isa: class_of(t),
        info: t as u64 | if mutable { MUTABLE } else { 0 },
        rc: AtomicU64::new(1),
        alloc: crate::allocator::retain_for_object(alloc),
    };
    // SAFETY: fresh memory of the size of `T` from the allocator (16-byte
    // aligned, as CF allocators return).
    unsafe { p.write(build(h)) };
    p
}

/// Free an object whose count reached zero: its type's clean-up, then its
/// memory back to its allocator.
///
/// # Safety
/// `cf` is an object of `T` made by [`make`] and nobody uses it any more.
pub unsafe fn destroy<T>(cf: Ref) {
    let p = cf as *mut T;
    // SAFETY: the caller's contract.
    let alloc = unsafe { header(cf) }.alloc;
    // SAFETY: as above; the value is dropped once.
    unsafe { core::ptr::drop_in_place(p) };
    crate::allocator::deallocate(alloc, p as *mut c_void);
    crate::allocator::release_for_object(alloc);
}

/// Run the clean-up of a type and free the object.
///
/// # Safety
/// `cf`'s count reached zero.
unsafe fn finalize(cf: Ref) {
    // SAFETY: the caller's contract; each type frees its own kind.
    unsafe {
        match type_of(cf) {
            id::STRING => string::destroy(cf),
            id::ARRAY => crate::array::destroy(cf),
            id::DICTIONARY | id::SET => crate::dict::destroy(cf),
            id::DATA => crate::data::destroy(cf),
            id::NUMBER => crate::number::destroy(cf),
            id::DATE => crate::date::destroy(cf),
            id::URL => crate::url::destroy(cf),
            id::ALLOCATOR => crate::allocator::destroy(cf),
            id::BUNDLE => crate::bundle::destroy(cf),
            id::SOURCE | id::TIMER | id::OBSERVER | id::RUN_LOOP => crate::runloop::destroy(cf),
            _ => {}
        }
    }
}

pub fn retain(cf: Ref) -> Ref {
    if cf.is_null() {
        crash("CFRetain: NULL");
    }
    if is_constant_string(cf) {
        return cf;
    }
    // SAFETY: a CF object (the API's contract).
    let h = unsafe { header(cf) };
    if h.info & IMMORTAL == 0 {
        h.rc.fetch_add(1, Ordering::Relaxed);
    }
    cf
}

pub fn release(cf: Ref) {
    if cf.is_null() {
        crash("CFRelease: NULL");
    }
    if is_constant_string(cf) {
        return;
    }
    // SAFETY: a CF object (the API's contract).
    let h = unsafe { header(cf) };
    if h.info & IMMORTAL != 0 {
        return;
    }
    if h.rc.fetch_sub(1, Ordering::AcqRel) == 1 {
        // SAFETY: the last reference went.
        unsafe { finalize(cf) };
    }
}

/// Retain `cf` unless it is null.
pub fn retain_opt(cf: Ref) -> Ref {
    if !cf.is_null() {
        retain(cf);
    }
    cf
}

pub fn release_opt(cf: Ref) {
    if !cf.is_null() {
        release(cf);
    }
}

pub fn equal(a: Ref, b: Ref) -> bool {
    if a.is_null() || b.is_null() {
        crash("CFEqual: NULL");
    }
    if a == b {
        return true;
    }
    let (ta, tb) = (type_of(a), type_of(b));
    // A number equals a Boolean of the same value, either way round.
    if (ta == id::NUMBER && tb == id::BOOLEAN) || (ta == id::BOOLEAN && tb == id::NUMBER) {
        return crate::number::number_or_bool(a) == crate::number::number_or_bool(b);
    }
    if ta != tb {
        return false;
    }
    match ta {
        id::STRING => string::units(a)[..] == string::units(b)[..],
        id::NUMBER => crate::number::equal(a, b),
        id::DATA => crate::data::bytes(a) == crate::data::bytes(b),
        id::ARRAY => crate::array::equal(a, b),
        id::DICTIONARY | id::SET => crate::dict::equal(a, b),
        id::DATE => crate::date::time(a) == crate::date::time(b),
        id::URL => crate::url::equal(a, b),
        _ => false,
    }
}

pub fn hash(cf: Ref) -> u64 {
    if cf.is_null() {
        crash("CFHash: NULL");
    }
    match type_of(cf) {
        id::STRING => poly_cf::text::hash_units(&string::units(cf)),
        id::NUMBER | id::BOOLEAN => poly_cf::number::hash(crate::number::number_or_bool(cf)),
        id::DATA => poly_cf::text::hash_bytes(crate::data::bytes(cf)),
        id::ARRAY => crate::array::count(cf) as u64,
        id::DICTIONARY | id::SET => crate::dict::count(cf) as u64,
        id::DATE => crate::date::time(cf).to_bits(),
        id::URL => crate::url::hash(cf),
        id::NULL => 1984,
        _ => cf as u64 >> 4,
    }
}

/// Text helpers for descriptions.
pub fn units_of(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// `<CFType 0xADDR [0xALLOC]>` with the object's and its allocator's
/// addresses.
pub fn prefix(cf: Ref, name: &str) -> alloc::string::String {
    let alloc = if is_constant_string(cf) {
        crate::allocator::system_default()
    } else {
        // SAFETY: a CF object made here.
        let a = unsafe { header(cf) }.alloc;
        if a.is_null() { crate::allocator::system_default() } else { a }
    };
    alloc::format!("<{name} {:p} [{:p}]>", cf, alloc)
}

/// `CFCopyDescription`'s text.
pub fn describe(cf: Ref) -> Vec<u16> {
    let t = type_of(cf);
    let name = type_name(t).unwrap_or("CFType");
    match t {
        id::STRING => {
            let mut v = units_of(&prefix(cf, name));
            v.extend(units_of("{contents = \""));
            v.extend_from_slice(&string::units(cf));
            v.extend(units_of("\"}"));
            v
        }
        id::NULL => units_of(&prefix(cf, name)),
        id::BOOLEAN => units_of(&alloc::format!(
            "{}{{value = {}}}",
            prefix(cf, name),
            if crate::number::bool_value(cf) { "true" } else { "false" }
        )),
        id::NUMBER => units_of(&alloc::format!("{}{}", prefix(cf, name), crate::number::description_body(cf))),
        id::DATA => {
            let b = crate::data::bytes(cf);
            units_of(&alloc::format!(
                "{}{{length = {}, capacity = {}, bytes = {}}}",
                prefix(cf, name),
                b.len(),
                crate::data::capacity(cf),
                poly_cf::text::data_bytes(b)
            ))
        }
        id::ARRAY => crate::array::describe(cf),
        id::DICTIONARY => crate::dict::describe(cf),
        id::SET => crate::dict::describe_set(cf),
        id::DATE => units_of(&alloc::format!("{}{{time = {}}}", prefix(cf, name), crate::date::time(cf))),
        id::URL => {
            let mut v = units_of(&prefix(cf, name));
            v.extend(units_of("{string = "));
            v.extend_from_slice(&string::units(crate::url::string(cf)));
            v.extend(units_of("}"));
            v
        }
        _ => units_of(&prefix(cf, name)),
    }
}

/// The text of an object inside a collection or for `%@`: a string as
/// itself, a number as its value, a Boolean as `true`/`false`, the rest
/// as its description.
pub fn formatting(cf: Ref) -> Vec<u16> {
    match type_of(cf) {
        id::STRING => string::units(cf).to_vec(),
        id::NUMBER => units_of(&crate::number::value_text(cf)),
        id::BOOLEAN => units_of(if crate::number::bool_value(cf) { "true" } else { "false" }),
        _ => describe(cf),
    }
}

// ----- exports -----

#[unsafe(no_mangle)]
pub extern "C" fn CFRetain(cf: Ref) -> Ref {
    retain(cf)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRelease(cf: Ref) {
    release(cf)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFAutorelease(cf: Ref) -> Ref {
    // No autorelease pool yet (it comes with the Objective-C runtime): the
    // object stays alive, as if the pool were never drained.
    cf
}

#[unsafe(no_mangle)]
pub extern "C" fn CFGetRetainCount(cf: Ref) -> CFIndex {
    if cf.is_null() {
        crash("CFGetRetainCount: NULL");
    }
    if is_constant_string(cf) {
        return IMMORTAL_COUNT;
    }
    // SAFETY: a CF object.
    let h = unsafe { header(cf) };
    if h.info & IMMORTAL != 0 { IMMORTAL_COUNT } else { h.rc.load(Ordering::Relaxed) as CFIndex }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFGetTypeID(cf: Ref) -> CFTypeID {
    type_of(cf)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFEqual(a: Ref, b: Ref) -> Boolean {
    equal(a, b) as Boolean
}

#[unsafe(no_mangle)]
pub extern "C" fn CFHash(cf: Ref) -> CFHashCode {
    hash(cf) as CFHashCode
}

#[unsafe(no_mangle)]
pub extern "C" fn CFCopyDescription(cf: Ref) -> Ref {
    if cf.is_null() {
        crash("CFCopyDescription: NULL");
    }
    string::from_units(core::ptr::null(), describe(cf), false)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFCopyTypeIDDescription(t: CFTypeID) -> Ref {
    match type_name(t) {
        Some(n) => string::from_units(core::ptr::null(), units_of(n), false),
        None => core::ptr::null(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFGetAllocator(cf: Ref) -> Ref {
    if is_constant_string(cf) {
        return crate::allocator::system_default();
    }
    // SAFETY: a CF object.
    let a = unsafe { header(cf) }.alloc;
    if a.is_null() { crate::allocator::system_default() } else { a }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFShow(cf: Ref) {
    let mut out = Vec::new();
    if cf.is_null() {
        out.extend_from_slice(b"(null)");
    } else {
        let text = if type_of(cf) == id::STRING { string::units(cf).to_vec() } else { describe(cf) };
        poly_cf::encoding::to_utf8_lossy(&text, &mut out);
    }
    out.push(b'\n');
    crate::sys::stderr(&out);
}

// The type identifiers of classes v0 makes no objects of yet.
#[unsafe(no_mangle)]
pub extern "C" fn CFUUIDGetTypeID() -> CFTypeID {
    id::UUID
}
#[unsafe(no_mangle)]
pub extern "C" fn CFErrorGetTypeID() -> CFTypeID {
    id::ERROR
}
#[unsafe(no_mangle)]
pub extern "C" fn CFLocaleGetTypeID() -> CFTypeID {
    id::LOCALE
}
#[unsafe(no_mangle)]
pub extern "C" fn CFCharacterSetGetTypeID() -> CFTypeID {
    id::CHARACTER_SET
}

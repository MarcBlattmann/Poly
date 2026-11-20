//! CFNumber, CFBoolean and CFNull (`<CoreFoundation/CFNumber.h>`,
//! `<CoreFoundation/CFBase.h>`). Values and conversions are
//! `poly_cf::number`'s.

use crate::base::{self, Boolean, CFIndex, Header, Ref, Shared, id};
use core::ffi::c_void;
use poly_cf::number::{self as num, Num};

#[repr(C)]
pub struct Number {
    h: Header,
    /// The stored fixed-size type (`CFNumberGetType`).
    ty: i64,
    value: Num,
}

// SAFETY: the static numbers are never written.
unsafe impl Sync for Number {}

#[repr(C)]
pub struct BooleanObj {
    h: Header,
    value: bool,
}

// SAFETY: never written.
unsafe impl Sync for BooleanObj {}

#[repr(C)]
pub struct NullObj {
    h: Header,
}

// SAFETY: never written.
unsafe impl Sync for NullObj {}

static NAN: Number =
    Number { h: Header::immortal(&base::NUMBER_CLASS, id::NUMBER), ty: num::FLOAT64, value: Num::Float(f64::NAN) };
static POS_INF: Number =
    Number { h: Header::immortal(&base::NUMBER_CLASS, id::NUMBER), ty: num::FLOAT64, value: Num::Float(f64::INFINITY) };
static NEG_INF: Number = Number {
    h: Header::immortal(&base::NUMBER_CLASS, id::NUMBER),
    ty: num::FLOAT64,
    value: Num::Float(f64::NEG_INFINITY),
};
static TRUE: BooleanObj = BooleanObj { h: Header::immortal(&base::BOOLEAN_CLASS, id::BOOLEAN), value: true };
static FALSE: BooleanObj = BooleanObj { h: Header::immortal(&base::BOOLEAN_CLASS, id::BOOLEAN), value: false };
static NULL: NullObj = NullObj { h: Header::immortal(&base::NULL_CLASS, id::NULL) };

#[unsafe(no_mangle)]
pub static kCFNumberNaN: Shared<Number> = Shared(&NAN);
#[unsafe(no_mangle)]
pub static kCFNumberPositiveInfinity: Shared<Number> = Shared(&POS_INF);
#[unsafe(no_mangle)]
pub static kCFNumberNegativeInfinity: Shared<Number> = Shared(&NEG_INF);
#[unsafe(no_mangle)]
pub static kCFBooleanTrue: Shared<BooleanObj> = Shared(&TRUE);
#[unsafe(no_mangle)]
pub static kCFBooleanFalse: Shared<BooleanObj> = Shared(&FALSE);
#[unsafe(no_mangle)]
pub static kCFNull: Shared<NullObj> = Shared(&NULL);

pub fn boolean(v: bool) -> Ref {
    (if v { &TRUE } else { &FALSE }) as *const BooleanObj as Ref
}

fn get<'a>(cf: Ref) -> &'a Number {
    if base::type_of(cf) != id::NUMBER {
        base::crash("not a CFNumber");
    }
    // SAFETY: a number object.
    unsafe { &*(cf as *const Number) }
}

pub fn value(cf: Ref) -> Num {
    get(cf).value
}

pub fn bool_value(cf: Ref) -> bool {
    // SAFETY: a Boolean object (checked by type).
    base::type_of(cf) == id::BOOLEAN && unsafe { (*(cf as *const BooleanObj)).value }
}

/// A number's value, or a Boolean's as 1 or 0.
pub fn number_or_bool(cf: Ref) -> Num {
    if base::type_of(cf) == id::BOOLEAN { Num::Int(i64::from(bool_value(cf))) } else { value(cf) }
}

pub fn equal(a: Ref, b: Ref) -> bool {
    num::compare(value(a), value(b)) == 0
}

/// A new number of fixed-size type `ty` (or the constant NaN).
pub fn make(alloc: Ref, ty: i64, v: Num) -> Ref {
    if let Num::Float(d) = v
        && d.is_nan()
    {
        return &NAN as *const Number as Ref;
    }
    base::make(alloc, id::NUMBER, false, |h| Number { h, ty, value: v }) as Ref
}

pub fn from_i64(v: i64) -> Ref {
    make(core::ptr::null(), num::SINT64, Num::Int(v))
}

/// # Safety
/// The count reached zero.
pub unsafe fn destroy(cf: Ref) {
    // SAFETY: a number made by `make`.
    unsafe { base::destroy::<Number>(cf) }
}

/// `{value = +1, type = kCFNumberSInt32Type}`.
pub fn description_body(cf: Ref) -> alloc::string::String {
    let n = get(cf);
    let v = match n.value {
        Num::Int(i) => alloc::format!("{i:+}"),
        Num::Float(d) => {
            let digits = if n.ty == num::FLOAT32 { 10 } else { 20 };
            let mut out = alloc::vec::Vec::new();
            let spec = alloc::format!("%+.{digits}f");
            let mut slots = poly_libsystem::printf::Slots { ints: &[], doubles: &[d] };
            struct Bytes<'a>(&'a mut alloc::vec::Vec<u8>);
            impl poly_libsystem::printf::Sink for Bytes<'_> {
                fn put(&mut self, b: &[u8]) {
                    self.0.extend_from_slice(b);
                }
            }
            // SAFETY: one double in the slots; no pointers are read.
            let _ = unsafe { poly_libsystem::printf::format(spec.as_bytes(), &mut slots, &mut Bytes(&mut out)) };
            alloc::string::String::from_utf8(out).unwrap_or_default()
        }
    };
    alloc::format!("{{value = {v}, type = {}}}", num::type_name(n.ty))
}

/// The value as text (`7`, `2.5`).
pub fn value_text(cf: Ref) -> alloc::string::String {
    match value(cf) {
        Num::Int(i) => alloc::format!("{i}"),
        Num::Float(d) => poly_cf::text::short_double(d),
    }
}

// ----- exports -----

#[unsafe(no_mangle)]
pub extern "C" fn CFNumberGetTypeID() -> usize {
    id::NUMBER
}

/// # Safety
/// `p` points to a value of type `t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFNumberCreate(alloc: Ref, t: CFIndex, p: *const c_void) -> Ref {
    if p.is_null() {
        base::crash("CFNumberCreate: NULL value");
    }
    let Some(c) = num::canonical(t as i64) else { return core::ptr::null() };
    let size = num::byte_size(c);
    // SAFETY: the caller's contract: `size` bytes at `p`.
    let b = unsafe { core::slice::from_raw_parts(p as *const u8, size) };
    match num::from_bytes(c, b) {
        Some(v) => make(alloc, c, v),
        None => core::ptr::null(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFNumberGetType(n: Ref) -> CFIndex {
    get(n).ty as CFIndex
}

#[unsafe(no_mangle)]
pub extern "C" fn CFNumberGetByteSize(n: Ref) -> CFIndex {
    num::byte_size(get(n).ty) as CFIndex
}

#[unsafe(no_mangle)]
pub extern "C" fn CFNumberIsFloatType(n: Ref) -> Boolean {
    num::is_float_type(get(n).ty) as Boolean
}

/// # Safety
/// `p` is null or has room for a value of type `t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFNumberGetValue(n: Ref, t: CFIndex, p: *mut c_void) -> Boolean {
    let Some((bytes, exact)) = num::convert(value(n), t as i64) else { return 0 };
    if !p.is_null() {
        // SAFETY: the caller's contract.
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), p as *mut u8, num::byte_size(t as i64)) };
    }
    exact as Boolean
}

#[unsafe(no_mangle)]
pub extern "C" fn CFNumberCompare(a: Ref, b: Ref, _ctx: *mut c_void) -> CFIndex {
    num::compare(number_or_bool(a), number_or_bool(b)) as CFIndex
}

#[unsafe(no_mangle)]
pub extern "C" fn CFBooleanGetTypeID() -> usize {
    id::BOOLEAN
}

#[unsafe(no_mangle)]
pub extern "C" fn CFBooleanGetValue(b: Ref) -> Boolean {
    if b.is_null() {
        base::crash("CFBooleanGetValue: NULL");
    }
    bool_value(b) as Boolean
}

#[unsafe(no_mangle)]
pub extern "C" fn CFNullGetTypeID() -> usize {
    id::NULL
}

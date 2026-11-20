//! CFDate and the absolute time (`<CoreFoundation/CFDate.h>`): seconds
//! since 2001-01-01 00:00:00 UTC.

use crate::base::{self, CFIndex, Header, Ref, id};

#[repr(C)]
pub struct Date {
    h: Header,
    t: f64,
}

pub fn time(cf: Ref) -> f64 {
    if base::type_of(cf) != id::DATE {
        base::crash("not a CFDate");
    }
    // SAFETY: a date object.
    unsafe { (*(cf as *const Date)).t }
}

pub fn make(alloc: Ref, t: f64) -> Ref {
    base::make(alloc, id::DATE, false, |h| Date { h, t }) as Ref
}

/// # Safety
/// The count reached zero.
pub unsafe fn destroy(cf: Ref) {
    // SAFETY: a date made by `make`.
    unsafe { base::destroy::<Date>(cf) }
}

/// The current absolute time.
pub fn now() -> f64 {
    crate::sys::unix_time() - poly_cf::plist::SECONDS_1970_TO_2001
}

#[unsafe(no_mangle)]
pub static kCFAbsoluteTimeIntervalSince1970: f64 = poly_cf::plist::SECONDS_1970_TO_2001;
#[unsafe(no_mangle)]
pub static kCFAbsoluteTimeIntervalSince1904: f64 = 3_061_152_000.0;

#[unsafe(no_mangle)]
pub extern "C" fn CFAbsoluteTimeGetCurrent() -> f64 {
    now()
}

#[unsafe(no_mangle)]
pub extern "C" fn CFDateGetTypeID() -> usize {
    id::DATE
}

#[unsafe(no_mangle)]
pub extern "C" fn CFDateCreate(alloc: Ref, t: f64) -> Ref {
    make(alloc, t)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFDateGetAbsoluteTime(d: Ref) -> f64 {
    time(d)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFDateGetTimeIntervalSinceDate(d: Ref, other: Ref) -> f64 {
    time(d) - time(other)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFDateCompare(a: Ref, b: Ref, _ctx: *mut core::ffi::c_void) -> CFIndex {
    let (x, y) = (time(a), time(b));
    (x > y) as CFIndex - (x < y) as CFIndex
}

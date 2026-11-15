//! `LdrRegisterDllNotification` and `LdrUnregisterDllNotification` (ADR 0196, the sheets of
//! `spec-chrome-win-s2b`): callbacks the loader calls after a library has been loaded (reason 1)
//! and after it was detached and before its memory goes (reason 2), on the loading thread under
//! the loader lock, in the order of the registrations.
//!
//! The notification data is `LDR_DLL_NOTIFICATION_DATA` (`winternl.h`, Learn: "LDR_DLL_NOTIFICATION"):
//! `Flags`, `FullDllName`, `BaseDllName`, `DllBase`, `SizeOfImage`. Only real loads and unloads
//! tell: a library that was loaded already is only counted, a failing load and a data-file load
//! never reach the loader, so nothing here asks about them.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};
use poly_ldr::ModuleInfo;
use poly_ntobj::NtStatus;
use poly_rtl::string::UnicodeString;

const STATUS_INVALID_PARAMETER: NtStatus = NtStatus(0xC000_000D);
const STATUS_DLL_NOT_FOUND: NtStatus = NtStatus(0xC000_0135);

/// `LDR_DLL_NOTIFICATION_REASON_LOADED` and `..._UNLOADED`.
pub const REASON_LOADED: u32 = 1;
pub const REASON_UNLOADED: u32 = 2;

/// `LDR_DLL_NOTIFICATION_DATA`.
#[repr(C)]
struct Data {
    flags: u32,
    full_name: *const UnicodeString,
    base_name: *const UnicodeString,
    base: *mut u8,
    size_of_image: u32,
}

/// `PLDR_DLL_NOTIFICATION_FUNCTION`.
type Callback = unsafe extern "system" fn(reason: u32, data: *const Data, context: usize);

/// One registration; its address is the cookie.
struct Registration {
    callback: usize,
    context: usize,
}

struct List {
    flag: AtomicBool,
    list: UnsafeCell<Vec<*mut Registration>>,
}

// SAFETY: access is serialized by `flag`; the pointers are `Box`es this module owns.
unsafe impl Sync for List {}

static REGISTRATIONS: List = List { flag: AtomicBool::new(false), list: UnsafeCell::new(Vec::new()) };

fn with_list<R>(f: impl FnOnce(&mut Vec<*mut Registration>) -> R) -> R {
    while REGISTRATIONS.flag.swap(true, Ordering::Acquire) {
        core::hint::spin_loop();
    }
    // SAFETY: `flag` makes this the only accessor.
    let r = f(unsafe { &mut *REGISTRATIONS.list.get() });
    REGISTRATIONS.flag.store(false, Ordering::Release);
    r
}

/// `LdrRegisterDllNotification(Flags, NotificationFunction, Context, Cookie*)`: flags must be 0,
/// the function and the cookie pointer must not be NULL (`STATUS_INVALID_PARAMETER`, nothing
/// written).
pub unsafe fn register(flags: u32, callback: usize, context: usize, cookie: *mut usize) -> NtStatus {
    if flags != 0 || callback == 0 || cookie.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    let r = Box::into_raw(Box::new(Registration { callback, context }));
    with_list(|l| l.push(r));
    // SAFETY: checked non-null; the caller's variable.
    unsafe { cookie.write(r as usize) };
    NtStatus(0)
}

/// `LdrUnregisterDllNotification(Cookie)`: `STATUS_DLL_NOT_FOUND` for a cookie that is not
/// registered (NULL, made up, already unregistered).
pub fn unregister(cookie: usize) -> NtStatus {
    let found = with_list(|l| match l.iter().position(|&r| r as usize == cookie) {
        Some(i) => Some(l.remove(i)),
        None => None,
    });
    match found {
        Some(r) => {
            // SAFETY: made by `register` with `Box::into_raw` and taken off the list just now.
            drop(unsafe { Box::from_raw(r) });
            NtStatus(0)
        }
        None => STATUS_DLL_NOT_FOUND,
    }
}

/// Bases of the modules a load as a data file brought in: nobody is told about them, nor when
/// they go.
static QUIET: List2 = List2 { flag: AtomicBool::new(false), list: UnsafeCell::new(Vec::new()) };

struct List2 {
    flag: AtomicBool,
    list: UnsafeCell<Vec<u64>>,
}

// SAFETY: access is serialized by `flag`.
unsafe impl Sync for List2 {}

fn with_quiet<R>(f: impl FnOnce(&mut Vec<u64>) -> R) -> R {
    while QUIET.flag.swap(true, Ordering::Acquire) {
        core::hint::spin_loop();
    }
    // SAFETY: `flag` makes this the only accessor.
    let r = f(unsafe { &mut *QUIET.list.get() });
    QUIET.flag.store(false, Ordering::Release);
    r
}

/// `modules` were loaded as data files: no notification now or when they are unloaded.
pub fn keep_quiet(modules: &[ModuleInfo]) {
    with_quiet(|q| q.extend(modules.iter().map(|m| m.base)));
}

/// Tell every registered callback, in the order of registration, that `modules` were loaded
/// ([`REASON_LOADED`]) or are being unloaded ([`REASON_UNLOADED`]). The list is copied first, so a
/// callback may register and unregister.
pub fn notify(reason: u32, modules: &[ModuleInfo]) {
    // Modules that came in quietly go quietly (their base is forgotten with them).
    let quiet: Vec<&ModuleInfo> = if reason == REASON_UNLOADED {
        modules
            .iter()
            .filter(|m| with_quiet(|q| q.iter().position(|&b| b == m.base).map(|i| q.remove(i)).is_some()))
            .collect()
    } else {
        Vec::new()
    };
    let modules: Vec<ModuleInfo> =
        modules.iter().filter(|m| !quiet.iter().any(|q| q.base == m.base)).cloned().collect();
    let modules = &modules[..];
    let snapshot: Vec<(*mut Registration, usize, usize)> = with_list(|l| {
        // SAFETY: the list holds live registrations.
        l.iter().map(|&r| (r, unsafe { (*r).callback }, unsafe { (*r).context })).collect()
    });
    if snapshot.is_empty() {
        return;
    }
    for m in modules {
        let name = |units: &[u16]| UnicodeString {
            length: (units.len() * 2) as u16,
            maximum_length: (units.len() * 2) as u16,
            buffer: units.as_ptr() as *mut u16,
        };
        let (full, base) = (name(&m.path), name(&m.name));
        let data = Data {
            flags: 0,
            full_name: &full,
            base_name: &base,
            base: m.base as *mut u8,
            size_of_image: m.size as u32,
        };
        for &(r, callback, context) in &snapshot {
            // A callback unregistered by an earlier one is not called.
            if !with_list(|l| l.contains(&r)) {
                continue;
            }
            // SAFETY: the function was registered as a `PLDR_DLL_NOTIFICATION_FUNCTION`; `data`
            // and the strings live through the call.
            unsafe { core::mem::transmute::<usize, Callback>(callback)(reason, &data, context) };
        }
    }
}

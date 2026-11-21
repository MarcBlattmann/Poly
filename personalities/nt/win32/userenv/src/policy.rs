//! `EnterCriticalPolicySection` / `LeaveCriticalPolicySection` (sheets of the same names; Learn,
//! `userenv.h`). Poly has no Group Policy engine to hold back, so the sections are the
//! process's bookkeeping in `poly_kernelbase::policy::Sections`.

use core::sync::atomic::{AtomicBool, Ordering};
use poly_kernelbase::policy::Sections;

#[cfg_attr(target_arch = "x86", link(name = "kernelbase", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernelbase", kind = "raw-dylib"))]
unsafe extern "system" {
    fn SetLastError(error: u32);
    fn GetCurrentProcessId() -> u32;
    fn ProcessIdToSessionId(pid: u32, session: *mut u32) -> i32;
}

/// `ERROR_ACCESS_DENIED` (`winerror.h`).
const ERROR_ACCESS_DENIED: u32 = 5;

/// The process's sections, behind a spin lock (no `std` here).
struct Table {
    busy: AtomicBool,
    sections: core::cell::UnsafeCell<Sections>,
}

// SAFETY: `busy` serialises every access to `sections`.
unsafe impl Sync for Table {}

static TABLE: Table = Table { busy: AtomicBool::new(false), sections: core::cell::UnsafeCell::new(Sections::new()) };

fn with<R>(f: impl FnOnce(&mut Sections) -> R) -> R {
    while TABLE.busy.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
    // SAFETY: the lock is held.
    let r = f(unsafe { &mut *TABLE.sections.get() });
    TABLE.busy.store(false, Ordering::Release);
    r
}

/// Whether the process runs in session 0, where no user is logged on interactively (a service).
fn non_interactive() -> bool {
    let mut session = 0u32;
    // SAFETY: plain calls; `session` is a live out parameter.
    unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) != 0 && session == 0 }
}

/// `EnterCriticalPolicySection(bMachine)`: a new section value. Any non-zero flag selects the
/// machine section (rule `flag`). The user section needs an interactive session: without one it
/// is NULL with `ERROR_ACCESS_DENIED` (rule `user`).
#[unsafe(no_mangle)]
pub extern "system" fn EnterCriticalPolicySection(machine: i32) -> usize {
    if machine == 0 && non_interactive() {
        // SAFETY: a plain call.
        unsafe { SetLastError(ERROR_ACCESS_DENIED) };
        return 0;
    }
    with(Sections::enter)
}

/// `LeaveCriticalPolicySection(hSection)`: TRUE for a held section, from any thread; NULL is
/// `ERROR_INVALID_PARAMETER`, anything else `ERROR_INVALID_HANDLE`.
#[unsafe(no_mangle)]
pub extern "system" fn LeaveCriticalPolicySection(section: usize) -> i32 {
    match with(|s| s.leave(section)) {
        Ok(()) => 1,
        Err(e) => {
            // SAFETY: a plain call.
            unsafe { SetLastError(e) };
            0
        }
    }
}

//! Power requests (sheets `kernel32.dll/PowerCreateRequest`, `PowerSetRequest`,
//! `PowerClearRequest`): Chrome keeps the machine awake while a download runs. A request is an
//! unnamed event of this process (a real handle: `GetHandleInformation`, `CloseHandle` and waits
//! work on it) that [`REQUESTS`] knows as a request; the counts are
//! `poly_kernelbase::power::Requests`. `CloseHandle` forgets it, and `DuplicateHandle` refuses it
//! (as on Windows, a request handle is not duplicated).

use crate::base::*;
use crate::nt::{self, Handle};
use core::cell::UnsafeCell;
use core::sync::atomic::AtomicUsize;
use poly_kernelbase::error::*;
use poly_kernelbase::power::{self, Context, Requests};

/// The process's requests, under [`LOCK`].
struct Locked(UnsafeCell<Requests>);
// SAFETY: every access is under `LOCK`.
unsafe impl Sync for Locked {}
static REQUESTS: Locked = Locked(UnsafeCell::new(Requests::new()));
/// An `SRWLOCK` (one pointer, zero when free).
static LOCK: AtomicUsize = AtomicUsize::new(0);

fn with_requests<R>(f: impl FnOnce(&mut Requests) -> R) -> R {
    // SAFETY: a valid SRW lock word.
    unsafe { nt::RtlAcquireSRWLockExclusive(LOCK.as_ptr()) };
    // SAFETY: the lock is held, so this is the only reference.
    let r = f(unsafe { &mut *REQUESTS.0.get() });
    // SAFETY: held by this thread.
    unsafe { nt::RtlReleaseSRWLockExclusive(LOCK.as_ptr()) };
    r
}

/// Whether `h` is one of this process's power request handles.
pub fn is_request(h: Handle) -> bool {
    with_requests(|r| r.is_request(h as usize))
}

/// `CloseHandle(h)` closed `h`: it is no request any more.
pub fn closed(h: Handle) {
    with_requests(|r| r.forget(h as usize));
}

/// `EVENT_ALL_ACCESS` (`winnt.h`).
const EVENT_ALL_ACCESS: u32 = 0x001F_0003;
/// `SynchronizationEvent` (`EVENT_TYPE`, `ntdef.h`).
const SYNCHRONIZATION_EVENT: u32 = 1;

/// `PowerCreateRequest(PREASON_CONTEXT)`: a new request with nothing set, or `INVALID_HANDLE_VALUE`
/// and `ERROR_INVALID_PARAMETER` for a context it does not take.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PowerCreateRequest(context: *const u8) -> Handle {
    if context.is_null() {
        return fail(ERROR_INVALID_PARAMETER, INVALID_HANDLE_VALUE);
    }
    let ptr = core::mem::size_of::<usize>();
    // SAFETY: the caller's REASON_CONTEXT (`winnt.h`): Version 0, Flags 4, then the union at 8:
    // SimpleReasonString, or LocalizedReasonModule, LocalizedReasonId, ReasonStringCount,
    // ReasonStrings (pointers of this build's width, so the union starts at 8 in both).
    let c = unsafe {
        let version = context.cast::<u32>().read_unaligned();
        let flags = context.add(4).cast::<u32>().read_unaligned();
        let first = context.add(8).cast::<usize>().read_unaligned();
        let count = context.add(8 + ptr + 4).cast::<u32>().read_unaligned();
        let strings = context.add(8 + ptr + 8).cast::<usize>().read_unaligned();
        let simple_len = if first == 0 { None } else { wide_arg(first as *const u16).map(<[u16]>::len) };
        // The detailed form counts as given only with a module and its strings.
        Context { version, flags, simple_len, detailed_module: first != 0 && strings != 0, detailed_count: count }
    };
    if let Err(e) = power::check_context(&c) {
        return fail(e, INVALID_HANDLE_VALUE);
    }
    let mut h: Handle = 0;
    // SAFETY: valid pointers; an unnamed event.
    let st = unsafe { nt::NtCreateEvent(&mut h, EVENT_ALL_ACCESS, core::ptr::null(), SYNCHRONIZATION_EVENT, 0) };
    if !st.is_success() {
        return fail_status(st, INVALID_HANDLE_VALUE);
    }
    with_requests(|r| r.add(h as usize));
    h
}

/// `PowerSetRequest(HANDLE, POWER_REQUEST_TYPE)`.
#[unsafe(no_mangle)]
pub extern "system" fn PowerSetRequest(request: Handle, kind: u32) -> i32 {
    match with_requests(|r| r.set(request as usize, kind)) {
        Ok(()) => TRUE,
        Err(e) => fail(e, FALSE),
    }
}

/// `PowerClearRequest(HANDLE, POWER_REQUEST_TYPE)`.
#[unsafe(no_mangle)]
pub extern "system" fn PowerClearRequest(request: Handle, kind: u32) -> i32 {
    match with_requests(|r| r.clear(request as usize, kind)) {
        Ok(()) => TRUE,
        Err(e) => fail(e, FALSE),
    }
}

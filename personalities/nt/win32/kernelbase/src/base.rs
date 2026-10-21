//! What every part of `kernelbase` shares: the last error, the TEB and PEB,
//! the process heap as the allocator, the process-wide lock and state (the
//! current directory, the standard handles), and turning a Win32 path into
//! an `OBJECT_ATTRIBUTES`.

use crate::nt::{self, Handle, ObjectAttributes, UnicodeString};
use alloc::vec::Vec;
use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicPtr, Ordering};
use poly_kernelbase::path::{self, Base};
use poly_ntobj::NtStatus;
use poly_rtl::critsec::CriticalSection;

// ----- the last error -----------------------------------------------------------

/// `SetLastError`.
pub fn set_last_error(error: u32) {
    // SAFETY: an ntdll export without preconditions.
    unsafe { nt::RtlSetLastWin32Error(error) }
}

/// `GetLastError`.
pub fn last_error() -> u32 {
    // SAFETY: as above.
    unsafe { nt::RtlGetLastWin32Error() }
}

/// Set the last error from a failed `NTSTATUS` (`RtlNtStatusToDosError`).
pub fn set_status(status: NtStatus) {
    // SAFETY: as above.
    set_last_error(unsafe { nt::RtlNtStatusToDosError(status) })
}

/// Fail with `status`: set the last error, return `value`.
pub fn fail_status<T>(status: NtStatus, value: T) -> T {
    set_status(status);
    value
}

/// Fail with a Win32 `error`, returning `value`.
pub fn fail<T>(error: u32, value: T) -> T {
    set_last_error(error);
    value
}

pub const TRUE: i32 = 1;
pub const FALSE: i32 = 0;
/// `INVALID_HANDLE_VALUE`.
pub const INVALID_HANDLE_VALUE: Handle = -1;

// ----- TEB and PEB ----------------------------------------------------------------

/// The current TEB (`NtCurrentTeb`: `gs:[0x30]`, `winnt.h`).
pub fn teb() -> u64 {
    let teb: u64;
    // SAFETY: GS is the TEB in every NT thread (ADR 0063).
    unsafe { core::arch::asm!("mov {}, gs:[0x30]", out(reg) teb, options(nostack, pure, readonly)) };
    teb
}

/// The PEB (`TEB.ProcessEnvironmentBlock`, TEB + 0x60, `winternl.h`).
pub fn peb() -> u64 {
    // SAFETY: the TEB holds the PEB pointer.
    unsafe { ((teb() + 0x60) as *const u64).read() }
}

/// `RTL_USER_PROCESS_PARAMETERS` (`PEB.ProcessParameters`, PEB + 0x20).
pub fn parameters() -> u64 {
    // SAFETY: the kernel wrote the pointer (ADR 0073).
    unsafe { ((peb() + 0x20) as *const u64).read() }
}

/// A `UNICODE_STRING` in process memory as a slice (empty if unset).
///
/// # Safety
/// `at` is a valid `UNICODE_STRING` whose buffer lives as long as `'a`.
pub unsafe fn unicode_at<'a>(at: u64) -> &'a [u16] {
    // SAFETY: the caller's contract.
    unsafe {
        let len = usize::from((at as *const u16).read()) / 2;
        let buf = ((at + 8) as *const *const u16).read();
        if buf.is_null() { &[] } else { core::slice::from_raw_parts(buf, len) }
    }
}

// ----- the heap ---------------------------------------------------------------------

static PROCESS_HEAP: AtomicPtr<u8> = AtomicPtr::new(core::ptr::null_mut());

/// The process heap (`GetProcessHeap`).
pub fn process_heap() -> *mut u8 {
    let h = PROCESS_HEAP.load(Ordering::Acquire);
    if !h.is_null() {
        return h;
    }
    let mut h = core::ptr::null_mut();
    // SAFETY: room for one handle.
    unsafe { nt::RtlGetProcessHeaps(1, &mut h) };
    PROCESS_HEAP.store(h, Ordering::Release);
    h
}

/// `kernelbase`'s own allocations come from the process heap.
pub struct ProcessHeap;

// SAFETY: the NT heap is serialized and returns 16-byte aligned blocks;
// larger alignments are not used by this crate (checked).
unsafe impl GlobalAlloc for ProcessHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.align() > 16 {
            return core::ptr::null_mut();
        }
        // SAFETY: a valid heap.
        unsafe { nt::RtlAllocateHeap(process_heap(), 0, layout.size().max(1)) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, _: Layout) {
        // SAFETY: `ptr` came from `alloc`.
        unsafe { nt::RtlFreeHeap(process_heap(), 0, ptr) };
    }
    unsafe fn realloc(&self, ptr: *mut u8, _: Layout, size: usize) -> *mut u8 {
        // SAFETY: `ptr` came from `alloc`.
        unsafe { nt::RtlReAllocateHeap(process_heap(), 0, ptr, size.max(1)) }
    }
}

// ----- process-wide state -------------------------------------------------------------

/// What `kernelbase` keeps for the process, behind [`LOCK`].
pub struct State {
    /// The current directory, absolute, no trailing separator unless a
    /// root (`path::as_current_dir`).
    pub current_dir: Vec<u16>,
    /// `STD_INPUT_HANDLE`, `STD_OUTPUT_HANDLE`, `STD_ERROR_HANDLE`.
    pub std: [Handle; 3],
    /// The environment block `kernelbase` allocated last (freed when
    /// replaced); null while the kernel's original is in use.
    pub env_alloc: *mut u16,
    /// The console connection (0: no console), ADR 0080.
    pub console: Handle,
    /// The console the process started on (`AttachConsole(ATTACH_PARENT_PROCESS)`).
    pub parent_console: u32,
    /// `SetConsoleCtrlHandler`'s list.
    pub ctrl: poly_console::ctrl::CtrlHandlers,
}

struct Shared {
    lock: CriticalSection,
    state: UnsafeCell<State>,
}

// SAFETY: `state` is only touched with `lock` held (`with_state`).
unsafe impl Sync for Shared {}

static SHARED: Shared = Shared {
    lock: CriticalSection::new(),
    state: UnsafeCell::new(State {
        current_dir: Vec::new(),
        std: [0; 3],
        env_alloc: core::ptr::null_mut(),
        console: 0,
        parent_console: 0,
        ctrl: poly_console::ctrl::CtrlHandlers::new(),
    }),
};

/// Run `f` with the process state, under the process lock (a recursive
/// critical section, so a nested call from the same thread is fine as long
/// as it does not hold the borrow).
pub fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let cs = &SHARED.lock as *const CriticalSection as *mut CriticalSection;
    // SAFETY: a static, initialised critical section.
    unsafe { nt::RtlEnterCriticalSection(cs) };
    // SAFETY: the lock gives exclusive access; `f` does not re-enter
    // `with_state` (no caller does).
    let r = f(unsafe { &mut *SHARED.state.get() });
    // SAFETY: we entered it.
    unsafe { nt::RtlLeaveCriticalSection(cs) };
    r
}

/// The current directory.
pub fn current_dir() -> Vec<u16> {
    with_state(|s| s.current_dir.clone())
}

// ----- paths ------------------------------------------------------------------------------

/// The per-drive current directory `=X:` from the environment.
fn drive_dir(drive: u16) -> Option<Vec<u16>> {
    let name = [u16::from(b'='), drive, u16::from(b':')];
    crate::process::env_var(&name)
}

/// A Win32 path as an NT path (`\??\...`), normalised against the current
/// directory, or the Win32 error.
pub fn nt_path(win32: &[u16]) -> Result<Vec<u16>, u32> {
    let cur = current_dir();
    let base = Base { current: &cur, drive_dir: &drive_dir };
    path::dos_to_nt(win32, &base)
}

/// The full path (`GetFullPathNameW`) and where its file part starts.
pub fn full_path(win32: &[u16]) -> Result<(Vec<u16>, Option<usize>), u32> {
    let cur = current_dir();
    let base = Base { current: &cur, drive_dir: &drive_dir };
    path::full_path(win32, &base)
}

/// An NT path held with the `UNICODE_STRING` and `OBJECT_ATTRIBUTES` that
/// point at it (case-insensitive, as every Win32 file name is).
pub struct NtName {
    units: Vec<u16>,
    string: UnicodeString,
}

impl NtName {
    pub fn new(units: Vec<u16>) -> alloc::boxed::Box<NtName> {
        let mut b = alloc::boxed::Box::new(NtName { units, string: UnicodeString::new(&[]) });
        b.string = UnicodeString::new(&b.units);
        b
    }

    pub fn attributes(&self) -> ObjectAttributes {
        ObjectAttributes::new(Some(&self.string), nt::OBJ_CASE_INSENSITIVE)
    }
}

/// The NT name of a Win32 path, or the Win32 error.
pub fn nt_name(win32: &[u16]) -> Result<alloc::boxed::Box<NtName>, u32> {
    nt_path(win32).map(NtName::new)
}

/// A NUL-terminated string from the caller, or `None` for a null pointer.
///
/// # Safety
/// `p` is null or a NUL-terminated UTF-16 string.
pub unsafe fn wide_arg<'a>(p: *const u16) -> Option<&'a [u16]> {
    // SAFETY: the caller's contract.
    if p.is_null() { None } else { Some(unsafe { poly_kernelbase::text::wide_at(p) }) }
}

/// A NUL-terminated ANSI string from the caller as UTF-16.
///
/// # Safety
/// `p` is null or a NUL-terminated string.
pub unsafe fn ansi_arg(p: *const u8) -> Option<Vec<u16>> {
    if p.is_null() {
        return None;
    }
    // SAFETY: the caller's contract.
    let bytes = unsafe { core::slice::from_raw_parts(p, poly_kernelbase::text::strlen(p)) };
    Some(poly_kernelbase::text::from_ansi(bytes))
}

/// A string with a NUL appended, for APIs that take NUL-terminated input.
pub fn with_nul(s: &[u16]) -> Vec<u16> {
    let mut v = Vec::with_capacity(s.len() + 1);
    v.extend_from_slice(s);
    v.push(0);
    v
}

/// Copy `s` and a NUL to `buf` (which holds at least `s.len() + 1` units).
///
/// # Safety
/// `buf` is writable for `s.len() + 1` units.
pub unsafe fn put_wide(buf: *mut u16, s: &[u16]) {
    // SAFETY: the caller's contract.
    unsafe {
        core::ptr::copy_nonoverlapping(s.as_ptr(), buf, s.len());
        buf.add(s.len()).write(0);
    }
}

/// The "size" convention (`poly_kernelbase::text::sized`): copy `s` to
/// `buf` if it fits, else return the size needed.
///
/// # Safety
/// `buf` is writable for `capacity` units (or null with capacity 0).
pub unsafe fn return_sized(s: &[u16], buf: *mut u16, capacity: u32) -> u32 {
    match poly_kernelbase::text::sized(s.len(), capacity) {
        poly_kernelbase::text::Sized::Copy(n) => {
            // SAFETY: `capacity` > `s.len()`.
            unsafe { put_wide(buf, s) };
            n
        }
        poly_kernelbase::text::Sized::Needed(n) => n,
    }
}

/// Close an NT handle (ignoring the result).
pub fn close(h: Handle) {
    // SAFETY: closing a handle is always memory-safe.
    unsafe { nt::NtClose(h) };
}

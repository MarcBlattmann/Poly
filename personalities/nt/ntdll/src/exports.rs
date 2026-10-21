//! The exported entry points: thin wrappers over the host-tested crates.
//! Signatures follow the public reference pages (Learn) and `winnt.h` /
//! `ntdef.h` layouts; the work is the crates'.

use crate::heap::process_heap;
use crate::sys::SysPlatform;
use crate::{run_callouts, with_loader};
use poly_ntobj::NtStatus;
use poly_rtl::critsec::CriticalSection;
use poly_rtl::heap::{self, Heap, HeapError};
use poly_rtl::string::{Allocator, AnsiString, UnicodeString};
use poly_rtl::{codepage, status, string, version};

const SUCCESS: NtStatus = NtStatus(0);

/// The string functions' allocator: the process heap.
struct HeapAlloc;

impl Allocator for HeapAlloc {
    fn alloc(&self, bytes: usize) -> *mut u8 {
        // SAFETY: the process heap is valid.
        unsafe { process_heap().allocate(&SysPlatform, 0, bytes) }.unwrap_or(core::ptr::null_mut())
    }
    fn free(&self, p: *mut u8) {
        // SAFETY: `p` came from `alloc`.
        unsafe { process_heap().free(&SysPlatform, 0, p) };
    }
}

fn heap_result(r: Result<*mut u8, HeapError>, flags: u32, heap_flags: u32) -> *mut u8 {
    match r {
        Ok(p) => p,
        Err(HeapError::NoMemory) if heap::raises(heap_flags, flags) => {
            // HEAP_GENERATE_EXCEPTIONS: raise STATUS_NO_MEMORY (ADR 0071).
            // SAFETY: a fail-fast-style raise; int 0x29 does not return.
            unsafe { core::arch::asm!("int 0x29", in("ecx") 1u32, options(noreturn)) }
        }
        Err(_) => core::ptr::null_mut(),
    }
}

// ----- strings (ntdef.h; Learn's Rtl*String* pages) -------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlInitUnicodeString(dest: *mut UnicodeString, src: *const u16) {
    // SAFETY: the caller's pointers (the Rtl* contract).
    unsafe { string::init_unicode_string(dest, src) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlInitUnicodeStringEx(dest: *mut UnicodeString, src: *const u16) -> NtStatus {
    // SAFETY: as above.
    unsafe { string::init_unicode_string_ex(dest, src) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlInitAnsiString(dest: *mut AnsiString, src: *const u8) {
    // SAFETY: as above.
    unsafe { string::init_ansi_string(dest, src) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlInitString(dest: *mut AnsiString, src: *const u8) {
    // SAFETY: as above.
    unsafe { string::init_ansi_string(dest, src) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlCopyUnicodeString(dest: *mut UnicodeString, src: *const UnicodeString) {
    // SAFETY: as above.
    unsafe { string::copy_unicode_string(dest, src) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlCompareUnicodeString(a: *const UnicodeString, b: *const UnicodeString, ci: u8) -> i32 {
    // SAFETY: as above.
    unsafe { string::compare_unicode_string(a, b, ci != 0) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlEqualUnicodeString(a: *const UnicodeString, b: *const UnicodeString, ci: u8) -> u8 {
    // SAFETY: as above.
    unsafe { string::equal_unicode_string(a, b, ci != 0) as u8 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlPrefixUnicodeString(p: *const UnicodeString, s: *const UnicodeString, ci: u8) -> u8 {
    // SAFETY: as above.
    unsafe { string::prefix_unicode_string(p, s, ci != 0) as u8 }
}

#[unsafe(no_mangle)]
pub extern "C" fn RtlUpcaseUnicodeChar(c: u16) -> u16 {
    string::upcase_char(c)
}

#[unsafe(no_mangle)]
pub extern "C" fn RtlDowncaseUnicodeChar(c: u16) -> u16 {
    string::downcase_char(c)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlUpcaseUnicodeString(d: *mut UnicodeString, s: *const UnicodeString, alloc: u8) -> NtStatus {
    // SAFETY: the Rtl* contract.
    unsafe { string::case_unicode_string(d, s, alloc != 0, true, &HeapAlloc) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlDowncaseUnicodeString(
    d: *mut UnicodeString,
    s: *const UnicodeString,
    alloc: u8,
) -> NtStatus {
    // SAFETY: as above.
    unsafe { string::case_unicode_string(d, s, alloc != 0, false, &HeapAlloc) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlAnsiStringToUnicodeString(
    d: *mut UnicodeString,
    s: *const AnsiString,
    alloc: u8,
) -> NtStatus {
    // SAFETY: as above.
    unsafe { string::ansi_to_unicode_string(d, s, alloc != 0, &HeapAlloc) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlUnicodeStringToAnsiString(
    d: *mut AnsiString,
    s: *const UnicodeString,
    alloc: u8,
) -> NtStatus {
    // SAFETY: as above.
    unsafe { string::unicode_to_ansi_string(d, s, alloc != 0, &HeapAlloc) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlAnsiStringToUnicodeSize(s: *const AnsiString) -> u32 {
    // SAFETY: as above.
    unsafe { string::ansi_to_unicode_size(s) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlFreeUnicodeString(s: *mut UnicodeString) {
    // SAFETY: a string an alloc call made.
    unsafe { string::free_unicode_string(s, &HeapAlloc) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlFreeAnsiString(s: *mut AnsiString) {
    // SAFETY: as above.
    unsafe { string::free_ansi_string(s, &HeapAlloc) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlAppendUnicodeToString(d: *mut UnicodeString, s: *const u16) -> NtStatus {
    // SAFETY: the Rtl* contract.
    unsafe { string::append_unicode_to_string(d, s) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlAppendUnicodeStringToString(d: *mut UnicodeString, s: *const UnicodeString) -> NtStatus {
    // SAFETY: as above.
    unsafe { string::append_unicode_string_to_string(d, s) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlCreateUnicodeString(d: *mut UnicodeString, s: *const u16) -> u8 {
    // SAFETY: as above.
    unsafe { string::create_unicode_string(d, s, &HeapAlloc) as u8 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlMultiByteToUnicodeN(
    out: *mut u16,
    max: u32,
    written: *mut u32,
    mb: *const u8,
    mb_len: u32,
) -> NtStatus {
    // SAFETY: the Rtl* contract: valid buffers of the lengths given.
    let n = unsafe { string::multibyte_to_unicode_n(out, max as usize, mb, mb_len as usize) };
    if !written.is_null() {
        // SAFETY: a valid out-pointer when non-null.
        unsafe { written.write(n as u32) };
    }
    SUCCESS
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlUnicodeToMultiByteN(
    out: *mut u8,
    max: u32,
    written: *mut u32,
    uni: *const u16,
    uni_len: u32,
) -> NtStatus {
    // SAFETY: as above.
    let n = unsafe { string::unicode_to_multibyte_n(out, max as usize, uni, uni_len as usize) };
    if !written.is_null() {
        // SAFETY: a valid out-pointer.
        unsafe { written.write(n as u32) };
    }
    SUCCESS
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlCompareString(a: *const AnsiString, b: *const AnsiString, ci: u8) -> i32 {
    // SAFETY: the Rtl* contract.
    unsafe { string::compare_string(a, b, ci != 0) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlEqualString(a: *const AnsiString, b: *const AnsiString, ci: u8) -> u8 {
    // SAFETY: as above.
    unsafe { string::equal_string(a, b, ci != 0) as u8 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlIntegerToUnicodeString(value: u32, base: u32, d: *mut UnicodeString) -> NtStatus {
    // SAFETY: as above.
    unsafe { string::integer_to_unicode_string(value, base, d) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlUnicodeStringToInteger(s: *const UnicodeString, base: u32, value: *mut u32) -> NtStatus {
    // SAFETY: as above.
    unsafe { string::unicode_string_to_integer(s, base, value) }
}

#[unsafe(no_mangle)]
pub extern "C" fn RtlUpperChar(c: u8) -> u8 {
    codepage::upcase(c)
}

// ----- status and last error ------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn RtlNtStatusToDosError(status: NtStatus) -> u32 {
    status::nt_status_to_dos_error(status)
}

#[unsafe(no_mangle)]
pub extern "C" fn RtlNtStatusToDosErrorNoTeb(status: NtStatus) -> u32 {
    status::nt_status_to_dos_error(status)
}

/// `TEB.LastErrorValue` (TEB + 0x68, `winternl.h`).
const TEB_LAST_ERROR: u64 = 0x68;
/// `TEB.LastStatusValue` (TEB + 0x1250 on x64).
const TEB_LAST_STATUS: u64 = 0x1250;

#[unsafe(no_mangle)]
pub extern "C" fn RtlGetLastWin32Error() -> u32 {
    // SAFETY: the TEB is valid.
    unsafe { ((crate::lock::teb() + TEB_LAST_ERROR) as *const u32).read() }
}

#[unsafe(no_mangle)]
pub extern "C" fn RtlSetLastWin32Error(err: u32) {
    // SAFETY: the TEB is valid and writable.
    unsafe { ((crate::lock::teb() + TEB_LAST_ERROR) as *mut u32).write(err) }
}

#[unsafe(no_mangle)]
pub extern "C" fn RtlGetLastNtStatus() -> NtStatus {
    // SAFETY: the TEB is valid.
    NtStatus(unsafe { ((crate::lock::teb() + TEB_LAST_STATUS) as *const u32).read() })
}

#[unsafe(no_mangle)]
pub extern "C" fn RtlSetLastWin32ErrorAndNtStatusFromNtStatus(status: NtStatus) {
    // SAFETY: the TEB is valid and writable.
    unsafe {
        ((crate::lock::teb() + TEB_LAST_STATUS) as *mut u32).write(status.0);
        ((crate::lock::teb() + TEB_LAST_ERROR) as *mut u32).write(status::nt_status_to_dos_error(status));
    }
}

// ----- version --------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlGetVersion(info: *mut u8) -> NtStatus {
    // SAFETY: the RtlGetVersion contract: the size field is readable.
    unsafe { version::get_version(info) }
}

// ----- critical sections ----------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlInitializeCriticalSection(cs: *mut CriticalSection) -> NtStatus {
    // SAFETY: `cs` points at writable RTL_CRITICAL_SECTION storage.
    unsafe { (*cs).init(0) };
    SUCCESS
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlInitializeCriticalSectionAndSpinCount(cs: *mut CriticalSection, spin: u32) -> NtStatus {
    // SAFETY: as above.
    unsafe { (*cs).init(u64::from(spin)) };
    SUCCESS
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlInitializeCriticalSectionEx(cs: *mut CriticalSection, spin: u32, _flags: u32) -> NtStatus {
    // SAFETY: as above.
    unsafe { (*cs).init(u64::from(spin)) };
    SUCCESS
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlEnterCriticalSection(cs: *mut CriticalSection) -> NtStatus {
    // SAFETY: `cs` is an initialised critical section.
    unsafe { (*cs).enter(&SysPlatform) };
    SUCCESS
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlLeaveCriticalSection(cs: *mut CriticalSection) -> NtStatus {
    // SAFETY: as above.
    unsafe { (*cs).leave(&SysPlatform) };
    SUCCESS
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlTryEnterCriticalSection(cs: *mut CriticalSection) -> u8 {
    // SAFETY: as above.
    unsafe { (*cs).try_enter(&SysPlatform) as u8 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlDeleteCriticalSection(cs: *mut CriticalSection) -> NtStatus {
    // SAFETY: as above.
    unsafe { (*cs).delete(&SysPlatform) };
    SUCCESS
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlIsCriticalSectionLocked(cs: *mut CriticalSection) -> u8 {
    // SAFETY: as above.
    unsafe { (*cs).is_locked() as u8 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlIsCriticalSectionLockedByThread(cs: *mut CriticalSection) -> u8 {
    // SAFETY: as above.
    unsafe { (*cs).is_owned_by(SysPlatform.current_thread_id()) as u8 }
}

impl SysPlatform {
    fn current_thread_id(&self) -> u64 {
        crate::lock::TID.with()
    }
}

// ----- the heap -------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlCreateHeap(
    flags: u32,
    base: *mut u8,
    reserve: usize,
    _commit: usize,
    _lock: *mut u8,
    _params: *mut u8,
) -> *mut u8 {
    let b = if base.is_null() { None } else { Some(base) };
    // SAFETY: `base`, if given, is the caller's memory for the heap.
    unsafe { Heap::create(&SysPlatform, flags, b, reserve) }.map_or(core::ptr::null_mut(), |h| h.0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlDestroyHeap(handle: *mut u8) -> *mut u8 {
    // SAFETY: `handle` is a heap this crate made; it is gone afterwards.
    unsafe { Heap(handle).destroy(&SysPlatform) };
    core::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlAllocateHeap(handle: *mut u8, flags: u32, size: usize) -> *mut u8 {
    // SAFETY: `handle` is a valid heap.
    let heap_flags = unsafe { Heap(handle).flags() };
    // SAFETY: as above.
    heap_result(unsafe { Heap(handle).allocate(&SysPlatform, flags, size) }, flags, heap_flags)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlFreeHeap(handle: *mut u8, flags: u32, ptr: *mut u8) -> u8 {
    // SAFETY: `handle` is a valid heap; `ptr` is any value.
    unsafe { Heap(handle).free(&SysPlatform, flags, ptr) as u8 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlReAllocateHeap(handle: *mut u8, flags: u32, ptr: *mut u8, size: usize) -> *mut u8 {
    // SAFETY: as above.
    let heap_flags = unsafe { Heap(handle).flags() };
    // SAFETY: as above.
    heap_result(unsafe { Heap(handle).reallocate(&SysPlatform, flags, ptr, size) }, flags, heap_flags)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlSizeHeap(handle: *mut u8, flags: u32, ptr: *const u8) -> usize {
    // SAFETY: as above.
    unsafe { Heap(handle).size(&SysPlatform, flags, ptr) }.unwrap_or(usize::MAX)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlValidateHeap(handle: *mut u8, flags: u32, ptr: *const u8) -> u8 {
    // SAFETY: as above.
    unsafe { Heap(handle).validate(&SysPlatform, flags, ptr) as u8 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlLockHeap(handle: *mut u8) -> u8 {
    // SAFETY: a valid heap.
    unsafe { Heap(handle).lock(&SysPlatform) };
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlUnlockHeap(handle: *mut u8) -> u8 {
    // SAFETY: a valid heap the caller locked.
    unsafe { Heap(handle).unlock(&SysPlatform) };
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn RtlProcessHeap() -> *mut u8 {
    process_heap().0
}

#[unsafe(no_mangle)]
pub extern "C" fn RtlGetProcessHeaps(count: u32, heaps: *mut *mut u8) -> u32 {
    if count >= 1 && !heaps.is_null() {
        // SAFETY: room for at least one handle.
        unsafe { heaps.write(process_heap().0) };
    }
    1
}

// ----- memory intrinsics (winnt.h macros) -----------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlZeroMemory(dst: *mut u8, len: usize) {
    // SAFETY: the caller's buffer of `len` bytes.
    unsafe { core::ptr::write_bytes(dst, 0, len) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlFillMemory(dst: *mut u8, len: usize, fill: u8) {
    // SAFETY: as above.
    unsafe { core::ptr::write_bytes(dst, fill, len) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlMoveMemory(dst: *mut u8, src: *const u8, len: usize) {
    // SAFETY: overlapping is allowed; both buffers hold `len` bytes.
    unsafe { core::ptr::copy(src, dst, len) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlCopyMemory(dst: *mut u8, src: *const u8, len: usize) {
    // SAFETY: non-overlapping buffers of `len` bytes.
    unsafe { core::ptr::copy_nonoverlapping(src, dst, len) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlCompareMemory(a: *const u8, b: *const u8, len: usize) -> usize {
    // SAFETY: both buffers hold `len` bytes.
    let (a, b) = unsafe { (core::slice::from_raw_parts(a, len), core::slice::from_raw_parts(b, len)) };
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

// ----- the loader -----------------------------------------------------------

/// `LDR_LOCK_LOADER_LOCK` disposition: the loader lock critical section is
/// kept in ntdll's own data.
static LOADER_LOCK: CriticalSection = CriticalSection::new();

fn to_wide(s: *const UnicodeString) -> alloc::vec::Vec<u16> {
    if s.is_null() {
        return alloc::vec::Vec::new();
    }
    // SAFETY: a valid UNICODE_STRING (the Ldr* contract).
    unsafe { (*s).units().to_vec() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn LdrLoadDll(
    search_path: *const u16,
    _flags: *mut u32,
    name: *const UnicodeString,
    handle: *mut *mut u8,
) -> NtStatus {
    let name = to_wide(name);
    let sp = if search_path.is_null() {
        None
    } else {
        // SAFETY: a NUL-terminated path when non-null.
        Some(unsafe { collect_wide(search_path) })
    };
    LOADER_LOCK.enter(&SysPlatform);
    let r = with_loader(|l, p| l.load(p, &name, sp.as_deref()));
    LOADER_LOCK.leave(&SysPlatform);
    match r {
        Ok((_, base, op)) => {
            if run_callouts(&op.callouts).is_err() {
                return NtStatus(0xC000_0142);
            }
            if !handle.is_null() {
                // SAFETY: a valid out-pointer.
                unsafe { handle.write(base as *mut u8) };
            }
            SUCCESS
        }
        Err(e) => e,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn LdrUnloadDll(handle: *mut u8) -> NtStatus {
    LOADER_LOCK.enter(&SysPlatform);
    let unload = with_loader(|l, _| l.unload(handle as u64));
    LOADER_LOCK.leave(&SysPlatform);
    match unload {
        Ok(u) => {
            let _ = run_callouts(&u.callouts);
            with_loader(|l, p| l.finish_unload(p, u));
            SUCCESS
        }
        Err(e) => e,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn LdrGetDllHandle(
    _path: *const u16,
    _flags: *mut u32,
    name: *const UnicodeString,
    handle: *mut *mut u8,
) -> NtStatus {
    let name = to_wide(name);
    match with_loader(|l, _| l.get_handle(&name)) {
        Ok(base) => {
            if !handle.is_null() {
                // SAFETY: a valid out-pointer.
                unsafe { handle.write(base as *mut u8) };
            }
            SUCCESS
        }
        Err(e) => e,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn LdrGetProcedureAddress(
    handle: *mut u8,
    name: *const AnsiString,
    ordinal: u32,
    address: *mut *mut u8,
) -> NtStatus {
    let by_name = !name.is_null();
    // SAFETY: a valid ANSI_STRING when non-null.
    let n = if by_name { Some(unsafe { (*name).bytes().to_vec() }) } else { None };
    LOADER_LOCK.enter(&SysPlatform);
    let r = with_loader(|l, p| l.get_procedure(p, handle as u64, n.as_deref(), ordinal as u16));
    LOADER_LOCK.leave(&SysPlatform);
    match r {
        Ok((a, op)) => {
            let _ = run_callouts(&op.callouts);
            if !address.is_null() {
                // SAFETY: a valid out-pointer.
                unsafe { address.write(a as *mut u8) };
            }
            SUCCESS
        }
        Err(e) => e,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn LdrAddRefDll(flags: u32, handle: *mut u8) -> NtStatus {
    with_loader(|l, _| l.add_ref(handle as u64, flags)).err().unwrap_or(SUCCESS)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn LdrDisableThreadCalloutsForDll(handle: *mut u8) -> NtStatus {
    with_loader(|l, _| l.disable_thread_calls(handle as u64)).err().unwrap_or(SUCCESS)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn LdrFindEntryForAddress(addr: *mut u8, entry: *mut u64) -> NtStatus {
    match with_loader(|l, _| l.find(addr as u64)) {
        Some(m) => {
            if !entry.is_null() {
                // SAFETY: a valid out-pointer.
                unsafe { entry.write(m.record) };
            }
            SUCCESS
        }
        None => NtStatus(0xC000_0135),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn LdrLockLoaderLock(_flags: u32, disposition: *mut u32, cookie: *mut usize) -> NtStatus {
    LOADER_LOCK.enter(&SysPlatform);
    if !disposition.is_null() {
        // SAFETY: a valid out-pointer. 1 = LOCK_ACQUIRED.
        unsafe { disposition.write(1) };
    }
    if !cookie.is_null() {
        // SAFETY: a valid out-pointer.
        unsafe { cookie.write(1) };
    }
    SUCCESS
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn LdrUnlockLoaderLock(_flags: u32, _cookie: usize) -> NtStatus {
    LOADER_LOCK.leave(&SysPlatform);
    SUCCESS
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn LdrShutdownProcess() -> NtStatus {
    let callouts = with_loader(|l, _| l.process_detach());
    let _ = run_callouts(&callouts);
    SUCCESS
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn LdrShutdownThread() -> NtStatus {
    let teb = crate::lock::teb();
    let callouts = with_loader(|l, _| l.thread_detach());
    let _ = run_callouts(&callouts);
    with_loader(|l, p| l.thread_exit(p, teb));
    SUCCESS
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn LdrResolveDelayLoadedAPI(
    parent: *mut u8,
    descriptor: *const u8,
    _dllhook: *mut u8,
    _funchook: *mut u8,
    thunk: *mut u8,
    _flags: u32,
) -> *mut u8 {
    LOADER_LOCK.enter(&SysPlatform);
    let r = with_loader(|l, p| l.resolve_delay(p, parent as u64, descriptor as u64, thunk as u64));
    LOADER_LOCK.leave(&SysPlatform);
    match r {
        Ok((addr, op)) => {
            let _ = run_callouts(&op.callouts);
            addr as *mut u8
        }
        // A failure raises; the helper does not return a sentinel.
        Err(_f) => {
            // SAFETY: a fail-fast; does not return.
            unsafe { core::arch::asm!("int 0x29", in("ecx") 4u32, options(noreturn)) }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn ApiSetQueryApiSetPresence(name: *const UnicodeString, present: *mut u8) -> NtStatus {
    let n = to_wide(name);
    if !present.is_null() {
        // SAFETY: a valid out-pointer.
        unsafe { present.write(poly_ldr::apiset::present(&n) as u8) };
    }
    SUCCESS
}

/// A NUL-terminated wide string into a vector.
///
/// # Safety
/// `p` is NUL-terminated.
unsafe fn collect_wide(p: *const u16) -> alloc::vec::Vec<u16> {
    let mut v = alloc::vec::Vec::new();
    let mut i = 0;
    loop {
        // SAFETY: the caller's contract.
        let c = unsafe { *p.add(i) };
        if c == 0 {
            break;
        }
        v.push(c);
        i += 1;
    }
    v
}

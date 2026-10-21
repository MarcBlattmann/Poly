//! Errors, time, the version and system information, messages and the
//! debugger check (Learn's `errhandlingapi.h`, `sysinfoapi.h`,
//! `profileapi.h`, `winbase.h` and `debugapi.h` pages).

use crate::base::*;
use crate::nt;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};
use poly_kernelbase::error::*;
use poly_kernelbase::message::{self, *};
use poly_kernelbase::time;
use poly_kernelbase::version::{self, Compat};

/// `GetLastError`.
#[unsafe(no_mangle)]
pub extern "system" fn GetLastError() -> u32 {
    last_error()
}

/// `SetLastError`.
#[unsafe(no_mangle)]
pub extern "system" fn SetLastError(error: u32) {
    set_last_error(error)
}

/// `IsDebuggerPresent`: `PEB.BeingDebugged` (PEB + 2, `winternl.h`); Poly
/// has no user-mode debugger yet, so it is 0.
#[unsafe(no_mangle)]
pub extern "system" fn IsDebuggerPresent() -> i32 {
    // SAFETY: the PEB is readable.
    i32::from(unsafe { ((peb() + 2) as *const u8).read() })
}

// ----- time ------------------------------------------------------------------------------

fn counter() -> (u64, u64) {
    let (mut c, mut f) = (0i64, 0i64);
    // SAFETY: valid pointers.
    unsafe { nt::NtQueryPerformanceCounter(&mut c, &mut f) };
    (c as u64, (f as u64).max(1))
}

/// Milliseconds since the system started.
fn uptime_ms() -> u64 {
    let (c, f) = counter();
    (u128::from(c) * 1000 / u128::from(f)) as u64
}

/// `GetTickCount`: Learn: wraps after 49.7 days.
#[unsafe(no_mangle)]
pub extern "system" fn GetTickCount() -> u32 {
    uptime_ms() as u32
}

/// `GetTickCount64`.
#[unsafe(no_mangle)]
pub extern "system" fn GetTickCount64() -> u64 {
    uptime_ms()
}

/// `QueryPerformanceCounter`: Learn: "On systems that run Windows XP or
/// later, the function will always succeed".
#[unsafe(no_mangle)]
pub unsafe extern "system" fn QueryPerformanceCounter(out: *mut i64) -> i32 {
    // SAFETY: the caller's out-pointer.
    unsafe { out.write(counter().0 as i64) };
    TRUE
}

/// `QueryPerformanceFrequency`: fixed at boot (10 MHz on Poly).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn QueryPerformanceFrequency(out: *mut i64) -> i32 {
    // SAFETY: the caller's out-pointer.
    unsafe { out.write(counter().1 as i64) };
    TRUE
}

/// `GetSystemTimeAsFileTime`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetSystemTimeAsFileTime(out: *mut u64) {
    let mut t = 0i64;
    // SAFETY: valid pointers; FILETIME is two DWORDs, written as one u64
    // (unaligned-safe).
    unsafe {
        nt::NtQuerySystemTime(&mut t);
        (out as *mut u8).cast::<u64>().write_unaligned(t as u64);
    }
}

/// `GetSystemTimePreciseAsFileTime`: the same clock.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetSystemTimePreciseAsFileTime(out: *mut u64) {
    // SAFETY: as above.
    unsafe { GetSystemTimeAsFileTime(out) }
}

// ----- calendar times (ADR 0079, second slice; `poly_kernelbase::time`) -------------------

fn now() -> u64 {
    let mut t = 0i64;
    // SAFETY: a valid pointer.
    unsafe { nt::NtQuerySystemTime(&mut t) };
    t as u64
}

/// Write a `SYSTEMTIME` (16 bytes) for `ft`.
///
/// # Safety
/// `out` is writable for 16 bytes.
unsafe fn put_system_time(out: *mut u8, ft: u64) -> bool {
    match time::to_system_time(ft) {
        Some(st) => {
            let b = st.to_bytes();
            // SAFETY: the caller's contract.
            unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), out, 16) };
            true
        }
        None => false,
    }
}

/// `GetSystemTime`: the current UTC time.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetSystemTime(out: *mut u8) {
    // SAFETY: the caller's SYSTEMTIME.
    unsafe { put_system_time(out, now()) };
}

/// `GetLocalTime`: Poly's local time is UTC (`poly_kernelbase::time`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetLocalTime(out: *mut u8) {
    // SAFETY: the caller's SYSTEMTIME.
    unsafe { put_system_time(out, time::to_local(now())) };
}

/// `FileTimeToSystemTime`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn FileTimeToSystemTime(ft: *const u8, out: *mut u8) -> i32 {
    // SAFETY: the caller's FILETIME (unaligned) and SYSTEMTIME.
    let ok = unsafe { put_system_time(out, ft.cast::<u64>().read_unaligned()) };
    if ok { TRUE } else { fail(ERROR_INVALID_PARAMETER, FALSE) }
}

/// `SystemTimeToFileTime`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SystemTimeToFileTime(st: *const u8, out: *mut u8) -> i32 {
    let mut b = [0u8; 16];
    // SAFETY: the caller's SYSTEMTIME.
    unsafe { core::ptr::copy_nonoverlapping(st, b.as_mut_ptr(), 16) };
    match time::to_file_time(&time::SystemTime::from_bytes(&b)) {
        Some(ft) => {
            // SAFETY: the caller's FILETIME.
            unsafe { out.cast::<u64>().write_unaligned(ft) };
            TRUE
        }
        None => fail(ERROR_INVALID_PARAMETER, FALSE),
    }
}

/// `CompareFileTime`: -1, 0 or 1.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CompareFileTime(a: *const u8, b: *const u8) -> i32 {
    // SAFETY: the caller's FILETIMEs.
    unsafe { time::compare(a.cast::<u64>().read_unaligned(), b.cast::<u64>().read_unaligned()) }
}

/// `FileTimeToLocalFileTime`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn FileTimeToLocalFileTime(ft: *const u8, out: *mut u8) -> i32 {
    // SAFETY: the caller's FILETIMEs.
    unsafe { out.cast::<u64>().write_unaligned(time::to_local(ft.cast::<u64>().read_unaligned())) };
    TRUE
}

/// `LocalFileTimeToFileTime`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LocalFileTimeToFileTime(ft: *const u8, out: *mut u8) -> i32 {
    // SAFETY: the caller's FILETIMEs.
    unsafe { out.cast::<u64>().write_unaligned(time::from_local(ft.cast::<u64>().read_unaligned())) };
    TRUE
}

// ----- the version and the system ---------------------------------------------------------

/// What the executable's manifest declares (read once).
fn compat() -> Compat {
    static CACHE: AtomicU32 = AtomicU32::new(0);
    match CACHE.load(Ordering::Relaxed) {
        1 => return Compat::Legacy,
        2 => return Compat::Win81,
        3 => return Compat::Win10,
        _ => {}
    }
    let c = manifest_of(crate::process::exe_base()).map_or(Compat::Legacy, |m| version::manifest_compat(&m));
    let code = match c {
        Compat::Legacy => 1,
        Compat::Win81 => 2,
        Compat::Win10 => 3,
    };
    CACHE.store(code, Ordering::Relaxed);
    c
}

/// The manifest of the image mapped at `base` (its resource directory is
/// data directory 2 of the optional header, PE format).
fn manifest_of(base: u64) -> Option<Vec<u8>> {
    if base == 0 {
        return None;
    }
    // SAFETY: a mapped image's headers and its resource section are
    // readable; RVAs are bounded by the sizes the headers give.
    unsafe {
        let nt_off = ((base + 0x3C) as *const u32).read() as u64;
        let opt = base + nt_off + 24;
        if (opt as *const u16).read() != 0x20B {
            return None;
        }
        // PE32+: the data directories start at optional header + 112; the
        // resource table is entry 2.
        let count = ((opt + 108) as *const u32).read();
        if count < 3 {
            return None;
        }
        let rva = ((opt + 112 + 16) as *const u32).read() as u64;
        let size = ((opt + 112 + 20) as *const u32).read() as usize;
        let image_size = ((opt + 56) as *const u32).read() as u64;
        if rva == 0 || rva + size as u64 > image_size {
            return None;
        }
        let rsrc = core::slice::from_raw_parts((base + rva) as *const u8, size);
        let (data, len) = version::find_manifest(rsrc)?;
        if u64::from(data) + u64::from(len) > image_size {
            return None;
        }
        Some(core::slice::from_raw_parts((base + u64::from(data)) as *const u8, len as usize).to_vec())
    }
}

/// `GetVersion`.
#[unsafe(no_mangle)]
pub extern "system" fn GetVersion() -> u32 {
    version::get_version(version::reported(compat()))
}

/// `GetVersionExW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetVersionExW(info: *mut u8) -> i32 {
    if info.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: the caller's structure: its first DWORD is its size.
    let size = unsafe { (info as *const u32).read_unaligned() } as usize;
    if size != poly_rtl::version::INFO_SIZE as usize && size != poly_rtl::version::INFO_EX_SIZE as usize {
        return fail(ERROR_INSUFFICIENT_BUFFER, FALSE);
    }
    // SAFETY: `size` bytes of the caller's structure.
    let buf = unsafe { core::slice::from_raw_parts_mut(info, size) };
    match version::fill(buf, version::reported(compat())) {
        Ok(()) => TRUE,
        Err(e) => fail(e, FALSE),
    }
}

/// The processor count (`NtQuerySystemInformation(SystemBasicInformation)`).
pub(crate) fn processors() -> u32 {
    let mut info = [0u8; 64];
    // SAFETY: a valid buffer of the class's size.
    let st = unsafe { nt::NtQuerySystemInformation(0, info.as_mut_ptr(), 64, core::ptr::null_mut()) };
    if st.is_success() { u32::from(info[56]).max(1) } else { 1 }
}

/// `GetSystemInfo`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetSystemInfo(out: *mut u8) {
    let info = version::system_info(processors());
    // SAFETY: the caller's SYSTEM_INFO (48 bytes).
    unsafe { core::ptr::copy_nonoverlapping(info.as_ptr(), out, info.len()) };
}

/// `GetNativeSystemInfo`: the same for a 64-bit process.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetNativeSystemInfo(out: *mut u8) {
    // SAFETY: as above.
    unsafe { GetSystemInfo(out) }
}

// ----- FormatMessageW ------------------------------------------------------------------------

/// The insert arguments: a `va_list` (on x64 a pointer to 8-byte slots) or,
/// with `FORMAT_MESSAGE_ARGUMENT_ARRAY`, an array of `DWORD_PTR` — the same
/// layout here.
struct Slots(*const u64);

impl message::Args for Slots {
    fn slot(&self, index: usize) -> Option<u64> {
        if self.0.is_null() || index == 0 {
            return None;
        }
        // SAFETY: the caller promised an argument for every insert.
        Some(unsafe { self.0.add(index - 1).read_unaligned() })
    }
    fn wide(&self, address: u64) -> Vec<u16> {
        // SAFETY: an insert of type s is a NUL-terminated string.
        unsafe { wide_arg(address as *const u16) }.map(<[u16]>::to_vec).unwrap_or_default()
    }
    fn ansi(&self, address: u64) -> Vec<u16> {
        // SAFETY: as above, in the ANSI code page.
        unsafe { ansi_arg(address as *const u8) }.unwrap_or_default()
    }
}

/// `FormatMessageW`: system messages (`FORMAT_MESSAGE_FROM_SYSTEM`) and
/// caller strings (`FORMAT_MESSAGE_FROM_STRING`); a module's message table
/// (`FORMAT_MESSAGE_FROM_HMODULE` alone) is not supported yet
/// (`ERROR_RESOURCE_TYPE_NOT_FOUND`-like: Poly reports
/// `ERROR_MR_MID_NOT_FOUND`). The result is the length without the NUL.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn FormatMessageW(
    flags: u32,
    source: *const u16,
    id: u32,
    lang: u32,
    buffer: *mut u16,
    size: u32,
    args: *const u64,
) -> u32 {
    // A va_list is a pointer to the slots; with ARGUMENT_ARRAY `args` is
    // the array itself. Without ARGUMENT_ARRAY Learn passes `va_list*`.
    let slots = if args.is_null() {
        None
    } else if flags & FORMAT_MESSAGE_ARGUMENT_ARRAY != 0 {
        Some(Slots(args))
    } else {
        // SAFETY: a `va_list*`: the address of the slot pointer.
        Some(Slots(unsafe { (args as *const *const u64).read() }))
    };
    let a = slots.as_ref().map(|s| s as &dyn message::Args);
    let text = if flags & FORMAT_MESSAGE_FROM_STRING != 0 {
        // SAFETY: the caller's string.
        let Some(t) = (unsafe { wide_arg(source) }) else { return fail(ERROR_INVALID_PARAMETER, 0) };
        message::format(t, flags, a)
    } else if flags & FORMAT_MESSAGE_FROM_SYSTEM != 0 {
        message::system(id, lang, flags, a)
    } else {
        Err(ERROR_MR_MID_NOT_FOUND)
    };
    let text = match text {
        Ok(t) => t,
        Err(e) => return fail(e, 0),
    };
    if flags & FORMAT_MESSAGE_ALLOCATE_BUFFER != 0 {
        // `lpBuffer` receives a LocalAlloc'd buffer of at least `nSize`
        // characters.
        let units = (text.len() + 1).max(size as usize);
        let p = crate::memory::LocalAlloc(0, units * 2) as *mut u16;
        if p.is_null() {
            return fail(ERROR_NOT_ENOUGH_MEMORY, 0);
        }
        // SAFETY: `p` holds `units` > `text.len()` units; `buffer` is the
        // caller's `LPWSTR*`.
        unsafe {
            put_wide(p, &text);
            (buffer as *mut *mut u16).write(p);
        }
        return text.len() as u32;
    }
    if text.len() >= size as usize {
        return fail(ERROR_INSUFFICIENT_BUFFER, 0);
    }
    // SAFETY: the caller's buffer of `size` > `text.len()` units.
    unsafe { put_wide(buffer, &text) };
    text.len() as u32
}

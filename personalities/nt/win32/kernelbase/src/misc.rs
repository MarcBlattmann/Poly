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

// ----- FormatMessageW / FormatMessageA -------------------------------------------------------

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
        // SAFETY: a wide string insert is a NUL-terminated string.
        unsafe { wide_arg(address as *const u16) }.map(<[u16]>::to_vec).unwrap_or_default()
    }
    fn ansi(&self, address: u64) -> Vec<u16> {
        // SAFETY: as above, in the ANSI code page.
        unsafe { ansi_arg(address as *const u8) }.unwrap_or_default()
    }
}

/// `MESSAGE_TABLE_NAME` resources of type `RT_MESSAGETABLE` in the module
/// at `base`, by language (empty if it has none).
///
/// # Safety
/// `base` is a loaded module's base address.
unsafe fn message_tables<'a>(base: u64) -> Vec<(u32, &'a [u8])> {
    use poly_pe::{Pe, ResourceKey, ResourceName};
    let mut out = Vec::new();
    if base == 0 {
        return out;
    }
    // SAFETY: a mapped image starts with its headers: `e_lfanew` at 0x3C,
    // the optional header 24 bytes after the PE signature, `SizeOfImage`
    // at offset 56 in it (PE format spec); the whole image is mapped.
    let image = unsafe {
        let nt = u64::from(((base + 0x3C) as *const u32).read_unaligned());
        let size = ((base + nt + 24 + 56) as *const u32).read_unaligned();
        core::slice::from_raw_parts(base as *const u8, size as usize)
    };
    let Ok(pe) = Pe::parse_mapped(image) else { return out };
    let Ok(Some(root)) = pe.resources() else { return out };
    let dir =
        |d: poly_pe::ResourceDirectory<'a>, id| d.find(ResourceKey::Id(id)).ok().flatten()?.directory().ok().flatten();
    let Some(names) = dir(root, message::RT_MESSAGETABLE) else { return out };
    let Some(langs) = dir(names, message::MESSAGE_TABLE_NAME) else { return out };
    for e in langs.entries().flatten() {
        let (ResourceName::Id(lang), Ok(Some(data))) = (e.name, e.data()) else { continue };
        if let Ok(bytes) = pe.resource_bytes(&data) {
            out.push((lang, bytes));
        }
    }
    out
}

/// What both `FormatMessage`s share: find the message definition and
/// format it, as UTF-16. `ansi` is true for `FormatMessageA`, whose
/// `FROM_STRING` source the caller has converted into `string`.
///
/// # Safety
/// The arguments are `FormatMessage`'s.
unsafe fn format_message(
    flags: u32,
    source: *const u8,
    string: Option<&[u16]>,
    id: u32,
    lang: u32,
    args: *const u64,
    ansi: bool,
) -> Result<Vec<u16>, u32> {
    let ignore = flags & FORMAT_MESSAGE_IGNORE_INSERTS != 0;
    // A va_list is a pointer to the slots; with ARGUMENT_ARRAY `args` is
    // the array itself. Without ARGUMENT_ARRAY Learn passes `va_list*`.
    // With IGNORE_INSERTS Learn says the arguments are ignored.
    let slots = if args.is_null() || ignore {
        None
    } else if flags & FORMAT_MESSAGE_ARGUMENT_ARRAY != 0 {
        Some(Slots(args))
    } else {
        // SAFETY: a `va_list*`: the address of the slot pointer.
        Some(Slots(unsafe { (args as *const *const u64).read() }))
    };
    let a = slots.as_ref().map(|s| s as &dyn message::Args);
    if flags & FORMAT_MESSAGE_FROM_STRING != 0 {
        let t = string.ok_or(ERROR_INVALID_PARAMETER)?;
        return message::format(t, flags, ansi, a);
    }
    let from_module = flags & FORMAT_MESSAGE_FROM_HMODULE != 0;
    let from_system = flags & FORMAT_MESSAGE_FROM_SYSTEM != 0;
    if from_module {
        let base = if source.is_null() { crate::process::exe_base() } else { source as u64 };
        // SAFETY: the caller's module handle.
        let tables = unsafe { message_tables(base) };
        match message::module(&tables, id, lang, flags, ansi, a) {
            Ok(t) => return Ok(t),
            // Learn: with FROM_SYSTEM too, the system table is searched if
            // the module does not have the message.
            Err(e) if !from_system => return Err(e),
            Err(_) => {}
        }
    }
    if from_system {
        return message::system(id, lang, flags, ansi, a);
    }
    // No source at all (ADR 0079's choice).
    Err(ERROR_MR_MID_NOT_FOUND)
}

/// Hands the message to the caller: in a `LocalAlloc`'d buffer of at least
/// `size` units placed at `*buffer` (`FORMAT_MESSAGE_ALLOCATE_BUFFER`) or
/// copied into `buffer` of `size` units; the result is the length without
/// the NUL. `T` is `u16` for `FormatMessageW`, `u8` for `FormatMessageA`.
///
/// # Safety
/// `buffer` is the caller's `lpBuffer` for these flags.
unsafe fn deliver<T: Copy + Default>(flags: u32, text: &[T], buffer: *mut T, size: u32) -> u32 {
    let unit = core::mem::size_of::<T>();
    if flags & FORMAT_MESSAGE_ALLOCATE_BUFFER != 0 {
        if buffer.is_null() {
            return fail(ERROR_INVALID_PARAMETER, 0);
        }
        // Learn: a message longer than 128K bytes is ERROR_MORE_DATA.
        if core::mem::size_of_val(text) > MAX_ALLOCATED_BYTES {
            return fail(ERROR_MORE_DATA, 0);
        }
        let units = (text.len() + 1).max(size as usize);
        let p = crate::memory::LocalAlloc(0, units * unit) as *mut T;
        if p.is_null() {
            return fail(ERROR_NOT_ENOUGH_MEMORY, 0);
        }
        // SAFETY: `p` holds `units` > `text.len()` units; `buffer` is the
        // caller's `LPTSTR*`.
        unsafe {
            core::ptr::copy_nonoverlapping(text.as_ptr(), p, text.len());
            p.add(text.len()).write(T::default());
            (buffer as *mut *mut T).write(p);
        }
        return text.len() as u32;
    }
    if text.len() >= size as usize {
        return fail(ERROR_INSUFFICIENT_BUFFER, 0);
    }
    if buffer.is_null() {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    // SAFETY: the caller's buffer of `size` > `text.len()` units.
    unsafe {
        core::ptr::copy_nonoverlapping(text.as_ptr(), buffer, text.len());
        buffer.add(text.len()).write(T::default());
    }
    text.len() as u32
}

/// `FormatMessageW`: a message from a string (`FORMAT_MESSAGE_FROM_STRING`),
/// a module's message table (`FORMAT_MESSAGE_FROM_HMODULE`, NULL for the
/// executable; `ERROR_RESOURCE_TYPE_NOT_FOUND` if it has none) and/or the
/// system table (`FORMAT_MESSAGE_FROM_SYSTEM`, also for `HRESULT_FROM_WIN32`
/// values), formatted (ADR 0101). The result is the length without the NUL.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn FormatMessageW(
    flags: u32,
    source: *const u8,
    id: u32,
    lang: u32,
    buffer: *mut u16,
    size: u32,
    args: *const u64,
) -> u32 {
    // SAFETY: with FROM_STRING the source is the caller's string.
    let string = if flags & FORMAT_MESSAGE_FROM_STRING != 0 { unsafe { wide_arg(source as *const u16) } } else { None };
    // SAFETY: the caller's arguments.
    match unsafe { format_message(flags, source, string, id, lang, args, false) } {
        // SAFETY: the caller's buffer.
        Ok(text) => unsafe { deliver(flags, &text, buffer, size) },
        Err(e) => fail(e, 0),
    }
}

/// `FormatMessageA`: the same in the ANSI code page: a `FROM_STRING`
/// source, a plain `s` insert and the result are ANSI (`!S!` and `!ls!`
/// inserts are wide), and sizes count bytes.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn FormatMessageA(
    flags: u32,
    source: *const u8,
    id: u32,
    lang: u32,
    buffer: *mut u8,
    size: u32,
    args: *const u64,
) -> u32 {
    // SAFETY: with FROM_STRING the source is the caller's ANSI string.
    let string = if flags & FORMAT_MESSAGE_FROM_STRING != 0 { unsafe { ansi_arg(source) } } else { None };
    // SAFETY: the caller's arguments.
    match unsafe { format_message(flags, source, string.as_deref(), id, lang, args, true) } {
        // SAFETY: the caller's buffer.
        Ok(text) => unsafe { deliver(flags, &poly_kernelbase::text::to_ansi(&text), buffer, size) },
        Err(e) => fail(e, 0),
    }
}

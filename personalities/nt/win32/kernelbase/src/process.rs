//! The process: its environment, command line, current directory and
//! standard handles; modules; identities; ending it (Learn's
//! `processenv.h`, `processthreadsapi.h` and `libloaderapi.h` pages).
//!
//! The environment block and the command line are where the kernel put
//! them (ADR 0073): `RTL_USER_PROCESS_PARAMETERS.CommandLine` (+ 0x70,
//! `winternl.h`) and Poly's own environment pointer and size (offsets
//! 0x80 and 0x88). `SetEnvironmentVariableW` replaces the block there, so
//! `ntdll` (the DLL search order's `Path`) sees the change too.
//!
//! **The standard handles** (ADR 0079): a console host is a later Phase 5
//! item; until then a process gets what every Poly process starts with on
//! descriptors 0, 1 and 2, the system console: `\??\Z:\dev\console`
//! (`Z:` is the whole VFS, ADR 0067), opened on first use, for reading
//! (input) and writing (output and error). If it cannot be opened the
//! handle is NULL, which Learn allows ("If an application does not have
//! associated standard handles ... the return value is NULL").

use crate::base::*;
use crate::nt::{self, Handle};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicPtr, Ordering};
use poly_kernelbase::error::*;
use poly_kernelbase::{env, text};

const PP_COMMAND_LINE: u64 = 0x70;
const PP_IMAGE_PATH: u64 = 0x60;
const PP_ENVIRONMENT: u64 = 0x80;
const PP_ENVIRONMENT_SIZE: u64 = 0x88;

// ----- the environment ---------------------------------------------------------------

/// The current environment block (both final NULs included).
///
/// # Safety
/// The caller holds the process lock, and does not keep the slice past it.
unsafe fn block<'a>() -> &'a [u16] {
    let pp = parameters();
    if pp == 0 {
        return &[0, 0];
    }
    // SAFETY: the kernel (or `set_block`) wrote a valid pointer and size.
    unsafe {
        let at = ((pp + PP_ENVIRONMENT) as *const *const u16).read();
        let bytes = ((pp + PP_ENVIRONMENT_SIZE) as *const u64).read() as usize;
        if at.is_null() || bytes < 4 { &[0, 0] } else { core::slice::from_raw_parts(at, bytes / 2) }
    }
}

/// The value of variable `name`.
pub fn env_var(name: &[u16]) -> Option<Vec<u16>> {
    // SAFETY: under the lock; copied out before it is released.
    with_state(|_| unsafe { env::get(block(), name).map(<[u16]>::to_vec) })
}

/// A copy of the whole environment block (a child's, `CreateProcessW`).
pub fn env_block() -> Vec<u16> {
    // SAFETY: under the lock; copied out before it is released.
    with_state(|_| unsafe { block().to_vec() })
}

/// Replace the block with `new` (in the process heap).
fn set_block(state: &mut State, new: &[u16]) -> bool {
    let pp = parameters();
    if pp == 0 {
        return false;
    }
    // SAFETY: a valid heap.
    let p = unsafe { nt::RtlAllocateHeap(process_heap(), 0, new.len() * 2) } as *mut u16;
    if p.is_null() {
        return false;
    }
    // SAFETY: `p` holds `new.len()` units; the parameters are writable
    // (one private read-write allocation, ADR 0073).
    unsafe {
        core::ptr::copy_nonoverlapping(new.as_ptr(), p, new.len());
        ((pp + PP_ENVIRONMENT) as *mut *mut u16).write(p);
        ((pp + PP_ENVIRONMENT_SIZE) as *mut u64).write((new.len() * 2) as u64);
    }
    if !state.env_alloc.is_null() {
        // SAFETY: the block we allocated before.
        unsafe { nt::RtlFreeHeap(process_heap(), 0, state.env_alloc as *mut u8) };
    }
    state.env_alloc = p;
    true
}

/// `GetEnvironmentVariableW`: Learn: the length without the NUL, or the
/// size needed with it; 0 and `ERROR_ENVVAR_NOT_FOUND` if not set. An
/// empty value returns 0 with the last error cleared (to be confirmed).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetEnvironmentVariableW(name: *const u16, buf: *mut u16, size: u32) -> u32 {
    // SAFETY: the caller's NUL-terminated name.
    let Some(name) = (unsafe { wide_arg(name) }) else { return fail(ERROR_INVALID_PARAMETER, 0) };
    match env_var(name) {
        Some(v) => {
            set_last_error(ERROR_SUCCESS);
            // SAFETY: the caller's buffer of `size` units.
            unsafe { return_sized(&v, buf, size) }
        }
        None => fail(ERROR_ENVVAR_NOT_FOUND, 0),
    }
}

/// `SetEnvironmentVariableW`: a null value deletes the variable.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetEnvironmentVariableW(name: *const u16, value: *const u16) -> i32 {
    // SAFETY: the caller's strings.
    let (Some(name), value) = (unsafe { wide_arg(name) }, unsafe { wide_arg(value) }) else {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    };
    with_state(|s| {
        // SAFETY: under the lock.
        let new = match env::set(unsafe { block() }, name, value) {
            Ok(b) => b,
            Err(e) => return fail(e, FALSE),
        };
        if set_block(s, &new) { TRUE } else { fail(ERROR_NOT_ENOUGH_MEMORY, FALSE) }
    })
}

/// `GetEnvironmentStringsW`: a copy of the block, freed with
/// `FreeEnvironmentStringsW`.
#[unsafe(no_mangle)]
pub extern "system" fn GetEnvironmentStringsW() -> *mut u16 {
    with_state(|_| {
        // SAFETY: under the lock.
        let b = unsafe { block() };
        // SAFETY: a valid heap.
        let p = unsafe { nt::RtlAllocateHeap(process_heap(), 0, b.len() * 2) } as *mut u16;
        if p.is_null() {
            return fail(ERROR_NOT_ENOUGH_MEMORY, p);
        }
        // SAFETY: `p` holds `b.len()` units.
        unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), p, b.len()) };
        p
    })
}

/// `FreeEnvironmentStringsW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn FreeEnvironmentStringsW(block: *mut u16) -> i32 {
    // SAFETY: a block `GetEnvironmentStringsW` returned.
    unsafe { nt::RtlFreeHeap(process_heap(), 0, block as *mut u8) as i32 }
}

/// `ExpandEnvironmentStringsW`: Learn: returns the size of the expanded
/// string *including* the NUL; if the buffer is too small, that size and
/// nothing is copied.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ExpandEnvironmentStringsW(src: *const u16, dst: *mut u16, size: u32) -> u32 {
    // SAFETY: the caller's string.
    let Some(src) = (unsafe { wide_arg(src) }) else { return fail(ERROR_INVALID_PARAMETER, 0) };
    let out = env::expand(src, &env_var);
    let needed = out.len() as u32 + 1;
    if size >= needed {
        // SAFETY: the caller's buffer holds `size` units.
        unsafe { put_wide(dst, &out) };
    }
    needed
}

// ----- the command line --------------------------------------------------------------

/// `GetCommandLineW`: the process parameters' command line.
#[unsafe(no_mangle)]
pub extern "system" fn GetCommandLineW() -> *const u16 {
    static EMPTY: [u16; 1] = [0];
    let pp = parameters();
    if pp == 0 {
        return EMPTY.as_ptr();
    }
    // SAFETY: a UNICODE_STRING the kernel wrote, NUL-terminated (ADR 0073).
    let buf = unsafe { ((pp + PP_COMMAND_LINE + 8) as *const *const u16).read() };
    if buf.is_null() { EMPTY.as_ptr() } else { buf }
}

/// `GetCommandLineA`: the command line in the ANSI code page, converted
/// once.
#[unsafe(no_mangle)]
pub extern "system" fn GetCommandLineA() -> *const u8 {
    static ANSI: AtomicPtr<u8> = AtomicPtr::new(core::ptr::null_mut());
    let p = ANSI.load(Ordering::Acquire);
    if !p.is_null() {
        return p;
    }
    // SAFETY: a NUL-terminated string.
    let w = unsafe { text::wide_at(GetCommandLineW()) };
    let mut a = text::to_ansi(w);
    a.push(0);
    let leaked = alloc::boxed::Box::leak(a.into_boxed_slice()).as_mut_ptr();
    match ANSI.compare_exchange(core::ptr::null_mut(), leaked, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => leaked,
        Err(other) => other,
    }
}

/// The executable's DOS path.
pub fn image_path() -> Vec<u16> {
    let pp = parameters();
    if pp == 0 {
        return Vec::new();
    }
    // SAFETY: a UNICODE_STRING the kernel wrote.
    unsafe { unicode_at(pp + PP_IMAGE_PATH).to_vec() }
}

// ----- the current directory ------------------------------------------------------------

/// Poly's own: the starting current directory in the process parameters
/// (`UNICODE_STRING`, ADR 0079, `poly_kernelbase::process`).
const PP_CURRENT_DIRECTORY: u64 = poly_kernelbase::process::PP_CURRENT_DIRECTORY as u64;

/// The initial current directory: the one the creator passed
/// (`CreateProcessW`'s `lpCurrentDirectory`, or the creator's own), else
/// the executable's directory (a process the kernel started, ADR 0079).
pub fn init_current_dir() {
    let pp = parameters();
    // SAFETY: the parameters' UNICODE_STRING (zero if not set).
    let given = if pp == 0 { Vec::new() } else { unsafe { unicode_at(pp + PP_CURRENT_DIRECTORY).to_vec() } };
    if !given.is_empty() {
        let dir = poly_kernelbase::path::as_current_dir(&given);
        with_state(|s| s.current_dir = dir);
        return;
    }
    let img = image_path();
    let dir = match img.iter().rposition(|&c| c == u16::from(b'\\')) {
        Some(i) => poly_kernelbase::path::as_current_dir(&img[..=i]),
        None => poly_kernelbase::wide("C:\\"),
    };
    with_state(|s| s.current_dir = dir);
}

/// `GetCurrentDirectoryW`: the "size" convention.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetCurrentDirectoryW(size: u32, buf: *mut u16) -> u32 {
    let cur = current_dir();
    // SAFETY: the caller's buffer.
    unsafe { return_sized(&cur, buf, size) }
}

/// `SetCurrentDirectoryW`: the directory must exist (it is opened as a
/// directory: a file is `ERROR_DIRECTORY`, a missing one
/// `ERROR_FILE_NOT_FOUND` / `ERROR_PATH_NOT_FOUND`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetCurrentDirectoryW(path: *const u16) -> i32 {
    // SAFETY: the caller's string.
    let Some(path) = (unsafe { wide_arg(path) }) else { return fail(ERROR_INVALID_PARAMETER, FALSE) };
    let (full, _) = match full_path(path) {
        Ok(f) => f,
        Err(e) => return fail(e, FALSE),
    };
    let name = NtName::new(poly_kernelbase::path::to_nt(&full));
    let oa = name.attributes();
    let mut h: Handle = 0;
    let mut iosb = nt::IoStatusBlock::default();
    use poly_kernelbase::file::*;
    // SAFETY: valid pointers.
    let st = unsafe {
        nt::NtOpenFile(
            &mut h,
            FILE_LIST_DIRECTORY | SYNCHRONIZE,
            &oa,
            &mut iosb,
            7,
            FILE_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT | FILE_OPEN_FOR_BACKUP_INTENT,
        )
    };
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    close(h);
    let dir = poly_kernelbase::path::as_current_dir(&full);
    with_state(|s| s.current_dir = dir);
    TRUE
}

// ----- standard handles -------------------------------------------------------------------

/// `STD_INPUT_HANDLE`, `STD_OUTPUT_HANDLE`, `STD_ERROR_HANDLE`
/// (`processenv.h`, as `DWORD`s).
const STD_INPUT_HANDLE: u32 = -10i32 as u32;
const STD_ERROR_HANDLE: u32 = -12i32 as u32;

fn std_index(which: u32) -> Option<usize> {
    if (STD_ERROR_HANDLE..=STD_INPUT_HANDLE).contains(&which) {
        Some((STD_INPUT_HANDLE - which) as usize)
    } else {
        None
    }
}

/// Open the console for one standard handle (ADR 0079).
fn open_console(write: bool) -> Handle {
    use poly_kernelbase::file::*;
    let name = NtName::new(poly_kernelbase::wide("\\??\\Z:\\dev\\console"));
    let oa = name.attributes();
    let mut h: Handle = 0;
    let mut iosb = nt::IoStatusBlock::default();
    let access = if write { GENERIC_WRITE } else { GENERIC_READ };
    // SAFETY: valid pointers.
    let st = unsafe {
        nt::NtCreateFile(
            &mut h,
            access | SYNCHRONIZE | FILE_READ_ATTRIBUTES,
            &oa,
            &mut iosb,
            core::ptr::null(),
            0,
            3,
            FILE_OPEN,
            FILE_SYNCHRONOUS_IO_NONALERT | FILE_NON_DIRECTORY_FILE,
            core::ptr::null(),
            0,
        )
    };
    if st.is_success() { h } else { 0 }
}

static STD_OPENED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// The standard handles, opened on first use.
fn std_handles() -> [Handle; 3] {
    if !STD_OPENED.load(Ordering::Acquire) {
        let opened = [open_console(false), open_console(true), open_console(true)];
        let keep = with_state(|s| {
            if STD_OPENED.swap(true, Ordering::AcqRel) {
                false
            } else {
                for (slot, h) in s.std.iter_mut().zip(opened) {
                    if *slot == 0 {
                        *slot = h;
                    } else if h != 0 {
                        close(h);
                    }
                }
                true
            }
        });
        if !keep {
            for h in opened.into_iter().filter(|&h| h != 0) {
                close(h);
            }
        }
    }
    with_state(|s| s.std)
}

/// `GetStdHandle`: an unknown selector is `INVALID_HANDLE_VALUE` with
/// `ERROR_INVALID_HANDLE` (Learn).
#[unsafe(no_mangle)]
pub extern "system" fn GetStdHandle(which: u32) -> Handle {
    match std_index(which) {
        Some(i) => std_handles()[i],
        None => fail(ERROR_INVALID_HANDLE, INVALID_HANDLE_VALUE),
    }
}

/// `SetStdHandle`.
#[unsafe(no_mangle)]
pub extern "system" fn SetStdHandle(which: u32, handle: Handle) -> i32 {
    match std_index(which) {
        Some(i) => {
            let _ = std_handles();
            with_state(|s| s.std[i] = handle);
            TRUE
        }
        None => fail(ERROR_INVALID_HANDLE, FALSE),
    }
}

// ----- identities and exit ----------------------------------------------------------------

/// `GetCurrentProcess`: the pseudo-handle -1.
#[unsafe(no_mangle)]
pub extern "system" fn GetCurrentProcess() -> Handle {
    nt::CURRENT_PROCESS
}

/// `GetCurrentThread`: the pseudo-handle -2.
#[unsafe(no_mangle)]
pub extern "system" fn GetCurrentThread() -> Handle {
    nt::CURRENT_THREAD
}

/// `GetCurrentProcessId`: `TEB.ClientId.UniqueProcess` (TEB + 0x40, ADR
/// 0063).
#[unsafe(no_mangle)]
pub extern "system" fn GetCurrentProcessId() -> u32 {
    // SAFETY: the TEB is valid.
    unsafe { ((teb() + 0x40) as *const u64).read() as u32 }
}

/// `GetCurrentThreadId`: `TEB.ClientId.UniqueThread` (TEB + 0x48).
#[unsafe(no_mangle)]
pub extern "system" fn GetCurrentThreadId() -> u32 {
    // SAFETY: the TEB is valid.
    unsafe { ((teb() + 0x48) as *const u64).read() as u32 }
}

/// `ExitProcess`: every DLL's `DLL_PROCESS_DETACH`, then the end of the
/// process (`RtlExitUserProcess`, ADR 0071).
#[unsafe(no_mangle)]
pub extern "system" fn ExitProcess(code: u32) -> ! {
    // SAFETY: never returns.
    unsafe { nt::RtlExitUserProcess(code) }
}

/// `TerminateProcess`: no detach notifications.
#[unsafe(no_mangle)]
pub extern "system" fn TerminateProcess(process: Handle, code: u32) -> i32 {
    // SAFETY: a handle value; the kernel checks it.
    let st = unsafe { nt::NtTerminateProcess(process, poly_ntobj::NtStatus(code)) };
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}

// ----- modules ------------------------------------------------------------------------------

/// The loaded modules from the PEB's `InMemoryOrderModuleList` (`PEB.Ldr`
/// at PEB + 0x18, the list head at + 0x20 of `PEB_LDR_DATA`, each entry's
/// links at + 0x10, `DllBase` at + 0x30, `FullDllName` at + 0x48;
/// `winternl.h`): `(base, full path)`.
fn modules() -> Vec<(u64, Vec<u16>)> {
    let mut out = Vec::new();
    // SAFETY: ntdll keeps the loader data in process memory (ADR 0071);
    // the list is circular through its head.
    unsafe {
        let ldr = ((peb() + 0x18) as *const u64).read();
        if ldr == 0 {
            return out;
        }
        let head = ldr + 0x20;
        let mut link = (head as *const u64).read();
        let mut guard = 0;
        while link != head && link != 0 && guard < 4096 {
            let entry = link - 0x10;
            let base = ((entry + 0x30) as *const u64).read();
            out.push((base, unicode_at(entry + 0x48).to_vec()));
            link = (link as *const u64).read();
            guard += 1;
        }
    }
    out
}

/// Whether the image at `base` is a DLL (`IMAGE_FILE_DLL` in the COFF
/// header's characteristics, PE format).
fn is_dll(base: u64) -> bool {
    // SAFETY: a mapped image's headers are readable.
    unsafe {
        let nt_off = ((base + 0x3C) as *const u32).read() as u64;
        let ch = ((base + nt_off + 0x16) as *const u16).read();
        ch & 0x2000 != 0
    }
}

/// The executable's base (`GetModuleHandleW(NULL)`).
pub fn exe_base() -> u64 {
    modules().into_iter().map(|(b, _)| b).find(|&b| b != 0 && !is_dll(b)).unwrap_or(0)
}

/// `GetModuleHandleW`: Learn: NULL is the executable; a name without an
/// extension gets `.dll`; only loaded modules are found
/// (`ERROR_MOD_NOT_FOUND`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetModuleHandleW(name: *const u16) -> *mut u8 {
    // SAFETY: the caller's string.
    let Some(name) = (unsafe { wide_arg(name) }) else { return exe_base() as *mut u8 };
    let s = nt::UnicodeString::new(name);
    let mut h = core::ptr::null_mut();
    // SAFETY: valid pointers.
    let st = unsafe { nt::LdrGetDllHandle(core::ptr::null(), core::ptr::null_mut(), &s, &mut h) };
    if st.is_success() { h } else { fail_status(st, core::ptr::null_mut()) }
}

/// `GetModuleFileNameW`: the module's full path, with the "truncate"
/// convention (Learn: a too-small buffer gets `nSize - 1` characters and a
/// NUL, the function returns `nSize` and sets `ERROR_INSUFFICIENT_BUFFER`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetModuleFileNameW(module: *mut u8, buf: *mut u16, size: u32) -> u32 {
    let base = if module.is_null() { exe_base() } else { module as u64 };
    let Some((_, path)) = modules().into_iter().find(|(b, _)| *b == base) else {
        return fail(ERROR_MOD_NOT_FOUND, 0);
    };
    let (n, ret, short) = text::truncated(path.len(), size);
    if size > 0 {
        // SAFETY: the caller's buffer of `size` units; `n < size`.
        unsafe { put_wide(buf, &path[..n]) };
    }
    if short {
        set_last_error(ERROR_INSUFFICIENT_BUFFER);
    }
    ret
}

/// `LoadLibraryExW`: `LdrLoadDll`. A name with a relative path is made
/// absolute against the current directory first. The data-file and
/// search-path flags are not supported yet (Poly loads the module as an
/// image either way, ADR 0079).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LoadLibraryExW(name: *const u16, file: Handle, flags: u32) -> *mut u8 {
    let _ = (file, flags);
    // SAFETY: the caller's string.
    let Some(name) = (unsafe { wide_arg(name) }) else { return fail(ERROR_INVALID_PARAMETER, core::ptr::null_mut()) };
    let has_path = name.iter().any(|&c| c == u16::from(b'\\') || c == u16::from(b'/'));
    let owned: Vec<u16> = if has_path {
        match full_path(name) {
            Ok((p, _)) => p,
            Err(e) => return fail(e, core::ptr::null_mut()),
        }
    } else {
        name.to_vec()
    };
    let s = nt::UnicodeString::new(&owned);
    let mut h = core::ptr::null_mut();
    // SAFETY: valid pointers.
    let st = unsafe { nt::LdrLoadDll(core::ptr::null(), core::ptr::null_mut(), &s, &mut h) };
    if st.is_success() { h } else { fail_status(st, core::ptr::null_mut()) }
}

/// `LoadLibraryW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LoadLibraryW(name: *const u16) -> *mut u8 {
    // SAFETY: as `LoadLibraryExW`.
    unsafe { LoadLibraryExW(name, 0, 0) }
}

/// `FreeLibrary`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn FreeLibrary(module: *mut u8) -> i32 {
    // SAFETY: a module handle; ntdll checks it.
    let st = unsafe { nt::LdrUnloadDll(module) };
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}

/// `GetProcAddress`: a name, or an ordinal in the low word of the pointer
/// (`MAKEINTRESOURCE`, Learn).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetProcAddress(module: *mut u8, name: *const u8) -> *mut u8 {
    let mut addr = core::ptr::null_mut();
    let module = if module.is_null() { exe_base() as *mut u8 } else { module };
    let st = if (name as usize) >> 16 == 0 {
        // SAFETY: valid pointers; by ordinal.
        unsafe { nt::LdrGetProcedureAddress(module, core::ptr::null(), name as u32, &mut addr) }
    } else {
        // SAFETY: the caller's NUL-terminated name.
        let len = unsafe { text::strlen(name) }.min(0xFFFE) as u16;
        let s = nt::AnsiString { length: len, maximum: len, buffer: name };
        // SAFETY: valid pointers.
        unsafe { nt::LdrGetProcedureAddress(module, &s, 0, &mut addr) }
    };
    if st.is_success() { addr } else { fail_status(st, core::ptr::null_mut()) }
}

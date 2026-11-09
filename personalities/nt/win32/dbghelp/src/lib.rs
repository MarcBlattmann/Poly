//! Our `dbghelp.dll` (ADR 0155): what Chrome and its crash handler
//! (crashpad) import. Poly has no symbol files and does not write minidumps
//! yet, so the symbol handler initialises and finds nothing
//! (`ERROR_MOD_NOT_FOUND`, Learn's error for an address in no loaded
//! module), `StackWalk64` ends the walk at once, and `MiniDumpWriteDump`
//! fails (`ERROR_NOT_SUPPORTED`): Chrome's crash reporting then reports that
//! it could not write the dump and the process ends as it would anyway.
//! The handler itself starts and waits for crashes.

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

use core::sync::atomic::{AtomicU32, Ordering};
use poly_win32_netrt as rt;

const TRUE: i32 = 1;
const FALSE: i32 = 0;
/// `winerror.h`.
const ERROR_NOT_SUPPORTED: u32 = 50;
const ERROR_MOD_NOT_FOUND: u32 = 126;
const ERROR_INVALID_ADDRESS: u32 = 487;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

fn no(e: u32) -> i32 {
    rt::set_error(e);
    FALSE
}

/// `SymInitialize(hProcess, UserSearchPath, fInvadeProcess)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SymInitialize(_process: isize, _path: *const u8, _invade: i32) -> i32 {
    TRUE
}

/// `SymInitializeW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SymInitializeW(_process: isize, _path: *const u16, _invade: i32) -> i32 {
    TRUE
}

/// `SymCleanup(hProcess)`.
#[unsafe(no_mangle)]
pub extern "system" fn SymCleanup(_process: isize) -> i32 {
    TRUE
}

static OPTIONS: AtomicU32 = AtomicU32::new(0x2); // SYMOPT_CASE_INSENSITIVE, dbghelp.h

/// `SymSetOptions(SymOptions)`: returns the options now set.
#[unsafe(no_mangle)]
pub extern "system" fn SymSetOptions(options: u32) -> u32 {
    OPTIONS.store(options, Ordering::SeqCst);
    options
}

/// `SymGetOptions()`.
#[unsafe(no_mangle)]
pub extern "system" fn SymGetOptions() -> u32 {
    OPTIONS.load(Ordering::SeqCst)
}

/// `SymSetSearchPathW(hProcess, SearchPath)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SymSetSearchPathW(_process: isize, _path: *const u16) -> i32 {
    TRUE
}

/// `SymGetSearchPathW(hProcess, SearchPath, SearchPathLength)`: the path is empty.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SymGetSearchPathW(_process: isize, path: *mut u16, length: u32) -> i32 {
    if path.is_null() || length == 0 {
        return no(ERROR_INSUFFICIENT_BUFFER);
    }
    // SAFETY: the caller's buffer holds at least one character.
    unsafe { path.write(0) };
    TRUE
}

/// `SymFromAddr(hProcess, Address, Displacement, Symbol)`: no module is loaded for symbols.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SymFromAddr(
    _process: isize,
    _address: u64,
    _displacement: *mut u64,
    _symbol: *mut u8,
) -> i32 {
    no(ERROR_MOD_NOT_FOUND)
}

/// `SymGetLineFromAddr64(hProcess, qwAddr, pdwDisplacement, Line64)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SymGetLineFromAddr64(
    _process: isize,
    _address: u64,
    _displacement: *mut u32,
    _line: *mut u8,
) -> i32 {
    no(ERROR_MOD_NOT_FOUND)
}

/// `SymGetModuleInfo64(hProcess, qwAddr, ModuleInfo)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SymGetModuleInfo64(_process: isize, _address: u64, _info: *mut u8) -> i32 {
    no(ERROR_MOD_NOT_FOUND)
}

/// `SymGetModuleBase64(hProcess, qwAddr)`: 0, no module.
#[unsafe(no_mangle)]
pub extern "system" fn SymGetModuleBase64(_process: isize, _address: u64) -> u64 {
    rt::set_error(ERROR_MOD_NOT_FOUND);
    0
}

/// `SymFunctionTableAccess64(hProcess, AddrBase)`: no function table.
#[unsafe(no_mangle)]
pub extern "system" fn SymFunctionTableAccess64(_process: isize, _address: u64) -> *mut u8 {
    core::ptr::null_mut()
}

/// `SymSrvGetFileIndexInfo(File, Info, Flags)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SymSrvGetFileIndexInfo(_file: *const u8, _info: *mut u8, _flags: u32) -> i32 {
    no(ERROR_NOT_SUPPORTED)
}

/// `StackWalk64(...)`: the walk ends at once (`FALSE`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn StackWalk64(
    _machine: u32,
    _process: isize,
    _thread: isize,
    _frame: *mut u8,
    _context: *mut u8,
    _read_memory: usize,
    _function_table: usize,
    _module_base: usize,
    _translate: usize,
) -> i32 {
    no(ERROR_INVALID_ADDRESS)
}

/// `MiniDumpWriteDump(hProcess, ProcessId, hFile, DumpType, ExceptionParam, UserStreamParam,
/// CallbackParam)`: no dump is written.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn MiniDumpWriteDump(
    _process: isize,
    _pid: u32,
    _file: isize,
    _dump_type: u32,
    _exception: *const u8,
    _user_streams: *const u8,
    _callback: *const u8,
) -> i32 {
    no(ERROR_NOT_SUPPORTED)
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn dbghelp_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}

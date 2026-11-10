//! Our `shell32.dll` (ADR 0155): `CommandLineToArgvW` and `SHGetFolderPathW`, which Chrome's
//! processes call when they start (its crash handler reads its arguments with the first and
//! finds the user's local application data with the second). The behaviour is Learn's
//! (`shellapi.h`, `shlobj_core.h`); the logic is `poly_kernelbase::shell`. Every other
//! shell32 function is bound to polystub by the loader.

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

use alloc::vec::Vec;
use poly_kernelbase::shell;
use poly_win32_rt as _;

/// `ERROR_INVALID_PARAMETER` (`winerror.h`).
const ERROR_INVALID_PARAMETER: u32 = 87;
/// `ERROR_NOT_ENOUGH_MEMORY` (`winerror.h`).
const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
/// `MAX_PATH` (`minwindef.h`): the size of the buffer `SHGetFolderPathW` fills.
const MAX_PATH: usize = 260;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetProcessHeap() -> *mut u8;
    fn HeapAlloc(heap: *mut u8, flags: u32, bytes: usize) -> *mut u8;
    fn HeapFree(heap: *mut u8, flags: u32, p: *mut u8) -> i32;
    fn LocalAlloc(flags: u32, bytes: usize) -> *mut u8;
    fn SetLastError(error: u32);
    fn GetModuleFileNameW(module: *mut u8, buf: *mut u16, size: u32) -> u32;
    fn GetEnvironmentVariableW(name: *const u16, buf: *mut u16, size: u32) -> u32;
    fn CreateDirectoryW(name: *const u16, security: *const u8) -> i32;
}

/// The process heap (16-byte aligned) for `alloc`.
struct ProcessHeap;

// SAFETY: HeapAlloc returns 16-byte aligned blocks; stricter alignments are refused.
unsafe impl core::alloc::GlobalAlloc for ProcessHeap {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        if layout.align() > 16 {
            return core::ptr::null_mut();
        }
        // SAFETY: the process heap.
        unsafe { HeapAlloc(GetProcessHeap(), 0, layout.size().max(1)) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, _: core::alloc::Layout) {
        // SAFETY: a block from `alloc`.
        unsafe { HeapFree(GetProcessHeap(), 0, ptr) };
    }
}

#[global_allocator]
static ALLOC: ProcessHeap = ProcessHeap;

/// `CommandLineToArgvW`: the arguments of a command line, in one `LocalAlloc` block the
/// caller frees with `LocalFree`. An empty command line gives the executable's path.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CommandLineToArgvW(cmd_line: *const u16, num_args: *mut i32) -> *mut *mut u16 {
    if cmd_line.is_null() || num_args.is_null() {
        // SAFETY: a plain call.
        unsafe { SetLastError(ERROR_INVALID_PARAMETER) };
        return core::ptr::null_mut();
    }
    let mut len = 0;
    // SAFETY: a NUL-terminated string.
    while unsafe { *cmd_line.add(len) } != 0 {
        len += 1;
    }
    // SAFETY: `len` characters before the NUL.
    let mut args = shell::command_line_to_argv(unsafe { core::slice::from_raw_parts(cmd_line, len) });
    if args.is_empty() {
        let mut path = [0u16; 1024];
        // SAFETY: the buffer holds 1024 characters.
        let n = unsafe { GetModuleFileNameW(core::ptr::null_mut(), path.as_mut_ptr(), 1024) } as usize;
        args.push(Vec::from(&path[..n.min(1023)]));
    }
    let size = shell::argv_block_size(&args);
    // SAFETY: LMEM_FIXED (0): the handle is the block's address.
    let block = unsafe { LocalAlloc(0, size) };
    if block.is_null() {
        // SAFETY: a plain call.
        unsafe { SetLastError(ERROR_NOT_ENOUGH_MEMORY) };
        return core::ptr::null_mut();
    }
    // SAFETY: `block` is `size` writable bytes; `num_args` is the caller's out-pointer.
    unsafe {
        shell::write_argv_block(&args, block as u64, core::slice::from_raw_parts_mut(block, size));
        num_args.write(args.len() as i32);
    }
    block.cast()
}

/// `SHGetFolderPathW`: the path of a `CSIDL` folder (no trailing backslash) in the caller's
/// `MAX_PATH` buffer. `hwnd`, the token (the current user) and `SHGFP_TYPE_*` (current and
/// default paths are the same on Poly) do not change the result.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SHGetFolderPathW(
    _hwnd: *mut u8,
    folder: i32,
    _token: *mut u8,
    _flags: u32,
    path: *mut u16,
) -> u32 {
    if path.is_null() {
        return shell::E_INVALIDARG;
    }
    let f = match shell::folder(folder as u32) {
        Ok(f) => f,
        Err(e) => return e,
    };
    let mut name = [0u16; 32];
    for (to, c) in name.iter_mut().zip(f.variable.encode_utf16()) {
        *to = c;
    }
    let mut out = [0u16; MAX_PATH];
    // SAFETY: `name` is NUL-terminated (shorter than 32); `out` holds MAX_PATH characters.
    let n = unsafe { GetEnvironmentVariableW(name.as_ptr(), out.as_mut_ptr(), MAX_PATH as u32) } as usize;
    let suffix = f.suffix.encode_utf16().count();
    if n == 0 || n + suffix >= MAX_PATH {
        return shell::E_FAIL;
    }
    for (to, c) in out[n..].iter_mut().zip(f.suffix.encode_utf16()) {
        *to = c;
    }
    if folder as u32 & shell::CSIDL_FLAG_CREATE != 0 {
        // SAFETY: `out` is NUL-terminated. An existing folder is fine.
        unsafe { CreateDirectoryW(out.as_ptr(), core::ptr::null()) };
    }
    // SAFETY: Learn: the caller's buffer is MAX_PATH characters.
    unsafe { core::ptr::copy_nonoverlapping(out.as_ptr(), path, n + suffix + 1) };
    shell::S_OK
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn shell32_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}

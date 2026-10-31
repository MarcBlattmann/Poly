//! What our `bcrypt.dll` and `crypt32.dll` share (ADR 0096):
//!
//! - the global allocator: the process heap (`HeapAlloc`, 16-byte aligned);
//! - [`SpinLock`], for the few globals (no threads are blocked for long);
//! - [`Handles`]: the addresses of live objects, so a handle a caller
//!   passes is checked before it is dereferenced (a stale or made-up
//!   handle is refused, not followed);
//! - wide string helpers and [`random`] (`advapi32`'s `SystemFunction036`,
//!   the kernel's `/dev/urandom`, as Learn's "system-preferred RNG").

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};
use poly_win32_rt as _;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetProcessHeap() -> *mut u8;
    fn HeapAlloc(heap: *mut u8, flags: u32, bytes: usize) -> *mut u8;
    fn HeapFree(heap: *mut u8, flags: u32, p: *mut u8) -> i32;
    fn LocalAlloc(flags: u32, bytes: usize) -> *mut u8;
    fn SetLastError(e: u32);
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SystemFunction036(buffer: *mut u8, length: u32) -> u8;
}

struct ProcessHeap;

// SAFETY: HeapAlloc returns 16-byte aligned blocks; stricter alignments
// are refused.
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

/// A spin lock around a value.
pub struct SpinLock<T> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
}

// SAFETY: the value is reached only while holding the lock.
unsafe impl<T: Send> Sync for SpinLock<T> {}

impl<T> SpinLock<T> {
    pub const fn new(value: T) -> Self {
        SpinLock { locked: AtomicBool::new(false), value: UnsafeCell::new(value) }
    }

    /// Run `f` with the value locked.
    pub fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        while self.locked.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            core::hint::spin_loop();
        }
        // SAFETY: the lock is held, so this is the only reference.
        let r = f(unsafe { &mut *self.value.get() });
        self.locked.store(false, Ordering::Release);
        r
    }
}

/// The addresses of live boxed objects of one DLL.
pub struct Handles(SpinLock<Vec<usize>>);

impl Handles {
    pub const fn new() -> Self {
        Handles(SpinLock::new(Vec::new()))
    }

    /// Box `v` and register it; its address is the handle.
    pub fn add<T>(&self, v: T) -> usize {
        let p = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(v)) as usize;
        self.0.with(|l| l.push(p));
        p
    }

    /// Whether `h` is live.
    pub fn contains(&self, h: usize) -> bool {
        h != 0 && self.0.with(|l| l.contains(&h))
    }

    /// Unregister `h`; true if it was live. The caller then frees it with
    /// [`Handles::free`].
    pub fn remove(&self, h: usize) -> bool {
        self.0.with(|l| match l.iter().position(|&x| x == h) {
            Some(i) => {
                l.swap_remove(i);
                true
            }
            None => false,
        })
    }

    /// Free an object [`Handles::add`] made, after [`Handles::remove`].
    ///
    /// # Safety
    /// `h` came from `add::<T>` and was removed; nothing else uses it.
    pub unsafe fn free<T>(h: usize) {
        // SAFETY: the caller's contract: a Box<T> leaked by `add`.
        drop(unsafe { alloc::boxed::Box::from_raw(h as *mut T) });
    }
}

impl Default for Handles {
    fn default() -> Self {
        Self::new()
    }
}

/// Fill `buf` with random bytes; false if the random device failed.
pub fn random(buf: &mut [u8]) -> bool {
    let mut done = 0;
    while done < buf.len() {
        let n = (buf.len() - done).min(1 << 20);
        // SAFETY: `n` bytes of the caller's buffer.
        if unsafe { SystemFunction036(buf.as_mut_ptr().add(done), n as u32) } == 0 {
            return false;
        }
        done += n;
    }
    true
}

/// [`random`] as the closure the algorithms take (failures leave zeros,
/// which the callers' checks of [`random`] up front rule out).
pub fn rng(buf: &mut [u8]) {
    let _ = random(buf);
}

/// The length of a NUL-terminated UTF-16 string.
///
/// # Safety
/// `s` is a NUL-terminated string.
pub unsafe fn wcslen(s: *const u16) -> usize {
    let mut n = 0;
    // SAFETY: the caller's contract.
    while unsafe { *s.add(n) } != 0 {
        n += 1;
    }
    n
}

/// A NUL-terminated UTF-16 string as a `String` (None for NULL or
/// unpaired surrogates).
///
/// # Safety
/// `s` is null or a NUL-terminated string.
pub unsafe fn wide_str(s: *const u16) -> Option<String> {
    if s.is_null() {
        return None;
    }
    // SAFETY: the caller's contract.
    let units = unsafe { core::slice::from_raw_parts(s, wcslen(s)) };
    String::from_utf16(units).ok()
}

/// A NUL-terminated byte string as a `String` (Latin-1 bytes; None for
/// NULL).
///
/// # Safety
/// `s` is null or a NUL-terminated string.
pub unsafe fn ansi_str(s: *const u8) -> Option<String> {
    if s.is_null() {
        return None;
    }
    let mut n = 0;
    // SAFETY: the caller's contract.
    while unsafe { *s.add(n) } != 0 {
        n += 1;
    }
    // SAFETY: `n` bytes before the NUL.
    let b = unsafe { core::slice::from_raw_parts(s, n) };
    Some(b.iter().map(|&c| char::from(c)).collect())
}

/// `len` bytes at `p` (an empty slice for NULL or zero).
///
/// # Safety
/// `p` is null or readable for `len` bytes, and not written meanwhile.
pub unsafe fn bytes<'a>(p: *const u8, len: usize) -> &'a [u8] {
    if p.is_null() || len == 0 {
        &[]
    } else {
        // SAFETY: the caller's contract.
        unsafe { core::slice::from_raw_parts(p, len) }
    }
}

/// `len` writable bytes at `p` (empty for NULL or zero).
///
/// # Safety
/// `p` is null or writable for `len` bytes, with no other reference.
pub unsafe fn bytes_mut<'a>(p: *mut u8, len: usize) -> &'a mut [u8] {
    if p.is_null() || len == 0 {
        &mut []
    } else {
        // SAFETY: the caller's contract.
        unsafe { core::slice::from_raw_parts_mut(p, len) }
    }
}

/// `LocalAlloc(LMEM_FIXED)` copy of `data` (what CryptoAPI returns for the
/// caller to `LocalFree`), or null.
pub fn local_copy(data: &[u8]) -> *mut u8 {
    // SAFETY: no preconditions.
    let p = unsafe { LocalAlloc(0, data.len().max(1)) };
    if !p.is_null() {
        // SAFETY: a fresh block of at least `data.len()` bytes.
        unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len()) };
    }
    p
}

/// `LocalAlloc` of `len` zeroed bytes (`LPTR`).
pub fn local_alloc(len: usize) -> *mut u8 {
    // SAFETY: no preconditions; LMEM_ZEROINIT (0x40).
    unsafe { LocalAlloc(0x40, len.max(1)) }
}

/// Set the thread's last error.
pub fn set_last_error(e: u32) {
    // SAFETY: no preconditions.
    unsafe { SetLastError(e) }
}

// ----- Files (crypt32's system stores and DPAPI master keys) ------------------------------

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        sa: *const u8,
        disp: u32,
        flags: u32,
        tmpl: isize,
    ) -> isize;
    fn ReadFile(h: isize, buf: *mut u8, len: u32, read: *mut u32, ov: *mut u8) -> i32;
    fn WriteFile(h: isize, buf: *const u8, len: u32, written: *mut u32, ov: *mut u8) -> i32;
    fn CloseHandle(h: isize) -> i32;
    fn CreateDirectoryW(name: *const u16, sa: *const u8) -> i32;
    fn DeleteFileW(name: *const u16) -> i32;
    fn FindFirstFileW(pattern: *const u16, data: *mut u8) -> isize;
    fn FindNextFileW(h: isize, data: *mut u8) -> i32;
    fn FindClose(h: isize) -> i32;
    fn GetEnvironmentVariableW(name: *const u16, buf: *mut u16, size: u32) -> u32;
    fn GetSystemTimeAsFileTime(ft: *mut u64);
}

/// `s` as a NUL-terminated UTF-16 string.
pub fn wz(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// An environment variable.
pub fn env(name: &str) -> Option<String> {
    let n = wz(name);
    let mut buf = alloc::vec![0u16; 512];
    // SAFETY: a NUL-terminated name and a buffer of `buf.len()` characters.
    let len = unsafe { GetEnvironmentVariableW(n.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if len == 0 || len >= buf.len() {
        return None;
    }
    String::from_utf16(&buf[..len]).ok()
}

/// The current time as a FILETIME.
pub fn now() -> u64 {
    let mut ft = 0u64;
    // SAFETY: the out parameter is ours.
    unsafe { GetSystemTimeAsFileTime(&mut ft) };
    ft
}

/// A whole file, or None.
pub fn read_file(path: &str) -> Option<Vec<u8>> {
    let p = wz(path);
    // GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE, OPEN_EXISTING (`winnt.h`, `fileapi.h`).
    // SAFETY: a NUL-terminated name.
    let h = unsafe { CreateFileW(p.as_ptr(), 0x8000_0000, 3, core::ptr::null(), 3, 0, 0) };
    if h == -1 {
        return None;
    }
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let mut n = 0u32;
        // SAFETY: our buffer.
        let ok = unsafe { ReadFile(h, buf.as_mut_ptr(), buf.len() as u32, &mut n, core::ptr::null_mut()) };
        if ok == 0 || n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n as usize]);
    }
    // SAFETY: our handle.
    unsafe { CloseHandle(h) };
    Some(out)
}

/// Write a whole file (replacing it); true on success.
pub fn write_file(path: &str, data: &[u8]) -> bool {
    let p = wz(path);
    // GENERIC_WRITE, no sharing, CREATE_ALWAYS.
    // SAFETY: a NUL-terminated name.
    let h = unsafe { CreateFileW(p.as_ptr(), 0x4000_0000, 0, core::ptr::null(), 2, 0, 0) };
    if h == -1 {
        return false;
    }
    let mut n = 0u32;
    // SAFETY: our data.
    let ok = unsafe { WriteFile(h, data.as_ptr(), data.len() as u32, &mut n, core::ptr::null_mut()) };
    // SAFETY: our handle.
    unsafe { CloseHandle(h) };
    ok != 0 && n as usize == data.len()
}

/// Delete a file; true on success.
pub fn delete_file(path: &str) -> bool {
    let p = wz(path);
    // SAFETY: a NUL-terminated name.
    unsafe { DeleteFileW(p.as_ptr()) != 0 }
}

/// Make a directory and its parents (existing ones are fine).
pub fn create_dirs(path: &str) {
    let mut at = 0;
    while let Some(i) = path[at..].find('\\') {
        let end = at + i;
        if end > 2 {
            let p = wz(&path[..end]);
            // SAFETY: a NUL-terminated name.
            unsafe { CreateDirectoryW(p.as_ptr(), core::ptr::null()) };
        }
        at = end + 1;
    }
    let p = wz(path);
    // SAFETY: a NUL-terminated name.
    unsafe { CreateDirectoryW(p.as_ptr(), core::ptr::null()) };
}

/// The names of the files (not directories) in a directory.
pub fn list_dir(dir: &str) -> Vec<String> {
    let mut out = Vec::new();
    let pattern = wz(&alloc::format!("{dir}\\*"));
    // WIN32_FIND_DATAW (592 bytes): dwFileAttributes at 0, cFileName at 44.
    let mut data = [0u8; 592];
    // SAFETY: a NUL-terminated pattern and a WIN32_FIND_DATAW buffer.
    let h = unsafe { FindFirstFileW(pattern.as_ptr(), data.as_mut_ptr()) };
    if h == -1 {
        return out;
    }
    loop {
        let attrs = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        // FILE_ATTRIBUTE_DIRECTORY (0x10).
        if attrs & 0x10 == 0 {
            let name: Vec<u16> = data[44..44 + 520]
                .chunks(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .take_while(|&c| c != 0)
                .collect();
            if let Ok(s) = String::from_utf16(&name) {
                out.push(s);
            }
        }
        // SAFETY: our search handle and buffer.
        if unsafe { FindNextFileW(h, data.as_mut_ptr()) } == 0 {
            break;
        }
    }
    // SAFETY: our search handle.
    unsafe { FindClose(h) };
    out
}

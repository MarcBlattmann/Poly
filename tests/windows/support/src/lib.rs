//! `poly-wtest`: the support library of the Windows test programs
//! (ADR 0075), the counterpart of `tests/linux/include/poly_test.h`.
//!
//! A test is one Rust file, `#![no_std]` and `#![no_main]`, built by
//! `cargo xtask pipeline win build` for `x86_64-pc-windows-msvc` into a
//! self-contained console `.exe` that imports from `ntdll.dll` (and, once
//! Poly has it, `kernel32.dll`) by name only, through `raw-dylib`
//! declarations: no C runtime, no import libraries, no Microsoft SDK.
//!
//! ```ignore
//! #![no_std]
//! #![no_main]
//! use poly_wtest::*;
//!
//! #[link(name = "ntdll", kind = "raw-dylib")]
//! unsafe extern "system" {
//!     fn NtQuerySystemTime(time: *mut i64) -> NTSTATUS;
//! }
//!
//! poly_wtest::main!(test);
//!
//! fn test(t: &mut Test) {
//!     let mut now = 0i64;
//!     // SAFETY: `now` is a valid, writable LARGE_INTEGER.
//!     let status = unsafe { NtQuerySystemTime(&mut now) };
//!     t.expect_status("query", status, STATUS_SUCCESS);
//!     t.check("positive", now > 0, now);
//! }
//! ```
//!
//! **Output protocol.** Observations go into the *observation file*,
//! `poly-test.out` in the directory of the executable, one line each,
//! byte for byte the format of the Linux corpus:
//!
//! ```text
//! ok <check>
//! not ok <check> (got <value>)
//! summary <passed> passed, <failed> failed
//! ```
//!
//! The process ends with `NtTerminateProcess(NtCurrentProcess(), code)`:
//! 0 when every check passed, 1 otherwise, [`EXIT_NO_OUTPUT`] when the
//! observation file cannot be created, [`EXIT_PANIC`] on a panic. Never
//! print addresses, handles, times or anything else that differs between
//! runs or machines: the comparison is exact.
//!
//! Why a file and not standard output: standard handles are a `kernel32`
//! (`GetStdHandle`) and console matter that Poly does not have yet, and the
//! native way to reach them goes through an undocumented field of
//! `RTL_USER_PROCESS_PARAMETERS`. The file needs only documented native
//! calls (`NtCreateFile`, `NtWriteFile`, `NtClose`, `NtTerminateProcess`)
//! and documented layouts (`winternl.h`: `TEB.ProcessEnvironmentBlock`,
//! `PEB.ProcessParameters`, `RTL_USER_PROCESS_PARAMETERS.ImagePathName`).
//! The Windows runner (`tools/pipeline/windows/poly-oracle.ps1`) reads the
//! file after the run; Poly's kernel reports it (`poly.test.stdout=`).
//!
//! Constants and layouts are ABI facts from the public SDK headers, cited
//! per block. Written for Poly from Microsoft Learn; no code is copied
//! from anywhere.

#![no_std]

use core::ffi::c_void;
use core::sync::atomic::{AtomicPtr, Ordering};

pub type NTSTATUS = i32;
pub type HANDLE = *mut c_void;

/// `NtCurrentProcess()`: the pseudo-handle of the calling process
/// (Learn: "ZwCurrentProcess" / "NtCurrentProcess macro", `(HANDLE)-1`).
pub const NT_CURRENT_PROCESS: HANDLE = -1isize as HANDLE;

// ntstatus.h
pub const STATUS_SUCCESS: NTSTATUS = 0;
pub const STATUS_TIMEOUT: NTSTATUS = 0x102;
pub const STATUS_PENDING: NTSTATUS = 0x103;
pub const STATUS_BUFFER_OVERFLOW: NTSTATUS = 0x8000_0005_u32 as i32;
pub const STATUS_NO_MORE_FILES: NTSTATUS = 0x8000_0006_u32 as i32;
pub const STATUS_NOT_IMPLEMENTED: NTSTATUS = 0xC000_0002_u32 as i32;
pub const STATUS_INFO_LENGTH_MISMATCH: NTSTATUS = 0xC000_0004_u32 as i32;
pub const STATUS_ACCESS_VIOLATION: NTSTATUS = 0xC000_0005_u32 as i32;
pub const STATUS_INVALID_HANDLE: NTSTATUS = 0xC000_0008_u32 as i32;
pub const STATUS_INVALID_PARAMETER: NTSTATUS = 0xC000_000D_u32 as i32;
pub const STATUS_END_OF_FILE: NTSTATUS = 0xC000_0011_u32 as i32;
pub const STATUS_CONFLICTING_ADDRESSES: NTSTATUS = 0xC000_0018_u32 as i32;
pub const STATUS_ACCESS_DENIED: NTSTATUS = 0xC000_0022_u32 as i32;
pub const STATUS_BUFFER_TOO_SMALL: NTSTATUS = 0xC000_0023_u32 as i32;
pub const STATUS_OBJECT_NAME_INVALID: NTSTATUS = 0xC000_0033_u32 as i32;
pub const STATUS_OBJECT_NAME_NOT_FOUND: NTSTATUS = 0xC000_0034_u32 as i32;
pub const STATUS_OBJECT_NAME_COLLISION: NTSTATUS = 0xC000_0035_u32 as i32;
pub const STATUS_OBJECT_PATH_NOT_FOUND: NTSTATUS = 0xC000_003A_u32 as i32;
pub const STATUS_SHARING_VIOLATION: NTSTATUS = 0xC000_0043_u32 as i32;
pub const STATUS_INVALID_PAGE_PROTECTION: NTSTATUS = 0xC000_0045_u32 as i32;
pub const STATUS_DELETE_PENDING: NTSTATUS = 0xC000_0056_u32 as i32;
pub const STATUS_FREE_VM_NOT_AT_BASE: NTSTATUS = 0xC000_009F_u32 as i32;
pub const STATUS_MEMORY_NOT_ALLOCATED: NTSTATUS = 0xC000_00A0_u32 as i32;

// winnt.h: memory allocation types and page protections.
pub const MEM_COMMIT: u32 = 0x1000;
pub const MEM_RESERVE: u32 = 0x2000;
pub const MEM_DECOMMIT: u32 = 0x4000;
pub const MEM_RELEASE: u32 = 0x8000;
pub const PAGE_NOACCESS: u32 = 0x01;
pub const PAGE_READONLY: u32 = 0x02;
pub const PAGE_READWRITE: u32 = 0x04;
pub const PAGE_EXECUTE: u32 = 0x10;
pub const PAGE_EXECUTE_READ: u32 = 0x20;
pub const PAGE_EXECUTE_READWRITE: u32 = 0x40;
pub const PAGE_GUARD: u32 = 0x100;

// winnt.h: access rights, share modes, file attributes.
pub const DELETE: u32 = 0x0001_0000;
pub const SYNCHRONIZE: u32 = 0x0010_0000;
pub const GENERIC_READ: u32 = 0x8000_0000;
pub const GENERIC_WRITE: u32 = 0x4000_0000;
pub const GENERIC_EXECUTE: u32 = 0x2000_0000;
pub const GENERIC_ALL: u32 = 0x1000_0000;
pub const FILE_READ_DATA: u32 = 0x0001;
pub const FILE_WRITE_DATA: u32 = 0x0002;
pub const FILE_APPEND_DATA: u32 = 0x0004;
pub const FILE_READ_ATTRIBUTES: u32 = 0x0080;
pub const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;
pub const FILE_SHARE_READ: u32 = 0x1;
pub const FILE_SHARE_WRITE: u32 = 0x2;
pub const FILE_SHARE_DELETE: u32 = 0x4;
pub const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
pub const FILE_ATTRIBUTE_TEMPORARY: u32 = 0x100;

// winternl.h: create dispositions and create options of NtCreateFile.
pub const FILE_SUPERSEDE: u32 = 0;
pub const FILE_OPEN: u32 = 1;
pub const FILE_CREATE: u32 = 2;
pub const FILE_OPEN_IF: u32 = 3;
pub const FILE_OVERWRITE: u32 = 4;
pub const FILE_OVERWRITE_IF: u32 = 5;
pub const FILE_DIRECTORY_FILE: u32 = 0x0000_0001;
pub const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x0000_0020;
pub const FILE_NON_DIRECTORY_FILE: u32 = 0x0000_0040;
pub const FILE_DELETE_ON_CLOSE: u32 = 0x0000_1000;

// wdm.h: IO_STATUS_BLOCK.Information after NtCreateFile.
pub const FILE_SUPERSEDED: usize = 0;
pub const FILE_OPENED: usize = 1;
pub const FILE_CREATED: usize = 2;
pub const FILE_OVERWRITTEN: usize = 3;
pub const FILE_EXISTS: usize = 4;
pub const FILE_DOES_NOT_EXIST: usize = 5;

// ntdef.h / winternl.h: object attributes.
pub const OBJ_CASE_INSENSITIVE: u32 = 0x40;

/// `UNICODE_STRING` (ntdef.h): lengths in bytes, not counting a NUL.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct UnicodeString {
    pub length: u16,
    pub maximum_length: u16,
    pub buffer: *const u16,
}

impl UnicodeString {
    /// The counted string over `s` (no NUL needed).
    pub fn of(s: &[u16]) -> UnicodeString {
        let bytes = (s.len() * 2) as u16;
        UnicodeString { length: bytes, maximum_length: bytes, buffer: s.as_ptr() }
    }
}

/// `OBJECT_ATTRIBUTES` (ntdef.h; `InitializeObjectAttributes`).
#[repr(C)]
#[derive(Debug)]
pub struct ObjectAttributes {
    pub length: u32,
    pub root_directory: HANDLE,
    pub object_name: *const UnicodeString,
    pub attributes: u32,
    pub security_descriptor: *const c_void,
    pub security_quality_of_service: *const c_void,
}

impl ObjectAttributes {
    /// `InitializeObjectAttributes(&oa, name, OBJ_CASE_INSENSITIVE, NULL, NULL)`.
    pub fn named(name: &UnicodeString) -> ObjectAttributes {
        ObjectAttributes {
            length: core::mem::size_of::<ObjectAttributes>() as u32,
            root_directory: core::ptr::null_mut(),
            object_name: name,
            attributes: OBJ_CASE_INSENSITIVE,
            security_descriptor: core::ptr::null(),
            security_quality_of_service: core::ptr::null(),
        }
    }
}

/// `IO_STATUS_BLOCK` (winternl.h): `Status` (in a union with `Pointer`)
/// and `Information`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct IoStatusBlock {
    pub status: usize,
    pub information: usize,
}

impl IoStatusBlock {
    pub const fn new() -> IoStatusBlock {
        IoStatusBlock { status: 0, information: 0 }
    }

    /// The `Status` member.
    pub fn nt_status(&self) -> NTSTATUS {
        self.status as u32 as i32
    }
}

impl Default for IoStatusBlock {
    fn default() -> Self {
        IoStatusBlock::new()
    }
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    /// Learn: `NtCreateFile` (winternl.h) / `ZwCreateFile` (wdm.h).
    pub fn NtCreateFile(
        file_handle: *mut HANDLE,
        desired_access: u32,
        object_attributes: *const ObjectAttributes,
        io_status_block: *mut IoStatusBlock,
        allocation_size: *const i64,
        file_attributes: u32,
        share_access: u32,
        create_disposition: u32,
        create_options: u32,
        ea_buffer: *const c_void,
        ea_length: u32,
    ) -> NTSTATUS;
    /// Learn: `ZwWriteFile` (wdm.h).
    pub fn NtWriteFile(
        file_handle: HANDLE,
        event: HANDLE,
        apc_routine: *const c_void,
        apc_context: *const c_void,
        io_status_block: *mut IoStatusBlock,
        buffer: *const c_void,
        length: u32,
        byte_offset: *const i64,
        key: *const u32,
    ) -> NTSTATUS;
    /// Learn: `ZwReadFile` (wdm.h).
    pub fn NtReadFile(
        file_handle: HANDLE,
        event: HANDLE,
        apc_routine: *const c_void,
        apc_context: *const c_void,
        io_status_block: *mut IoStatusBlock,
        buffer: *mut c_void,
        length: u32,
        byte_offset: *const i64,
        key: *const u32,
    ) -> NTSTATUS;
    /// Learn: `NtClose` (winternl.h) / `ZwClose` (wdm.h).
    pub fn NtClose(handle: HANDLE) -> NTSTATUS;
    /// Learn: `ZwTerminateProcess` (ntddk.h).
    pub fn NtTerminateProcess(process: HANDLE, exit_status: NTSTATUS) -> NTSTATUS;
}

/// The observation file's name, in the executable's directory.
pub const OUTPUT_FILE: &str = "poly-test.out";
/// Exit status when the observation file cannot be created.
pub const EXIT_NO_OUTPUT: u32 = 2;
/// Exit status after a panic (a bug in the test, not an observation).
pub const EXIT_PANIC: u32 = 3;

/// The longest NT path [`path_beside_image`] builds, in UTF-16 units.
pub const MAX_PATH_UNITS: usize = 600;

/// UTF-16 of an ASCII string literal, at compile time: `utf16!("a.txt")`
/// is a `&'static [u16; 5]`.
#[macro_export]
macro_rules! utf16 {
    ($s:literal) => {{
        const S: &str = $s;
        const W: [u16; S.len()] = $crate::ascii_utf16::<{ S.len() }>(S);
        &W
    }};
}

/// The `const fn` behind [`utf16!`]; the string must be ASCII.
pub const fn ascii_utf16<const N: usize>(s: &str) -> [u16; N] {
    let b = s.as_bytes();
    assert!(b.len() == N);
    let mut out = [0u16; N];
    let mut i = 0;
    while i < N {
        assert!(b[i] < 0x80, "utf16!: ASCII only");
        out[i] = b[i] as u16;
        i += 1;
    }
    out
}

/// An NT path in a fixed buffer (no allocator in a test program).
#[derive(Debug)]
pub struct NtPath {
    buf: [u16; MAX_PATH_UNITS],
    len: usize,
}

impl NtPath {
    /// The path's UTF-16 units.
    pub fn units(&self) -> &[u16] {
        &self.buf[..self.len]
    }

    /// The counted string over it; valid while `self` is.
    pub fn unicode_string(&self) -> UnicodeString {
        UnicodeString::of(self.units())
    }

    fn push(&mut self, units: &[u16]) -> bool {
        if self.len + units.len() > self.buf.len() {
            return false;
        }
        self.buf[self.len..self.len + units.len()].copy_from_slice(units);
        self.len += units.len();
        true
    }
}

/// The executable's own path, `RTL_USER_PROCESS_PARAMETERS.ImagePathName`
/// (winternl.h: `TEB.ProcessEnvironmentBlock` at 0x60 of the TEB, which
/// `GS` points at on x64 (winnt.h `NtCurrentTeb`); `PEB.ProcessParameters`
/// at 0x20; `ImagePathName` at 0x60).
pub fn image_path() -> &'static [u16] {
    let peb: *const u8;
    // SAFETY: on x64 Windows (and Poly's NT personality, ADR 0063) GS
    // points at the current thread's TEB, whose ProcessEnvironmentBlock
    // field (winternl.h) is at 0x60; reading it has no side effects.
    unsafe { core::arch::asm!("mov {}, gs:[0x60]", out(reg) peb, options(nostack, readonly, preserves_flags)) };
    // SAFETY: the PEB and the process parameters it points at live as long
    // as the process; the offsets are winternl.h's x64 layout, and
    // ImagePathName's buffer holds `Length` bytes.
    unsafe {
        let params = *(peb.add(0x20) as *const *const u8);
        if params.is_null() {
            return &[];
        }
        let name = &*(params.add(0x60) as *const UnicodeString);
        if name.buffer.is_null() {
            return &[];
        }
        core::slice::from_raw_parts(name.buffer, usize::from(name.length) / 2)
    }
}

/// The NT path (`\??\...`) of `name` in the executable's directory (the
/// scratch directory the runner made for this test, on Windows; on Poly the
/// initramfs root). A `\\?\` prefix becomes `\??\`, a UNC path
/// `\??\UNC\...`. `None` if it does not fit.
pub fn path_beside_image(name: &[u16]) -> Option<NtPath> {
    const BACKSLASH: u16 = b'\\' as u16;
    let image = image_path();
    let dir_len = image.iter().rposition(|&c| c == BACKSLASH).map_or(0, |i| i + 1);
    let dir = &image[..dir_len];
    let mut p = NtPath { buf: [0; MAX_PATH_UNITS], len: 0 };
    let ok = if dir.starts_with(utf16!("\\??\\")) {
        p.push(dir)
    } else if dir.starts_with(utf16!("\\\\?\\")) || dir.starts_with(utf16!("\\\\.\\")) {
        p.push(utf16!("\\??\\")) && p.push(&dir[4..])
    } else if dir.starts_with(utf16!("\\\\")) {
        p.push(utf16!("\\??\\UNC\\")) && p.push(&dir[2..])
    } else {
        p.push(utf16!("\\??\\")) && p.push(dir)
    };
    (ok && p.push(name)).then_some(p)
}

/// The observation file, once opened.
static OUTPUT: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());

fn write_raw(bytes: &[u8]) {
    let handle = OUTPUT.load(Ordering::Relaxed);
    if handle.is_null() {
        return;
    }
    let mut iosb = IoStatusBlock::new();
    // SAFETY: `handle` is the observation file opened for synchronous
    // writes at the current position; `bytes` and `iosb` outlive the call.
    unsafe {
        NtWriteFile(
            handle,
            core::ptr::null_mut(),
            core::ptr::null(),
            core::ptr::null(),
            &mut iosb,
            bytes.as_ptr().cast(),
            bytes.len() as u32,
            core::ptr::null(),
            core::ptr::null(),
        );
    }
}

/// Write `bytes` into the observation file as they are (no line end
/// added): for programs whose observations are not checks, such as the
/// 7-Zip scenario driver (ADR 0088).
pub fn write_output(bytes: &[u8]) {
    write_raw(bytes);
}

/// SHA-256 (FIPS 180-4, "Secure Hash Standard"), fed in pieces.
#[derive(Clone, Debug)]
pub struct Sha256 {
    state: [u32; 8],
    block: [u8; 64],
    used: usize,
    total: u64,
}

/// FIPS 180-4 section 4.2.2: the first 32 bits of the fractional parts of
/// the cube roots of the first 64 primes.
const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
    0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
    0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
    0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
    0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
    0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
    0xc67178f2,
];

impl Sha256 {
    /// FIPS 180-4 section 5.3.3: the initial hash value.
    pub const fn new() -> Sha256 {
        Sha256 {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
            ],
            block: [0; 64],
            used: 0,
            total: 0,
        }
    }

    fn compress(&mut self) {
        let mut w = [0u32; 64];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            *word = u32::from_be_bytes([
                self.block[4 * i],
                self.block[4 * i + 1],
                self.block[4 * i + 2],
                self.block[4 * i + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(SHA256_K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (s, v) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *s = s.wrapping_add(v);
        }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.total = self.total.wrapping_add(data.len() as u64);
        while !data.is_empty() {
            let n = (64 - self.used).min(data.len());
            self.block[self.used..self.used + n].copy_from_slice(&data[..n]);
            self.used += n;
            data = &data[n..];
            if self.used == 64 {
                self.compress();
                self.used = 0;
            }
        }
    }

    /// The digest (FIPS 180-4 section 5.1.1's padding, then the state).
    pub fn finish(mut self) -> [u8; 32] {
        let bits = self.total.wrapping_mul(8);
        self.update(&[0x80]);
        while self.used != 56 {
            self.update(&[0]);
        }
        self.update(&bits.to_be_bytes());
        let mut out = [0u8; 32];
        for (i, s) in self.state.iter().enumerate() {
            out[4 * i..4 * i + 4].copy_from_slice(&s.to_be_bytes());
        }
        out
    }
}

impl Default for Sha256 {
    fn default() -> Self {
        Sha256::new()
    }
}

/// The RFC 4648 base64 alphabet.
const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Base64 (RFC 4648, with padding) written into the observation file as
/// the input arrives, so a large input needs no buffer of its size.
#[derive(Debug)]
pub struct Base64Writer {
    pending: [u8; 3],
    held: usize,
    out: [u8; 256],
    len: usize,
}

impl Base64Writer {
    pub const fn new() -> Base64Writer {
        Base64Writer { pending: [0; 3], held: 0, out: [0; 256], len: 0 }
    }

    fn emit(&mut self, group: [u8; 3], n: usize) {
        if self.len + 4 > self.out.len() {
            write_raw(&self.out[..self.len]);
            self.len = 0;
        }
        let v = u32::from(group[0]) << 16 | u32::from(group[1]) << 8 | u32::from(group[2]);
        for i in 0..4 {
            self.out[self.len + i] = if i <= n { BASE64[(v >> (18 - 6 * i) & 63) as usize] } else { b'=' };
        }
        self.len += 4;
    }

    pub fn update(&mut self, data: &[u8]) {
        for &b in data {
            self.pending[self.held] = b;
            self.held += 1;
            if self.held == 3 {
                self.emit(self.pending, 3);
                self.held = 0;
            }
        }
    }

    /// The last group, padded, and everything still buffered.
    pub fn finish(mut self) {
        if self.held > 0 {
            let mut g = [0u8; 3];
            g[..self.held].copy_from_slice(&self.pending[..self.held]);
            self.emit(g, self.held);
        }
        write_raw(&self.out[..self.len]);
    }
}

impl Default for Base64Writer {
    fn default() -> Self {
        Base64Writer::new()
    }
}

/// End the process with `code` (`NtTerminateProcess` on itself).
pub fn exit(code: u32) -> ! {
    let handle = OUTPUT.swap(core::ptr::null_mut(), Ordering::Relaxed);
    if !handle.is_null() {
        // SAFETY: the observation file's handle, closed once (swapped out).
        unsafe { NtClose(handle) };
    }
    loop {
        // SAFETY: terminating the current process does not return.
        unsafe { NtTerminateProcess(NT_CURRENT_PROCESS, code as NTSTATUS) };
    }
}

/// Create the observation file (replacing an older one). Called by
/// [`main!`] before the test runs.
pub fn open_output() -> Result<(), NTSTATUS> {
    let Some(path) = path_beside_image(utf16!("poly-test.out")) else {
        return Err(STATUS_OBJECT_NAME_INVALID);
    };
    let name = path.unicode_string();
    let attributes = ObjectAttributes::named(&name);
    let mut handle: HANDLE = core::ptr::null_mut();
    let mut iosb = IoStatusBlock::new();
    // SAFETY: every pointer is to a live local; no EA buffer.
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            GENERIC_WRITE | SYNCHRONIZE,
            &attributes,
            &mut iosb,
            core::ptr::null(),
            FILE_ATTRIBUTE_NORMAL,
            FILE_SHARE_READ,
            FILE_OVERWRITE_IF,
            FILE_SYNCHRONOUS_IO_NONALERT | FILE_NON_DIRECTORY_FILE,
            core::ptr::null(),
            0,
        )
    };
    if status != STATUS_SUCCESS {
        return Err(status);
    }
    OUTPUT.store(handle, Ordering::Relaxed);
    Ok(())
}

/// A small line buffer: one `NtWriteFile` per line.
struct Line {
    buf: [u8; 256],
    len: usize,
}

impl Line {
    const fn new() -> Line {
        Line { buf: [0; 256], len: 0 }
    }

    fn str(&mut self, s: &str) -> &mut Self {
        for &b in s.as_bytes() {
            if self.len == self.buf.len() {
                self.flush();
            }
            self.buf[self.len] = b;
            self.len += 1;
        }
        self
    }

    fn dec(&mut self, v: i64) -> &mut Self {
        let mut digits = [0u8; 20];
        let mut i = digits.len();
        let mut u = v.unsigned_abs();
        loop {
            i -= 1;
            digits[i] = b'0' + (u % 10) as u8;
            u /= 10;
            if u == 0 {
                break;
            }
        }
        if v < 0 {
            self.str("-");
        }
        self.str(core::str::from_utf8(&digits[i..]).unwrap_or("?"))
    }

    /// `0x` and at least `width` lowercase hex digits.
    fn hex(&mut self, v: u64, width: usize) -> &mut Self {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let significant = (16 - v.leading_zeros() as usize / 4).max(1);
        let n = significant.max(width).min(16);
        let mut digits = [0u8; 16];
        for (i, d) in digits[..n].iter_mut().enumerate() {
            *d = HEX[((v >> (4 * (n - 1 - i))) & 0xf) as usize];
        }
        self.str("0x").str(core::str::from_utf8(&digits[..n]).unwrap_or("?"))
    }

    fn flush(&mut self) {
        write_raw(&self.buf[..self.len]);
        self.len = 0;
    }

    fn end(&mut self) {
        self.str("\n");
        self.flush();
    }
}

/// The checks of one run: what [`main!`] hands the test function.
#[derive(Debug, Default)]
pub struct Test {
    pub passed: u32,
    pub failed: u32,
}

impl Test {
    pub const fn new() -> Test {
        Test { passed: 0, failed: 0 }
    }

    fn pass(&mut self, name: &str) {
        self.passed += 1;
        Line::new().str("ok ").str(name).end();
    }

    /// `ok <name>` if `cond`, else `not ok <name> (got <got>)`.
    pub fn check(&mut self, name: &str, cond: bool, got: i64) -> bool {
        if cond {
            self.pass(name);
        } else {
            self.failed += 1;
            Line::new().str("not ok ").str(name).str(" (got ").dec(got).str(")").end();
        }
        cond
    }

    /// `got == want`.
    pub fn expect(&mut self, name: &str, got: i64, want: i64) -> bool {
        self.check(name, got == want, got)
    }

    /// An `NTSTATUS` equals `want`; a failure shows both in hex.
    pub fn expect_status(&mut self, name: &str, got: NTSTATUS, want: NTSTATUS) -> bool {
        if got == want {
            self.pass(name);
            true
        } else {
            self.failed += 1;
            Line::new()
                .str("not ok ")
                .str(name)
                .str(" (got ")
                .hex(u64::from(got as u32), 8)
                .str(", want ")
                .hex(u64::from(want as u32), 8)
                .str(")")
                .end();
            false
        }
    }

    /// An observation that is not a check: `value <name> <v>` (only for
    /// values that are the same on every run and machine).
    pub fn value(&mut self, name: &str, v: i64) {
        Line::new().str("value ").str(name).str(" ").dec(v).end();
    }

    /// `value <name> 0x<v>` (flags, statuses).
    pub fn value_hex(&mut self, name: &str, v: u64) {
        Line::new().str("value ").str(name).str(" ").hex(v, 1).end();
    }

    /// A free-form line (`# comment` lines are a good use).
    pub fn line(&mut self, s: &str) {
        Line::new().str(s).end();
    }

    /// `summary <passed> passed, <failed> failed`; the exit status.
    pub fn finish(&self) -> u32 {
        Line::new()
            .str("summary ")
            .dec(i64::from(self.passed))
            .str(" passed, ")
            .dec(i64::from(self.failed))
            .str(" failed")
            .end();
        u32::from(self.failed != 0)
    }
}

/// The program's entry point (`mainCRTStartup`, the linker's `/ENTRY`):
/// open the observation file, run `$test(&mut Test)`, write the summary
/// and end the process with its status.
#[macro_export]
macro_rules! main {
    ($test:path) => {
        #[unsafe(no_mangle)]
        pub extern "system" fn mainCRTStartup() -> ! {
            if $crate::open_output().is_err() {
                $crate::exit($crate::EXIT_NO_OUTPUT);
            }
            let mut t = $crate::Test::new();
            $test(&mut t);
            let code = t.finish();
            $crate::exit(code)
        }
    };
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    Line::new().str("panic").end();
    exit(EXIT_PANIC)
}

/// `_fltused`: the MSVC toolchain's marker that floating point is used;
/// the linker looks for it when an object uses floats and no C runtime is
/// linked.
#[unsafe(no_mangle)]
#[used]
pub static _fltused: i32 = 0;

// `__chkstk`: the x64 stack probe the compiler calls before a frame larger
// than a page, so the guard page below the stack is touched in order
// (Learn: "/Gs (Control stack checking calls)", "x64 prolog and epilog":
// RAX = the frame size, every register but R10/R11 preserved, RSP
// unchanged; the caller subtracts RAX itself). Written from that contract.
core::arch::global_asm!(
    r#"
    .text
    .globl __chkstk
__chkstk:
    push rcx
    push rax
    mov  rcx, rax
    lea  rax, [rsp + 24]
2:
    cmp  rcx, 0x1000
    jb   3f
    sub  rax, 0x1000
    test byte ptr [rax], 0
    sub  rcx, 0x1000
    jmp  2b
3:
    pop  rax
    pop  rcx
    ret
"#
);

//! The shared part of the small network and identity DLLs (ADR 0155):
//! `iphlpapi`, `netapi32`, `secur32`, `dhcpcsvc`, `dnsapi`. Each of them
//! links this crate, which supplies the process heap as the global
//! allocator, the last-error helpers and two requests to `\Device\Afd`
//! (`poly_winsock::afd`): the interface list (`IF_LIST`) and the host name
//! (`NET_INFO`).

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use poly_win32_rt as _;
use poly_winsock::afd as proto;

pub type Handle = isize;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn GetLastError() -> u32;
    pub fn SetLastError(e: u32);
    fn GetProcessHeap() -> Handle;
    fn HeapAlloc(heap: Handle, flags: u32, bytes: usize) -> *mut u8;
    fn HeapFree(heap: Handle, flags: u32, mem: *mut u8) -> i32;
    pub fn CreateEventW(sa: *const u8, manual: i32, initial: i32, name: *const u16) -> Handle;
    pub fn SetEvent(h: Handle) -> i32;
    pub fn CloseHandle(h: Handle) -> i32;
    pub fn WaitForSingleObject(h: Handle, ms: u32) -> u32;
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtCreateFile(
        handle: *mut Handle,
        access: u32,
        attributes: *const ObjectAttributes,
        iosb: *mut IoStatusBlock,
        allocation: *const i64,
        file_attributes: u32,
        share: u32,
        disposition: u32,
        options: u32,
        ea: *const u8,
        ea_len: u32,
    ) -> u32;
    fn NtDeviceIoControlFile(
        handle: Handle,
        event: Handle,
        apc: *const u8,
        apc_context: *const u8,
        iosb: *mut IoStatusBlock,
        code: u32,
        input: *const u8,
        input_len: u32,
        output: *mut u8,
        output_len: u32,
    ) -> u32;
    fn NtClose(handle: Handle) -> u32;
}

/// `HEAP_ZERO_MEMORY` (`winnt.h`).
const HEAP_ZERO_MEMORY: u32 = 0x8;

struct ProcessHeap;

// SAFETY: HeapAlloc returns blocks aligned to 16 bytes on x64 (Learn:
// "HeapAlloc"); stricter alignments are refused.
unsafe impl core::alloc::GlobalAlloc for ProcessHeap {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        if layout.align() > 16 {
            return core::ptr::null_mut();
        }
        heap_alloc(layout.size().max(1))
    }
    unsafe fn dealloc(&self, ptr: *mut u8, _: core::alloc::Layout) {
        // SAFETY: a block of `alloc`.
        unsafe { heap_free(ptr) };
    }
}

#[global_allocator]
static ALLOC: ProcessHeap = ProcessHeap;

/// Zeroed bytes of the process heap (null on failure). Memory the API
/// hands to a caller (`FreeMibTable`, `NetApiBufferFree`) comes from here.
pub fn heap_alloc(n: usize) -> *mut u8 {
    // SAFETY: kernel32 exports without preconditions.
    unsafe { HeapAlloc(GetProcessHeap(), HEAP_ZERO_MEMORY, n) }
}

/// Free a block of [`heap_alloc`] (null is fine).
pub unsafe fn heap_free(p: *mut u8) {
    if !p.is_null() {
        // SAFETY: the caller passes a block of the process heap.
        unsafe { HeapFree(GetProcessHeap(), 0, p) };
    }
}

/// A new block holding `bytes` (null on failure).
pub fn heap_copy(bytes: &[u8]) -> *mut u8 {
    let p = heap_alloc(bytes.len().max(1));
    if !p.is_null() {
        // SAFETY: `p` holds `bytes.len()` bytes.
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len()) };
    }
    p
}

/// Set the thread's last error.
pub fn set_error(e: u32) {
    // SAFETY: no preconditions.
    unsafe { SetLastError(e) }
}

/// A NUL-terminated UTF-16 string as a `String` (`None` for null).
pub unsafe fn wstr(p: *const u16) -> Option<String> {
    if p.is_null() {
        return None;
    }
    let mut n = 0;
    // SAFETY: a NUL-terminated string.
    let units = unsafe {
        while *p.add(n) != 0 {
            n += 1;
        }
        core::slice::from_raw_parts(p, n)
    };
    Some(char::decode_utf16(units.iter().copied()).map(|c| c.unwrap_or('\u{fffd}')).collect())
}

/// `s` as UTF-16 with a NUL.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// `IO_STATUS_BLOCK` (`wdm.h`).
#[repr(C)]
#[derive(Default)]
struct IoStatusBlock {
    status: usize,
    information: usize,
}

#[repr(C)]
struct UnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *const u16,
}

#[repr(C)]
struct ObjectAttributes {
    length: u32,
    root: Handle,
    name: *const UnicodeString,
    attributes: u32,
    security: *const u8,
    qos: *const u8,
}

const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const SYNCHRONIZE: u32 = 0x0010_0000;
const FILE_SHARE_READ_WRITE: u32 = 3;
const FILE_OPEN_IF: u32 = 3;
const OBJ_CASE_INSENSITIVE: u32 = 0x40;
const STATUS_PENDING: u32 = 0x103;

/// A request on a fresh endpoint of `\Device\Afd`: the output on success.
fn afd_request(code: u32, out: &mut [u8]) -> Option<usize> {
    let name: Vec<u16> = proto::ENDPOINT.encode_utf16().collect();
    let us = UnicodeString {
        length: (name.len() * 2) as u16,
        maximum_length: (name.len() * 2) as u16,
        buffer: name.as_ptr(),
    };
    let oa = ObjectAttributes {
        length: core::mem::size_of::<ObjectAttributes>() as u32,
        root: 0,
        name: &us,
        attributes: OBJ_CASE_INSENSITIVE,
        security: core::ptr::null(),
        qos: core::ptr::null(),
    };
    let mut h: Handle = 0;
    let mut iosb = IoStatusBlock::default();
    // SAFETY: every pointer is to a live local.
    let s = unsafe {
        NtCreateFile(
            &mut h,
            GENERIC_READ | GENERIC_WRITE | SYNCHRONIZE,
            &oa,
            &mut iosb,
            core::ptr::null(),
            0,
            FILE_SHARE_READ_WRITE,
            FILE_OPEN_IF,
            0,
            core::ptr::null(),
            0,
        )
    };
    if s != 0 {
        return None;
    }
    let mut iosb = IoStatusBlock::default();
    // SAFETY: `out` is a live slice; the handle is ours.
    let s = unsafe {
        NtDeviceIoControlFile(
            h,
            0,
            core::ptr::null(),
            core::ptr::null(),
            &mut iosb,
            code,
            core::ptr::null(),
            0,
            out.as_mut_ptr(),
            out.len() as u32,
        )
    };
    // SAFETY: closing our handle.
    unsafe { NtClose(h) };
    if s >= 0xC000_0000 || s == STATUS_PENDING {
        return None;
    }
    Some(iosb.information)
}

/// Poly's network interfaces (`IF_LIST`); the loopback alone if the device does not answer.
pub fn interfaces() -> Vec<poly_iphlp::IfInfo> {
    let mut out = alloc::vec![0u8; poly_iphlp::wire::HEADER_SIZE + poly_iphlp::wire::MAX_INTERFACES * poly_iphlp::wire::RECORD_SIZE];
    match afd_request(proto::IF_LIST, &mut out).and_then(|n| poly_iphlp::wire::decode(&out[..n.min(out.len())])) {
        Some(list) => list,
        None => alloc::vec![poly_iphlp::IfInfo::loopback()],
    }
}

/// The host name (`NET_INFO`), `poly` if the device does not answer.
pub fn host_name() -> String {
    let mut out = [0u8; proto::NetInfo::SIZE];
    afd_request(proto::NET_INFO, &mut out)
        .and_then(|_| proto::NetInfo::decode(&out))
        .map_or_else(|| String::from("poly"), |n| String::from(n.host_name()))
}

/// The DNS servers (`NET_INFO`).
pub fn dns_servers() -> Vec<[u8; 4]> {
    let mut out = [0u8; proto::NetInfo::SIZE];
    afd_request(proto::NET_INFO, &mut out)
        .and_then(|_| proto::NetInfo::decode(&out))
        .map(|n| n.servers().to_vec())
        .unwrap_or_default()
}

//! The process side of the loader ([`poly_ldr::Process`]) and the heap
//! ([`poly_rtl::heap::Platform`]), over `ntdll`'s system calls and its own
//! process heap. In-process, so [`Process::memory`] is a slice of our own
//! address space.

use crate::heap::process_heap;
use crate::syscalls::*;
use crate::{NtStatus, lock::TID};
use alloc::vec::Vec;
use poly_ldr::Process;
use poly_rtl::critsec::Waiter;
use poly_rtl::heap::Platform;

const MEM_COMMIT: u32 = 0x1000;
const MEM_RESERVE: u32 = 0x2000;
const MEM_RELEASE: u32 = 0x8000;
const PAGE_READWRITE: u32 = 0x04;
/// `FILE_READ_DATA | SYNCHRONIZE`.
const FILE_READ: u32 = 0x0001 | 0x0010_0000;
/// `FILE_SHARE_READ`.
const SHARE_READ: u32 = 0x01;
/// `FILE_OPEN`.
const OPEN: u32 = 1;
/// `FILE_NON_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT`.
const OPTIONS: u32 = 0x40 | 0x20;
/// `FileStandardInformation` (bytes 0..8: AllocationSize; 8..16: EndOfFile).
const FILE_STANDARD_INFORMATION: u32 = 5;

/// The loader's and heap's view of the running process.
pub struct Sys;

impl Process for Sys {
    fn read_file(&mut self, nt_path: &[u16]) -> Result<Vec<u8>, NtStatus> {
        // OBJECT_ATTRIBUTES with a UNICODE_STRING name (winternl.h).
        let name = UnicodeName::new(nt_path);
        let oa = ObjectAttributes::new(&name);
        let mut handle: isize = 0;
        let mut iosb = [0u8; 16];
        // SAFETY: valid out-pointers and a well-formed OBJECT_ATTRIBUTES.
        let st = unsafe {
            NtCreateFile(
                &mut handle,
                FILE_READ,
                oa.as_ptr(),
                iosb.as_mut_ptr(),
                core::ptr::null(),
                0,
                SHARE_READ,
                OPEN,
                OPTIONS,
                core::ptr::null(),
                0,
            )
        };
        if !st.is_success() {
            return Err(st);
        }
        let read = (|| {
            let mut info = [0u8; 24];
            // SAFETY: valid pointers; FileStandardInformation is 24 bytes.
            let st = unsafe {
                NtQueryInformationFile(handle, iosb.as_mut_ptr(), info.as_mut_ptr(), 24, FILE_STANDARD_INFORMATION)
            };
            if !st.is_success() {
                return Err(st);
            }
            let size = u64::from_le_bytes(info[8..16].try_into().unwrap()) as usize;
            let mut buf = alloc::vec![0u8; size];
            let mut done = 0usize;
            while done < size {
                let offset = done as i64;
                // SAFETY: `buf` holds `size` bytes; we read into the tail.
                let st = unsafe {
                    NtReadFile(
                        handle,
                        0,
                        core::ptr::null(),
                        core::ptr::null(),
                        iosb.as_mut_ptr(),
                        buf[done..].as_mut_ptr(),
                        (size - done) as u32,
                        &offset,
                        core::ptr::null(),
                    )
                };
                if !st.is_success() {
                    return Err(st);
                }
                let got = u64::from_le_bytes(iosb[8..16].try_into().unwrap()) as usize;
                if got == 0 {
                    break;
                }
                done += got;
            }
            buf.truncate(done);
            Ok(buf)
        })();
        // SAFETY: a handle we opened.
        unsafe { NtClose(handle) };
        read
    }

    fn allocate(&mut self, base: Option<u64>, size: u64) -> Result<u64, NtStatus> {
        let mut addr = base.unwrap_or(0) as *mut u8;
        let mut sz = size as usize;
        // SAFETY: valid in/out pointers; the kernel places the range.
        let st = unsafe {
            NtAllocateVirtualMemory(CURRENT_PROCESS, &mut addr, 0, &mut sz, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE)
        };
        if st.is_success() { Ok(addr as u64) } else { Err(st) }
    }

    fn free(&mut self, base: u64) {
        let mut addr = base as *mut u8;
        let mut sz = 0usize;
        // SAFETY: an allocation we made; MEM_RELEASE takes the whole range.
        unsafe { NtFreeVirtualMemory(CURRENT_PROCESS, &mut addr, &mut sz, MEM_RELEASE) };
    }

    fn protect(&mut self, addr: u64, size: u64, protect: u32) -> Result<(), NtStatus> {
        let mut a = addr as *mut u8;
        let mut sz = size as usize;
        let mut old = 0u32;
        // SAFETY: valid pointers; the range is ours.
        let st = unsafe { NtProtectVirtualMemory(CURRENT_PROCESS, &mut a, &mut sz, protect, &mut old) };
        if st.is_success() { Ok(()) } else { Err(st) }
    }

    fn memory(&mut self, addr: u64, len: usize) -> Result<&mut [u8], NtStatus> {
        // The loader only asks for memory it has mapped in this process.
        // SAFETY: that memory is mapped and readable/writable; the borrow
        // does not outlive the call.
        Ok(unsafe { core::slice::from_raw_parts_mut(addr as *mut u8, len) })
    }

    fn alloc(&mut self, size: usize) -> Result<u64, NtStatus> {
        // SAFETY: the process heap is valid once `crate::init` ran.
        let p = unsafe { process_heap().allocate(&SysPlatform, poly_rtl::heap::flags::ZERO_MEMORY, size) };
        p.map(|p| p as u64).map_err(|_| NtStatus(0xC000_0017))
    }

    fn free_alloc(&mut self, addr: u64) {
        // SAFETY: the pointer came from `alloc`.
        unsafe { process_heap().free(&SysPlatform, 0, addr as *mut u8) };
    }
}

/// The heap's platform: 64 KiB-granular virtual memory and the lock's
/// primitives.
pub struct SysPlatform;

impl Platform for SysPlatform {
    fn allocate(&self, size: usize, _execute: bool) -> *mut u8 {
        let mut addr = core::ptr::null_mut();
        let mut sz = size;
        // SAFETY: valid pointers; the kernel places the range.
        let st = unsafe {
            NtAllocateVirtualMemory(CURRENT_PROCESS, &mut addr, 0, &mut sz, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE)
        };
        if st.is_success() { addr } else { core::ptr::null_mut() }
    }

    fn release(&self, base: *mut u8, _size: usize) {
        let mut addr = base;
        let mut sz = 0usize;
        // SAFETY: an allocation we made.
        unsafe { NtFreeVirtualMemory(CURRENT_PROCESS, &mut addr, &mut sz, MEM_RELEASE) };
    }
}

impl Waiter for SysPlatform {
    fn current_thread(&self) -> u64 {
        TID.with()
    }
    fn create_event(&self) -> u64 {
        let mut h: isize = 0;
        // SAFETY: valid out-pointer; a synchronization event (type 1).
        let st = unsafe { NtCreateEvent(&mut h, 0x1F0003, core::ptr::null(), 1, 0) };
        if st.is_success() { h as u64 } else { 0 }
    }
    fn wait(&self, event: u64) {
        // SAFETY: a handle we created.
        unsafe { NtWaitForSingleObject(event as isize, 0, core::ptr::null()) };
    }
    fn set(&self, event: u64) {
        // SAFETY: a handle we created.
        unsafe { NtSetEvent(event as isize, core::ptr::null_mut()) };
    }
    fn close(&self, event: u64) {
        // SAFETY: a handle we created.
        unsafe { NtClose(event as isize) };
    }
}

// ----- OBJECT_ATTRIBUTES / UNICODE_STRING (winternl.h) ----------------------

#[repr(C)]
struct UnicodeName {
    length: u16,
    maximum: u16,
    _pad: u32,
    buffer: *const u16,
}

impl UnicodeName {
    fn new(units: &[u16]) -> UnicodeName {
        let bytes = (units.len() * 2).min(0xFFFE) as u16;
        UnicodeName { length: bytes, maximum: bytes, _pad: 0, buffer: units.as_ptr() }
    }
}

#[repr(C)]
struct ObjectAttributes {
    length: u32,
    _pad0: u32,
    root: isize,
    name: *const UnicodeName,
    attributes: u32,
    _pad1: u32,
    security: *const u8,
    qos: *const u8,
}

impl ObjectAttributes {
    const OBJ_CASE_INSENSITIVE: u32 = 0x40;

    fn new(name: &UnicodeName) -> ObjectAttributes {
        ObjectAttributes {
            length: core::mem::size_of::<ObjectAttributes>() as u32,
            _pad0: 0,
            root: 0,
            name,
            attributes: Self::OBJ_CASE_INSENSITIVE,
            _pad1: 0,
            security: core::ptr::null(),
            qos: core::ptr::null(),
        }
    }

    fn as_ptr(&self) -> *const u8 {
        self as *const ObjectAttributes as *const u8
    }
}

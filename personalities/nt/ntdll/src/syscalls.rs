//! The system calls `ntdll` makes (and exports): declared here, defined by
//! the stubs `build.rs` generates (`$OUT_DIR/syscalls.s`, included below).
//! The convention is `ntdll`'s own (ADR 0063): the service number in EAX,
//! arguments as for a `call` with RCX moved to R10, an `NTSTATUS` in EAX.

use core::arch::global_asm;

global_asm!(include_str!(env!("NTDLL_SYSCALLS")));

use crate::NtStatus;

unsafe extern "system" {
    pub fn NtAllocateVirtualMemory(
        process: isize,
        base: *mut *mut u8,
        zero_bits: usize,
        size: *mut usize,
        ty: u32,
        protect: u32,
    ) -> NtStatus;
    pub fn NtFreeVirtualMemory(process: isize, base: *mut *mut u8, size: *mut usize, ty: u32) -> NtStatus;
    pub fn NtProtectVirtualMemory(
        process: isize,
        base: *mut *mut u8,
        size: *mut usize,
        new: u32,
        old: *mut u32,
    ) -> NtStatus;
    pub fn NtTerminateProcess(process: isize, status: NtStatus) -> NtStatus;
    pub fn NtTerminateThread(thread: isize, status: NtStatus) -> NtStatus;
    pub fn NtClose(handle: isize) -> NtStatus;
    pub fn NtCreateEvent(handle: *mut isize, access: u32, attributes: *const u8, kind: u32, initial: u8) -> NtStatus;
    pub fn NtSetEvent(handle: isize, previous: *mut i32) -> NtStatus;
    pub fn NtWaitForSingleObject(handle: isize, alertable: u8, timeout: *const i64) -> NtStatus;
    pub fn NtContinue(context: *mut u8, test_alert: u8) -> NtStatus;
    pub fn NtRaiseException(record: *const u8, context: *mut u8, first_chance: u8) -> NtStatus;
    pub fn NtCreateFile(
        handle: *mut isize,
        access: u32,
        attributes: *const u8,
        iosb: *mut u8,
        allocation: *const i64,
        file_attributes: u32,
        share: u32,
        disposition: u32,
        options: u32,
        ea: *const u8,
        ea_len: u32,
    ) -> NtStatus;
    pub fn NtReadFile(
        handle: isize,
        event: isize,
        apc: *const u8,
        apc_ctx: *const u8,
        iosb: *mut u8,
        buffer: *mut u8,
        len: u32,
        offset: *const i64,
        key: *const u32,
    ) -> NtStatus;
    pub fn NtQueryInformationFile(handle: isize, iosb: *mut u8, info: *mut u8, len: u32, class: u32) -> NtStatus;
}

/// The current process pseudo-handle (`NtCurrentProcess()`).
pub const CURRENT_PROCESS: isize = -1;
/// The current thread pseudo-handle.
pub const CURRENT_THREAD: isize = -2;

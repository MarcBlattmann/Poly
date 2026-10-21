//! What `kernelbase` imports from our `ntdll` (ADR 0071, 0079), and the
//! NT structures it passes: `UNICODE_STRING`, `OBJECT_ATTRIBUTES`
//! (`ntdef.h`), `IO_STATUS_BLOCK` (`wdm.h`).
//!
//! The imports are `raw-dylib`: rustc writes the import entries itself, so
//! no import library is needed; the loader binds them like any import.

use poly_ntobj::NtStatus;
use poly_rtl::critsec::CriticalSection;

pub type Handle = isize;

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn NtClose(handle: Handle) -> NtStatus;
    pub fn NtCreateFile(
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
    ) -> NtStatus;
    pub fn NtOpenFile(
        handle: *mut Handle,
        access: u32,
        attributes: *const ObjectAttributes,
        iosb: *mut IoStatusBlock,
        share: u32,
        options: u32,
    ) -> NtStatus;
    pub fn NtReadFile(
        handle: Handle,
        event: Handle,
        apc: *const u8,
        apc_context: *const u8,
        iosb: *mut IoStatusBlock,
        buffer: *mut u8,
        length: u32,
        offset: *const i64,
        key: *const u32,
    ) -> NtStatus;
    pub fn NtWriteFile(
        handle: Handle,
        event: Handle,
        apc: *const u8,
        apc_context: *const u8,
        iosb: *mut IoStatusBlock,
        buffer: *const u8,
        length: u32,
        offset: *const i64,
        key: *const u32,
    ) -> NtStatus;
    pub fn NtQueryInformationFile(
        handle: Handle,
        iosb: *mut IoStatusBlock,
        info: *mut u8,
        length: u32,
        class: u32,
    ) -> NtStatus;
    pub fn NtSetInformationFile(
        handle: Handle,
        iosb: *mut IoStatusBlock,
        info: *const u8,
        length: u32,
        class: u32,
    ) -> NtStatus;
    pub fn NtQueryDirectoryFile(
        handle: Handle,
        event: Handle,
        apc: *const u8,
        apc_context: *const u8,
        iosb: *mut IoStatusBlock,
        info: *mut u8,
        length: u32,
        class: u32,
        single: u8,
        mask: *const UnicodeString,
        restart: u8,
    ) -> NtStatus;
    pub fn NtQueryAttributesFile(attributes: *const ObjectAttributes, info: *mut u8) -> NtStatus;
    pub fn NtQueryVolumeInformationFile(
        handle: Handle,
        iosb: *mut IoStatusBlock,
        info: *mut u8,
        length: u32,
        class: u32,
    ) -> NtStatus;
    pub fn NtFlushBuffersFile(handle: Handle, iosb: *mut IoStatusBlock) -> NtStatus;
    pub fn NtDeviceIoControlFile(
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
    ) -> NtStatus;

    pub fn NtCreateEvent(
        handle: *mut Handle,
        access: u32,
        attributes: *const ObjectAttributes,
        kind: u32,
        initial: u8,
    ) -> NtStatus;
    pub fn NtSetEvent(handle: Handle, previous: *mut i32) -> NtStatus;
    pub fn NtResetEvent(handle: Handle, previous: *mut i32) -> NtStatus;
    pub fn NtCreateMutant(handle: *mut Handle, access: u32, attributes: *const ObjectAttributes, owner: u8)
    -> NtStatus;
    pub fn NtReleaseMutant(handle: Handle, previous: *mut i32) -> NtStatus;
    pub fn NtCreateSemaphore(
        handle: *mut Handle,
        access: u32,
        attributes: *const ObjectAttributes,
        initial: i32,
        maximum: i32,
    ) -> NtStatus;
    pub fn NtReleaseSemaphore(handle: Handle, count: i32, previous: *mut i32) -> NtStatus;
    pub fn NtWaitForSingleObject(handle: Handle, alertable: u8, timeout: *const i64) -> NtStatus;
    pub fn NtWaitForMultipleObjects(
        count: u32,
        handles: *const Handle,
        wait_type: u32,
        alertable: u8,
        timeout: *const i64,
    ) -> NtStatus;
    pub fn NtDelayExecution(alertable: u8, interval: *const i64) -> NtStatus;
    pub fn NtYieldExecution() -> NtStatus;

    pub fn NtAllocateVirtualMemory(
        process: Handle,
        base: *mut *mut u8,
        zero_bits: usize,
        size: *mut usize,
        kind: u32,
        protect: u32,
    ) -> NtStatus;
    pub fn NtFreeVirtualMemory(process: Handle, base: *mut *mut u8, size: *mut usize, kind: u32) -> NtStatus;
    pub fn NtProtectVirtualMemory(
        process: Handle,
        base: *mut *mut u8,
        size: *mut usize,
        protect: u32,
        old: *mut u32,
    ) -> NtStatus;
    pub fn NtQueryVirtualMemory(
        process: Handle,
        base: *const u8,
        class: u32,
        info: *mut u8,
        length: usize,
        returned: *mut usize,
    ) -> NtStatus;

    pub fn NtCreateSection(
        handle: *mut Handle,
        access: u32,
        attributes: *const ObjectAttributes,
        maximum_size: *const i64,
        protect: u32,
        allocation: u32,
        file: Handle,
    ) -> NtStatus;
    pub fn NtOpenSection(handle: *mut Handle, access: u32, attributes: *const ObjectAttributes) -> NtStatus;
    pub fn NtMapViewOfSection(
        section: Handle,
        process: Handle,
        base: *mut *mut u8,
        zero_bits: usize,
        commit: usize,
        offset: *mut i64,
        size: *mut usize,
        inherit: u32,
        allocation: u32,
        protect: u32,
    ) -> NtStatus;
    pub fn NtUnmapViewOfSection(process: Handle, base: *const u8) -> NtStatus;
    pub fn NtFlushVirtualMemory(
        process: Handle,
        base: *mut *mut u8,
        size: *mut usize,
        iosb: *mut IoStatusBlock,
    ) -> NtStatus;

    pub fn NtCreateUserProcess(
        process: *mut Handle,
        thread: *mut Handle,
        process_access: u32,
        thread_access: u32,
        process_attributes: *const ObjectAttributes,
        thread_attributes: *const ObjectAttributes,
        process_flags: u32,
        thread_flags: u32,
        parameters: *const u8,
        create_info: *mut u8,
        attribute_list: *const u8,
    ) -> NtStatus;
    pub fn NtQueryInformationProcess(
        handle: Handle,
        class: u32,
        info: *mut u8,
        length: u32,
        returned: *mut u32,
    ) -> NtStatus;

    pub fn NtCreateThreadEx(
        handle: *mut Handle,
        access: u32,
        attributes: *const ObjectAttributes,
        process: Handle,
        start: *const u8,
        argument: *mut u8,
        flags: u32,
        zero_bits: usize,
        stack_commit: usize,
        stack_reserve: usize,
        attribute_list: *const u8,
    ) -> NtStatus;
    pub fn NtQueryInformationThread(
        handle: Handle,
        class: u32,
        info: *mut u8,
        length: u32,
        returned: *mut u32,
    ) -> NtStatus;
    pub fn NtTerminateProcess(process: Handle, status: NtStatus) -> NtStatus;

    pub fn NtQuerySystemTime(time: *mut i64) -> NtStatus;
    pub fn NtQueryPerformanceCounter(counter: *mut i64, frequency: *mut i64) -> NtStatus;
    pub fn NtQuerySystemInformation(class: u32, info: *mut u8, length: u32, returned: *mut u32) -> NtStatus;

    pub fn RtlNtStatusToDosError(status: NtStatus) -> u32;
    pub fn RtlGetLastWin32Error() -> u32;
    pub fn RtlSetLastWin32Error(error: u32);
    pub fn RtlExitUserProcess(status: u32) -> !;
    pub fn RtlExitUserThread(status: u32) -> !;

    pub fn RtlCreateHeap(
        flags: u32,
        base: *mut u8,
        reserve: usize,
        commit: usize,
        lock: *mut u8,
        params: *mut u8,
    ) -> *mut u8;
    pub fn RtlDestroyHeap(heap: *mut u8) -> *mut u8;
    pub fn RtlAllocateHeap(heap: *mut u8, flags: u32, size: usize) -> *mut u8;
    pub fn RtlFreeHeap(heap: *mut u8, flags: u32, ptr: *mut u8) -> u8;
    pub fn RtlReAllocateHeap(heap: *mut u8, flags: u32, ptr: *mut u8, size: usize) -> *mut u8;
    pub fn RtlSizeHeap(heap: *mut u8, flags: u32, ptr: *const u8) -> usize;
    pub fn RtlGetProcessHeaps(count: u32, heaps: *mut *mut u8) -> u32;

    pub fn RtlInitializeCriticalSection(cs: *mut CriticalSection) -> NtStatus;
    pub fn RtlInitializeCriticalSectionAndSpinCount(cs: *mut CriticalSection, spin: u32) -> NtStatus;
    pub fn RtlInitializeCriticalSectionEx(cs: *mut CriticalSection, spin: u32, flags: u32) -> NtStatus;
    pub fn RtlEnterCriticalSection(cs: *mut CriticalSection) -> NtStatus;
    pub fn RtlLeaveCriticalSection(cs: *mut CriticalSection) -> NtStatus;
    pub fn RtlTryEnterCriticalSection(cs: *mut CriticalSection) -> u8;
    pub fn RtlDeleteCriticalSection(cs: *mut CriticalSection) -> NtStatus;

    pub fn LdrLoadDll(path: *const u16, flags: *mut u32, name: *const UnicodeString, handle: *mut *mut u8) -> NtStatus;
    pub fn LdrUnloadDll(handle: *mut u8) -> NtStatus;
    pub fn LdrGetDllHandle(
        path: *const u16,
        flags: *mut u32,
        name: *const UnicodeString,
        handle: *mut *mut u8,
    ) -> NtStatus;
    pub fn LdrGetProcedureAddress(
        handle: *mut u8,
        name: *const AnsiString,
        ordinal: u32,
        address: *mut *mut u8,
    ) -> NtStatus;
}

/// `NtCurrentProcess()`, `NtCurrentThread()`.
pub const CURRENT_PROCESS: Handle = -1;
pub const CURRENT_THREAD: Handle = -2;

/// `UNICODE_STRING` (`ntdef.h`): lengths in bytes.
#[repr(C)]
pub struct UnicodeString {
    pub length: u16,
    pub maximum: u16,
    pub buffer: *const u16,
}

impl UnicodeString {
    /// A string over `units` (which must outlive it).
    pub fn new(units: &[u16]) -> UnicodeString {
        let bytes = (units.len() * 2).min(0xFFFE) as u16;
        UnicodeString { length: bytes, maximum: bytes, buffer: units.as_ptr() }
    }
}

/// `ANSI_STRING` (`ntdef.h`).
#[repr(C)]
pub struct AnsiString {
    pub length: u16,
    pub maximum: u16,
    pub buffer: *const u8,
}

/// `OBJECT_ATTRIBUTES` (`ntdef.h`, x64: 48 bytes).
#[repr(C)]
pub struct ObjectAttributes {
    pub length: u32,
    pub root: Handle,
    pub name: *const UnicodeString,
    pub attributes: u32,
    pub security_descriptor: *const u8,
    pub security_qos: *const u8,
}

/// `OBJ_CASE_INSENSITIVE`, `OBJ_OPENIF` (`ntdef.h`).
pub const OBJ_CASE_INSENSITIVE: u32 = 0x40;
pub const OBJ_OPENIF: u32 = 0x80;
/// `OBJ_INHERIT` (`ntdef.h`).
pub const OBJ_INHERIT: u32 = 0x2;

impl ObjectAttributes {
    pub fn new(name: Option<&UnicodeString>, attributes: u32) -> ObjectAttributes {
        ObjectAttributes {
            length: core::mem::size_of::<ObjectAttributes>() as u32,
            root: 0,
            name: name.map_or(core::ptr::null(), |n| n as *const UnicodeString),
            attributes,
            security_descriptor: core::ptr::null(),
            security_qos: core::ptr::null(),
        }
    }
}

/// `IO_STATUS_BLOCK` (`wdm.h`).
#[repr(C)]
#[derive(Default)]
pub struct IoStatusBlock {
    pub status: u64,
    pub information: u64,
}

/// `STATUS_*` values `kernelbase` looks at (`ntstatus.h`).
pub const STATUS_PENDING: NtStatus = NtStatus(0x103);
pub const STATUS_OBJECT_NAME_EXISTS: NtStatus = NtStatus(0x4000_0000);
pub const STATUS_NO_MORE_FILES: NtStatus = NtStatus(0x8000_0006);
pub const STATUS_END_OF_FILE: NtStatus = NtStatus(0xC000_0011);
pub const STATUS_NO_SUCH_FILE: NtStatus = NtStatus(0xC000_000F);

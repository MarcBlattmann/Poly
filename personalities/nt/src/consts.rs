//! Constants of the memory, section, information and process services.
//!
//! Every value is from a public Windows SDK or WDK header, named as there:
//! `winnt.h` (`MEM_*`, `PAGE_*`, `SEC_*`, `PROCESS_*`), `wdm.h`
//! (`SECTION_INHERIT`, `EVENT_TYPE`, `MEMORY_INFORMATION_CLASS`),
//! `winternl.h` and `wdm.h` (`OBJECT_INFORMATION_CLASS`). Their meanings
//! are from Microsoft Learn ("Memory Protection Constants", "VirtualAlloc",
//! "ZwAllocateVirtualMemory", "ZwFreeVirtualMemory", "ZwCreateSection",
//! "ZwMapViewOfSection", "NtQueryObject").

// ----- allocation types and states (`winnt.h`)

pub const MEM_COMMIT: u32 = 0x0000_1000;
pub const MEM_RESERVE: u32 = 0x0000_2000;
pub const MEM_DECOMMIT: u32 = 0x0000_4000;
pub const MEM_RELEASE: u32 = 0x0000_8000;
pub const MEM_FREE: u32 = 0x0001_0000;
pub const MEM_PRIVATE: u32 = 0x0002_0000;
pub const MEM_MAPPED: u32 = 0x0004_0000;
pub const MEM_RESET: u32 = 0x0008_0000;
pub const MEM_TOP_DOWN: u32 = 0x0010_0000;
pub const MEM_WRITE_WATCH: u32 = 0x0020_0000;
pub const MEM_PHYSICAL: u32 = 0x0040_0000;
pub const MEM_RESET_UNDO: u32 = 0x0100_0000;
pub const MEM_IMAGE: u32 = 0x0100_0000;
pub const MEM_LARGE_PAGES: u32 = 0x2000_0000;

// ----- page protections (`winnt.h`, Learn "Memory Protection Constants")

pub const PAGE_NOACCESS: u32 = 0x01;
pub const PAGE_READONLY: u32 = 0x02;
pub const PAGE_READWRITE: u32 = 0x04;
pub const PAGE_WRITECOPY: u32 = 0x08;
pub const PAGE_EXECUTE: u32 = 0x10;
pub const PAGE_EXECUTE_READ: u32 = 0x20;
pub const PAGE_EXECUTE_READWRITE: u32 = 0x40;
pub const PAGE_EXECUTE_WRITECOPY: u32 = 0x80;
pub const PAGE_GUARD: u32 = 0x100;
pub const PAGE_NOCACHE: u32 = 0x200;
pub const PAGE_WRITECOMBINE: u32 = 0x400;

// ----- section attributes (`winnt.h`, Learn "CreateFileMappingW")

pub const SEC_FILE: u32 = 0x0080_0000;
pub const SEC_IMAGE: u32 = 0x0100_0000;
pub const SEC_RESERVE: u32 = 0x0400_0000;
pub const SEC_COMMIT: u32 = 0x0800_0000;
pub const SEC_NOCACHE: u32 = 0x1000_0000;
pub const SEC_WRITECOMBINE: u32 = 0x4000_0000;
pub const SEC_LARGE_PAGES: u32 = 0x8000_0000;
/// `SEC_IMAGE | SEC_NOCACHE` (Learn: "CreateFileMappingW").
pub const SEC_IMAGE_NO_EXECUTE: u32 = 0x1100_0000;

/// `SECTION_INHERIT` (`wdm.h`).
pub const VIEW_SHARE: u32 = 1;
pub const VIEW_UNMAP: u32 = 2;

// ----- process and thread rights (`winnt.h`)

pub const PROCESS_TERMINATE: u32 = 0x0001;
pub const PROCESS_CREATE_THREAD: u32 = 0x0002;
pub const PROCESS_VM_OPERATION: u32 = 0x0008;
pub const PROCESS_VM_READ: u32 = 0x0010;
pub const PROCESS_VM_WRITE: u32 = 0x0020;
pub const PROCESS_DUP_HANDLE: u32 = 0x0040;
pub const PROCESS_QUERY_INFORMATION: u32 = 0x0400;
pub const PROCESS_SUSPEND_RESUME: u32 = 0x0800;
pub const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

pub const THREAD_TERMINATE: u32 = 0x0001;
pub const THREAD_SUSPEND_RESUME: u32 = 0x0002;
pub const THREAD_GET_CONTEXT: u32 = 0x0008;
pub const THREAD_SET_CONTEXT: u32 = 0x0010;
pub const THREAD_QUERY_INFORMATION: u32 = 0x0040;
pub const THREAD_QUERY_LIMITED_INFORMATION: u32 = 0x0800;

// ----- NtCreateThreadEx `CreateFlags` (Learn / `ntpsapi.h`, documented via
// the `THREAD_CREATE_FLAGS_*` names).
pub const THREAD_CREATE_FLAGS_CREATE_SUSPENDED: u32 = 0x0001;

// ----- information classes

/// `PROCESSINFOCLASS` (`winternl.h`): `ProcessBasicInformation`, the one we
/// answer (`ProcessImageFileName` and friends are not answered yet).
pub const PROCESS_BASIC_INFORMATION: u32 = 0;
/// `ProcessTimes` (`ntddk.h`'s `PROCESSINFOCLASS`): `KERNEL_USER_TIMES`,
/// the creation, exit, kernel and user times as four `LARGE_INTEGER`s
/// (ADR 0087).
pub const PROCESS_TIMES: u32 = 4;
pub const KERNEL_USER_TIMES_SIZE: usize = 32;

/// `THREADINFOCLASS` (`winternl.h` / `ntpsapi.h`).
pub const THREAD_BASIC_INFORMATION: u32 = 0;

/// `sizeof(PROCESS_BASIC_INFORMATION)` on x64 (`winternl.h`): `NTSTATUS
/// ExitStatus` (0, 4 pad), `PPEB PebBaseAddress` (8), `ULONG_PTR
/// AffinityMask` (16), `KPRIORITY BasePriority` (24, 4 pad),
/// `ULONG_PTR UniqueProcessId` (32), `ULONG_PTR InheritedFromUniqueProcessId`
/// (40): 48 bytes.
pub const PROCESS_BASIC_INFORMATION_SIZE: usize = 48;
/// `sizeof(THREAD_BASIC_INFORMATION)` on x64 (`ntpsapi.h`): `NTSTATUS
/// ExitStatus` (0, 4 pad), `PTEB TebBaseAddress` (8), `CLIENT_ID ClientId`
/// (16: two `HANDLE`s), `KAFFINITY AffinityMask` (32), `KPRIORITY Priority`
/// (40), `LONG BasePriority` (44): 48 bytes.
pub const THREAD_BASIC_INFORMATION_SIZE: usize = 48;

// ----- CONTEXT (x64, `winnt.h`)
//
// `CONTEXT` is 1232 bytes on x64. We fill in the control and integer
// registers (and the two segment selectors a user thread has); the legacy
// floating-point / XMM save area and the debug registers are left for later
// milestones (floating-point context is `CONTEXT_FLOATING_POINT`, not asked
// for here). The field offsets are from `winnt.h`.

/// `sizeof(CONTEXT)` on x64.
pub const CONTEXT_SIZE: usize = 1232;
/// `CONTEXT_AMD64`, the architecture bits every flag carries.
pub const CONTEXT_AMD64: u32 = 0x0010_0000;
/// `CONTEXT_CONTROL`: SegSs, Rsp, SegCs, Rip, EFlags.
pub const CONTEXT_CONTROL: u32 = CONTEXT_AMD64 | 0x1;
/// `CONTEXT_INTEGER`: Rax..R15.
pub const CONTEXT_INTEGER: u32 = CONTEXT_AMD64 | 0x2;
/// `CONTEXT_SEGMENTS`: SegDs, SegEs, SegFs, SegGs.
pub const CONTEXT_SEGMENTS: u32 = CONTEXT_AMD64 | 0x4;
/// `CONTEXT_FLOATING_POINT`: the XMM state (not filled in yet).
pub const CONTEXT_FLOATING_POINT: u32 = CONTEXT_AMD64 | 0x8;
/// `CONTEXT_FULL`.
pub const CONTEXT_FULL: u32 = CONTEXT_CONTROL | CONTEXT_INTEGER | CONTEXT_FLOATING_POINT;

pub const CTX_FLAGS: usize = 0x30;
pub const CTX_SEG_CS: usize = 0x38;
pub const CTX_SEG_SS: usize = 0x42;
pub const CTX_EFLAGS: usize = 0x44;
pub const CTX_RAX: usize = 0x78;
pub const CTX_RCX: usize = 0x80;
pub const CTX_RDX: usize = 0x88;
pub const CTX_RBX: usize = 0x90;
pub const CTX_RSP: usize = 0x98;
pub const CTX_RBP: usize = 0xA0;
pub const CTX_RSI: usize = 0xA8;
pub const CTX_RDI: usize = 0xB0;
pub const CTX_R8: usize = 0xB8;
pub const CTX_R15: usize = 0xF0;
pub const CTX_RIP: usize = 0xF8;

// ----- information classes

/// `MEMORY_INFORMATION_CLASS` (`wdm.h`): `MemoryBasicInformation`.
pub const MEMORY_BASIC_INFORMATION_CLASS: u32 = 0;

/// `OBJECT_INFORMATION_CLASS` (`winternl.h`, `wdm.h`).
pub const OBJECT_BASIC_INFORMATION: u32 = 0;
pub const OBJECT_NAME_INFORMATION: u32 = 1;
pub const OBJECT_TYPE_INFORMATION: u32 = 2;

/// `EVENT_TYPE` (`wdm.h`).
pub const NOTIFICATION_EVENT: u32 = 0;
pub const SYNCHRONIZATION_EVENT: u32 = 1;

// ----- sizes of the structures the services read and write

/// `sizeof(OBJECT_ATTRIBUTES)` on x64 (`ntdef.h`): `ULONG Length` (0),
/// `HANDLE RootDirectory` (8), `PUNICODE_STRING ObjectName` (16), `ULONG
/// Attributes` (24), `PVOID SecurityDescriptor` (32), `PVOID
/// SecurityQualityOfService` (40).
pub const OBJECT_ATTRIBUTES_SIZE: usize = 48;
/// `sizeof(UNICODE_STRING)` on x64 (`ntdef.h`): `USHORT Length` (0),
/// `USHORT MaximumLength` (2), `PWSTR Buffer` (8).
pub const UNICODE_STRING_SIZE: usize = 16;
/// `sizeof(MEMORY_BASIC_INFORMATION)` on x64 (`winnt.h`): `BaseAddress`
/// (0), `AllocationBase` (8), `AllocationProtect` (16), `PartitionId`
/// (20), `RegionSize` (24), `State` (32), `Protect` (36), `Type` (40).
pub const MEMORY_BASIC_INFORMATION_SIZE: usize = 48;
/// `sizeof(PUBLIC_OBJECT_BASIC_INFORMATION)` (`winternl.h`): four `ULONG`s
/// (`Attributes`, `GrantedAccess`, `HandleCount`, `PointerCount`) and
/// `ULONG Reserved[10]`.
pub const OBJECT_BASIC_INFORMATION_SIZE: usize = 56;
/// `sizeof(PUBLIC_OBJECT_TYPE_INFORMATION)` on x64 (`winternl.h`):
/// `UNICODE_STRING TypeName` and `ULONG Reserved[22]`.
pub const OBJECT_TYPE_INFORMATION_SIZE: usize = 104;
/// `sizeof(OBJECT_DIRECTORY_INFORMATION)` on x64 (Learn:
/// "NtQueryDirectoryObject"): two `UNICODE_STRING`s, `Name` and
/// `TypeName`.
pub const OBJECT_DIRECTORY_INFORMATION_SIZE: usize = 32;

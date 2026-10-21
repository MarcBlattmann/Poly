//! Poly's NT system service numbers.
//!
//! Windows renumbers its system services with almost every build, and only
//! `ntdll` (and `win32u`) call them, so the numbers are not an ABI that
//! applications see. Poly's own `ntdll` is the only caller of ours, so we
//! pick them (ADR 0001, ADR 0063) and keep them stable within Poly: a number
//! is never reused for another service. The layout of a number is the one
//! the x64 `syscall` stubs of `ntdll` use, `table << 12 | index`:
//!
//! | Table (bits 12–13) | Indices | Services |
//! |---|---|---|
//! | 0, executive | `0x000`–`0x03F` | object manager: handles, directories, symbolic links, events |
//! | 0, executive | `0x040`–`0x07F` | memory: virtual memory, sections, views |
//! | 0, executive | `0x080`–`0x0BF` | processes and threads |
//! | 0, executive | `0x0C0`–`0x0FF` | file I/O (ADR 0067) |
//! | 1, win32k | | none yet (Phase 7) |
//! | 3, Poly | `0x000`–`0x0FF` | Poly's own services, for its tests and tools; never called by `ntdll` |
//!
//! Each service takes its arguments in the order of the documented `Zw*`
//! routine of the same name (Learn, Windows Driver Kit reference), or of
//! the documented Win32 function for the ones the WDK does not document,
//! as noted on [`Service`]. `cargo xtask` will generate our `ntdll`'s stubs
//! from [`SERVICES`].

/// One NT system service Poly implements.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Service {
    // ----- object manager
    /// `NtClose(Handle)`.
    Close,
    /// `NtDuplicateObject(SourceProcessHandle, SourceHandle,
    /// TargetProcessHandle, TargetHandle*, DesiredAccess, HandleAttributes,
    /// Options)`.
    DuplicateObject,
    /// `NtQueryObject(Handle, ObjectInformationClass, ObjectInformation,
    /// ObjectInformationLength, ReturnLength*)`.
    QueryObject,
    /// `NtMakeTemporaryObject(Handle)`.
    MakeTemporaryObject,
    /// `NtCreateDirectoryObject(DirectoryHandle*, DesiredAccess,
    /// ObjectAttributes*)`.
    CreateDirectoryObject,
    /// `NtOpenDirectoryObject(DirectoryHandle*, DesiredAccess,
    /// ObjectAttributes*)`.
    OpenDirectoryObject,
    /// `NtQueryDirectoryObject(DirectoryHandle, Buffer, Length,
    /// ReturnSingleEntry, RestartScan, Context*, ReturnLength*)` (Learn:
    /// "NtQueryDirectoryObject").
    QueryDirectoryObject,
    /// `NtCreateSymbolicLinkObject(LinkHandle*, DesiredAccess,
    /// ObjectAttributes*, LinkTarget*)`.
    CreateSymbolicLinkObject,
    /// `NtOpenSymbolicLinkObject(LinkHandle*, DesiredAccess,
    /// ObjectAttributes*)`.
    OpenSymbolicLinkObject,
    /// `NtQuerySymbolicLinkObject(LinkHandle, LinkTarget*,
    /// ReturnedLength*)`.
    QuerySymbolicLinkObject,
    /// `NtCreateEvent(EventHandle*, DesiredAccess, ObjectAttributes*,
    /// EventType, InitialState)`.
    CreateEvent,
    /// `NtOpenEvent(EventHandle*, DesiredAccess, ObjectAttributes*)`.
    OpenEvent,
    /// `NtSetEvent(EventHandle, PreviousState*)`.
    SetEvent,
    /// `NtResetEvent(EventHandle, PreviousState*)`.
    ResetEvent,
    /// `NtPulseEvent(EventHandle, PreviousState*)`.
    PulseEvent,
    /// `NtClearEvent(EventHandle)`.
    ClearEvent,
    /// `NtQueryEvent(EventHandle, EventInformationClass, EventInformation,
    /// EventInformationLength, ReturnLength*)`.
    QueryEvent,

    // ----- dispatcher objects: mutants, semaphores, timers, waits, APCs
    /// `NtCreateMutant(MutantHandle*, DesiredAccess, ObjectAttributes*,
    /// InitialOwner)`.
    CreateMutant,
    /// `NtOpenMutant(MutantHandle*, DesiredAccess, ObjectAttributes*)`.
    OpenMutant,
    /// `NtReleaseMutant(MutantHandle, PreviousCount*)`.
    ReleaseMutant,
    /// `NtQueryMutant(MutantHandle, MutantInformationClass,
    /// MutantInformation, Length, ReturnLength*)`.
    QueryMutant,
    /// `NtCreateSemaphore(SemaphoreHandle*, DesiredAccess,
    /// ObjectAttributes*, InitialCount, MaximumCount)`.
    CreateSemaphore,
    /// `NtOpenSemaphore(SemaphoreHandle*, DesiredAccess, ObjectAttributes*)`.
    OpenSemaphore,
    /// `NtReleaseSemaphore(SemaphoreHandle, ReleaseCount, PreviousCount*)`.
    ReleaseSemaphore,
    /// `NtQuerySemaphore(SemaphoreHandle, SemaphoreInformationClass,
    /// SemaphoreInformation, Length, ReturnLength*)`.
    QuerySemaphore,
    /// `NtCreateTimer(TimerHandle*, DesiredAccess, ObjectAttributes*,
    /// TimerType)`.
    CreateTimer,
    /// `NtOpenTimer(TimerHandle*, DesiredAccess, ObjectAttributes*)`.
    OpenTimer,
    /// `NtSetTimer(TimerHandle, DueTime*, TimerApcRoutine, TimerContext,
    /// ResumeTimer, Period, PreviousState*)`.
    SetTimer,
    /// `NtCancelTimer(TimerHandle, CurrentState*)`.
    CancelTimer,
    /// `NtQueryTimer(TimerHandle, TimerInformationClass, TimerInformation,
    /// Length, ReturnLength*)`.
    QueryTimer,
    /// `NtWaitForSingleObject(Handle, Alertable, Timeout*)`.
    WaitForSingleObject,
    /// `NtWaitForMultipleObjects(Count, Handles[], WaitType, Alertable,
    /// Timeout*)`.
    WaitForMultipleObjects,
    /// `NtSignalAndWaitForSingleObject(SignalHandle, WaitHandle, Alertable,
    /// Timeout*)`.
    SignalAndWaitForSingleObject,
    /// `NtQueueApcThread(ThreadHandle, ApcRoutine, ApcArgument1,
    /// ApcArgument2, ApcArgument3)`.
    QueueApcThread,
    /// `NtTestAlert()`.
    TestAlert,
    /// `NtDelayExecution(Alertable, DelayInterval*)`.
    DelayExecution,
    /// `NtYieldExecution()`.
    YieldExecution,

    // ----- memory
    /// `NtAllocateVirtualMemory(ProcessHandle, BaseAddress*, ZeroBits,
    /// RegionSize*, AllocationType, Protect)`.
    AllocateVirtualMemory,
    /// `NtFreeVirtualMemory(ProcessHandle, BaseAddress*, RegionSize*,
    /// FreeType)`.
    FreeVirtualMemory,
    /// `NtProtectVirtualMemory(ProcessHandle, BaseAddress*, RegionSize*,
    /// NewProtect, OldProtect*)`: the WDK does not document it; the
    /// parameters are those of `VirtualProtectEx` with the address and
    /// size passed by reference and updated, as in
    /// `NtAllocateVirtualMemory`.
    ProtectVirtualMemory,
    /// `NtQueryVirtualMemory(ProcessHandle, BaseAddress,
    /// MemoryInformationClass, MemoryInformation, MemoryInformationLength,
    /// ReturnLength*)`.
    QueryVirtualMemory,
    /// `NtCreateSection(SectionHandle*, DesiredAccess, ObjectAttributes*,
    /// MaximumSize*, SectionPageProtection, AllocationAttributes,
    /// FileHandle)`.
    CreateSection,
    /// `NtOpenSection(SectionHandle*, DesiredAccess, ObjectAttributes*)`.
    OpenSection,
    /// `NtMapViewOfSection(SectionHandle, ProcessHandle, BaseAddress*,
    /// ZeroBits, CommitSize, SectionOffset*, ViewSize*, InheritDisposition,
    /// AllocationType, Win32Protect)`.
    MapViewOfSection,
    /// `NtUnmapViewOfSection(ProcessHandle, BaseAddress)`.
    UnmapViewOfSection,

    // ----- processes and threads
    /// `NtTerminateProcess(ProcessHandle, ExitStatus)`.
    TerminateProcess,
    /// `NtTerminateThread(ThreadHandle, ExitStatus)`.
    TerminateThread,
    /// `NtCreateThreadEx(ThreadHandle*, DesiredAccess, ObjectAttributes*,
    /// ProcessHandle, StartRoutine, Argument, CreateFlags, ZeroBits,
    /// StackSize, MaximumStackSize, AttributeList)`.
    CreateThreadEx,
    /// `NtOpenProcess(ProcessHandle*, DesiredAccess, ObjectAttributes*,
    /// ClientId*)`.
    OpenProcess,
    /// `NtOpenThread(ThreadHandle*, DesiredAccess, ObjectAttributes*,
    /// ClientId*)`.
    OpenThread,
    /// `NtQueryInformationProcess(ProcessHandle, ProcessInformationClass,
    /// ProcessInformation, Length, ReturnLength*)`.
    QueryInformationProcess,
    /// `NtQueryInformationThread(ThreadHandle, ThreadInformationClass,
    /// ThreadInformation, Length, ReturnLength*)`.
    QueryInformationThread,
    /// `NtSuspendThread(ThreadHandle, PreviousSuspendCount*)`.
    SuspendThread,
    /// `NtResumeThread(ThreadHandle, PreviousSuspendCount*)`.
    ResumeThread,
    /// `NtGetContextThread(ThreadHandle, Context*)`.
    GetContextThread,
    /// `NtSetContextThread(ThreadHandle, Context*)`.
    SetContextThread,

    // ----- file I/O (ADR 0067; `crate::io::services`)
    /// `NtCreateFile(FileHandle*, DesiredAccess, ObjectAttributes*,
    /// IoStatusBlock*, AllocationSize*, FileAttributes, ShareAccess,
    /// CreateDisposition, CreateOptions, EaBuffer, EaLength)`.
    CreateFile,
    /// `NtOpenFile(FileHandle*, DesiredAccess, ObjectAttributes*,
    /// IoStatusBlock*, ShareAccess, OpenOptions)`.
    OpenFile,
    /// `NtReadFile(FileHandle, Event, ApcRoutine, ApcContext,
    /// IoStatusBlock*, Buffer, Length, ByteOffset*, Key*)`.
    ReadFile,
    /// `NtWriteFile(FileHandle, Event, ApcRoutine, ApcContext,
    /// IoStatusBlock*, Buffer, Length, ByteOffset*, Key*)`.
    WriteFile,
    /// `NtQueryInformationFile(FileHandle, IoStatusBlock*, FileInformation,
    /// Length, FileInformationClass)`.
    QueryInformationFile,
    /// `NtSetInformationFile(FileHandle, IoStatusBlock*, FileInformation,
    /// Length, FileInformationClass)`.
    SetInformationFile,
    /// `NtQueryDirectoryFile(FileHandle, Event, ApcRoutine, ApcContext,
    /// IoStatusBlock*, FileInformation, Length, FileInformationClass,
    /// ReturnSingleEntry, FileName*, RestartScan)`.
    QueryDirectoryFile,
    /// `NtQueryVolumeInformationFile(FileHandle, IoStatusBlock*,
    /// FsInformation, Length, FsInformationClass)`.
    QueryVolumeInformationFile,
    /// `NtFlushBuffersFile(FileHandle, IoStatusBlock*)`.
    FlushBuffersFile,
    /// `NtLockFile(FileHandle, Event, ApcRoutine, ApcContext,
    /// IoStatusBlock*, ByteOffset*, Length*, Key, FailImmediately,
    /// ExclusiveLock)`.
    LockFile,
    /// `NtUnlockFile(FileHandle, IoStatusBlock*, ByteOffset*, Length*, Key)`.
    UnlockFile,
    /// `NtDeleteFile(ObjectAttributes*)`.
    DeleteFile,
    /// `NtCancelIoFile(FileHandle, IoStatusBlock*)`.
    CancelIoFile,
    /// `NtCancelIoFileEx(FileHandle, IoRequestToCancel*, IoStatusBlock*)`
    /// (the WDK does not document it; the parameters are `CancelIoEx`'s
    /// with the caller's status block added, as in `NtCancelIoFile`).
    CancelIoFileEx,
    /// `NtQueryAttributesFile(ObjectAttributes*, FileInformation*)`
    /// (`FILE_BASIC_INFORMATION`).
    QueryAttributesFile,
    /// `NtQueryFullAttributesFile(ObjectAttributes*, FileInformation*)`
    /// (`FILE_NETWORK_OPEN_INFORMATION`).
    QueryFullAttributesFile,

    // ----- Poly
    /// `PolyTrace(a, b, c)`: logs three values on the kernel log (a test
    /// program reports what went wrong before it gives up). Returns
    /// `STATUS_SUCCESS`.
    PolyTrace,
    /// `PolyResumeApc()`: the tail of the APC trampoline. After a user APC
    /// routine returns, the kernel restores the context the thread had when
    /// the APC interrupted it (our stand-in for `NtContinue`, until `ntdll`
    /// provides `KiUserApcDispatcher`). Never returns to the trampoline.
    PolyResumeApc,
}

/// Table numbers (bits 12–13 of a service number).
pub const TABLE_EXECUTIVE: u32 = 0;
pub const TABLE_WIN32K: u32 = 1;
pub const TABLE_POLY: u32 = 3;

/// Every service with its number and its name as `ntdll` exports it.
pub const SERVICES: [(Service, u32, &str); 74] = [
    (Service::Close, 0x000, "NtClose"),
    (Service::DuplicateObject, 0x001, "NtDuplicateObject"),
    (Service::QueryObject, 0x002, "NtQueryObject"),
    (Service::MakeTemporaryObject, 0x003, "NtMakeTemporaryObject"),
    (Service::CreateDirectoryObject, 0x004, "NtCreateDirectoryObject"),
    (Service::OpenDirectoryObject, 0x005, "NtOpenDirectoryObject"),
    (Service::QueryDirectoryObject, 0x006, "NtQueryDirectoryObject"),
    (Service::CreateSymbolicLinkObject, 0x007, "NtCreateSymbolicLinkObject"),
    (Service::OpenSymbolicLinkObject, 0x008, "NtOpenSymbolicLinkObject"),
    (Service::QuerySymbolicLinkObject, 0x009, "NtQuerySymbolicLinkObject"),
    (Service::CreateEvent, 0x00A, "NtCreateEvent"),
    (Service::OpenEvent, 0x00B, "NtOpenEvent"),
    (Service::SetEvent, 0x00C, "NtSetEvent"),
    (Service::ResetEvent, 0x00D, "NtResetEvent"),
    (Service::PulseEvent, 0x00E, "NtPulseEvent"),
    (Service::ClearEvent, 0x00F, "NtClearEvent"),
    (Service::QueryEvent, 0x010, "NtQueryEvent"),
    (Service::CreateMutant, 0x020, "NtCreateMutant"),
    (Service::OpenMutant, 0x021, "NtOpenMutant"),
    (Service::ReleaseMutant, 0x022, "NtReleaseMutant"),
    (Service::QueryMutant, 0x023, "NtQueryMutant"),
    (Service::CreateSemaphore, 0x024, "NtCreateSemaphore"),
    (Service::OpenSemaphore, 0x025, "NtOpenSemaphore"),
    (Service::ReleaseSemaphore, 0x026, "NtReleaseSemaphore"),
    (Service::QuerySemaphore, 0x027, "NtQuerySemaphore"),
    (Service::CreateTimer, 0x028, "NtCreateTimer"),
    (Service::OpenTimer, 0x029, "NtOpenTimer"),
    (Service::SetTimer, 0x02A, "NtSetTimer"),
    (Service::CancelTimer, 0x02B, "NtCancelTimer"),
    (Service::QueryTimer, 0x02C, "NtQueryTimer"),
    (Service::WaitForSingleObject, 0x02D, "NtWaitForSingleObject"),
    (Service::WaitForMultipleObjects, 0x02E, "NtWaitForMultipleObjects"),
    (Service::SignalAndWaitForSingleObject, 0x02F, "NtSignalAndWaitForSingleObject"),
    (Service::QueueApcThread, 0x030, "NtQueueApcThread"),
    (Service::TestAlert, 0x031, "NtTestAlert"),
    (Service::DelayExecution, 0x032, "NtDelayExecution"),
    (Service::YieldExecution, 0x033, "NtYieldExecution"),
    (Service::AllocateVirtualMemory, 0x040, "NtAllocateVirtualMemory"),
    (Service::FreeVirtualMemory, 0x041, "NtFreeVirtualMemory"),
    (Service::ProtectVirtualMemory, 0x042, "NtProtectVirtualMemory"),
    (Service::QueryVirtualMemory, 0x043, "NtQueryVirtualMemory"),
    (Service::CreateSection, 0x044, "NtCreateSection"),
    (Service::OpenSection, 0x045, "NtOpenSection"),
    (Service::MapViewOfSection, 0x046, "NtMapViewOfSection"),
    (Service::UnmapViewOfSection, 0x047, "NtUnmapViewOfSection"),
    (Service::TerminateProcess, 0x080, "NtTerminateProcess"),
    (Service::TerminateThread, 0x081, "NtTerminateThread"),
    (Service::CreateThreadEx, 0x082, "NtCreateThreadEx"),
    (Service::OpenProcess, 0x083, "NtOpenProcess"),
    (Service::OpenThread, 0x084, "NtOpenThread"),
    (Service::QueryInformationProcess, 0x085, "NtQueryInformationProcess"),
    (Service::QueryInformationThread, 0x086, "NtQueryInformationThread"),
    (Service::SuspendThread, 0x087, "NtSuspendThread"),
    (Service::ResumeThread, 0x088, "NtResumeThread"),
    (Service::GetContextThread, 0x089, "NtGetContextThread"),
    (Service::SetContextThread, 0x08A, "NtSetContextThread"),
    (Service::CreateFile, 0x0C0, "NtCreateFile"),
    (Service::OpenFile, 0x0C1, "NtOpenFile"),
    (Service::ReadFile, 0x0C2, "NtReadFile"),
    (Service::WriteFile, 0x0C3, "NtWriteFile"),
    (Service::QueryInformationFile, 0x0C4, "NtQueryInformationFile"),
    (Service::SetInformationFile, 0x0C5, "NtSetInformationFile"),
    (Service::QueryDirectoryFile, 0x0C6, "NtQueryDirectoryFile"),
    (Service::QueryVolumeInformationFile, 0x0C7, "NtQueryVolumeInformationFile"),
    (Service::FlushBuffersFile, 0x0C8, "NtFlushBuffersFile"),
    (Service::LockFile, 0x0C9, "NtLockFile"),
    (Service::UnlockFile, 0x0CA, "NtUnlockFile"),
    (Service::DeleteFile, 0x0CB, "NtDeleteFile"),
    (Service::CancelIoFile, 0x0CC, "NtCancelIoFile"),
    (Service::CancelIoFileEx, 0x0CD, "NtCancelIoFileEx"),
    (Service::QueryAttributesFile, 0x0CE, "NtQueryAttributesFile"),
    (Service::QueryFullAttributesFile, 0x0CF, "NtQueryFullAttributesFile"),
    (Service::PolyTrace, TABLE_POLY << 12, "PolyTrace"),
    (Service::PolyResumeApc, (TABLE_POLY << 12) | 1, "PolyResumeApc"),
];

impl Service {
    /// The service number `ntdll` puts in `eax`.
    pub fn number(self) -> u32 {
        SERVICES.iter().find(|(s, _, _)| *s == self).map(|&(_, n, _)| n).expect("every service is in SERVICES")
    }

    /// The exported name (`NtClose`).
    pub fn name(self) -> &'static str {
        SERVICES.iter().find(|(s, _, _)| *s == self).map(|&(_, _, n)| n).expect("every service is in SERVICES")
    }

    /// The service a number selects, if Poly implements it. Only the low 14
    /// bits count (table and index), as on Windows.
    pub fn from_number(number: u32) -> Option<Service> {
        let number = number & 0x3fff;
        SERVICES.iter().find(|&&(_, n, _)| n == number).map(|&(s, _, _)| s)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn numbers_and_names_are_unique_and_round_trip() {
        let numbers: HashSet<u32> = SERVICES.iter().map(|s| s.1).collect();
        let names: HashSet<&str> = SERVICES.iter().map(|s| s.2).collect();
        assert_eq!(numbers.len(), SERVICES.len());
        assert_eq!(names.len(), SERVICES.len());
        for (s, n, name) in SERVICES {
            assert_eq!(Service::from_number(n), Some(s));
            assert_eq!(s.number(), n);
            assert_eq!(s.name(), name);
        }
        assert_eq!(Service::from_number(0x7ff), None);
        // Bits above the table are ignored.
        assert_eq!(Service::from_number(0x1_0000 | 0x040), Some(Service::AllocateVirtualMemory));
    }

    #[test]
    fn ranges_hold_their_groups() {
        assert!(Service::QueryObject.number() < 0x40);
        assert!((0x40..0x80).contains(&Service::MapViewOfSection.number()));
        assert!((0x80..0xC0).contains(&Service::TerminateProcess.number()));
        assert!((0xC0..0x100).contains(&Service::CreateFile.number()));
        assert!((0xC0..0x100).contains(&Service::QueryFullAttributesFile.number()));
        assert_eq!(Service::PolyTrace.number() >> 12, TABLE_POLY);
    }
}

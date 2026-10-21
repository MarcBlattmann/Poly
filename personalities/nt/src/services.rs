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
//! | 0, executive | `0x100`–`0x13F` | registry ([`REGISTRY_SERVICES`], ADR 0066) |
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

    // ----- registry (ADR 0066)
    /// `NtCreateKey(KeyHandle*, DesiredAccess, ObjectAttributes*,
    /// TitleIndex, Class*, CreateOptions, Disposition*)`.
    CreateKey,
    /// `NtOpenKey(KeyHandle*, DesiredAccess, ObjectAttributes*)`.
    OpenKey,
    /// `NtOpenKeyEx(KeyHandle*, DesiredAccess, ObjectAttributes*,
    /// OpenOptions)`.
    OpenKeyEx,
    /// `NtDeleteKey(KeyHandle)`.
    DeleteKey,
    /// `NtDeleteValueKey(KeyHandle, ValueName*)`.
    DeleteValueKey,
    /// `NtEnumerateKey(KeyHandle, Index, KeyInformationClass,
    /// KeyInformation, Length, ResultLength*)`.
    EnumerateKey,
    /// `NtEnumerateValueKey(KeyHandle, Index, KeyValueInformationClass,
    /// KeyValueInformation, Length, ResultLength*)`.
    EnumerateValueKey,
    /// `NtQueryKey(KeyHandle, KeyInformationClass, KeyInformation, Length,
    /// ResultLength*)`.
    QueryKey,
    /// `NtQueryValueKey(KeyHandle, ValueName*, KeyValueInformationClass,
    /// KeyValueInformation, Length, ResultLength*)`.
    QueryValueKey,
    /// `NtSetValueKey(KeyHandle, ValueName*, TitleIndex, Type, Data*,
    /// DataSize)`.
    SetValueKey,
    /// `NtFlushKey(KeyHandle)`.
    FlushKey,
    /// `NtNotifyChangeKey(KeyHandle, Event, ApcRoutine, ApcContext,
    /// IoStatusBlock*, CompletionFilter, WatchTree, Buffer, BufferSize,
    /// Asynchronous)`.
    NotifyChangeKey,
    /// `NtSetInformationKey(KeyHandle, KeySetInformationClass,
    /// KeySetInformation, Length)`.
    SetInformationKey,

    // ----- Poly
    /// `PolyTrace(a, b, c)`: logs three values on the kernel log (a test
    /// program reports what went wrong before it gives up). Returns
    /// `STATUS_SUCCESS`.
    PolyTrace,
}

/// Table numbers (bits 12–13 of a service number).
pub const TABLE_EXECUTIVE: u32 = 0;
pub const TABLE_WIN32K: u32 = 1;
pub const TABLE_POLY: u32 = 3;

/// Every service with its number and its name as `ntdll` exports it.
pub const SERVICES: [(Service, u32, &str); 24] = [
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
    (Service::AllocateVirtualMemory, 0x040, "NtAllocateVirtualMemory"),
    (Service::FreeVirtualMemory, 0x041, "NtFreeVirtualMemory"),
    (Service::ProtectVirtualMemory, 0x042, "NtProtectVirtualMemory"),
    (Service::QueryVirtualMemory, 0x043, "NtQueryVirtualMemory"),
    (Service::CreateSection, 0x044, "NtCreateSection"),
    (Service::OpenSection, 0x045, "NtOpenSection"),
    (Service::MapViewOfSection, 0x046, "NtMapViewOfSection"),
    (Service::UnmapViewOfSection, 0x047, "NtUnmapViewOfSection"),
    (Service::TerminateProcess, 0x080, "NtTerminateProcess"),
    (Service::PolyTrace, TABLE_POLY << 12, "PolyTrace"),
];

/// The registry services (ADR 0066), kept in their own table so the
/// groups grow independently; [`all`] chains every table.
pub const REGISTRY_SERVICES: [(Service, u32, &str); 13] = [
    (Service::CreateKey, 0x100, "NtCreateKey"),
    (Service::OpenKey, 0x101, "NtOpenKey"),
    (Service::OpenKeyEx, 0x102, "NtOpenKeyEx"),
    (Service::DeleteKey, 0x103, "NtDeleteKey"),
    (Service::DeleteValueKey, 0x104, "NtDeleteValueKey"),
    (Service::EnumerateKey, 0x105, "NtEnumerateKey"),
    (Service::EnumerateValueKey, 0x106, "NtEnumerateValueKey"),
    (Service::QueryKey, 0x107, "NtQueryKey"),
    (Service::QueryValueKey, 0x108, "NtQueryValueKey"),
    (Service::SetValueKey, 0x109, "NtSetValueKey"),
    (Service::FlushKey, 0x10A, "NtFlushKey"),
    (Service::NotifyChangeKey, 0x10B, "NtNotifyChangeKey"),
    (Service::SetInformationKey, 0x10C, "NtSetInformationKey"),
];

/// Every service Poly implements, from every table.
pub fn all() -> impl Iterator<Item = &'static (Service, u32, &'static str)> {
    SERVICES.iter().chain(REGISTRY_SERVICES.iter())
}

impl Service {
    /// The service number `ntdll` puts in `eax`.
    pub fn number(self) -> u32 {
        all().find(|(s, _, _)| *s == self).map(|&(_, n, _)| n).expect("every service is in a table")
    }

    /// The exported name (`NtClose`).
    pub fn name(self) -> &'static str {
        all().find(|(s, _, _)| *s == self).map(|&(_, _, n)| n).expect("every service is in a table")
    }

    /// The service a number selects, if Poly implements it. Only the low 14
    /// bits count (table and index), as on Windows.
    pub fn from_number(number: u32) -> Option<Service> {
        let number = number & 0x3fff;
        all().find(|&&(_, n, _)| n == number).map(|&(s, _, _)| s)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn numbers_and_names_are_unique_and_round_trip() {
        let numbers: HashSet<u32> = all().map(|s| s.1).collect();
        let names: HashSet<&str> = all().map(|s| s.2).collect();
        assert_eq!(numbers.len(), all().count());
        assert_eq!(names.len(), all().count());
        for &(s, n, name) in all() {
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
        assert_eq!(Service::PolyTrace.number() >> 12, TABLE_POLY);
        assert!(REGISTRY_SERVICES.iter().all(|s| (0x100..0x140).contains(&s.1)));
    }
}

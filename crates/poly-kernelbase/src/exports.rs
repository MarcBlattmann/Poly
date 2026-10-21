//! Which functions `kernel32.dll` forwards, and where (ADR 0079).
//!
//! Since Windows 7 most of `kernel32`'s functions are implemented in
//! `kernelbase` and reached through API-set contracts (Learn: "Windows API
//! sets"; each function's reference page names its contract in the
//! "API set" row of its requirements). Our `kernel32.dll` does the same:
//! each name below is an export forwarder to `<contract>.<name>`, which
//! the loader resolves with its API-set table (`poly_ldr::apiset`: every
//! `api-ms-win-core-*` contract is hosted by our `kernelbase.dll`). The
//! contract for each function is the one its Learn page lists, as Poly
//! reads it (to be confirmed by the spec agent against a real
//! `kernel32.dll`'s forwarder strings, which are export-table data).
//!
//! [`KERNEL32_OWN`] are the functions whose pages list no API set:
//! `kernel32` implements them itself (on top of `kernelbase`).

/// `(function, contract)`: `kernel32!function` forwards to
/// `contract.function`.
pub const KERNEL32_FORWARDS: &[(&str, &str)] = &[
    // Files (fileapi.h).
    ("CreateFileW", "api-ms-win-core-file-l1-1-0"),
    ("CreateFileA", "api-ms-win-core-file-l1-1-0"),
    ("ReadFile", "api-ms-win-core-file-l1-1-0"),
    ("WriteFile", "api-ms-win-core-file-l1-1-0"),
    ("GetFileSize", "api-ms-win-core-file-l1-1-0"),
    ("GetFileSizeEx", "api-ms-win-core-file-l1-1-0"),
    ("SetFilePointer", "api-ms-win-core-file-l1-1-0"),
    ("SetFilePointerEx", "api-ms-win-core-file-l1-1-0"),
    ("SetEndOfFile", "api-ms-win-core-file-l1-1-0"),
    ("FlushFileBuffers", "api-ms-win-core-file-l1-1-0"),
    ("GetFileType", "api-ms-win-core-file-l1-1-0"),
    ("DeleteFileW", "api-ms-win-core-file-l1-1-0"),
    ("DeleteFileA", "api-ms-win-core-file-l1-1-0"),
    ("CreateDirectoryW", "api-ms-win-core-file-l1-1-0"),
    ("CreateDirectoryA", "api-ms-win-core-file-l1-1-0"),
    ("RemoveDirectoryW", "api-ms-win-core-file-l1-1-0"),
    ("RemoveDirectoryA", "api-ms-win-core-file-l1-1-0"),
    ("GetFileAttributesW", "api-ms-win-core-file-l1-1-0"),
    ("GetFileAttributesA", "api-ms-win-core-file-l1-1-0"),
    ("SetFileAttributesW", "api-ms-win-core-file-l1-1-0"),
    ("FindFirstFileW", "api-ms-win-core-file-l1-1-0"),
    ("FindNextFileW", "api-ms-win-core-file-l1-1-0"),
    ("FindClose", "api-ms-win-core-file-l1-1-0"),
    ("GetFullPathNameW", "api-ms-win-core-file-l1-1-0"),
    ("GetFullPathNameA", "api-ms-win-core-file-l1-1-0"),
    ("GetTempPathW", "api-ms-win-core-file-l1-1-0"),
    ("MoveFileExW", "api-ms-win-core-file-l2-1-0"),
    ("CloseHandle", "api-ms-win-core-handle-l1-1-0"),
    // The process environment (processenv.h).
    ("GetStdHandle", "api-ms-win-core-processenvironment-l1-1-0"),
    ("SetStdHandle", "api-ms-win-core-processenvironment-l1-1-0"),
    ("GetCommandLineW", "api-ms-win-core-processenvironment-l1-1-0"),
    ("GetCommandLineA", "api-ms-win-core-processenvironment-l1-1-0"),
    ("GetEnvironmentVariableW", "api-ms-win-core-processenvironment-l1-1-0"),
    ("SetEnvironmentVariableW", "api-ms-win-core-processenvironment-l1-1-0"),
    ("GetEnvironmentStringsW", "api-ms-win-core-processenvironment-l1-1-0"),
    ("FreeEnvironmentStringsW", "api-ms-win-core-processenvironment-l1-1-0"),
    ("ExpandEnvironmentStringsW", "api-ms-win-core-processenvironment-l1-1-0"),
    ("GetCurrentDirectoryW", "api-ms-win-core-processenvironment-l1-1-0"),
    ("SetCurrentDirectoryW", "api-ms-win-core-processenvironment-l1-1-0"),
    // Processes and threads (processthreadsapi.h).
    ("GetCurrentProcess", "api-ms-win-core-processthreads-l1-1-0"),
    ("GetCurrentProcessId", "api-ms-win-core-processthreads-l1-1-0"),
    ("GetCurrentThread", "api-ms-win-core-processthreads-l1-1-0"),
    ("GetCurrentThreadId", "api-ms-win-core-processthreads-l1-1-0"),
    ("ExitProcess", "api-ms-win-core-processthreads-l1-1-0"),
    ("TerminateProcess", "api-ms-win-core-processthreads-l1-1-0"),
    ("CreateThread", "api-ms-win-core-processthreads-l1-1-0"),
    ("ExitThread", "api-ms-win-core-processthreads-l1-1-0"),
    ("GetExitCodeThread", "api-ms-win-core-processthreads-l1-1-0"),
    // Synchronisation (synchapi.h).
    ("CreateEventW", "api-ms-win-core-synch-l1-1-0"),
    ("SetEvent", "api-ms-win-core-synch-l1-1-0"),
    ("ResetEvent", "api-ms-win-core-synch-l1-1-0"),
    ("CreateMutexW", "api-ms-win-core-synch-l1-1-0"),
    ("ReleaseMutex", "api-ms-win-core-synch-l1-1-0"),
    ("CreateSemaphoreW", "api-ms-win-core-synch-l1-1-0"),
    ("ReleaseSemaphore", "api-ms-win-core-synch-l1-1-0"),
    ("WaitForSingleObject", "api-ms-win-core-synch-l1-1-0"),
    ("WaitForSingleObjectEx", "api-ms-win-core-synch-l1-1-0"),
    ("WaitForMultipleObjects", "api-ms-win-core-synch-l1-1-0"),
    ("WaitForMultipleObjectsEx", "api-ms-win-core-synch-l1-1-0"),
    ("Sleep", "api-ms-win-core-synch-l1-2-0"),
    ("SleepEx", "api-ms-win-core-synch-l1-1-0"),
    ("InitializeCriticalSection", "api-ms-win-core-synch-l1-1-0"),
    ("InitializeCriticalSectionAndSpinCount", "api-ms-win-core-synch-l1-1-0"),
    ("InitializeCriticalSectionEx", "api-ms-win-core-synch-l1-1-0"),
    ("EnterCriticalSection", "api-ms-win-core-synch-l1-1-0"),
    ("LeaveCriticalSection", "api-ms-win-core-synch-l1-1-0"),
    ("TryEnterCriticalSection", "api-ms-win-core-synch-l1-1-0"),
    ("DeleteCriticalSection", "api-ms-win-core-synch-l1-1-0"),
    ("InitializeSRWLock", "api-ms-win-core-synch-l1-1-0"),
    ("AcquireSRWLockExclusive", "api-ms-win-core-synch-l1-1-0"),
    ("AcquireSRWLockShared", "api-ms-win-core-synch-l1-1-0"),
    ("ReleaseSRWLockExclusive", "api-ms-win-core-synch-l1-1-0"),
    ("ReleaseSRWLockShared", "api-ms-win-core-synch-l1-1-0"),
    ("TryAcquireSRWLockExclusive", "api-ms-win-core-synch-l1-1-0"),
    ("TryAcquireSRWLockShared", "api-ms-win-core-synch-l1-1-0"),
    // Modules (libloaderapi.h).
    ("GetModuleHandleW", "api-ms-win-core-libraryloader-l1-1-0"),
    ("GetModuleFileNameW", "api-ms-win-core-libraryloader-l1-1-0"),
    ("LoadLibraryExW", "api-ms-win-core-libraryloader-l1-1-0"),
    ("LoadLibraryW", "api-ms-win-core-libraryloader-l1-2-0"),
    ("GetProcAddress", "api-ms-win-core-libraryloader-l1-1-0"),
    ("FreeLibrary", "api-ms-win-core-libraryloader-l1-1-0"),
    // Memory (memoryapi.h, heapapi.h, winbase.h).
    ("VirtualAlloc", "api-ms-win-core-memory-l1-1-0"),
    ("VirtualFree", "api-ms-win-core-memory-l1-1-0"),
    ("VirtualProtect", "api-ms-win-core-memory-l1-1-0"),
    ("VirtualQuery", "api-ms-win-core-memory-l1-1-0"),
    ("HeapCreate", "api-ms-win-core-heap-l1-1-0"),
    ("HeapDestroy", "api-ms-win-core-heap-l1-1-0"),
    ("HeapAlloc", "api-ms-win-core-heap-l1-1-0"),
    ("HeapFree", "api-ms-win-core-heap-l1-1-0"),
    ("HeapReAlloc", "api-ms-win-core-heap-l1-1-0"),
    ("HeapSize", "api-ms-win-core-heap-l1-1-0"),
    ("GetProcessHeap", "api-ms-win-core-heap-l1-1-0"),
    ("LocalAlloc", "api-ms-win-core-heap-l2-1-0"),
    ("LocalFree", "api-ms-win-core-heap-l2-1-0"),
    ("LocalReAlloc", "api-ms-win-core-heap-l2-1-0"),
    ("LocalLock", "api-ms-win-core-heap-l2-1-0"),
    ("LocalUnlock", "api-ms-win-core-heap-l2-1-0"),
    ("LocalSize", "api-ms-win-core-heap-l2-1-0"),
    // Time, version, system information (sysinfoapi.h, profileapi.h).
    ("GetTickCount", "api-ms-win-core-sysinfo-l1-1-0"),
    ("GetTickCount64", "api-ms-win-core-sysinfo-l1-1-0"),
    ("GetSystemTimeAsFileTime", "api-ms-win-core-sysinfo-l1-1-0"),
    ("GetSystemTimePreciseAsFileTime", "api-ms-win-core-sysinfo-l1-2-0"),
    ("GetSystemInfo", "api-ms-win-core-sysinfo-l1-1-0"),
    ("GetNativeSystemInfo", "api-ms-win-core-sysinfo-l1-1-0"),
    ("GetVersion", "api-ms-win-core-sysinfo-l1-1-0"),
    ("GetVersionExW", "api-ms-win-core-sysinfo-l1-1-0"),
    ("QueryPerformanceCounter", "api-ms-win-core-profile-l1-1-0"),
    ("QueryPerformanceFrequency", "api-ms-win-core-profile-l1-1-0"),
    // Errors and debugging (errhandlingapi.h, winbase.h, debugapi.h).
    ("GetLastError", "api-ms-win-core-errorhandling-l1-1-0"),
    ("SetLastError", "api-ms-win-core-errorhandling-l1-1-0"),
    ("FormatMessageW", "api-ms-win-core-localization-l1-2-0"),
    ("IsDebuggerPresent", "api-ms-win-core-debug-l1-1-0"),
    // The console (consoleapi.h, consoleapi2.h, consoleapi3.h; ADR 0080).
    ("AllocConsole", "api-ms-win-core-console-l1-1-0"),
    ("FreeConsole", "api-ms-win-core-console-l1-1-0"),
    ("AttachConsole", "api-ms-win-core-console-l1-2-0"),
    ("GetConsoleMode", "api-ms-win-core-console-l1-1-0"),
    ("SetConsoleMode", "api-ms-win-core-console-l1-1-0"),
    ("ReadConsoleW", "api-ms-win-core-console-l1-1-0"),
    ("ReadConsoleA", "api-ms-win-core-console-l1-1-0"),
    ("WriteConsoleW", "api-ms-win-core-console-l1-1-0"),
    ("WriteConsoleA", "api-ms-win-core-console-l1-1-0"),
    ("ReadConsoleInputW", "api-ms-win-core-console-l1-1-0"),
    ("PeekConsoleInputW", "api-ms-win-core-console-l1-1-0"),
    ("WriteConsoleInputW", "api-ms-win-core-console-l2-1-0"),
    ("GetNumberOfConsoleInputEvents", "api-ms-win-core-console-l1-1-0"),
    ("FlushConsoleInputBuffer", "api-ms-win-core-console-l2-1-0"),
    ("GetConsoleScreenBufferInfo", "api-ms-win-core-console-l2-1-0"),
    ("GetConsoleScreenBufferInfoEx", "api-ms-win-core-console-l2-1-0"),
    ("SetConsoleTextAttribute", "api-ms-win-core-console-l2-1-0"),
    ("SetConsoleCursorPosition", "api-ms-win-core-console-l2-1-0"),
    ("FillConsoleOutputCharacterW", "api-ms-win-core-console-l2-1-0"),
    ("FillConsoleOutputCharacterA", "api-ms-win-core-console-l2-1-0"),
    ("FillConsoleOutputAttribute", "api-ms-win-core-console-l2-1-0"),
    ("GetConsoleCP", "api-ms-win-core-console-l1-1-0"),
    ("SetConsoleCP", "api-ms-win-core-console-l2-1-0"),
    ("GetConsoleOutputCP", "api-ms-win-core-console-l1-1-0"),
    ("SetConsoleOutputCP", "api-ms-win-core-console-l1-1-0"),
    ("SetConsoleCtrlHandler", "api-ms-win-core-console-l1-1-0"),
    ("GenerateConsoleCtrlEvent", "api-ms-win-core-console-l2-1-0"),
    ("GetConsoleWindow", "api-ms-win-core-console-l3-2-0"),
    ("SetConsoleTitleW", "api-ms-win-core-console-l2-1-0"),
    ("GetConsoleTitleW", "api-ms-win-core-console-l2-1-0"),
    // ----- the second slice (ADR 0079).
    // Processes (processthreadsapi.h).
    ("CreateProcessW", "api-ms-win-core-processthreads-l1-1-0"),
    ("GetExitCodeProcess", "api-ms-win-core-processthreads-l1-1-0"),
    ("GetProcessId", "api-ms-win-core-processthreads-l1-1-0"),
    // Thread-local storage (processthreadsapi.h) and fiber-local storage
    // (fibersapi.h).
    ("TlsAlloc", "api-ms-win-core-processthreads-l1-1-0"),
    ("TlsFree", "api-ms-win-core-processthreads-l1-1-0"),
    ("TlsGetValue", "api-ms-win-core-processthreads-l1-1-0"),
    ("TlsSetValue", "api-ms-win-core-processthreads-l1-1-0"),
    ("FlsAlloc", "api-ms-win-core-fibers-l1-1-0"),
    ("FlsFree", "api-ms-win-core-fibers-l1-1-0"),
    ("FlsGetValue", "api-ms-win-core-fibers-l1-1-0"),
    ("FlsSetValue", "api-ms-win-core-fibers-l1-1-0"),
    // Code pages and strings (stringapiset.h, winnls.h).
    ("MultiByteToWideChar", "api-ms-win-core-string-l1-1-0"),
    ("WideCharToMultiByte", "api-ms-win-core-string-l1-1-0"),
    ("CompareStringOrdinal", "api-ms-win-core-string-l1-1-0"),
    ("GetACP", "api-ms-win-core-localization-l1-2-0"),
    ("GetOEMCP", "api-ms-win-core-localization-l1-2-0"),
    ("GetCPInfo", "api-ms-win-core-localization-l1-2-0"),
    ("IsValidCodePage", "api-ms-win-core-localization-l1-2-0"),
    // File mappings (memoryapi.h).
    ("CreateFileMappingW", "api-ms-win-core-memory-l1-1-0"),
    ("OpenFileMappingW", "api-ms-win-core-memory-l1-1-0"),
    ("MapViewOfFile", "api-ms-win-core-memory-l1-1-0"),
    ("MapViewOfFileEx", "api-ms-win-core-memory-l1-1-0"),
    ("UnmapViewOfFile", "api-ms-win-core-memory-l1-1-0"),
    ("FlushViewOfFile", "api-ms-win-core-memory-l1-1-0"),
    // Condition variables and one-time initialisation (synchapi.h).
    ("InitializeConditionVariable", "api-ms-win-core-synch-l1-2-0"),
    ("SleepConditionVariableCS", "api-ms-win-core-synch-l1-2-0"),
    ("SleepConditionVariableSRW", "api-ms-win-core-synch-l1-2-0"),
    ("WakeConditionVariable", "api-ms-win-core-synch-l1-2-0"),
    ("WakeAllConditionVariable", "api-ms-win-core-synch-l1-2-0"),
    ("InitOnceInitialize", "api-ms-win-core-synch-l1-2-0"),
    ("InitOnceExecuteOnce", "api-ms-win-core-synch-l1-2-0"),
    ("InitOnceBeginInitialize", "api-ms-win-core-synch-l1-2-0"),
    ("InitOnceComplete", "api-ms-win-core-synch-l1-2-0"),
    // File information and times (fileapi.h, timezoneapi.h, sysinfoapi.h).
    ("GetFileInformationByHandle", "api-ms-win-core-file-l1-1-0"),
    ("GetFileTime", "api-ms-win-core-file-l1-1-0"),
    ("SetFileTime", "api-ms-win-core-file-l1-1-0"),
    ("CompareFileTime", "api-ms-win-core-file-l1-1-0"),
    ("FileTimeToLocalFileTime", "api-ms-win-core-file-l1-1-0"),
    ("LocalFileTimeToFileTime", "api-ms-win-core-file-l1-1-0"),
    ("FileTimeToSystemTime", "api-ms-win-core-timezone-l1-1-0"),
    ("SystemTimeToFileTime", "api-ms-win-core-timezone-l1-1-0"),
    ("GetSystemTime", "api-ms-win-core-sysinfo-l1-1-0"),
    ("GetLocalTime", "api-ms-win-core-sysinfo-l1-1-0"),
    // ----- what 7-Zip's command line and MSVC's start-up code import (ADR
    // 0087). The contract is the oldest version of the one each page's
    // "APILocation" metadata names (as ADR 0083 reads advapi32's).
    ("ResumeThread", "api-ms-win-core-processthreads-l1-1-0"),
    ("SuspendThread", "api-ms-win-core-processthreads-l1-1-0"),
    ("GetProcessTimes", "api-ms-win-core-processthreads-l1-1-0"),
    ("GetStartupInfoW", "api-ms-win-core-processthreads-l1-1-0"),
    ("IsProcessorFeaturePresent", "api-ms-win-core-processthreads-l1-1-1"),
    ("GetProcessAffinityMask", "api-ms-win-core-processtopology-obsolete-l1-1-0"),
    ("SetProcessAffinityMask", "api-ms-win-core-processtopology-obsolete-l1-1-0"),
    ("SetThreadAffinityMask", "api-ms-win-core-processtopology-obsolete-l1-1-0"),
    ("OpenEventW", "api-ms-win-core-synch-l1-1-0"),
    ("RaiseException", "api-ms-win-core-errorhandling-l1-1-0"),
    ("SetUnhandledExceptionFilter", "api-ms-win-core-errorhandling-l1-1-0"),
    ("UnhandledExceptionFilter", "api-ms-win-core-errorhandling-l1-1-0"),
    ("InitializeSListHead", "api-ms-win-core-interlocked-l1-1-0"),
    ("InterlockedPushEntrySList", "api-ms-win-core-interlocked-l1-1-0"),
    ("InterlockedPopEntrySList", "api-ms-win-core-interlocked-l1-1-0"),
    ("InterlockedFlushSList", "api-ms-win-core-interlocked-l1-1-0"),
    ("QueryDepthSList", "api-ms-win-core-interlocked-l1-1-0"),
    ("GetModuleHandleA", "api-ms-win-core-libraryloader-l1-1-0"),
    ("GetLargePageMinimum", "api-ms-win-core-memory-l1-1-1"),
    ("GlobalMemoryStatusEx", "api-ms-win-core-sysinfo-l1-1-0"),
    ("DeviceIoControl", "api-ms-win-core-io-l1-1-0"),
    ("GetDiskFreeSpaceW", "api-ms-win-core-file-l1-1-0"),
    ("GetDiskFreeSpaceExW", "api-ms-win-core-file-l1-1-0"),
    ("GetLogicalDrives", "api-ms-win-core-file-l1-1-0"),
    ("GetLogicalDriveStringsW", "api-ms-win-core-file-l1-1-0"),
    ("AreFileApisANSI", "api-ms-win-core-file-l1-2-2"),
    ("SetFileApisToANSI", "api-ms-win-core-file-l1-2-2"),
    ("SetFileApisToOEM", "api-ms-win-core-file-l1-2-2"),
    ("FindFirstStreamW", "api-ms-win-core-file-l1-2-2"),
    ("FindNextStreamW", "api-ms-win-core-file-l1-2-2"),
    ("CreateHardLinkW", "api-ms-win-core-file-l2-1-0"),
    ("MoveFileWithProgressW", "api-ms-win-core-file-l2-1-0"),
    ("MoveFileW", "api-ms-win-core-kernel32-legacy-l1-1-0"),
    ("FileTimeToDosDateTime", "api-ms-win-core-kernel32-legacy-l1-1-0"),
    ("DosDateTimeToFileTime", "api-ms-win-core-kernel32-legacy-l1-1-0"),
];

/// `(function, contract)` of the `kernel32` exports that forward to our
/// `ntdll` (ADR 0087): their pages' "APILocation" metadata names
/// `api-ms-win-core-rtlsupport-*` and `ntdll.dll`, which Poly's API-set
/// table gives to `ntdll` (ADR 0071).
pub const KERNEL32_NTDLL_FORWARDS: &[(&str, &str)] = &[
    ("RtlCaptureContext", "api-ms-win-core-rtlsupport-l1-1-0"),
    ("RtlLookupFunctionEntry", "api-ms-win-core-rtlsupport-l1-1-0"),
    ("RtlVirtualUnwind", "api-ms-win-core-rtlsupport-l1-1-0"),
    ("RtlUnwindEx", "api-ms-win-core-rtlsupport-l1-1-0"),
    ("RtlPcToFileHeader", "api-ms-win-core-rtlsupport-l1-1-0"),
];

/// The exports of our `oleaut32.dll` (ADR 0087) with the ordinals programs
/// import them by (Visual C++'s import library binds `oleaut32` by
/// ordinal): export-table data, from Wine's `oleaut32.spec`.
pub const OLEAUT32_EXPORTS: &[(&str, u16)] = &[
    ("SysAllocString", 2),
    ("SysReAllocString", 3),
    ("SysAllocStringLen", 4),
    ("SysReAllocStringLen", 5),
    ("SysFreeString", 6),
    ("SysStringLen", 7),
    ("VariantInit", 8),
    ("VariantClear", 9),
    ("VariantCopy", 10),
];

/// The exports of our `user32.dll` (ADR 0087): two string functions until
/// windows exist (Phase 6).
pub const USER32_EXPORTS: &[&str] = &["CharUpperW", "CharPrevExA"];

/// The functions `kernel32.dll` implements itself: the global-memory
/// functions and (second slice) the `lstr*` string functions, whose Learn
/// pages name no API set.
pub const KERNEL32_OWN: &[&str] = &[
    "GlobalAlloc",
    "GlobalFree",
    "GlobalSize",
    "GlobalLock",
    "GlobalUnlock",
    "lstrlenW",
    "lstrlenA",
    "lstrcmpW",
    "lstrcmpiW",
];

/// Every function `kernelbase.dll` must export: each one `kernel32` or
/// `advapi32` forwards ([`crate::advapi`], ADR 0083).
pub fn kernelbase_exports() -> impl Iterator<Item = &'static str> {
    KERNEL32_FORWARDS.iter().chain(crate::advapi::ADVAPI32_FORWARDS).map(|&(name, _)| name)
}

/// The forwarder string of `kernel32!name`, as the export directory holds
/// it (PE format: "Export Address Table": `DLLName.FunctionName`).
pub fn forwarder(name: &str) -> Option<alloc::string::String> {
    KERNEL32_FORWARDS
        .iter()
        .chain(KERNEL32_NTDLL_FORWARDS)
        .find(|&&(n, _)| n == name)
        .map(|&(n, c)| alloc::format!("{c}.{n}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    use poly_ldr::apiset::{ApiSet, resolve};
    use std::collections::BTreeSet;

    #[test]
    fn every_contract_is_hosted_by_kernelbase() {
        for &(name, contract) in KERNEL32_FORWARDS {
            let w: std::vec::Vec<u16> = contract.encode_utf16().collect();
            assert_eq!(resolve(&w), ApiSet::Host("kernelbase.dll"), "{name} -> {contract}");
        }
    }

    #[test]
    fn rtlsupport_contracts_go_to_ntdll() {
        for &(name, contract) in KERNEL32_NTDLL_FORWARDS {
            let w: std::vec::Vec<u16> = contract.encode_utf16().collect();
            assert_eq!(resolve(&w), ApiSet::Host("ntdll.dll"), "{name} -> {contract}");
        }
        assert_eq!(
            forwarder("RtlCaptureContext").as_deref(),
            Some("api-ms-win-core-rtlsupport-l1-1-0.RtlCaptureContext")
        );
        let ordinals: BTreeSet<u16> = OLEAUT32_EXPORTS.iter().map(|&(_, o)| o).collect();
        assert_eq!(ordinals.len(), OLEAUT32_EXPORTS.len());
    }

    #[test]
    fn names_are_unique() {
        let mut seen = BTreeSet::new();
        let ntdll = KERNEL32_NTDLL_FORWARDS.iter().map(|&(n, _)| n);
        for name in kernelbase_exports().chain(KERNEL32_OWN.iter().copied()).chain(ntdll) {
            assert!(seen.insert(name), "{name} listed twice");
        }
        assert_eq!(forwarder("CreateFileW").as_deref(), Some("api-ms-win-core-file-l1-1-0.CreateFileW"));
        assert_eq!(forwarder("GlobalAlloc"), None);
    }
}

//! The tables: signatures, the functions that do not return, tail-call or flush, and the data exports.

use super::*;

// ----- the tables -------------------------------------------------------------------------

/// Signatures `ret=args` from Microsoft Learn's prototypes, one letter per
/// value (`tools/wintrace/polytrace`: `i` 32-bit integer, `q` 64-bit
/// integer, `s` 16-bit, `c` 8-bit, `p` pointer, `h` handle, `w`/`a` wide /
/// ANSI string, `v` void). `oleaut32`'s functions are listed by name; the
/// program imports them by ordinal ([`ordinal_name`]).
pub const SIGNATURES: &[(&str, &str)] = &[
    // kernel32 (processthreadsapi.h, synchapi.h, fileapi.h, memoryapi.h, ...)
    ("CloseHandle", "i=h"),
    ("CompareFileTime", "i=pp"),
    ("CreateDirectoryW", "i=wp"),
    ("CreateEventW", "h=piiw"),
    ("CreateFileW", "h=wiipiih"),
    ("CreateHardLinkW", "i=wwp"),
    ("CreateSemaphoreW", "h=piiw"),
    ("CreateThread", "h=pqppip"),
    ("DeleteCriticalSection", "v=p"),
    ("DeleteFileW", "i=w"),
    ("DeviceIoControl", "i=hipipipp"),
    ("DosDateTimeToFileTime", "i=ssp"),
    ("EnterCriticalSection", "v=p"),
    ("ExitProcess", "v=i"),
    ("ExitThread", "v=i"),
    ("FileTimeToDosDateTime", "i=ppp"),
    ("FileTimeToLocalFileTime", "i=pp"),
    ("FileTimeToSystemTime", "i=pp"),
    ("FindClose", "i=h"),
    ("FindFirstFileW", "h=wp"),
    ("FindFirstStreamW", "h=wipi"),
    ("FindNextFileW", "i=hp"),
    ("FindNextStreamW", "i=hp"),
    ("FormatMessageW", "i=ipiipip"),
    ("FreeLibrary", "i=h"),
    ("GetCommandLineW", "p="),
    ("GetConsoleMode", "i=hp"),
    ("GetConsoleScreenBufferInfo", "i=hp"),
    ("GetCurrentDirectoryW", "i=ip"),
    ("GetCurrentProcess", "h="),
    ("GetCurrentProcessId", "i="),
    ("GetCurrentThreadId", "i="),
    ("GetDiskFreeSpaceExW", "i=wppp"),
    ("GetDiskFreeSpaceW", "i=wpppp"),
    ("GetEnvironmentVariableW", "i=wpi"),
    ("GetExitCodeThread", "i=hp"),
    ("GetFileAttributesW", "i=w"),
    ("GetFileInformationByHandle", "i=hp"),
    ("GetFileSize", "i=hp"),
    ("GetFileSizeEx", "i=hp"),
    ("GetFullPathNameW", "i=wipp"),
    ("GetLargePageMinimum", "q="),
    ("GetLastError", "i="),
    ("GetLogicalDriveStringsW", "i=ip"),
    ("GetModuleFileNameW", "i=hpi"),
    ("GetModuleHandleA", "h=a"),
    ("GetModuleHandleW", "h=w"),
    ("GetOEMCP", "i="),
    ("GetACP", "i="),
    ("GetProcAddress", "p=ha"),
    ("GetProcessAffinityMask", "i=hpp"),
    ("GetProcessHeap", "h="),
    ("GetProcessTimes", "i=hpppp"),
    ("GetStartupInfoW", "v=p"),
    ("GetStdHandle", "h=i"),
    ("GetSystemInfo", "v=p"),
    ("GetSystemTimeAsFileTime", "v=p"),
    ("GetTempPathW", "i=ip"),
    ("GetTickCount", "i="),
    ("GetTickCount64", "q="),
    ("GetVersion", "i="),
    ("GlobalMemoryStatusEx", "i=p"),
    ("HeapAlloc", "p=hiq"),
    ("HeapFree", "i=hip"),
    ("InitializeCriticalSection", "v=p"),
    ("IsProcessorFeaturePresent", "i=i"),
    ("LeaveCriticalSection", "v=p"),
    ("LoadLibraryExW", "h=whi"),
    ("LoadLibraryW", "h=w"),
    ("LocalFileTimeToFileTime", "i=pp"),
    ("LocalFree", "p=p"),
    ("MapViewOfFile", "p=hiiiq"),
    ("MoveFileW", "i=ww"),
    ("MoveFileWithProgressW", "i=wwppi"),
    ("MultiByteToWideChar", "i=iipipi"),
    ("OpenEventW", "h=iiw"),
    ("OpenFileMappingW", "h=iiw"),
    ("QueryPerformanceCounter", "i=p"),
    ("QueryPerformanceFrequency", "i=p"),
    ("RaiseException", "v=iiip"),
    ("ReadConsoleW", "i=hpipp"),
    ("ReadFile", "i=hpipp"),
    ("ReleaseSemaphore", "i=hip"),
    ("RemoveDirectoryW", "i=w"),
    ("ResetEvent", "i=h"),
    ("ResumeThread", "i=h"),
    ("SetConsoleCtrlHandler", "i=pi"),
    ("SetConsoleMode", "i=hi"),
    ("SetCurrentDirectoryW", "i=w"),
    ("SetEndOfFile", "i=h"),
    ("SetEvent", "i=h"),
    ("SetFileApisToOEM", "v="),
    ("SetFileAttributesW", "i=wi"),
    ("SetFilePointer", "i=hipi"),
    ("SetFilePointerEx", "i=hqpi"),
    ("SetFileTime", "i=hppp"),
    ("SetLastError", "v=i"),
    ("SetProcessAffinityMask", "i=hq"),
    ("SetThreadAffinityMask", "q=hq"),
    ("SetThreadPriority", "i=hi"),
    ("Sleep", "v=i"),
    ("SuspendThread", "i=h"),
    ("TerminateProcess", "i=hi"),
    ("UnmapViewOfFile", "i=p"),
    ("VirtualAlloc", "p=pqii"),
    ("VirtualFree", "i=pqi"),
    ("WaitForMultipleObjects", "i=ipii"),
    ("WaitForSingleObject", "i=hi"),
    ("WideCharToMultiByte", "i=iipipipp"),
    ("WriteConsoleW", "i=hpipp"),
    ("WriteFile", "i=hpipp"),
    // advapi32 (securitybaseapi.h, winreg.h, ntsecapi.h)
    ("AdjustTokenPrivileges", "i=hipipp"),
    ("GetFileSecurityW", "i=wipip"),
    ("LookupPrivilegeValueW", "i=wwp"),
    ("OpenProcessToken", "i=hip"),
    ("RegCloseKey", "i=h"),
    ("RegOpenKeyExW", "i=hwiip"),
    ("RegQueryValueExW", "i=hwpppp"),
    ("SetFileSecurityW", "i=wip"),
    ("SystemFunction036", "c=pi"),
    // msvcrt (the C runtime reference)
    ("??1type_info@@UEAA@XZ", "v=p"),
    ("?terminate@@YAXXZ", "v="),
    ("_CxxThrowException", "v=pp"),
    ("_XcptFilter", "i=ip"),
    ("__C_specific_handler", "i=pppp"),
    ("__CxxFrameHandler", "i=pppp"),
    ("__CxxFrameHandler3", "i=pppp"),
    ("__dllonexit", "p=ppp"),
    ("__getmainargs", "i=pppip"),
    ("__set_app_type", "v=i"),
    ("__setusermatherr", "v=p"),
    ("__wgetmainargs", "i=pppip"),
    ("_beginthreadex", "h=pippip"),
    ("_c_exit", "v="),
    ("_cexit", "v="),
    ("_exit", "v=i"),
    ("_get_osfhandle", "h=i"),
    ("_initterm", "v=pp"),
    ("_isatty", "i=i"),
    ("_onexit", "p=p"),
    ("_purecall", "i="),
    ("abort", "v="),
    ("calloc", "p=qq"),
    ("exit", "v=i"),
    ("fflush", "i=p"),
    ("fgetc", "i=p"),
    ("fputc", "i=ip"),
    ("fputs", "i=ap"),
    ("free", "v=p"),
    ("malloc", "p=q"),
    ("memcmp", "i=ppq"),
    ("memcpy", "p=ppq"),
    ("memmove", "p=ppq"),
    ("memset", "p=piq"),
    ("realloc", "p=pq"),
    ("strcmp", "i=aa"),
    ("strlen", "q=a"),
    ("wcscmp", "i=ww"),
    ("wcslen", "q=w"),
    ("wcsstr", "p=ww"),
    // oleaut32 (oleauto.h), imported by ordinal
    ("SysAllocString", "p=w"),
    ("SysAllocStringLen", "p=pi"),
    ("SysFreeString", "v=p"),
    ("SysReAllocString", "i=pw"),
    ("SysReAllocStringLen", "i=ppi"),
    ("SysStringLen", "i=p"),
    ("VariantClear", "i=p"),
    ("VariantCopy", "i=pp"),
    ("VariantInit", "v=p"),
    // user32 (winuser.h)
    ("CharPrevExA", "p=sppi"),
    ("CharUpperW", "p=w"),
];

/// Functions that do not return: their record is written when they begin
/// and the buffer goes to the file.
pub const NORETURN: &[&str] = &[
    "ExitProcess",
    "ExitThread",
    "FatalExit",
    "FatalAppExitA",
    "FatalAppExitW",
    "FreeLibraryAndExitThread",
    "RaiseException",
    "RaiseFailFastException",
    "exit",
    "_exit",
    "_Exit",
    "quick_exit",
    "abort",
    "_amsg_exit",
    "?terminate@@YAXXZ",
    "terminate",
    "_purecall",
    "_endthread",
    "_endthreadex",
    "_invalid_parameter_noinfo_noreturn",
];

/// Functions that capture, unwind or resume their caller's frame: the
/// thunk jumps to them, so they see the caller's own frame (their record
/// is written when they begin).
pub const TAIL: &[&str] = &[
    "_CxxThrowException",
    "longjmp",
    "_longjmp",
    "setjmp",
    "_setjmp",
    "_setjmpex",
    "__intrinsic_setjmp",
    "__intrinsic_setjmpex",
    "__C_specific_handler",
    "__CxxFrameHandler",
    "__CxxFrameHandler2",
    "__CxxFrameHandler3",
    "__CxxFrameHandler4",
    "__GSHandlerCheck",
    "__GSHandlerCheck_EH",
    "__GSHandlerCheck_SEH",
    "_local_unwind",
    "RtlUnwind",
    "RtlUnwindEx",
    "RtlCaptureContext",
    "RtlRestoreContext",
];

/// Functions that may end the process (the buffer is written out first).
pub const FLUSH: &[&str] = &["TerminateProcess", "TerminateThread"];

/// The C runtimes' exported variables (Learn: "Global variables"; and
/// those of our own `msvcrt.dll`): forwarders in a proxy, not thunks.
pub const DATA: &[&str] = &[
    "_iob",
    "__argc",
    "__argv",
    "__wargv",
    "_environ",
    "_wenviron",
    "__initenv",
    "__winitenv",
    "_acmdln",
    "_wcmdln",
    "_pgmptr",
    "_wpgmptr",
    "_fmode",
    "_commode",
    "_ctype",
    "_pctype",
    "_pwctype",
    "__mb_cur_max",
    "_timezone",
    "_daylight",
    "_dstbias",
    "_tzname",
    "_sys_errlist",
    "_sys_nerr",
    "_osver",
    "_winver",
    "_winmajor",
    "_winminor",
    "_HUGE",
    "__badioinfo",
    "__pioinfo",
];

/// The logger's flags (`tools/wintrace/polytrace`: `NORETURN`, `TAIL`,
/// `FLUSH`).
pub(super) const F_NORETURN: u32 = 1;
pub(super) const F_TAIL: u32 = 2;
const F_FLUSH: u32 = 4;

/// The name of an `oleaut32` ordinal, from our `oleaut32.dll`'s export
/// table (`poly_kernelbase::exports::OLEAUT32_EXPORTS`, the coverage
/// map's ordinals).
pub fn ordinal_name(dll: &str, ordinal: u16) -> Option<&'static str> {
    if !dll.eq_ignore_ascii_case("oleaut32.dll") {
        return None;
    }
    poly_kernelbase::exports::OLEAUT32_EXPORTS.iter().find(|&&(_, o)| o == ordinal).map(|&(n, _)| n)
}

/// A function's signature, if [`SIGNATURES`] has it.
pub fn signature(dll: &str, sym: &Sym) -> Option<&'static str> {
    let name = match sym {
        Sym::Name(n) => n.as_str(),
        Sym::Ordinal(o) => ordinal_name(dll, *o)?,
    };
    SIGNATURES.iter().find(|&&(n, _)| n == name).map(|&(_, s)| s)
}

pub(super) fn flags_of(sym: &Sym) -> u32 {
    let Sym::Name(n) = sym else { return 0 };
    let mut f = 0;
    if NORETURN.contains(&n.as_str()) {
        f |= F_NORETURN;
    }
    if TAIL.contains(&n.as_str()) {
        f |= F_TAIL;
    }
    if FLUSH.contains(&n.as_str()) {
        f |= F_FLUSH;
    }
    f
}

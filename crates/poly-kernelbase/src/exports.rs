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
    // Named pipes, completion ports and overlapped I/O (ADR 0169).
    ("CreateJobObjectW", "api-ms-win-core-job-l1-1-0"),
    ("OpenJobObjectW", "api-ms-win-core-job-l1-1-0"),
    ("AssignProcessToJobObject", "api-ms-win-core-job-l1-1-0"),
    ("IsProcessInJob", "api-ms-win-core-job-l1-1-0"),
    ("QueryInformationJobObject", "api-ms-win-core-job-l1-1-0"),
    ("SetInformationJobObject", "api-ms-win-core-job-l1-1-0"),
    ("TerminateJobObject", "api-ms-win-core-job-l1-1-0"),
    ("InitializeProcThreadAttributeList", "api-ms-win-core-processthreads-l1-1-0"),
    ("UpdateProcThreadAttribute", "api-ms-win-core-processthreads-l1-1-0"),
    ("DeleteProcThreadAttributeList", "api-ms-win-core-processthreads-l1-1-0"),
    ("CreateFileMappingA", "api-ms-win-core-memory-l1-1-0"),
    ("CreateNamedPipeW", "api-ms-win-core-namedpipe-l1-1-0"),
    ("ConnectNamedPipe", "api-ms-win-core-namedpipe-l1-1-0"),
    ("DisconnectNamedPipe", "api-ms-win-core-namedpipe-l1-1-0"),
    ("PeekNamedPipe", "api-ms-win-core-namedpipe-l1-1-0"),
    ("GetNamedPipeInfo", "api-ms-win-core-namedpipe-l1-1-0"),
    ("GetNamedPipeHandleStateW", "api-ms-win-core-namedpipe-l1-1-0"),
    ("SetNamedPipeHandleState", "api-ms-win-core-namedpipe-l1-1-0"),
    ("TransactNamedPipe", "api-ms-win-core-namedpipe-l1-1-0"),
    ("CallNamedPipeW", "api-ms-win-core-namedpipe-l1-1-0"),
    ("WaitNamedPipeW", "api-ms-win-core-namedpipe-l1-1-0"),
    ("GetNamedPipeClientProcessId", "api-ms-win-core-namedpipe-l1-1-0"),
    ("GetNamedPipeServerProcessId", "api-ms-win-core-namedpipe-l1-1-0"),
    ("GetNamedPipeClientSessionId", "api-ms-win-core-namedpipe-l1-1-0"),
    ("GetNamedPipeServerSessionId", "api-ms-win-core-namedpipe-l1-1-0"),
    ("CancelIo", "api-ms-win-core-io-l1-1-0"),
    ("CancelIoEx", "api-ms-win-core-io-l1-1-0"),
    ("CancelSynchronousIo", "api-ms-win-core-io-l1-1-0"),
    ("CreateIoCompletionPort", "api-ms-win-core-io-l1-1-0"),
    ("GetOverlappedResult", "api-ms-win-core-io-l1-1-0"),
    ("GetOverlappedResultEx", "api-ms-win-core-io-l1-1-0"),
    ("GetQueuedCompletionStatus", "api-ms-win-core-io-l1-1-0"),
    ("GetQueuedCompletionStatusEx", "api-ms-win-core-io-l1-1-0"),
    ("PostQueuedCompletionStatus", "api-ms-win-core-io-l1-1-0"),
    ("ReadFileEx", "api-ms-win-core-file-l1-1-0"),
    ("WriteFileEx", "api-ms-win-core-file-l1-1-0"),
    ("SetFileCompletionNotificationModes", "api-ms-win-core-file-l1-1-0"),
    ("OpenProcess", "api-ms-win-core-processthreads-l1-1-0"),
    ("OpenThread", "api-ms-win-core-processthreads-l1-1-0"),
    ("GetThreadId", "api-ms-win-core-processthreads-l1-1-0"),
    ("QueueUserAPC", "api-ms-win-core-processthreads-l1-1-0"),
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
    ("FindFirstFileA", "api-ms-win-core-file-l1-1-0"),
    ("FindFirstFileExA", "api-ms-win-core-file-l1-1-0"),
    ("FindNextFileA", "api-ms-win-core-file-l1-1-0"),
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
    // Start-up calls of 32-bit programs (ADR 0186, stage 3).
    ("SetHandleCount", "api-ms-win-core-handle-l1-1-0"),
    ("GetThreadUILanguage", "api-ms-win-core-localization-l1-2-0"),
    ("SetThreadPreferredUILanguages", "api-ms-win-core-localization-l1-2-0"),
    // ANSI forms (ADR 0186, stage 3).
    ("GetEnvironmentStrings", "api-ms-win-core-processenvironment-l1-1-0"),
    ("GetEnvironmentStringsA", "api-ms-win-core-processenvironment-l1-1-0"),
    ("FreeEnvironmentStringsA", "api-ms-win-core-processenvironment-l1-1-0"),
    ("GetCurrentDirectoryA", "api-ms-win-core-processenvironment-l1-1-0"),
    ("GetModuleFileNameA", "api-ms-win-core-libraryloader-l1-1-0"),
    ("ExpandEnvironmentStringsW", "api-ms-win-core-processenvironment-l1-1-0"),
    ("GetCurrentDirectoryW", "api-ms-win-core-processenvironment-l1-1-0"),
    ("SetCurrentDirectoryW", "api-ms-win-core-processenvironment-l1-1-0"),
    // Processes and threads (processthreadsapi.h).
    ("GetCurrentProcess", "api-ms-win-core-processthreads-l1-1-0"),
    ("GetCurrentProcessId", "api-ms-win-core-processthreads-l1-1-0"),
    ("GetCurrentThread", "api-ms-win-core-processthreads-l1-1-0"),
    ("GetCurrentThreadId", "api-ms-win-core-processthreads-l1-1-0"),
    ("GetCurrentThreadStackLimits", "api-ms-win-core-processthreads-l1-1-0"),
    ("ExitProcess", "api-ms-win-core-processthreads-l1-1-0"),
    ("TerminateProcess", "api-ms-win-core-processthreads-l1-1-0"),
    ("ExitThread", "api-ms-win-core-processthreads-l1-1-0"),
    ("GetExitCodeThread", "api-ms-win-core-processthreads-l1-1-0"),
    // Synchronisation (synchapi.h).
    ("CreateEventW", "api-ms-win-core-synch-l1-1-0"),
    ("CreateWaitableTimerW", "api-ms-win-core-synch-l1-1-0"),
    ("SetEvent", "api-ms-win-core-synch-l1-1-0"),
    ("ResetEvent", "api-ms-win-core-synch-l1-1-0"),
    ("CreateMutexW", "api-ms-win-core-synch-l1-1-0"),
    ("ReleaseMutex", "api-ms-win-core-synch-l1-1-0"),
    ("CreateSemaphoreW", "api-ms-win-core-synch-l1-1-0"),
    // The ANSI forms (ADR 0155 step 11: V8 makes its semaphores with `CreateSemaphoreA`).
    ("CreateEventA", "api-ms-win-core-synch-l1-1-0"),
    ("CreateMutexA", "api-ms-win-core-synch-l1-1-0"),
    ("CreateSemaphoreA", "api-ms-win-core-synch-l1-1-0"),
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
    ("FreeLibraryAndExitThread", "api-ms-win-core-libraryloader-l1-1-0"),
    // ADR 0114: 7-Zip asks for it with `GetProcAddress`.
    ("SetDefaultDllDirectories", "api-ms-win-core-libraryloader-l1-1-0"),
    // Memory (memoryapi.h, heapapi.h, winbase.h).
    ("VirtualAlloc", "api-ms-win-core-memory-l1-1-0"),
    ("VirtualFree", "api-ms-win-core-memory-l1-1-0"),
    ("VirtualProtect", "api-ms-win-core-memory-l1-1-0"),
    ("VirtualLock", "api-ms-win-core-memory-l1-1-0"),
    ("VirtualUnlock", "api-ms-win-core-memory-l1-1-0"),
    ("VirtualQuery", "api-ms-win-core-memory-l1-1-0"),
    ("VirtualAllocEx", "api-ms-win-core-memory-l1-1-0"),
    ("VirtualFreeEx", "api-ms-win-core-memory-l1-1-0"),
    ("VirtualProtectEx", "api-ms-win-core-memory-l1-1-0"),
    ("VirtualQueryEx", "api-ms-win-core-memory-l1-1-0"),
    ("FlushInstructionCache", "api-ms-win-core-processthreads-l1-1-0"),
    ("ReadProcessMemory", "api-ms-win-core-memory-l1-1-0"),
    ("WriteProcessMemory", "api-ms-win-core-memory-l1-1-0"),
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
    ("FormatMessageA", "api-ms-win-core-localization-l1-2-0"),
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
    // CreateProcessW's handle inheritance and STARTUPINFO (ADR 0089;
    // processthreadsapi.h, handleapi.h).
    ("CreateProcessA", "api-ms-win-core-processthreads-l1-1-0"),
    ("DuplicateHandle", "api-ms-win-core-handle-l1-1-0"),
    ("GetHandleInformation", "api-ms-win-core-handle-l1-1-0"),
    ("SetHandleInformation", "api-ms-win-core-handle-l1-1-0"),
    // Anonymous pipes for the C runtime's _pipe and _popen (ADR 0093;
    // namedpipeapi.h).
    ("CreatePipe", "api-ms-win-core-namedpipe-l1-1-0"),
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
    // Double-byte code pages (ADR 0094; the contract is a guess, to be
    // confirmed by the spec agent).
    ("IsDBCSLeadByte", "api-ms-win-core-localization-l1-2-0"),
    ("IsDBCSLeadByteEx", "api-ms-win-core-localization-l1-2-0"),
    // File mappings (memoryapi.h).
    ("CreateFileMappingW", "api-ms-win-core-memory-l1-1-0"),
    ("OpenFileMappingW", "api-ms-win-core-memory-l1-1-0"),
    ("OpenFileMappingA", "api-ms-win-core-memory-l1-1-0"),
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
    ("TerminateThread", "api-ms-win-core-processthreads-l1-1-0"),
    ("GetThreadContext", "api-ms-win-core-processthreads-l1-1-0"),
    ("SetThreadContext", "api-ms-win-core-processthreads-l1-1-0"),
    ("SuspendThread", "api-ms-win-core-processthreads-l1-1-0"),
    // specs/windows/kernel32.dll/CreateRemoteThread.yaml (Chrome makes threads in its children).
    ("CreateRemoteThread", "api-ms-win-core-processthreads-l1-1-0"),
    ("CreateRemoteThreadEx", "api-ms-win-core-processthreads-l1-1-0"),
    ("GetProcessIdOfThread", "api-ms-win-core-processthreads-l1-1-0"),
    // specs/windows/kernel32.dll/GetThreadIdealProcessorEx.yaml, GetThreadGroupAffinity.yaml.
    ("GetThreadIdealProcessorEx", "api-ms-win-core-processthreads-l1-1-2"),
    ("SetThreadIdealProcessorEx", "api-ms-win-core-processthreads-l1-1-2"),
    ("GetThreadGroupAffinity", "api-ms-win-core-processtopology-l1-1-0"),
    ("SetThreadGroupAffinity", "api-ms-win-core-processtopology-l1-1-0"),
    ("GetProcessTimes", "api-ms-win-core-processthreads-l1-1-0"),
    ("GetStartupInfoW", "api-ms-win-core-processthreads-l1-1-0"),
    ("IsProcessorFeaturePresent", "api-ms-win-core-processthreads-l1-1-1"),
    ("GetProcessAffinityMask", "api-ms-win-core-processtopology-obsolete-l1-1-0"),
    ("SetProcessAffinityMask", "api-ms-win-core-processtopology-obsolete-l1-1-0"),
    ("SetThreadAffinityMask", "api-ms-win-core-processtopology-obsolete-l1-1-0"),
    ("OpenEventW", "api-ms-win-core-synch-l1-1-0"),
    ("OpenMutexW", "api-ms-win-core-synch-l1-1-0"),
    ("RaiseException", "api-ms-win-core-errorhandling-l1-1-0"),
    ("SetUnhandledExceptionFilter", "api-ms-win-core-errorhandling-l1-1-0"),
    ("UnhandledExceptionFilter", "api-ms-win-core-errorhandling-l1-1-0"),
    ("InitializeSListHead", "api-ms-win-core-interlocked-l1-1-0"),
    ("InterlockedPushEntrySList", "api-ms-win-core-interlocked-l1-1-0"),
    ("InterlockedPopEntrySList", "api-ms-win-core-interlocked-l1-1-0"),
    ("InterlockedFlushSList", "api-ms-win-core-interlocked-l1-1-0"),
    ("QueryDepthSList", "api-ms-win-core-interlocked-l1-1-0"),
    ("GetModuleHandleA", "api-ms-win-core-libraryloader-l1-1-0"),
    // Chrome's start-up (ADR 0155).
    ("LoadLibraryExA", "api-ms-win-core-libraryloader-l1-1-0"),
    ("LoadLibraryA", "api-ms-win-core-libraryloader-l1-1-0"),
    ("OutputDebugStringA", "api-ms-win-core-debug-l1-1-0"),
    ("OutputDebugStringW", "api-ms-win-core-debug-l1-1-0"),
    ("GetUserDefaultLCID", "api-ms-win-core-localization-l1-2-0"),
    ("GetSystemDefaultLCID", "api-ms-win-core-localization-l1-2-0"),
    ("GetUserDefaultLangID", "api-ms-win-core-localization-l1-2-0"),
    ("GetSystemDefaultLangID", "api-ms-win-core-localization-l1-2-0"),
    ("GetThreadLocale", "api-ms-win-core-localization-l1-2-0"),
    ("SetThreadLocale", "api-ms-win-core-localization-l1-2-0"),
    ("GetUserDefaultUILanguage", "api-ms-win-core-localization-l1-2-0"),
    ("GetSystemDefaultUILanguage", "api-ms-win-core-localization-l1-2-0"),
    ("GetUserDefaultLocaleName", "api-ms-win-core-localization-l1-2-0"),
    ("GetSystemDefaultLocaleName", "api-ms-win-core-localization-l1-2-0"),
    ("GetThreadPreferredUILanguages", "api-ms-win-core-localization-l1-2-0"),
    ("IsValidLocale", "api-ms-win-core-localization-l1-2-0"),
    ("CompareStringA", "api-ms-win-core-string-l1-1-0"),
    ("EnumCalendarInfoW", "api-ms-win-core-localization-l1-2-0"),
    ("EnumCalendarInfoA", "api-ms-win-core-localization-l1-2-0"),
    ("EnumCalendarInfoExW", "api-ms-win-core-localization-l1-2-0"),
    ("GetCPInfoExW", "api-ms-win-core-localization-l1-2-0"),
    ("GetCPInfoExA", "api-ms-win-core-localization-l1-2-0"),
    ("GetCalendarInfoW", "api-ms-win-core-localization-l1-2-0"),
    ("GetLocaleInfoA", "api-ms-win-core-localization-l1-2-0"),
    ("GetStringTypeA", "api-ms-win-core-string-l1-1-0"),
    ("GetStringTypeExA", "api-ms-win-core-string-l1-1-0"),
    ("GetStringTypeExW", "api-ms-win-core-string-l1-1-0"),
    ("LCMapStringA", "api-ms-win-core-string-l1-1-0"),
    ("GetLocaleInfoW", "api-ms-win-core-localization-l1-2-0"),
    ("LCMapStringW", "api-ms-win-core-localization-l1-2-0"),
    ("LCMapStringEx", "api-ms-win-core-localization-l1-2-0"),
    ("VerSetConditionMask", "api-ms-win-core-sysinfo-l1-1-0"),
    ("VerifyVersionInfoW", "api-ms-win-core-sysinfo-l1-1-0"),
    ("IsWow64Process", "api-ms-win-core-wow64-l1-1-0"),
    ("GetProductInfo", "api-ms-win-core-sysinfo-l1-2-0"),
    ("AddVectoredExceptionHandler", "api-ms-win-core-errorhandling-l1-1-0"),
    ("RemoveVectoredExceptionHandler", "api-ms-win-core-errorhandling-l1-1-0"),
    ("AddVectoredContinueHandler", "api-ms-win-core-errorhandling-l1-1-0"),
    ("RemoveVectoredContinueHandler", "api-ms-win-core-errorhandling-l1-1-0"),
    ("GetStringTypeW", "api-ms-win-core-string-l1-1-0"),
    ("CompareStringW", "api-ms-win-core-string-l1-1-0"),
    ("CompareStringEx", "api-ms-win-core-string-l1-1-0"),
    ("GetLargePageMinimum", "api-ms-win-core-memory-l1-1-1"),
    ("GlobalMemoryStatusEx", "api-ms-win-core-sysinfo-l1-1-0"),
    ("DeviceIoControl", "api-ms-win-core-io-l1-1-0"),
    ("GetDiskFreeSpaceW", "api-ms-win-core-file-l1-1-0"),
    ("GetDiskFreeSpaceExW", "api-ms-win-core-file-l1-1-0"),
    // Download path (sheets of spec-chrome-dl, ADR 0223).
    ("GetVolumeInformationW", "api-ms-win-core-file-l1-1-0"),
    ("GetVolumePathNameW", "api-ms-win-core-file-l1-1-0"),
    ("PowerCreateRequest", "api-ms-win-core-kernel32-legacy-l1-1-0"),
    ("PowerSetRequest", "api-ms-win-core-kernel32-legacy-l1-1-0"),
    ("PowerClearRequest", "api-ms-win-core-kernel32-legacy-l1-1-0"),
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
    // Tokens (processthreadsapi.h, ADR 0171):
    // `advapi32` forwards the token functions to the same contract.
    ("OpenProcessToken", "api-ms-win-core-processthreads-l1-1-0"),
    ("OpenThreadToken", "api-ms-win-core-processthreads-l1-1-0"),
    ("SetThreadToken", "api-ms-win-core-processthreads-l1-1-0"),
    ("GetProcessMitigationPolicy", "api-ms-win-core-processthreads-l1-1-3"),
    ("SetProcessMitigationPolicy", "api-ms-win-core-processthreads-l1-1-3"),
    // ----- Chrome batch 2 (chrome-win-b2, ADR 0155). Files (fileapi.h,
    // winbase.h, minwinbase.h); the contracts are the ones the pages list.
    ("FindFirstFileExW", "api-ms-win-core-file-l1-1-0"),
    ("GetFileAttributesExW", "api-ms-win-core-file-l1-1-0"),
    ("GetFileInformationByHandleEx", "api-ms-win-core-file-l2-1-1"),
    ("SetFileInformationByHandle", "api-ms-win-core-file-l2-1-1"),
    ("LockFileEx", "api-ms-win-core-file-l1-1-0"),
    ("UnlockFileEx", "api-ms-win-core-file-l1-1-0"),
    ("ReplaceFileW", "api-ms-win-core-file-l1-2-0"),
    ("GetFinalPathNameByHandleW", "api-ms-win-core-file-l2-1-0"),
    ("GetLongPathNameW", "api-ms-win-core-file-l1-1-0"),
    ("CreateToolhelp32Snapshot", "api-ms-win-core-toolhelp-l1-1-0"),
    ("Process32FirstW", "api-ms-win-core-toolhelp-l1-1-0"),
    ("Process32NextW", "api-ms-win-core-toolhelp-l1-1-0"),
    ("GetShortPathNameW", "api-ms-win-core-file-l1-1-0"),
    // The ANSI forms of the environment, version and directory functions (impl-win-apps: the
    // programs of the app survey call them before anything else).
    ("GetEnvironmentVariableA", "api-ms-win-core-processenvironment-l1-1-0"),
    ("SetEnvironmentVariableA", "api-ms-win-core-processenvironment-l1-1-0"),
    ("GetVersionExA", "api-ms-win-core-sysinfo-l1-1-0"),
    ("GetSystemDirectoryA", "api-ms-win-core-sysinfo-l1-1-0"),
    ("GetWindowsDirectoryA", "api-ms-win-core-sysinfo-l1-1-0"),
    ("GetSystemWindowsDirectoryA", "api-ms-win-core-sysinfo-l1-1-0"),
    ("CreateNamedPipeA", "api-ms-win-core-namedpipe-l1-1-0"),
    ("FindResourceW", "api-ms-win-core-libraryloader-l1-1-0"),
    ("FindResourceExW", "api-ms-win-core-libraryloader-l1-1-0"),
    ("LoadResource", "api-ms-win-core-libraryloader-l1-1-0"),
    ("LockResource", "api-ms-win-core-libraryloader-l1-1-0"),
    ("SizeofResource", "api-ms-win-core-libraryloader-l1-1-0"),
    ("FreeResource", "api-ms-win-core-libraryloader-l1-1-0"),
    ("EnumResourceNamesW", "api-ms-win-core-libraryloader-l1-1-0"),
    ("EnumResourceTypesW", "api-ms-win-core-libraryloader-l1-1-0"),
    ("CopyFileW", "api-ms-win-core-file-l2-1-0"),
    ("ReadDirectoryChangesW", "api-ms-win-core-file-l2-1-0"),
    ("GetSystemDirectoryW", "api-ms-win-core-sysinfo-l1-1-0"),
    ("GetWindowsDirectoryW", "api-ms-win-core-sysinfo-l1-1-0"),
    ("GetSystemWindowsDirectoryW", "api-ms-win-core-sysinfo-l1-1-0"),
    // Loader and debugging (libloaderapi.h, werapi.h, processthreadsapi.h).
    ("GetModuleHandleExA", "api-ms-win-core-libraryloader-l1-1-0"),
    ("GetModuleHandleExW", "api-ms-win-core-libraryloader-l1-1-0"),
    ("WerRegisterRuntimeExceptionModule", "api-ms-win-core-windowserrorreporting-l1-1-0"),
    ("WerUnregisterRuntimeExceptionModule", "api-ms-win-core-windowserrorreporting-l1-1-0"),
    ("SetThreadDescription", "api-ms-win-core-processthreads-l1-1-3"),
    ("GetThreadDescription", "api-ms-win-core-processthreads-l1-1-3"),
    // Memory (memoryapi.h, heapapi.h). `VirtualAlloc2`, `MapViewOfFile3` and
    // `UnmapViewOfFile2` are `kernelbase` exports only (Learn lists no
    // `kernel32` export for them).
    ("DiscardVirtualMemory", "api-ms-win-core-memory-l1-1-3"),
    ("PrefetchVirtualMemory", "api-ms-win-core-memory-l1-1-4"),
    ("HeapSetInformation", "api-ms-win-core-heap-l2-1-0"),
    // Synchronisation (synchapi.h, threadpoollegacyapiset.h, processthreadsapi.h).
    ("CreateThreadpool", "api-ms-win-core-threadpool-l1-2-0"),
    ("CloseThreadpool", "api-ms-win-core-threadpool-l1-2-0"),
    ("SetThreadpoolThreadMaximum", "api-ms-win-core-threadpool-l1-2-0"),
    ("SetThreadpoolThreadMinimum", "api-ms-win-core-threadpool-l1-2-0"),
    ("SetThreadpoolStackInformation", "api-ms-win-core-threadpool-l1-2-0"),
    ("QueryThreadpoolStackInformation", "api-ms-win-core-threadpool-l1-2-0"),
    ("CreateThreadpoolCleanupGroup", "api-ms-win-core-threadpool-l1-2-0"),
    ("CloseThreadpoolCleanupGroup", "api-ms-win-core-threadpool-l1-2-0"),
    ("CloseThreadpoolCleanupGroupMembers", "api-ms-win-core-threadpool-l1-2-0"),
    ("CreateThreadpoolWork", "api-ms-win-core-threadpool-l1-2-0"),
    ("SubmitThreadpoolWork", "api-ms-win-core-threadpool-l1-2-0"),
    ("WaitForThreadpoolWorkCallbacks", "api-ms-win-core-threadpool-l1-2-0"),
    ("CloseThreadpoolWork", "api-ms-win-core-threadpool-l1-2-0"),
    ("CreateThreadpoolTimer", "api-ms-win-core-threadpool-l1-2-0"),
    ("SetThreadpoolTimer", "api-ms-win-core-threadpool-l1-2-0"),
    ("SetThreadpoolTimerEx", "api-ms-win-core-threadpool-l1-2-0"),
    ("IsThreadpoolTimerSet", "api-ms-win-core-threadpool-l1-2-0"),
    ("WaitForThreadpoolTimerCallbacks", "api-ms-win-core-threadpool-l1-2-0"),
    ("CloseThreadpoolTimer", "api-ms-win-core-threadpool-l1-2-0"),
    ("CreateThreadpoolWait", "api-ms-win-core-threadpool-l1-2-0"),
    ("SetThreadpoolWait", "api-ms-win-core-threadpool-l1-2-0"),
    ("SetThreadpoolWaitEx", "api-ms-win-core-threadpool-l1-2-0"),
    ("WaitForThreadpoolWaitCallbacks", "api-ms-win-core-threadpool-l1-2-0"),
    ("CloseThreadpoolWait", "api-ms-win-core-threadpool-l1-2-0"),
    ("CreateThreadpoolIo", "api-ms-win-core-threadpool-l1-2-0"),
    ("StartThreadpoolIo", "api-ms-win-core-threadpool-l1-2-0"),
    ("CancelThreadpoolIo", "api-ms-win-core-threadpool-l1-2-0"),
    ("WaitForThreadpoolIoCallbacks", "api-ms-win-core-threadpool-l1-2-0"),
    ("CloseThreadpoolIo", "api-ms-win-core-threadpool-l1-2-0"),
    ("TrySubmitThreadpoolCallback", "api-ms-win-core-threadpool-l1-2-0"),
    ("CallbackMayRunLong", "api-ms-win-core-threadpool-l1-2-0"),
    ("SetEventWhenCallbackReturns", "api-ms-win-core-threadpool-l1-2-0"),
    ("ReleaseSemaphoreWhenCallbackReturns", "api-ms-win-core-threadpool-l1-2-0"),
    ("ReleaseMutexWhenCallbackReturns", "api-ms-win-core-threadpool-l1-2-0"),
    ("LeaveCriticalSectionWhenCallbackReturns", "api-ms-win-core-threadpool-l1-2-0"),
    ("FreeLibraryWhenCallbackReturns", "api-ms-win-core-threadpool-l1-2-0"),
    ("DisassociateCurrentThreadFromCallback", "api-ms-win-core-threadpool-l1-2-0"),
    ("QueueUserWorkItem", "api-ms-win-core-threadpool-legacy-l1-1-0"),
    ("RegisterWaitForSingleObject", "api-ms-win-core-threadpool-legacy-l1-1-0"),
    ("UnregisterWait", "api-ms-win-core-threadpool-legacy-l1-1-0"),
    ("UnregisterWaitEx", "api-ms-win-core-threadpool-legacy-l1-1-0"),
    ("SwitchToThread", "api-ms-win-core-processthreads-l1-1-0"),
    // Locale, geography, time zones, system information (winnls.h,
    // timezoneapi.h, sysinfoapi.h, winbase.h, wow64apiset.h).
    ("GetLocaleInfoEx", "api-ms-win-core-localization-l1-2-0"),
    ("IsValidLocaleName", "api-ms-win-core-localization-l1-2-0"),
    // Group L of the Chrome installer's batch B0: locale names, enumerations, date and time formats.
    ("EnumSystemLocalesEx", "api-ms-win-core-localization-l1-2-0"),
    ("EnumSystemLocalesW", "api-ms-win-core-localization-l1-2-0"),
    ("GetDateFormatEx", "api-ms-win-core-localization-l1-2-0"),
    ("GetDateFormatW", "api-ms-win-core-localization-l1-2-0"),
    ("GetTimeFormatEx", "api-ms-win-core-localization-l1-2-0"),
    ("GetTimeFormatW", "api-ms-win-core-localization-l1-2-0"),
    ("LCIDToLocaleName", "api-ms-win-core-localization-l1-2-0"),
    ("LocaleNameToLCID", "api-ms-win-core-localization-l1-2-0"),
    ("GetUserPreferredUILanguages", "api-ms-win-core-localization-l1-2-0"),
    ("GetUserGeoID", "api-ms-win-core-localization-l1-2-0"),
    ("GetGeoInfoW", "api-ms-win-core-localization-l1-2-0"),
    ("GetTimeZoneInformation", "api-ms-win-core-timezone-l1-1-0"),
    ("GetDynamicTimeZoneInformation", "api-ms-win-core-timezone-l1-1-0"),
    ("SystemTimeToTzSpecificLocalTime", "api-ms-win-core-timezone-l1-1-0"),
    ("TzSpecificLocalTimeToSystemTime", "api-ms-win-core-timezone-l1-1-0"),
    ("SystemTimeToTzSpecificLocalTimeEx", "api-ms-win-core-timezone-l1-1-0"),
    ("TzSpecificLocalTimeToSystemTimeEx", "api-ms-win-core-timezone-l1-1-0"),
    ("GetLogicalProcessorInformation", "api-ms-win-core-sysinfo-l1-1-0"),
    ("GetLogicalProcessorInformationEx", "api-ms-win-core-sysinfo-l1-2-0"),
    ("GetComputerNameExW", "api-ms-win-core-sysinfo-l1-1-0"),
    ("GetComputerNameW", "api-ms-win-core-sysinfo-l1-1-0"),
    ("IsWow64Process2", "api-ms-win-core-wow64-l1-1-1"),
    // ADR 0186, stage 2 (wow64apiset.h).
    ("Wow64DisableWow64FsRedirection", "api-ms-win-core-wow64-l1-1-0"),
    ("Wow64RevertWow64FsRedirection", "api-ms-win-core-wow64-l1-1-0"),
    ("Wow64EnableWow64FsRedirection", "api-ms-win-core-wow64-l1-1-0"),
    ("GetSystemPowerStatus", "api-ms-win-core-sysinfo-l1-1-0"),
    // ADR 0155, step 9: what Chrome's browser needs to get past its start (Learn; the
    // contracts are the ones the functions' headers belong to, to be confirmed).
    ("QueryProcessCycleTime", "api-ms-win-core-realtime-l1-1-0"),
    // `QueryUnbiasedInterruptTime` is kernel32's too (its pair, `QueryUnbiasedInterruptTimePrecise`,
    // is `kernelbase`'s alone: the sheet).
    ("QueryUnbiasedInterruptTime", "api-ms-win-core-realtime-l1-1-0"),
    ("GetProcessInformation", "api-ms-win-core-processthreads-l1-1-3"),
    ("SetProcessInformation", "api-ms-win-core-processthreads-l1-1-3"),
    ("SetProcessShutdownParameters", "api-ms-win-core-processthreads-l1-1-1"),
    ("GetProcessShutdownParameters", "api-ms-win-core-processthreads-l1-1-1"),
    ("K32GetMappedFileNameW", "api-ms-win-core-psapi-l1-1-0"),
    // impl-updater-gap: the first call Google's ChromeSetup.exe makes that Poly lacked (the contract is
    // psapi's, as Poly reads Learn; to be confirmed).
    ("QueryFullProcessImageNameW", "api-ms-win-core-psapi-l1-1-0"),
    ("ProcessIdToSessionId", "api-ms-win-core-processthreads-l1-1-0"),
    // Chrome stage 2 (impl-chrome-s2; contracts from Learn's "API set" rows).
    ("SetErrorMode", "api-ms-win-core-errorhandling-l1-1-0"),
    ("GetErrorMode", "api-ms-win-core-errorhandling-l1-1-0"),
    ("SetThreadErrorMode", "api-ms-win-core-errorhandling-l1-1-0"),
    ("GetThreadErrorMode", "api-ms-win-core-errorhandling-l1-1-0"),
    ("GetCurrentProcessorNumber", "api-ms-win-core-processthreads-l1-1-0"),
    ("GetCurrentProcessorNumberEx", "api-ms-win-core-processthreads-l1-1-1"),
    ("GetProcessHandleCount", "api-ms-win-core-processthreads-l1-1-0"),
    // ADR 0196: the page of `GetAppContainerNamedObjectPath` names kernel32.dll and no API set (the contract is a guess).
    ("GetAppContainerNamedObjectPath", "api-ms-win-core-processthreads-l1-1-0"),
    // Chrome S3 hunt: scheduling settings and thread times (the browser, its GPU process and
    // crash handler call them all the time; Learn's "API set" rows).
    ("SetThreadPriority", "api-ms-win-core-processthreads-l1-1-0"),
    ("GetThreadPriority", "api-ms-win-core-processthreads-l1-1-0"),
    ("SetThreadPriorityBoost", "api-ms-win-core-processthreads-l1-1-0"),
    ("GetThreadPriorityBoost", "api-ms-win-core-processthreads-l1-1-0"),
    ("SetProcessPriorityBoost", "api-ms-win-core-processthreads-l1-1-2"),
    ("GetProcessPriorityBoost", "api-ms-win-core-processthreads-l1-1-2"),
    ("SetPriorityClass", "api-ms-win-core-processthreads-l1-1-0"),
    ("GetPriorityClass", "api-ms-win-core-processthreads-l1-1-0"),
    ("SetThreadInformation", "api-ms-win-core-processthreads-l1-1-3"),
    ("GetThreadTimes", "api-ms-win-core-processthreads-l1-1-0"),
    ("QueryThreadCycleTime", "api-ms-win-core-realtime-l1-1-0"),
    ("K32GetProcessMemoryInfo", "api-ms-win-core-psapi-l1-1-0"),
    ("K32GetModuleInformation", "api-ms-win-core-psapi-l1-1-0"),
    ("K32EnumProcessModules", "api-ms-win-core-psapi-l1-1-0"),
    // Batch B0 of the Chrome installers (docs/windows/chrome-installer-plan.md; Learn's "API set"
    // rows, the session one a guess: its page names kernel32.dll only).
    ("GetDriveTypeW", "api-ms-win-core-file-l1-1-0"),
    ("GetVolumeInformationByHandleW", "api-ms-win-core-file-l1-1-0"),
    ("GetTempPath2W", "api-ms-win-core-file-l1-2-4"),
    ("SetProcessWorkingSetSize", "api-ms-win-core-memory-l1-1-1"),
    ("SetProcessWorkingSetSizeEx", "api-ms-win-core-memory-l1-1-1"),
    ("GetProcessWorkingSetSize", "api-ms-win-core-memory-l1-1-1"),
    ("GetProcessWorkingSetSizeEx", "api-ms-win-core-memory-l1-1-1"),
    ("K32QueryWorkingSetEx", "api-ms-win-core-psapi-l1-1-0"),
    ("WTSGetActiveConsoleSessionId", "api-ms-win-core-kernel32-legacy-l1-1-0"),
];

/// `(function, contract)` of the `kernel32` exports that forward to our
/// `ntdll` (ADR 0087): their pages' "APILocation" metadata names
/// `api-ms-win-core-rtlsupport-*` and `ntdll.dll`, which Poly's API-set
/// table gives to `ntdll` (ADR 0071).
pub const KERNEL32_NTDLL_FORWARDS: &[(&str, &str)] = &[
    ("RtlCaptureContext", "api-ms-win-core-rtlsupport-l1-1-0"),
    // kernel32 forwards it to ntdll, which captures the stack (ADR 0155).
    ("RtlCaptureStackBackTrace", "api-ms-win-core-rtlsupport-l1-1-0"),
    ("RtlLookupFunctionEntry", "api-ms-win-core-rtlsupport-l1-1-0"),
    ("RtlVirtualUnwind", "api-ms-win-core-rtlsupport-l1-1-0"),
    ("RtlUnwindEx", "api-ms-win-core-rtlsupport-l1-1-0"),
    ("RtlPcToFileHeader", "api-ms-win-core-rtlsupport-l1-1-0"),
    // Function tables of code made at run time (Learn: "RtlAddFunctionTable", kernel32.dll;
    // ADR 0155 step 11).
    ("RtlAddFunctionTable", "api-ms-win-core-rtlsupport-l1-1-0"),
    ("RtlDeleteFunctionTable", "api-ms-win-core-rtlsupport-l1-1-0"),
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
    // ADR 0155, step 9: what chrome.dll delay-loads by ordinal (the pairs are the export table's).
    ("SafeArrayDestroy", 16),
    ("SafeArrayGetDim", 17),
    ("SafeArrayGetUBound", 19),
    ("SafeArrayGetLBound", 20),
    ("SafeArrayAccessData", 23),
    ("SafeArrayUnaccessData", 24),
    ("SafeArrayGetElement", 25),
    ("SafeArrayPutElement", 26),
    ("SafeArrayGetVartype", 77),
    ("SysStringByteLen", 149),
    ("SysAllocStringByteLen", 150),
    ("LoadTypeLib", 161),
    ("LoadRegTypeLib", 162),
    // Batch B0 of the Chrome installers (docs/windows/chrome-installer-plan.md).
    ("SystemTimeToVariantTime", 184),
    ("VarBstrCmp", 314),
    ("SafeArrayCreateVector", 411),
    ("OleLoadPicturePath", 424),
    // ADR 0220 (B2b): type libraries, error information, the automation marshaller.
    ("DllGetClassObject", 1),
    ("VariantCopyInd", 11),
    ("VariantChangeType", 12),
    ("SafeArrayCreate", 15),
    ("SafeArrayCopy", 27),
    ("DispCallFunc", 146),
    ("VariantChangeTypeEx", 147),
    ("RegisterTypeLib", 163),
    ("QueryPathOfRegTypeLib", 164),
    ("CreateTypeLib2", 180),
    ("LoadTypeLibEx", 183),
    ("UnRegisterTypeLib", 186),
    ("GetErrorInfo", 200),
    ("SetErrorInfo", 201),
    ("CreateErrorInfo", 202),
];

/// The exports of our `user32.dll` (ADR 0087): two string functions until
/// windows exist (Phase 6).
pub const USER32_EXPORTS: &[&str] = &["CharUpperW", "CharPrevExA"];

/// The functions `kernel32.dll` implements itself: the global-memory
/// functions and (second slice) the `lstr*` string functions, whose Learn
/// pages name no API set.
pub const KERNEL32_OWN: &[&str] = &[
    // The global atom table (Learn names no API set for these pages).
    "GlobalAddAtomW",
    "GlobalFindAtomW",
    "GlobalDeleteAtom",
    "GlobalGetAtomNameW",
    "GlobalAlloc",
    "GlobalFree",
    "GlobalSize",
    "GlobalLock",
    "GlobalUnlock",
    "lstrlenW",
    "lstrlenA",
    "lstrcmpW",
    "lstrcmpiW",
    // ADR 0089: `kernelbase` exports no `GetStartupInfoA` (the coverage
    // map's export lists), so `kernel32` converts `GetStartupInfoW`'s.
    "GetStartupInfoA",
    // ADR 0114: 7-Zip asks for them with `GetProcAddress`; their pages
    // (winbase.h) name no API set.
    "GetActiveProcessorGroupCount",
    "GetActiveProcessorCount",
    // Chrome S2c: Chrome's sandboxed children take `CreateThread`'s address from
    // kernel32's export table and run the bytes there, so it is code in kernel32 (a forwarder's
    // text is no instructions): the sheet `kernel32.dll/CreateThread`, rule `export_code`.
    "CreateThread",
    // Chrome batch 2: the ANSI comparisons (winbase.h) name no API set.
    "lstrcmpA",
    "lstrcmpiA",
];

/// Every function `kernelbase.dll` must export: each one `kernel32` or
/// `advapi32` forwards ([`crate::advapi`], ADR 0083).
pub fn kernelbase_exports() -> impl Iterator<Item = &'static str> {
    let advapi = crate::advapi::ADVAPI32_FORWARDS
        .iter()
        .filter(|&&(name, _)| !KERNEL32_FORWARDS.iter().any(|&(k, _)| k == name));
    KERNEL32_FORWARDS.iter().chain(advapi).map(|&(name, _)| name)
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

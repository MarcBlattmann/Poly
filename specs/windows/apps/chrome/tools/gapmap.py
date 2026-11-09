#!/usr/bin/env python3
"""Join the stable facts of one stage (chrometrace.py merge) with Poly's coverage map and group them
by DLL and area.

  gapmap.py table <merged.json> [type ...]    -> tab separated rows: type dll fn area state detail n_min n_max ok/fail errs rank
  gapmap.py summary <merged.json>...          -> counts per state per process type
"""
import json, os, re, sys, collections
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import covmap

DLL_AREA = {
    'ws2_32.dll': 'network', 'winhttp.dll': 'network', 'wininet.dll': 'network', 'iphlpapi.dll': 'network',
    'netapi32.dll': 'network', 'dhcpcsvc.dll': 'network', 'ndfapi.dll': 'network', 'urlmon.dll': 'network',
    'crypt32.dll': 'crypto', 'bcrypt.dll': 'crypto', 'bcryptprimitives.dll': 'crypto', 'ncrypt.dll': 'crypto',
    'wintrust.dll': 'crypto', 'cryptui.dll': 'crypto', 'secur32.dll': 'crypto', 'tbs.dll': 'crypto', 'credui.dll': 'crypto',
    'user32.dll': 'win32k: user32', 'gdi32.dll': 'win32k: gdi32', 'imm32.dll': 'win32k: input method',
    'uxtheme.dll': 'win32k: theme', 'dwmapi.dll': 'win32k: dwm', 'comctl32.dll': 'win32k: common controls',
    'comdlg32.dll': 'win32k: common dialogs', 'oleacc.dll': 'accessibility', 'uiautomationcore.dll': 'accessibility',
    'dwrite.dll': 'DirectWrite / fonts', 'ole32.dll': 'COM', 'oleaut32.dll': 'COM', 'propsys.dll': 'COM',
    'shell32.dll': 'shell', 'shlwapi.dll': 'shell', 'userenv.dll': 'shell', 'wtsapi32.dll': 'shell',
    'shcore.dll': 'shell', 'winspool.drv': 'shell: printing',
    'setupapi.dll': 'devices', 'cfgmgr32.dll': 'devices', 'hid.dll': 'devices', 'winusb.dll': 'devices', 'bthprops.cpl': 'devices',
    'mf.dll': 'media', 'mfplat.dll': 'media', 'mfreadwrite.dll': 'media', 'mmdevapi.dll': 'media', 'winmm.dll': 'media',
    'd3d11.dll': 'GPU / DXGI', 'd3d12.dll': 'GPU / DXGI', 'dxgi.dll': 'GPU / DXGI', 'dcomp.dll': 'GPU / DXGI',
    'd3dcompiler_47.dll': 'GPU / DXGI',
    'dbghelp.dll': 'diagnostics', 'wevtapi.dll': 'diagnostics', 'esent.dll': 'diagnostics', 'pdh.dll': 'diagnostics',
    'version.dll': 'misc: version',
}
RULES = [
    (r'^(Reg[A-Z]|Nt(Create|Open|Delete|Query|Set)(Key|ValueKey|KeyEx)|NtQueryValueKey|NtSetValueKey|RtlFormatCurrentUserKeyPath)', 'registry'),
    (r'(Token|Sid\b|Sid[A-Z]|Acl|Ace\b|SecurityDescriptor|SecurityInfo|Privilege|Impersonat|RevertToSelf|LogonUser|LookupAccount|MapGenericMask|AccessCheck|Trustee|ExplicitAccess|^Lsa|AllocateLocallyUniqueId|^IsValid(Acl|Sid|SecurityDescriptor))', 'security: tokens / ACLs / SIDs'),
    (r'(JobObject)', 'security: jobs'),
    (r'(MitigationPolicy|SetProcessInformation|GetProcessInformation|SetThreadInformation|SetProcessShutdownParameters|SetErrorMode|SetDefaultDllDirectories|SetDllDirectory|AddDllDirectory)', 'security: mitigations / process policy'),
    (r'(NamedPipe|IoCompletionPort|QueuedCompletionStatus|CancelIo|GetOverlappedResult|ReadDirectoryChanges|FileCompletionNotification|DuplicateHandle|ProcThreadAttribute|CreateProcess|CreateRemoteThread|OpenProcess|OpenThread|CreateFileMapping|MapViewOfFile|UnmapViewOfFile|FlushViewOfFile|GetHandleInformation|SetHandleInformation|GetExitCodeProcess|TerminateProcess|GetProcessId|GetThreadId|ProcessIdToSession|GetProcessHandleCount|CreateToolhelp32Snapshot|Process32|Module32|QueryFullProcessImageName|^K32|ReadProcessMemory|WriteProcessMemory|NtQueryInformationProcess|NtQueryObject|NtQuerySystemInformation|NtClose|NtWriteFile|TransactNamedPipe)', 'process / thread / IPC'),
    (r'(Critical|SRWLock|ConditionVariable|InitOnce|WaitFor|^Sleep|CreateEvent|SetEvent|ResetEvent|CreateMutex|ReleaseMutex|CreateSemaphore|ReleaseSemaphore|Waitable|^Tls|^Fls|RegisterWait|UnregisterWait|SwitchToThread|Interlocked|WaitOn|MsgWait|PowerCreateRequest|PowerSetRequest|PowerClearRequest|SetThreadExecutionState|InitializeSListHead)', 'sync / TLS'),
    (r'(CreateThread|ExitThread|ResumeThread|SuspendThread|SetThread|GetThread|GetCurrentThread|GetCurrentProcess|GetPriorityClass|SetPriorityClass|QueryThreadCycleTime|QueryProcessCycleTime|GetProcessTimes|GetProcessAffinity|FreeLibraryAndExitThread|GetCurrentProcessorNumber)', 'process / thread / IPC'),
    (r'(Virtual|Heap|Global|Local(Alloc|Free)|Discard|Prefetch)', 'memory'),
    (r'(LoadLibrary|FreeLibrary|GetProcAddress|GetModule|LoadResource|FindResource|LockResource|SizeofResource|EnumResource|RtlPcToFileHeader|RtlLookupFunctionEntry|RtlVirtualUnwind|RtlCaptureContext|RtlCaptureStackBackTrace|RtlAddFunctionTable|RtlDeleteFunctionTable|RtlUnwind|VectoredExceptionHandler|SetUnhandledExceptionFilter|UnhandledExceptionFilter|RaiseException|DebugBreak|IsDebugger|CheckRemoteDebugger|OutputDebugString|WerRegister|EncodePointer|GetLastError|SetLastError|ExitProcess|RtlGetLastNtStatus|RtlNtStatusToDosError)', 'loader / exceptions / debug'),
    (r'(File|Directory|Path|Volume|Drive|DosDevice|Disk|ReplaceFile|MoveFile|CopyFile|DeleteFile|LockFile|UnlockFile|FlushFileBuffers|ReadFile|WriteFile|SetEndOfFile|FindClose|FindNext|FindFirst|GetTempPath|GetTempFileName|GetSystemDirectory|GetWindowsDirectory|GetFinalPathName|GetShortPathName|GetLongPathName|GetFullPathName|SetCurrentDirectory|GetCurrentDirectory|GetPrivateProfile|DeviceIoControl|HardLink|NtWriteFile)', 'file system'),
    (r'(Console|StdHandle|AllocConsole|AttachConsole|FreeConsole|GetCommandLine|Environment|ExpandEnvironment|GetStartupInfo)', 'console / process environment'),
    (r'(Locale|LCMap|CompareString|MultiByteToWideChar|WideCharToMultiByte|GetACP|GetOEMCP|GetCPInfo|IsValid(CodePage|Locale)|Geo|Preferred.*Language|FoldString|GetStringType|GetNumberFormat|GetCurrencyFormat|GetDateFormat|GetTimeFormat|lstr|CharUpper|CharLower|UILanguage|DefaultLang|GetUserDefault|GetSystemDefault|ResolveLocaleName|EnumSystemLocales|FormatMessage)', 'NLS / text'),
    (r'(Time|Tick|PerformanceCounter|PerformanceFrequency|TzSpecific|TimeZone)', 'time'),
    (r'(SystemInfo|ProcessorInformation|ProcessorCount|ProcessorGroup|MaximumProcessor|GlobalMemoryStatus|ComputerName|GetVersion|VerifyVersion|VerSetCondition|GetProductInfo|ProcessorFeature|FirmwareType|SystemPowerStatus|GetPerformanceInfo|Package|IsWow64|IsThreadAFiber|ProcessMemoryInfo|WorkingSet)', 'system information'),
]


def area(dll, fn):
    for pat, name in RULES:
        if re.search(pat, fn):
            return name
    return DLL_AREA.get(dll, 'misc ({})'.format(dll))


def rows(merged, types=None):
    out = []
    for t, v in merged['types'].items():
        if types and t not in types:
            continue
        for key, f in v['stable'].items():
            dll, fn = key.split('!', 1)
            st, det = covmap.state(dll, fn)
            out.append({
                'type': t, 'dll': dll, 'fn': fn, 'area': area(dll, fn), 'state': st, 'detail': det,
                'n_min': f['n_min'], 'n_max': f['n_max'], 'zero_always': f['zero_always'], 'ok_never': f['ok_never'],
                'errs': f['errs'], 'rank': f['rank'],
            })
    return out


if __name__ == '__main__':
    cmd = sys.argv[1]
    if cmd == 'table':
        m = json.load(open(sys.argv[2]))
        for r in rows(m, sys.argv[3:] or None):
            print('\t'.join(str(r[k]) for k in ('type', 'dll', 'fn', 'area', 'state', 'detail', 'n_min', 'n_max', 'zero_always', 'ok_never', 'errs', 'rank')))
    elif cmd == 'summary':
        for fpath in sys.argv[2:]:
            m = json.load(open(fpath))
            c = collections.defaultdict(collections.Counter)
            for r in rows(m):
                c[r['type']][r['state']] += 1
            print(fpath)
            for t, cc in sorted(c.items()):
                print('  ', t, dict(cc))

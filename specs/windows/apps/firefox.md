# Firefox 160 headless on Poly: the next calls (observation note)

Observed by spec agent `spec-win-apps-6`, 2026-11-16. `firefox.exe -headless about:blank` (the unpacked installer of Mozilla's current 64-bit
release) on Poly (main plus the merge of `impl-win-apps-4`), `cargo xtask win-app run --mode console --kernel "poly.nt.trail"`; Windows
behaviour comes from test programs of our own. Nothing of Firefox or Windows was read or disassembled.

## Where it stops

The browser prints `*** You are running in headless mode.` and ends with a breakpoint (`0x80000003`; the launcher reports exit code 1) about
3.3 s after start. Before that it sets up its profile (all file calls succeed: `parent.lock`, `compatibility.ini`, `crashes\events`, the update
directory), then calls **`kernel32!CreateWaitableTimerExW`** twice (a stub on Poly that fails; the two stub lines come right before the breakpoint)
and the process fails fast. That the failed timer causes the breakpoint is the reading of the order of events, not proof. So the next call to implement is the waitable timer family (`CreateWaitableTimerExW`, `SetWaitableTimer`,
`SetWaitableTimerEx`, `CancelWaitableTimer`, `OpenWaitableTimerW`): the `CreateWaitableTimerExW` sheet has the flags, the access rules and
every observable behaviour (signal, auto and manual reset, period, completion routine, names).

## The other stubs on the way (each has a sheet and tests)

| Stub | Sheet | Windows answer in short |
|---|---|---|
| `kernel32!QueryFullProcessImageNameW` | `QueryFullProcessImageNameW` | Win32 or native path of a process; size includes the NUL |
| `kernel32!GetVolumePathNameW` | `GetVolumePathNameW` | volume root of a path, also for missing paths |
| `kernel32!SetThreadStackGuarantee` | `SetThreadStackGuarantee` | per thread, rounded up to 4096, only grows |
| `kernel32!GetCurrentPackageId`, `GetCurrentPackageFamilyName` | `GetCurrentPackageId` | 15700 (no package) |
| `kernel32!FlushInstructionCache`, `kernelbase!FlushInstructionCache` | `FlushInstructionCache` | always TRUE |
| `kernel32!SetDllDirectoryW` | `SetDllDirectoryW` | stores the text; affects `LoadLibraryW` |
| `kernel32!SetProcessDEPPolicy` | `SetProcessDEPPolicy` | FALSE, 50 in a 64-bit process |
| `kernel32!K32GetPerformanceInfo` | `K32GetPerformanceInfo` | the 104-byte record |
| `ntdll!RtlDuplicateUnicodeString`, `ntdll!RtlQueryPerformanceCounter` | the two sheets | flags 1 and 3, counter = QueryPerformanceCounter |
| `ole32!CoIncrementMTAUsage` | `CoIncrementMTAUsage` | implicit MTA for threads that never initialised COM |
| `rpcrt4!UuidCreate` | `UuidCreate` | version 4 random UUIDs |
| `advapi32!RegisterEventSourceW`, `ReportEventW`, `DeregisterEventSource` | `RegisterEventSourceW` | handle for any name; 6 for bad handles |
| `kernel32!GetTempPath2W` (a failed `GetProcAddress`) | `GetTempPath2W` | the same text as `GetTempPathW` |

Also seen, harmless on Poly so far: `NtDeviceIoControlFile` returning `STATUS_INVALID_DEVICE_REQUEST` for three handles just before the stubs and `ws2_32!WSASendMsg` bound to a stub.

## Not covered

What Firefox does after the timer calls succeed (not reachable until the timers exist); the content, GPU and RDD child processes; the sandbox
and `CreateProcessAsUser` paths; DirectComposition and WebRender initialisation.

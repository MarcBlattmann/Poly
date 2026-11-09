# spec-chrome-win: the map from Poly to Chrome for Windows (sandbox on)

Status: **PARTIAL, second version** (spec agents `spec-chrome-win` and `spec-chrome-win-2`; the sandboxed renderers are traced since the second one, section 4a; per-API fault tolerance of S1 is section 6a). S1, S2 (broker side), S3, S4 and S5 are recorded
twice on real Windows and reduced to stable facts; the open points are listed at the end (sandboxed renderers cannot be
traced; S5 only as "default GPU flags, headless"; no fault injection per API).
Branch `spec-chrome-win` (from `fa782a20`), pushed to origin. Nothing of Chrome is committed (only observations).

## 1. What Chrome needs, in one page

* **The pinned app**: Chrome for Testing 157.0.8103.38 win64, zip SHA-256 `ae0deef2...0586` (README of
  `specs/windows/apps/chrome/` has the URL and the layout). `chrome.exe` is a 4 MB launcher; `chrome.dll` is 311 MB and has
  all the code. **Windows Chrome is one program started in many roles**: a headless `--dump-dom` run is 15 to 20 processes
  (browser, 5 to 8 renderers for the page, the extensions and the top-chrome WebUI, GPU process, network service, storage
  service, processor-metrics utility, one or two crashpad handlers) and 150 to 250 threads. Poly has to run all of them.
* **Static import surface**: `chrome.exe` + `chrome.dll` + `chrome_elf.dll` import 1,340 functions from 63 system DLLs:
  only 534 + 248 + 209 are static, **778 + 118 + 61 are delay-loaded**. What a run actually calls through those imports is
  **578 distinct APIs over all stages (457 in S1)**. Another 38 names are resolved with `GetProcAddress` and used through
  pointers (section 5).
* **Against Poly's coverage map** (`coverage/windows/api`, at `fa782a20`): of the 578 APIs 27 are green, 239 claimed, 309
  missing and 3 not listed. In S1 alone: 27 green, 208 claimed, 220 missing, 2 not listed. **"claimed" = the map has a
  claim on the entry** (an implementer has done or is doing it; "in tree" means the name also appears in Poly's source of
  that DLL); only **green** means spec-tested. Nothing "claimed" was run against Chrome: Poly does not start Chrome yet.
* **Every delay-loaded DLL Chrome calls must be loadable and must export the called names** (ablation, section 6): with the
  proxy of any of 25 such DLLs removed, S1 crashes (`0xC06D007E`, or a `DebugBreak` of Chrome's failed delay-load); only
  `dhcpcsvc`, `pdh` and `setupapi` could be absent. So the **first, mechanical batch is an export surface**: every DLL and
  every name in the tables below exists, many as stubs.
* **S1 to S5 differ little in *which* APIs** (S2 adds 54, S3 62, S4 3, S5 2 over the stage before); they differ in *how
  correctly* the same APIs behave: S2 needs the sandbox's token, job, desktop and mitigation semantics, S3 needs a window
  manager with Chrome's message sequence (section 7) and fonts, S4 needs sockets that work.

## 2. How it was measured (and what it cannot see)

The README of `specs/windows/apps/chrome/` is the long form. Short:

* `cargo xtask win trace-kit chrome.exe chrome.dll chrome_elf.dll --agg` (new in this branch: several programs, one set of
  proxies; `tools/wintrace/polytrace-agg` counts instead of writing every call; the 7-Zip kit and its committed
  reference are untouched: `tools/wintrace/polytrace` has the same bytes as before, checked by building both).
* Each stage twice, per process (the type from `--type=`), **only what both runs share is kept**: a function called in both
  runs, its minimum and maximum count, whether it ever returned 0/NULL/-1, the last errors it set, the names it was
  given. 8 to 9 process types; the committed traces are `specs/windows/apps/chrome/traces/*.trace.json`.
* Machines: S1, S2, S4 (headless), S5 on the owner's PC (Windows 11 Enterprise 25H2, build 26200.9711); S3 and S4 (window)
  on **homeserver2** (Windows 11 Home 25H2, build 26200.9711), interactive session, `SendInput`, screenshot, `WM_CLOSE`. The PC
  was locked for the first hours and then in use, so its desktop could not be used for the windows.
* **Not seen**: calls through pointers (names are known, counts are not), instructions Chrome executes itself
  (`syscall`, CPUID), the work Windows' own DLLs do under a call. The tracer logs arguments at the call (output buffers
  are not read), so what a call returned in memory (a token, a path) is not recorded.
* **The sandbox and the tracer**: the first version's confined renderers died at start (event 3033 of the Code Integrity log: the
  creation-time "Microsoft-signed only" mitigation refuses the unsigned proxy DLLs). `spec-chrome-win-2` fixed that (section 4a): the
  traced browser clears that mitigation bit for its children (`POLYTRACE_RELAX`) and hands each confined child its trace as an
  inherited section (`POLYTRACE_INHERIT`). The S2 trace of this directory (`traces/s2-sandboxed.trace.json`) therefore has every
  process type; the older `traces/s2.trace.json` (no renderer) is kept as it was recorded. The scenario results of S2 are unchanged.

## 3. The stages, observed

| Stage | What | Result on Windows |
|---|---|---|
| S1 | `--headless --no-sandbox --disable-gpu --dump-dom` | 440 bytes of DOM (SHA-256 `7b79f8b4...813b`), exit 0, 3 to 5 s, traced or not |
| S2 | the same, sandbox on | identical stdout and exit 0 untraced (2.3 s); **traced: renderers die, the run stalls** (see above) |
| S3 | visible window, `--disable-gpu --no-first-run`, local page, mouse + keys, `WM_CLOSE` | title `poly-s3 clicks=1 keys=poly moved=1 - Google Chrome for Testing`, text drawn (screenshot `s3-window.png`), exit 0, 11 s; traced the same |
| S4 | window: `http://example.com/` then `https://www.iana.org/`; headless: both in frames | `Example Domain`, then `Internet Assigned Numbers Authority`; DOM `http=1 https=1` |
| S5 | default GPU flags: headless, and a window (traced, `--no-sandbox`) | headless: S1's DOM, the GPU process makes 23 more distinct calls; window: the same S3 result. With `--disable-gpu` (S1-S4) the GPU process still calls `D3D11CreateDevice`, `DCompositionCreateDevice`, `DwmGetCompositionTimingInfo`; with the GPU on it adds display queries instead (`QueryDisplayConfig`, `GetDisplayConfigBufferSizes`, `DisplayConfigGetDeviceInfo`, `GetMonitorInfoW`, `MonitorFromWindow`, `GetDC`, `WindowFromDC`). The actual D3D11/DXGI work happens in `libGLESv2.dll`/`libEGL.dll` (ANGLE), `D3DCompiler_47.dll`, `dxcompiler.dll`, `vk_swiftshader.dll`: they import only `kernel32`/`advapi32`/`user32` and load `d3d11.dll`, `dxgi.dll` by name, and are **not in this kit** (their calls into D3D are untraced) |

Process census, `processes per run (run 1 + run 2) / stable distinct APIs`:

| Process type | s1 | s2 | s3 | s4 (headless) | s4 (window) | s5 | s5 (window) |
|---|---|---|---|---|---|---|---|
| browser | 1+1 / 413 | 1+1 / 437 | 1+1 / 485 | 1+1 / 415 | 1+1 / 490 | 1+1 / 413 | 1+1 / 484 |
| renderer | 4+4 / 159 | - | 5+5 / 168 | 6+6 / 173 | 8+7 / 163 | 4+4 / 158 | 5+5 / 156 |
| renderer (extension) | 2+2 / 152 | - | 2+2 / 144 | 2+2 / 149 | 2+2 / 144 | 2+2 / 149 | 2+2 / 144 |
| renderer (top-chrome WebUI) | 1+1 / 161 | - | 1+1 / 160 | 1+1 / 164 | 1+1 / 160 | 1+1 / 164 | 1+1 / 160 |
| GPU process | 1+1 / 159 | 1+1 / 164 | 1+1 / 178 | 1+1 / 161 | 1+1 / 177 | 1+1 / 182 | 1+1 / 184 |
| network service | 1+1 / 214 | 1+1 / 214 | 1+1 / 213 | 1+1 / 213 | 1+1 / 212 | 1+1 / 213 | 1+1 / 211 |
| storage service | 1+1 / 133 | - | 1+1 / 131 | 1+1 / 135 | 1+1 / 131 | 1+1 / 133 | 1+1 / 131 |
| processor-metrics utility | - | 1+1 / 136 | 1+1 / 131 | - | 1+1 / 131 | - | 1+1 / 131 |
| crashpad handler | 1+2 / 118 | 2+1 / 103 | 1+2 / 117 | 1+2 / 118 | 2+1 / 117 | 2+1 / 118 | 2+1 / 117 |

Run to run, the processes differ only in number (renderers, crashpad handlers) and in a handful of calls (`unstable`
in the traces); everything else above is identical in both runs.

## 4. What the sandbox does (S2, the broker, observed in the browser process)

Chrome's browser process is the sandbox's broker; everything it does to create a confined child is a call through
`kernel32`/`advapi32`/`user32` and was traced. For each confined child (GPU process, network service, storage service, every
renderer; five were created in the S2 runs: the renderers never became usable) it does, in this order (counts are the browser's calls in one S2 run, equal in both runs; the children that died were started again, so 12 `CreateProcessAsUserW` are about 7 distinct children):

1. `CreateWindowStationW` (1) and `CreateDesktopW` (1): **an alternate window station and desktop** for the targets
   (`GetProcessWindowStation`, `SetProcessWindowStation`, `GetThreadDesktop`, `GetUserObjectInformationW`).
2. Tokens: `OpenProcessToken(self, TOKEN_ADJUST_PRIVILEGES|TOKEN_QUERY...)`, `DuplicateTokenEx` (17; access 136 or 983551 =
   `TOKEN_ALL_ACCESS`; levels 0..2, primary and impersonation), **`CreateRestrictedToken` (22**, flags 0 and 1 =
   `DISABLE_MAX_PRIVILEGE`, 7 or 11 SIDs to disable, 1 to 12 restricting SIDs), `SetTokenInformation` (47: class 25
   `TokenIntegrityLevel`, class 6 `TokenDefaultDacl`), `GetTokenInformation` (295; a call with a NULL buffer fails
   with `ERROR_INSUFFICIENT_BUFFER` 122: size probing), `CheckTokenMembership`, `AdjustTokenPrivileges`,
   `SetThreadToken`/`OpenThreadToken` (12 each), `AddMandatoryAce`, `SetEntriesInAclW` (31),
   `ConvertStringSecurityDescriptorToSecurityDescriptorW` with `D:(A;;GA;;;SY)(A;;GWGR;;;S-1-15-2-1)S:(ML;;;;;S-1-16-0)` and
   `D:P(A;;0x1F0003;;;SY)(A;;0x1F0003;;;IU)`, `DeriveAppContainerSidFromAppContainerName` (an AppContainer for some service).
3. Jobs: `CreateJobObjectW` (12), `SetInformationJobObject` (48: class 9 `JobObjectExtendedLimitInformation` 144 bytes, class 4
   `JobObjectBasicUIRestrictions`, class 7 completion-port association), `QueryInformationJobObject`,
   `AssignProcessToJobObject`, `TerminateJobObject`.
4. The process: `InitializeProcThreadAttributeList` (called twice per child: the sizing call fails with 122, then it fills; 5 or
   6 attributes), `UpdateProcThreadAttribute` (62) with `0x20002` (handle list, 24 bytes = 3 handles), `0x20007` (mitigation policy,
   16 bytes = two 64-bit masks), `0x2000E` (child process policy, 4 bytes), `0x2001A` (number 26, 4 bytes; its name to be read
   from `winbase.h`); **`CreateProcessAsUserW` (12; inherit handles = TRUE; creation flags 525324 = 0x8040C =
   `EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT | DETACHED_PROCESS | CREATE_SUSPENDED`; application name,
   command line `"chrome.exe" --type=... --service-sandbox-type=...`)**, then `AssignProcessToJobObject`, `DuplicateHandle`
   (692 to 696 in the browser, up to 906 over all processes: the handle hand-over), `ResumeThread` (430: mostly Chrome's
   stack-sampling profiler, which also makes 418 `SuspendThread` and 418 `GetThreadContext` calls in the browser), `SetProcessMitigationPolicy` (self, policy 2) and
   `GetProcessMitigationPolicy`. The unconfined children (crashpad handler, processor-metrics utility) use plain
   `CreateProcessW` with `EXTENDED_STARTUPINFO_PRESENT` (0x80000) and inherit handles = TRUE.
5. The target side (lowering the token, closing handles) happens inside the child. It is traced since `spec-chrome-win-2`: section 4a
   (the children call `SetTokenInformation`, `RevertToSelf`, `SetProcessMitigationPolicy`, `SetDefaultDllDirectories`; what Windows
   then refuses). The old note follows.
   The confined network service's last traced calls before it died in an early experiment were `NtQueryObject(h,
   ObjectTypeInformation)` followed by `DuplicateHandle(self, h, self, &new, 2, FALSE, 0)`; the children also call
   `GetProcessMitigationPolicy` and `SetProcessShutdownParameters`.

The semantics of every one of these calls are the job of **spec-win-sec** (tokens, ACLs, SDs, jobs; its sheets
`spec-win-sec-sd.md`, `-acl.md`, `-rtl.md` cover the SD/ACL calls above) and of the process attribute list / handle hand-over
of **spec-win-ipc**; this map only says *which calls, in which numbers, with which arguments*, so that their batches can be
ordered. `spec-win-sec` has since specified `CreateRestrictedToken`, `DuplicateTokenEx`, `SetTokenInformation`, the job calls and the
SD/ACL/SID families (149 entries, branch `spec-win-sec`); `spec-chrome-win-2` added the AclAPI trustee layer (`BuildExplicitAccessWithNameW`,
`BuildTrusteeWithSidW/NameW`, `SetEntriesInAclW`, `GetExplicitEntriesFromAclW`), `Get/SetNamedSecurityInfoW`, `LookupAccountSidW/NameW`,
`Get/SetProcessMitigationPolicy` and `DeriveAppContainerSidFromAppContainerName` (12 entries, section 10a). Still unspecified:
`CreateWindowStationW`/`CreateDesktopW`, `CreateAppContainerToken` and `AppContainerRegisterSid` (undocumented exports of `kernelbase`
that Chrome resolves by name: only their use is observable), `BuildSecurityDescriptorW`.

## 4a. The sandboxed targets seen from inside (S2 with every child traced)

Added by `spec-chrome-win-2`. The first map could not trace a sandboxed renderer. It can now: the S2 stage below has the browser, **all
renderers (normal, extension, top-chrome WebUI), the GPU process, the network service, the storage service and the crashpad handlers
traced**, three runs each, on homeserver2 (Windows 11 Home 25H2, 26200.9711), sandbox on (no `--no-sandbox`): exit 0 and the same 440-byte
DOM in about 3 s. The files: `traces/s2-sandboxed.trace.json` (`poly-chrometrace/1`, all process types, stable over three runs),
`traces/s2-sandbox-targets.json` (the comparison with S1, below), `tools/sandboxdiff.py`.

### Why the first tracer's renderers died, and the fix

* **Observed cause** (not a guess): the renderers exited with `0xC0000428` (`STATUS_INVALID_IMAGE_HASH`) about 2 ms after they were
  created. The `Microsoft-Windows-CodeIntegrity/Operational` log has one event 3033 per death: "Code Integrity determined that a process
  (`...\chrome.exe`) attempted to load `...\pt_kernel32.dll` that did not meet the Microsoft signing level requirements"
  (`RequestedPolicy` 8 = Microsoft, `ValidatedPolicy` 1 = unsigned). So the tracer's unsigned proxy DLLs are refused in a child that
  was **created** with the "Microsoft-signed binaries only" mitigation; neither `--disable-features=RendererCodeIntegrity` nor
  `WinSboxForceMsSigned`, nor the machine policy `RendererCodeIntegrityEnabled=0` changed this in Chrome for Testing 157.
* **The creation-time mitigation words** (`UpdateProcThreadAttribute`, attribute `0x20007`, two 64-bit words, logged by the tracer; the
  fields are the 2-bit `PROCESS_CREATION_MITIGATION_POLICY_*` fields of `winbase.h`, value 1 = always on):

| Child (`--type=`, sandbox) | word 0 | word 1 | Fields set |
|---|---|---|---|
| `gpu-process` | `0x0111000100011000` | `0x0100000000010000` | heap terminate-on-corruption (bit 12), bottom-up ASLR (16), extension points disabled (32), non-system fonts blocked (48), no remote images (52), no low-label images (56); word 1: restrict indirect branch prediction (16), FSCTL system calls disabled (56) |
| `utility` storage service (`service`) | `0x0111000110011000` | `0x0100000000010000` | as GPU plus **win32k system calls disabled (bit 28)** |
| `renderer` (all three kinds) | `0x0111100110011000` | `0x0100000020011000` | as storage plus **block non-Microsoft binaries (bit 44)**; word 1 also module-tampering protection (bit 12) and the CET shadow-stack field set to 2 (bits 28..29) |

  The unconfined children (crashpad handler, processor-metrics utility, the network service with `--service-sandbox-type=none`) get no
  `0x20007` attribute at all. All sandboxed children also get attribute `0x2000E` (child process policy, 4 bytes `01000000` =
  `PROCESS_CREATION_CHILD_PROCESS_RESTRICTED`), `0x2001A` (attribute number 26, 4 bytes `01000000`), `0x20002` (the handle list: 3 to 5 handles) and, for
  most of them, `0x2000D` (`PROC_THREAD_ATTRIBUTE_JOB_LIST`: one 8-byte job handle). Only the **renderers** have the Microsoft-signed bit at creation; the GPU
  and storage service get the same mitigation later, from inside (below), after their DLLs are loaded.
* **The fix, in the tracer, opt-in:** `POLYTRACE_RELAX=0x300000000000` makes the (traced) browser clear that 2-bit field in word 0
  before it calls the real `UpdateProcThreadAttribute`; nothing else of the sandbox is changed. A sandboxed renderer then loads the proxies.
  This is an interposition: the renderers of the S2 trace run **without** the Microsoft-signed-only mitigation (the other mitigations,
  restricted token, job, desktop, win32k lockdown are as Chrome made them). What the mitigation does to unsigned DLLs is therefore
  recorded here (event 3033) rather than traced. Poly can ignore it: its own DLLs are never "unsigned" in that sense.
* **Where the confined child writes its trace**: its start-up token cannot create a file anywhere, not even in a folder labelled
  Untrusted that allows `Everyone`, `Users`, `RESTRICTED`, `ANONYMOUS`, `ALL APPLICATION PACKAGES` and the null SID (`CreateFileW`
  failed with `ERROR_ACCESS_DENIED` (5) in the renderers and the storage service, reported through `OutputDebugStringA`). So with
  `POLYTRACE_INHERIT=1` the traced browser creates, for every child it makes with `CreateProcessAsUserW` and a handle list, a file-backed
  section (the trace file `polytrace-c<n>.jsonl`), appends its inheritable handle to the child's handle list and passes the value in
  the child's environment (`POLYTRACE_HANDLE`); the child maps the section in `DllMain`, before the sandbox has closed anything.

### What the targets do after the sandbox is applied (S2 compared with S1)

S1 = `--no-sandbox` (two runs), S2 = sandbox on (three runs), the same page, the same kit, per process type; the "sampled" calls are the
first four calls of each function and each call with a new name (paths, key names, DLL names). `traces/s2-sandbox-targets.json` has all of it.

* **Calls the target lowers itself with (new in every confined type, absent in S1)**: `advapi32!SetTokenInformation`, `RevertToSelf`,
  `RegDisablePredefinedCache`, `kernel32!SetProcessMitigationPolicy`, `SetDefaultDllDirectories`, `SetHandleInformation` (storage,
  renderers), `GetUserDefaultLCID/LangID/LocaleName` (the locale is read before the registry is closed), `GetProcessHandleCount`,
  `GetCurrentProcessorNumber`, `ntdll!NtQueryInformationProcess`; in the renderers also `EnumSystemLocalesEx`, `GetGeoInfoW`,
  `GetUserGeoID`, `GetTimeZoneInformation`, `GetDynamicTimeZoneInformation`, `GetSystemTimeAsFileTime`, `GetProcessHeaps`,
  `HeapDestroy`, `SetThreadAffinityMask`.
* **Calls Windows refused** (sampled records with a failing result and `ERROR_ACCESS_DENIED`/NTSTATUS `0xC0000022`, all of which
  **succeed in S1**):

| Process type | Call | Object | Result |
|---|---|---|---|
| renderers (all three kinds), storage service | `advapi32!RegOpenKeyExW` | `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion` | 5 `ACCESS_DENIED` |
| renderers, storage service | `ntdll!NtCreateKey` | (a key the target tries to create) | `0xC0000022` |
| renderers, storage, GPU, network service | `kernel32!DuplicateHandle` | (at least one of the sampled calls) | 5 `ACCESS_DENIED` |
| renderers (normal, WebUI) | `kernel32!LoadLibraryExW` | `DWriteCore.dll` (system directory) | 5 `ACCESS_DENIED`; `GetProcAddress("DWriteCoreCreateFactory")` then gives 127 |
| renderers, storage | `advapi32!RegQueryValueExW` | (12 to 16 calls per renderer) | never succeeds sandboxed, succeeds in S1 |

  Everything else the targets do that failed also fails in S1 (so Chrome's own probes, not the sandbox): `GetTokenInformation` size
  probes (122), `FlsGetValue` (87), `GetEnvironmentVariableW("PROGRAMFILES")` (203), `NtQueryValueKey` (`0xC0000034`),
  `SetThreadPriority` (400/401, background-mode mismatches), `GetModuleHandleA("verifier.dll")` and
  `GetModuleHandleW("msdia*.dll")` (126), `DiscardVirtualMemory` (158).
* **Calls the confined renderer never makes (present in S1)**: `user32!GetSysColor`, `SystemParametersInfoW`, `GetCaretBlinkTime`,
  `PeekMessageW`, `PostThreadMessageW` (win32k is disabled: Chrome does not call them), `RoActivateInstance`,
  `WindowsCreateString`, `WindowsDeleteString` (WinRT), `RegNotifyChangeKeyValue`, `RegisterWaitForSingleObject`. The GPU process makes
  every S1 call; the network service (unconfined) loses only `DeleteFileW`; the storage service loses `PeekMessageW` and
  `PostThreadMessageW`.
* **What Poly's checks must therefore reproduce** (first list): a confined target that was started with the restricted token, job and
  mitigations of section 4 can **read** (and load) the files and keys it got through inherited handles and its own image directory, can
  **not** open `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion`, cannot create registry keys, gets `ACCESS_DENIED` from (some of) its
  `DuplicateHandle` calls, cannot load `DWriteCore.dll` from the system directory (Chrome carries on: the DOM is identical), and
  stays connected to the browser through the handles it inherited. The exact token (restricting SIDs, groups, integrity) is
  spec-win-sec's `CreateRestrictedToken` family; this list is the *observable consequence* an `AccessCheck`/object-open implementation must
  show for S2 to behave as on Windows.

### Mitigation state of the running children (Windows' own report, untraced S2, `Get-ProcessMitigation`)

| | browser | crashpad | GPU | storage | renderers (6) | network (`none`) |
|---|---|---|---|---|---|---|
| Microsoft-signed only | off | off | **on** | **on** | **on** | off |
| child process creation disallowed | off | off | on | on | on | off |
| win32k system calls disabled | off | off | off | **on** | **on** | off |
| extension points / non-system fonts / strict handle checks / FSCTL calls disabled | off | off | on | on | on | off |
| no remote images, no low-label images | off | off | on | on | on | off |
| dynamic code blocked | on (browser only) | off | off | on (storage) | off | off |

All have DEP (with ATL thunk emulation), bottom-up and high-entropy ASLR on and force-relocate off.

## 5. Names resolved with `GetProcAddress` (called through pointers, counts unknown)

| DLL (map) | Name resolved with `GetProcAddress` | Poly | By |
|---|---|---|---|
| ? | `ChromeMain` | NOT LISTED (no entry in any DLL file) | BGMNRS |
| ? | `DWriteCoreCreateFactory` | NOT LISTED (no entry in any DLL file) | R |
| ? | `GetHandleVerifier` | NOT LISTED (no entry in any DLL file) | BCGMNRS |
| ? | `IsSandboxedProcess` | NOT LISTED (no entry in any DLL file) | B |
| crtdll.dll | `signal` | MISSING | BGMNRS |
| d3dcompiler_40.dll | `D3DCompile` | MISSING (app-provided) | G |
| d3dcompiler_40.dll | `D3DDisassemble` | MISSING (app-provided) | G |
| dcomp.dll | `DCompositionCreateDevice3` | MISSING | G |
| dnsapi.dll | `DnsIsZtEnabled` | MISSING | B |
| dxcore.dll | `DXCoreCreateAdapterFactory` | MISSING | G |
| dxgi.dll | `CreateDXGIFactory2` | MISSING | G |
| dxgi.dll | `DXGIGetDebugInterface1` | MISSING | G |
| kernel32.dll | `CompareStringEx` | MISSING | BCGMNRS |
| kernel32.dll | `GetThreadDescription` | MISSING | BGMNRS |
| kernel32.dll | `IsValidLocaleName` | MISSING | BCGMNRS |
| kernel32.dll | `IsWow64Process2` | MISSING | BCGMNRS |
| kernel32.dll | `RegisterApplicationRestart` | MISSING | B |
| kernel32.dll | `SetThreadDescription` | MISSING | BCGMNRS |
| kernelbase.dll | `AppContainerRegisterSid` | MISSING | B |
| kernelbase.dll | `AppPolicyGetProcessTerminationMethod` | MISSING | BC |
| kernelbase.dll | `CreateAppContainerToken` | MISSING | B |
| kernelbase.dll | `FlsGetValue2` | MISSING | BCGMNRS |
| kernelbase.dll | `IsEnclaveTypeSupported` | MISSING | M |
| kernelbase.dll | `IsUserCetAvailableInEnvironment` | MISSING | MR |
| kernelbase.dll | `MapViewOfFile3` | MISSING | R |
| kernelbase.dll | `UnmapViewOfFile2` | MISSING | R |
| kernelbase.dll | `VirtualAlloc2` | MISSING | R |
| mdmregistration.dll | `IsDeviceRegisteredWithManagement` | MISSING | B |
| msctf.dll | `SetInputScope` | MISSING | B |
| ntdll.dll | `LdrRegisterDllNotification` | MISSING | BR |
| ntdll.dll | `LdrUnregisterDllNotification` | MISSING | B |
| ntdll.dll | `NtQuerySection` | MISSING (syscall) | BCGMNRS |
| ntdll.dll | `RtlAddGrowableFunctionTable` | MISSING | R |
| ntdll.dll | `RtlDeleteGrowableFunctionTable` | MISSING | R |
| user32.dll | `EnableNonClientDpiScaling` | MISSING | B |
| user32.dll | `GetAutoRotationState` | MISSING | B |
| user32.dll | `GetSystemMetricsForDpi` | MISSING | B |
| user32.dll | `SetProcessDpiAwarenessContext` | MISSING | BG |

`ChromeMain` and `GetHandleVerifier` are Chrome's own exports. Everything with "NOT LISTED" is also added to the coverage map in
this branch (section 10). `LdrRegisterDllNotification`, `RtlAddGrowableFunctionTable` (V8's unwind tables for JIT code in
every renderer), `VirtualAlloc2`/`MapViewOfFile3`/`UnmapViewOfFile2` (renderer), `NtQuerySection` (everywhere),
`SetThreadDescription` and `CompareStringEx` are the ones every process uses.

Modules Chrome loads by name (besides the proxies): `chrome.dll`, `kernel32.dll`, `kernelbase.dll`, `ntdll.dll`,
`ucrtbase.dll`, `bcryptprimitives.dll`, `dbghelp.dll` and `msdia*.dll` (from `dbghelp`, crash reporting), `dnsapi.dll`,
`netapi32.dll`, `MDMRegistration.dll`, the GPU process: `d3d11.dll`, `dxgi.dll`, `dcomp.dll`, `dwmapi.dll`, `DXCore.dll`,
`mf.dll`, `D3DCompiler_47.dll`, `dxcompiler.dll` (from the app directory); renderers: `dwrite.dll`, `DWriteCore.dll`; the
browser also tries `optimization_guide_internal.dll` (not in the zip: the failure is tolerated). Third-party DLLs that appear
in the PC's trace (`AcLayers.dll`, `easyhook*.dll`, `kwsui64.dll`, `sftldr.dll`, `tmmon64.dll` ...) look like
injections of security software into the PC's processes, not Chrome's own loads.

## 6. Required to succeed, or may fail

Three kinds of evidence, in this order of weight:

1. **DLL ablation (S1, 28 delay-loaded DLLs that are called, proxy removed one at a time, `windows.json`):** crash for
   `advapi32`, `user32`, `gdi32`, the API sets `realtime`, `winrt`, `winrt-string`, `shcore-scaling`,
   `bcryptprimitives`, `comctl32`, `dwmapi`, `imm32`, `iphlpapi`, `mfplat`, `netapi32`, `ole32`, `oleacc`, `oleaut32`,
   `secur32`, `shell32`, `shlwapi`, `uiautomationcore`, `userenv`, `winhttp`, `wtsapi32`; hang for `api-ms-win-power-base`;
   **unaffected** (DOM identical): `dhcpcsvc`, `pdh`, `setupapi`. So *the DLL and the names must exist*; this says nothing yet
   about what they return.
2. **Failures Windows itself returned** (last column of the tables): an API that failed on Windows in a run that still gave
   the right result is tolerated *in that situation* (`CancelIoEx` -> `NOT_FOUND`, `ConnectNamedPipe` -> `PIPE_CONNECTED`,
   `GetQueuedCompletionStatus` -> `WAIT_TIMEOUT`/`BROKEN_PIPE`, `CreateNamedPipeW` -> `ALREADY_EXISTS`,
   `InitializeProcThreadAttributeList` -> `INSUFFICIENT_BUFFER` (the sizing call), `DuplicateHandle` -> `ACCESS_DENIED`,
   `GetFileAttributesW` and `RegOpenKeyExW` -> `FILE_NOT_FOUND`; see the last column for each API).
3. **Optional by nature, an API Chrome asks for after a feature check:** accessibility (`oleacc`, `uiautomationcore`),
   devices (`setupapi`, `cfgmgr32`, `hid`), performance counters (`pdh`), ETW (`EventRegister`, `TraceEvent`, `RegisterTraceGuidsW`, ...: Chrome registers providers and writes events,
   nothing it reads back was seen), power/session notifications (`Register*Notification`, `WTS*`),
   `Shell_NotifyIconW`, drag and drop, IME, `DwmSetWindowAttribute`. Expect a stub that returns failure to work for these;
   **verified per API for S1 since `spec-chrome-win-2`: section 6a** (the other stages were not injected).

Everything else is **assumed to be required to succeed** (not tested): process creation, pipes, IOCP, sections, handles, `VirtualAlloc`/`VirtualProtect`
(V8 and PartitionAlloc), SRW locks, TLS/FLS, file and registry basics, the code page and locale functions, the clock.

### 6a. Per-API fault injection, S1 (what may stay a stub)

Method: for each of the 220 APIs that the S1 table marks MISSING (section 8), one S1 run (`--headless --no-sandbox --disable-gpu --dump-dom`, kit with the counting logger) in which the API **is not called** and instead returns a failure from its first call, in every process of the run (`POLYTRACE_FAULT=<fn>:<ret>:<err>`; ret 0 and last error 50 `ERROR_NOT_SUPPORTED` for BOOL/pointer/DWORD results, `0xFFFFFFFFFFFFFFFF` for the two invalid-handle returners (`CreateToolhelp32Snapshot` and friends), `0x80004001` `E_NOTIMPL` for the HRESULT ones; a `void` API just does nothing). Machine: homeserver2, one run each, a bound of 30 s. Verdict: **TOLERATED** = exit 0 and the 440-byte DOM (SHA-256 prefix `7b79f8b4`) and no process ended abnormally; **TOLERATED-CHILD-CRASH** = the same output but a child process (renderer, GPU, utility) ended with the exit status in the CSV and was restarted or its work was not needed; **FATAL** = no DOM; **HANG** = still running after 30 s; **DIFFERENT** = exit 0 but other output; **NOT-CALLED** = the injection never fired (the API is not called in that run, or is a thunk the logger cannot skip: `TAIL`/`NORETURN` functions, `UpdateProcThreadAttribute`, `CreateProcessAsUserW`). Table: `traces/s1-fault-injection.csv` (api, class, injected result and error, verdict, exit, seconds, stdout bytes and digest, number of injection records, abnormal exit statuses).

| Verdict | APIs | Meaning for an implementer |
|---|---:|---|
| TOLERATED | 162 | may fail with the injected result (a stub that returns failure/`ERROR_NOT_SUPPORTED` is enough for S1) |
| TOLERATED-CHILD-CRASH | 9 | the browser survives, but a child dies: do not stub |
| FATAL | 23 | must work (Chrome ends, `0x80000003` = its own breakpoint/CHECK, `0xC0000005` = a null result used) |
| HANG | 8 | must work (Chrome waits for it forever) |
| DIFFERENT | 4 | exit 0 but the DOM is not the expected one: must work |
| NOT-CALLED | 14 | not decided by this run |

**Must work (FATAL, HANG, DIFFERENT, TOLERATED-CHILD-CRASH):**

* HANG: `AssignProcessToJobObject`, `CreateIoCompletionPort`, `CreateJobObjectW`, `CreateNamedPipeW`, `DisconnectNamedPipe`, `GetQueuedCompletionStatus`, `PostQueuedCompletionStatus`, `SetInformationJobObject`
* FATAL: `CoInitializeEx`, `ConnectNamedPipe`, `DWriteCreateFactory`, `GetAdaptersAddresses`, `GetCurrentThreadStackLimits`, `GetFileAttributesExW`, `GetFileVersionInfoSizeW`, `GetFileVersionInfoW`, `GetIfTable2`, `GetModuleHandleExA`, `GetThreadPreferredUILanguages`, `GetWindowThreadProcessId`, `InitCommonControlsEx`, `InitializeProcThreadAttributeList`, `LoadLibraryExA`, `NetUserGetInfo`, `QueryProcessCycleTime`, `RoInitialize`, `RtlFormatCurrentUserKeyPath`, `SHGetKnownFolderPath`, `SHGetPropertyStoreForWindow`, `VerQueryValueW`, `WindowsCreateString`
* DIFFERENT: `RegisterWaitForSingleObject`, `SetNamedPipeHandleState`, `TransactNamedPipe`, `UnregisterWaitEx`
* TOLERATED-CHILD-CRASH: `CoTaskMemAlloc`, `CreateStdAccessibleObject`, `GetFileInformationByHandleEx`, `GetGeoInfoW`, `GetModuleHandleExW`, `GetProcessInformation`, `IsWindowVisible`, `K32GetMappedFileNameW`, `OutputDebugStringW`

**Not decided (NOT-CALLED):** `CallNtPowerInformation`, `ConvertStringSecurityDescriptorToSecurityDescriptorW`, `GetProfileType`, `GetTraceEnableFlags`, `GetTraceEnableLevel`, `GetTraceLoggerHandle`, `ImmGetIMEFileNameW`, `LresultFromObject`, `TraceEvent`, `UiaDisconnectProvider`, `UiaHostProviderFromHwnd`, `UiaReturnRawElementProvider`, `UnregisterTraceGuids`, `UpdateProcThreadAttribute`.

**Tolerated (a failing stub is enough in S1):** `AddVectoredExceptionHandler`, `BuildExplicitAccessWithNameW`, `BuildSecurityDescriptorW`, `BuildTrusteeWithSidW`, `CancelIPChangeNotify`, `CancelIoEx`, `CertAddStoreToCollection`, `CertControlStore`, `CoCreateFreeThreadedMarshaler`, `CoCreateInstance`, `CoTaskMemFree`, `CoUninitialize`, `CreateBitmap`, `CreateFontIndirectW`, `CreateIconIndirect`, `CreateSemaphoreA`, `CreateToolhelp32Snapshot`, `CryptProtectMemory`, `CryptUnprotectMemory`, `DeleteProcThreadAttributeList`, `DestroyIcon`, `DhcpCApiInitialize`, `DhcpRequestParams`, `DiscardVirtualMemory`, `DwmSetWindowAttribute`, `EnumDisplayDevicesW`, `EnumThreadWindows`, `EventRegister`, `EventSetInformation`, `FindFirstFileExW`, `FindWindowExW`, `FreeMibTable`, `GetAncestor`, `GetCaretBlinkTime`, `GetClassNameW`, `GetComputerNameExW`, `GetComputerNameW`, `GetDesktopWindow`, `GetDeviceCaps`, `GetDoubleClickTime`, `GetDpiForMonitor`, `GetDynamicTimeZoneInformation`, `GetFinalPathNameByHandleW`, `GetKeyboardLayout`, `GetKeyboardLayoutList`, `GetKeyboardLayoutNameW`, `GetLastInputInfo`, `GetLocaleInfoEx`, `GetLogicalProcessorInformation`, `GetLogicalProcessorInformationEx`, `GetLongPathNameW`, `GetNamedSecurityInfoW`, `GetObjectW`, `GetPointerDevices`, `GetPriorityClass`, `GetProcessMitigationPolicy`, `GetProductInfo`, `GetSecurityInfo`, `GetStockObject`, `GetStringTypeW`, `GetSysColor`, `GetSystemDirectoryW`, `GetSystemMenu`, `GetSystemPowerStatus`, `GetTextMetricsW`, `GetThreadContext`, `GetThreadId`, `GetThreadPriority`, `GetThreadPriorityBoost`, `GetTimeZoneInformation`, `GetUserDefaultLangID`, `GetUserGeoID`, `GetUserNameExW`, `GetWindow`, `GetWindowDisplayAffinity`, `GetWindowLongW`, `GetWindowTextLengthW`, `HeapSetInformation`, `K32GetModuleInformation`, `K32GetProcessMemoryInfo`, `LoadIconW`, `LoadImageW`, `LockFileEx`, `LookupAccountNameW`, `MFStartup`, `Module32FirstW`, `Module32NextW`, `MonitorFromRect`, `MsgWaitForMultipleObjectsEx`, `NotifyAddrChange`, `NotifyWinEvent`, `OleInitialize`, `OleUninitialize`, `OpenProcess`, `PathMatchSpecW`, `PrefetchVirtualMemory`, `ProcessPrng`, `PropVariantClear`, `QueryThreadCycleTime`, `QueryUnbiasedInterruptTimePrecise`, `ReadDirectoryChangesW`, `ReadProcessMemory`, `RegNotifyChangeKeyValue`, `RegisterClassW`, `RegisterDeviceNotificationW`, `RegisterDragDrop`, `RegisterGPNotification`, `RegisterPointerDeviceNotifications`, `RegisterPowerSettingNotification`, `RegisterSuspendResumeNotification`, `RegisterTraceGuidsW`, `RegisterWindowMessageW`, `RemovePropW`, `ReplaceFileW`, `RevokeDragDrop`, `RoActivateInstance`, `RoGetActivationFactory`, `RoUninitialize`, `RtlCaptureStackBackTrace`, `SHGetFolderPathW`, `SetEntriesInAclW`, `SetErrorMode`, `SetFileInformationByHandle`, `SetNamedSecurityInfoW`, `SetParent`, `SetPriorityClass`, `SetProcessInformation`, `SetProcessShutdownParameters`, `SetPropW`, `SetSecurityInfo`, `SetThreadInformation`, `SetThreadPriority`, `SetThreadPriorityBoost`, `SetWindowLongW`, `Shell_NotifyIconW`, `SwitchToThread`, `SystemParametersInfoW`, `SystemTimeToTzSpecificLocalTime`, `TzSpecificLocalTimeToSystemTime`, `UnlockFileEx`, `UnregisterDeviceNotification`, `UnregisterPowerSettingNotification`, `VerSetConditionMask`, `VerifyVersionInfoW`, `WSAEnumNetworkEvents`, `WSAEventSelect`, `WSALookupServiceBeginW`, `WSALookupServiceEnd`, `WSALookupServiceNextW`, `WTSFreeMemory`, `WTSQuerySessionInformationW`, `WTSRegisterSessionNotification`, `WTSUnRegisterSessionNotification`, `WerRegisterRuntimeExceptionModule`, `WinHttpGetIEProxyConfigForCurrentUser`, `WindowsCreateStringReference`, `WindowsDeleteString`, `WriteProcessMemory`, `lstrcmpiA`, `timeBeginPeriod`, `timeEndPeriod`, `timeGetTime`.

Caveats: one API at a time (two tolerated APIs together may not be); one run each, so a race that needs a second failure is not seen; the S2/S3 stages call more (window, GPU, sandbox) and were not injected; "tolerated" says nothing about the *quality* of the page, only that the DOM digest and the exit code were unchanged. Several FATAL entries are APIs whose failure Chrome turns into its own `CHECK` (for example `GetModuleHandleExA`, `GetCurrentThreadStackLimits`, `CoInitializeEx`, `RoInitialize`, `InitCommonControlsEx`): they need a working implementation, not just a name.

## 7. Window messages (S3, browser UI thread; counts of the two runs)

Recorded with two per-thread hooks (`WH_CALLWNDPROC`: sent; `WH_GETMESSAGE`: posted), installed when a thread calls
`CreateWindowExW`; the table is the browser process of a window that opens, is clicked, typed into and closed
(counts are per run, the minimum and maximum of the two runs; the browser creates 17 windows in a run: message windows, the
top-level `Chrome_WidgetWin_1`, the render widget host and helper windows):

| Message | Sent (to the window procedure) | Posted (from the queue) |
|---|---:|---:|
| WM_GETMINMAXINFO (0x24) | 8-8 |  |
| WM_NCCREATE (0x81) | 17-17 |  |
| WM_NCCALCSIZE (0x83) | 29-29 |  |
| WM_CREATE (0x1) | 17-17 |  |
| WM_SIZE (0x5) | 18-18 |  |
| WM_MOVE (0x3) | 22-22 |  |
| 0x287 (0x287) | 3-3 |  |
| WM_WINDOWPOSCHANGING (0x46) | 43-43 |  |
| WM_WINDOWPOSCHANGED (0x47) | 22-22 |  |
| WM_GETICON (0x7F) | 24-24 |  |
| WM_CHANGEUISTATE (0x127) | 1-1 |  |
| WM_STYLECHANGING (0x7C) | 10-10 |  |
| WM_STYLECHANGED (0x7D) | 10-10 |  |
| WM_PARENTNOTIFY (0x210) | 9-9 |  |
| WM_GETOBJECT (0x3D) | 4-4 |  |
| WM_CHILDACTIVATE (0x22) | 28-28 |  |
| WM_GETTEXTLENGTH (0xE) | 9-9 |  |
| WM_SETTEXT (0xC) | 6-6 |  |
| WM_GETTEXT (0xD) | 5-5 |  |
| WM_SHOWWINDOW (0x18) | 9-9 |  |
| WM_ACTIVATEAPP (0x1C) | 12-12 |  |
| WM_NCACTIVATE (0x86) | 2-2 |  |
| WM_ACTIVATE (0x6) | 2-2 |  |
| WM_IME_SETCONTEXT (0x281) | 6-6 |  |
| WM_IME_NOTIFY (0x282) | 6-6 |  |
| WM_SETFOCUS (0x7) | 1-1 |  |
| WM_NCPAINT (0x85) | 5-5 |  |
| WM_ERASEBKGND (0x14) | 3-3 |  |
| WM_POWERBROADCAST (0x218) | 2-2 |  |
| WM_DWMNCRENDERINGCHANGED (0x31F) |  | 2-2 |
| WM_USER+0 (0x400) |  | 1-1 |
| 0x96 (0x96) |  | 4-5 |
| registered (RegisterWindowMessage) 0xC0B4 (0xC0B4) |  | 1-1 |
| 0x60 (0x60) |  | 6-8 |
| WM_NCHITTEST (0x84) | 26-28 |  |
| WM_SETCURSOR (0x20) | 11-12 |  |
| WM_MOUSEMOVE (0x200) |  | 18-20 |
| WM_PAINT (0xF) |  | 3-3 |
| WM_TIMER (0x113) |  | 4-4 |
| 0xAE (0xAE) | 5-5 |  |
| WM_CANCELMODE (0x1F) | 1-1 |  |
| WM_ENABLE (0xA) | 1-1 |  |
| 0x90 (0x90) | 9-9 |  |
| WM_DESTROY (0x2) | 9-9 |  |
| WM_NCDESTROY (0x82) | 9-9 |  |
| WM_LBUTTONDOWN (0x201) |  | 2-2 |
| WM_MOUSEACTIVATE (0x21) | 1-1 |  |
| WM_LBUTTONUP (0x202) |  | 1-1 |
| WM_CAPTURECHANGED (0x215) | 1-1 |  |
| WM_MOUSELEAVE (0x2A3) |  | 1-1 |
| WM_KEYDOWN (0x100) |  | 8-8 |
| WM_CHAR (0x102) |  | 4-4 |
| WM_KEYUP (0x101) |  | 4-4 |
| WM_CLOSE (0x10) |  | 2-2 |
| WM_KILLFOCUS (0x8) | 1-1 |  |
| 0x272 (0x272) | 1-1 |  |

The GPU process creates 4 and the crashpad handler 2 windows (`WM_GETMINMAXINFO`, `WM_NCCREATE`, `WM_NCCALCSIZE`, `WM_CREATE`,
`WM_SIZE`, `WM_MOVE`, ..., `WM_DESTROY`, `WM_NCDESTROY`; in the GPU process also `WM_SHOWWINDOW`, `WM_WINDOWPOSCHANGING`,
`WM_CHILDACTIVATE`, `WM_CANCELMODE`, `WM_ENABLE`: a child window it shows and destroys). Undocumented ones seen (shown as hex):
`0x90` (`WM_UAHDESTROYWINDOW`), `0xAE` (`WM_NCUAHDRAWCAPTION`), `0x287`, `0x60`, `0x96`, `0x272`, `0x127`, `0x3D`, `0x22`: Chrome
or user32 sends them itself; the document of `spec-win-gui-b` has the rules for what Poly's window manager must deliver. The
class names Chrome registers: see `RegisterClassExW`/`CreateWindowExW` objects in the traces (`Chrome_WidgetWin_0`,
`Chrome_WidgetWin_1`, `Chrome_StatusTrayWindow`, `Base_PowerMessageWindow`, `MSCTFIME UI`, `IME`).

## 8. The gap tables: every API Chrome called, by area and DLL

Columns: **Poly** (green / claimed / MISSING / NOT LISTED against `coverage/windows/api` and `NtServices`; "in tree" = the
name is in Poly's source), **By** (B browser, R renderers, G GPU, N network service, S storage, M processor-metrics utility,
C crashpad), the stages in which it was called in **both** runs, the highest call count in any run, and the failures
Windows returned. **Stub-safe candidates** are the rows with a failure column or in the optional areas of section 6.

Totals over every stage: 579 APIs; claimed 239, green 27, MISSING 311, NOT LISTED 2.

### process / thread / IPC  (62 APIs: claimed 23, MISSING 39)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| advapi32.dll | `CreateProcessAsUserW` | MISSING | B | . x . . . . . | 12 |  |
| kernel32.dll | `CancelIoEx` | MISSING | BGMNRS | x x x x x x x | 20 | NOT_FOUND |
| kernel32.dll | `ConnectNamedPipe` | MISSING | BCR | x x x x x x x | 46 | PIPE_CONNECTED |
| kernel32.dll | `CreateFileMappingW` | claimed (kernel32-2 in tree) | BGMNRS | x x x x x x x | 221 |  |
| kernel32.dll | `CreateIoCompletionPort` | MISSING | BCGMNRS | x x x x x x x | 79 |  |
| kernel32.dll | `CreateNamedPipeW` | MISSING | BCR | x x x x x x x | 47 | ALREADY_EXISTS |
| kernel32.dll | `CreateProcessW` | claimed (kernel32-2 in tree) | B | x x x x x x x | 16 |  |
| kernel32.dll | `CreateThread` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 98 |  |
| kernel32.dll | `CreateToolhelp32Snapshot` | MISSING | BR | x x x x x x x | 8 |  |
| kernel32.dll | `DeleteProcThreadAttributeList` | MISSING | B | x x x x x x x | 15 |  |
| kernel32.dll | `DisconnectNamedPipe` | MISSING | C | x x x x x x x | 17 |  |
| kernel32.dll | `DuplicateHandle` | claimed (impl-createprocess in tree) | BCGMNRS | x x x x x x x | 1333 | ACCESS_DENIED |
| kernel32.dll | `FlushViewOfFile` | claimed (kernel32-2 in tree) | BN | x x x x x x x | 23 |  |
| kernel32.dll | `GetCurrentProcess` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 5308 |  |
| kernel32.dll | `GetCurrentProcessId` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 802 |  |
| kernel32.dll | `GetCurrentThread` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 702 |  |
| kernel32.dll | `GetCurrentThreadId` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 216006 |  |
| kernel32.dll | `GetCurrentThreadStackLimits` | MISSING | BGMNRS | x x x x x x x | 65 |  |
| kernel32.dll | `GetExitCodeProcess` | claimed (kernel32-2 in tree) | BC | x x x x x x x | 16 |  |
| kernel32.dll | `GetHandleInformation` | claimed (impl-createprocess in tree) | CGMNRS | x x x x x x x | 173 |  |
| kernel32.dll | `GetPriorityClass` | MISSING | BR | x x x x x x x | 327 |  |
| kernel32.dll | `GetProcessId` | claimed (kernel32-2 in tree) | BC | x x x x x x x | 77 |  |
| kernel32.dll | `GetProcessTimes` | claimed (7zip in tree) | BCGMNRS | x x x x x x x | 14 |  |
| kernel32.dll | `GetQueuedCompletionStatus` | MISSING | BCGMNRS | x x x x x x x | 18187 | BROKEN_PIPE, WAIT_TIMEOUT, OPERATION_ABORTED |
| kernel32.dll | `GetThreadContext` | MISSING | BGNR | x x x x x x x | 579 |  |
| kernel32.dll | `GetThreadId` | MISSING | BCGMN | x x x x x x x | 6 |  |
| kernel32.dll | `GetThreadPreferredUILanguages` | MISSING | B | x x x x x x x | 2 |  |
| kernel32.dll | `GetThreadPriority` | MISSING | BCGMNRS | x x x x x x x | 34 | returned 0/NULL 8 x |
| kernel32.dll | `GetThreadPriorityBoost` | MISSING | BGNR | x x x x x x x | 579 |  |
| kernel32.dll | `InitializeProcThreadAttributeList` | MISSING | BC | x x x x x x x | 30 | INSUFFICIENT_BUFFER |
| kernel32.dll | `K32EnumProcessModules` | MISSING | B | . x . . . . . | 4 |  |
| kernel32.dll | `K32GetMappedFileNameW` | MISSING | B | x x x x x x x | 1377 |  |
| kernel32.dll | `K32GetModuleFileNameExW` | MISSING | B | . . x . x . x | 351 |  |
| kernel32.dll | `K32GetModuleInformation` | MISSING | BGNR | x x x x x x x | 368 |  |
| kernel32.dll | `K32GetProcessMemoryInfo` | MISSING | BGNRS | x x x x x x x | 33 |  |
| kernel32.dll | `MapViewOfFile` | claimed (kernel32-2 in tree) | BCGMNRS | x x x x x x x | 268 |  |
| kernel32.dll | `Module32FirstW` | MISSING | BR | x x x x x x x | 8 |  |
| kernel32.dll | `Module32NextW` | MISSING | BR | x x x x x x x | 490 | NO_MORE_FILES |
| kernel32.dll | `OpenProcess` | MISSING | BC | x x x x x x x | 18 |  |
| kernel32.dll | `PostQueuedCompletionStatus` | MISSING | BCGMNRS | x x x x x x x | 1577 |  |
| kernel32.dll | `QueryFullProcessImageNameW` | MISSING | B | . x x . x . x | 1 |  |
| kernel32.dll | `QueryProcessCycleTime` | MISSING | B | x x x x x x x | 90 |  |
| kernel32.dll | `QueryThreadCycleTime` | MISSING | BGNR | x x x x x x x | 317 |  |
| kernel32.dll | `ReadDirectoryChangesW` | MISSING | BN | x x x x x x x | 1 |  |
| kernel32.dll | `ReadProcessMemory` | MISSING | B | x x x x x x x | 703 |  |
| kernel32.dll | `ResumeThread` | claimed (7zip in tree) | BGNR | x x x x x x x | 579 |  |
| kernel32.dll | `SetHandleInformation` | claimed (impl-createprocess in tree) | B | x x x x x x x | 61 |  |
| kernel32.dll | `SetNamedPipeHandleState` | MISSING | BCGMNRS | x x x x x x x | 8 |  |
| kernel32.dll | `SetPriorityClass` | MISSING | BG | x x x x x x x | 21 |  |
| kernel32.dll | `SetThreadPriority` | MISSING | BGMNRS | x x x x x x x | 171 | INVALID_PARAMETER, 400, 401 |
| kernel32.dll | `SetThreadPriorityBoost` | MISSING | BGNR | x x x x x x x | 1158 |  |
| kernel32.dll | `SuspendThread` | claimed (7zip in tree) | BGNR | x x x x x x x | 579 | returned 0/NULL 579 x |
| kernel32.dll | `TerminateProcess` | claimed (kernel32 in tree) | BGMNR | x x x x x x x | 9 | ACCESS_DENIED |
| kernel32.dll | `TransactNamedPipe` | MISSING | BCGMNRS | x x x x x x x | 8 |  |
| kernel32.dll | `UnmapViewOfFile` | claimed (kernel32-2 in tree) | BCGMNRS | x x x x x x x | 235 |  |
| kernel32.dll | `UpdateProcThreadAttribute` | MISSING | BC | x x x x x x x | 62 |  |
| kernel32.dll | `WriteProcessMemory` | MISSING | B | x x x x x x x | 377 |  |
| ntdll.dll | `NtClose` | claimed (syscall in service table) | BCGMNRS | x x x x x x x | 48 | returned 0/NULL 48 x |
| ntdll.dll | `NtQueryInformationProcess` | claimed (syscall in service table) | B | . x x . x . x | 11 | returned 0/NULL 11 x |
| ntdll.dll | `NtQueryObject` | claimed (syscall in service table) | BGMNRS | x x x x x x x | 256 | returned 0/NULL 256 x |
| ntdll.dll | `NtQuerySystemInformation` | claimed (syscall in service table) | B | x x x x x x x | 4 | returned 0/NULL 2 x |
| user32.dll | `GetThreadDesktop` | MISSING | B | . x . . . . . | 2 |  |

### sync / TLS  (43 APIs: claimed 38, MISSING 5)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| kernel32.dll | `AcquireSRWLockExclusive` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 172179 |  |
| kernel32.dll | `AcquireSRWLockShared` | claimed (kernel32 in tree) | BNR | x x x x x x x | 316 |  |
| kernel32.dll | `CreateEventW` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 518 |  |
| kernel32.dll | `CreateMutexW` | claimed (kernel32 in tree) | BG | x x x x x x x | 4 | ALREADY_EXISTS |
| kernel32.dll | `CreateSemaphoreA` | MISSING | R | x . x x x x x | 8 |  |
| kernel32.dll | `CreateSemaphoreW` | claimed (kernel32 in tree) | CR | x x x x x x x | 9 |  |
| kernel32.dll | `DeleteCriticalSection` | claimed (kernel32 in tree) | BCGNRS | x x x x x x x | 54 |  |
| kernel32.dll | `EnterCriticalSection` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 247841 |  |
| kernel32.dll | `FlsAlloc` | claimed (kernel32-2 in tree) | BCGMNRS | x x x x x x x | 48 |  |
| kernel32.dll | `FlsFree` | claimed (kernel32-2 in tree) | BC | x . x x x x x | 4 |  |
| kernel32.dll | `FlsGetValue` | claimed (kernel32-2 in tree) | BCGMNRS | x x x x x x x | 236 | INVALID_PARAMETER |
| kernel32.dll | `FlsSetValue` | claimed (kernel32-2 in tree) | BCGMNRS | x x x x x x x | 920 |  |
| kernel32.dll | `InitializeConditionVariable` | claimed (kernel32-2 in tree) | BCGMNRS | x x x x x x x | 1789 |  |
| kernel32.dll | `InitializeCriticalSection` | claimed (kernel32 in tree) | BGMNRS | x x x x x x x | 428 |  |
| kernel32.dll | `InitializeCriticalSectionEx` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 1992 |  |
| kernel32.dll | `InitializeSListHead` | claimed (7zip in tree) | BCGMNRS | x x x x x x x | 24 |  |
| kernel32.dll | `InitializeSRWLock` | claimed (kernel32 in tree) | BR | x . x x x x x | 5 |  |
| kernel32.dll | `InitOnceExecuteOnce` | claimed (kernel32-2 in tree) | BCGMNRS | x x x x x x x | 43474 |  |
| kernel32.dll | `InterlockedFlushSList` | claimed (7zip in tree) | BC | x . x x x x x | 2 | returned 0/NULL 2 x |
| kernel32.dll | `InterlockedPushEntrySList` | claimed (7zip in tree) | BR | x x x x x x x | 4 | returned 0/NULL 1 x |
| kernel32.dll | `LeaveCriticalSection` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 247841 |  |
| kernel32.dll | `RegisterWaitForSingleObject` | MISSING | BCNR | x x x x x x x | 412 |  |
| kernel32.dll | `ReleaseMutex` | claimed (kernel32 in tree) | B | x x x x x x x | 1377 |  |
| kernel32.dll | `ReleaseSemaphore` | claimed (kernel32 in tree) | CR | x . x x x x x | 44 |  |
| kernel32.dll | `ReleaseSRWLockExclusive` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 533631 |  |
| kernel32.dll | `ReleaseSRWLockShared` | claimed (kernel32 in tree) | BNR | x x x x x x x | 316 |  |
| kernel32.dll | `ResetEvent` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 3150 |  |
| kernel32.dll | `SetEvent` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 2821 |  |
| kernel32.dll | `Sleep` | claimed (kernel32 in tree) | G | . . x . x . x | 4 |  |
| kernel32.dll | `SleepConditionVariableSRW` | claimed (kernel32-2 in tree) | BGNR | x x x x x x x | 112 |  |
| kernel32.dll | `SwitchToThread` | MISSING | R | x . x x x x x | 31188 | returned 0/NULL 30937 x |
| kernel32.dll | `TlsAlloc` | claimed (kernel32-2 in tree) | BCGMNRS | x x x x x x x | 40 |  |
| kernel32.dll | `TlsGetValue` | claimed (kernel32-2 in tree) | BCGMNRS | x x x x x x x | 20097 | returned 0/NULL 896 x |
| kernel32.dll | `TlsSetValue` | claimed (kernel32-2 in tree) | BCGMNRS | x x x x x x x | 339 |  |
| kernel32.dll | `TryAcquireSRWLockExclusive` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 501705 | returned 0/NULL 9134 x |
| kernel32.dll | `UnregisterWaitEx` | MISSING | BCN | x x x x x x x | 409 | IO_PENDING |
| kernel32.dll | `WaitForMultipleObjects` | claimed (kernel32 in tree) | CRS | x . x x x x x | 13 | returned 0/NULL 8 x |
| kernel32.dll | `WaitForSingleObject` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 3322 | returned 0/NULL 2825 x |
| kernel32.dll | `WakeAllConditionVariable` | claimed (kernel32-2 in tree) | BCGMNRS | x x x x x x x | 4712 |  |
| kernel32.dll | `WakeConditionVariable` | claimed (kernel32-2 in tree) | BGR | x x x x x x x | 328 |  |
| user32.dll | `MsgWaitForMultipleObjectsEx` | MISSING (name appears in tree) | BGN | x x x x x x x | 1765 | returned 0/NULL 1429 x |
| ws2_32.dll | `WSACreateEvent` | claimed (impl-winsock in tree) | BN | x x x x x x x | 53 |  |
| ws2_32.dll | `WSAResetEvent` | claimed (impl-winsock in tree) | N | x x x x x x x | 60 |  |

### memory  (14 APIs: claimed 9, MISSING 5)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| kernel32.dll | `DiscardVirtualMemory` | MISSING | NR | x . x x x x x | 6 | NOT_LOCKED |
| kernel32.dll | `GetProcessHeap` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 24 |  |
| kernel32.dll | `GlobalMemoryStatusEx` | claimed (7zip in tree) | BGRS | x x x x x x x | 71 |  |
| kernel32.dll | `HeapSetInformation` | MISSING | BGMNRS | x x x x x x x | 8 |  |
| kernel32.dll | `LocalFree` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 138 | returned 0/NULL 138 x |
| kernel32.dll | `PrefetchVirtualMemory` | MISSING | B | x x x x x x x | 2 |  |
| kernel32.dll | `RtlVirtualUnwind` | claimed (7zip) | BGNR | x x x x x x x | 8115 | returned 0/NULL 8115 x |
| kernel32.dll | `VirtualAlloc` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 12438 | INVALID_PARAMETER, 487 |
| kernel32.dll | `VirtualAllocEx` | MISSING | B | . x . . . . . | 34 |  |
| kernel32.dll | `VirtualFree` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 1764 |  |
| kernel32.dll | `VirtualProtect` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 696 |  |
| kernel32.dll | `VirtualProtectEx` | MISSING | B | . x . . . . . | 238 |  |
| kernel32.dll | `VirtualQuery` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 2939 |  |
| user32.dll | `MapVirtualKeyW` | claimed (spec-win-gui-b in tree) | B | x x x x x x x | 2 |  |

### loader / exceptions / debug  (22 APIs: claimed 14, MISSING 8)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| kernel32.dll | `AddVectoredExceptionHandler` | MISSING | BGMNRS | x x x x x x x | 16 |  |
| kernel32.dll | `ExitProcess` | claimed (kernel32 in tree) | BC | x . x x x x x | 2 |  |
| kernel32.dll | `FreeLibrary` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 37 |  |
| kernel32.dll | `GetLastError` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 24280 | returned 0/NULL 16572 x |
| kernel32.dll | `GetModuleFileNameA` | MISSING | G | . . . . . x x | 2 |  |
| kernel32.dll | `GetModuleFileNameW` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 514 |  |
| kernel32.dll | `GetModuleHandleA` | claimed (7zip in tree) | BGMNRS | x x x x x x x | 8 | MOD_NOT_FOUND |
| kernel32.dll | `GetModuleHandleExA` | MISSING | BG | x x x x x x x | 5 |  |
| kernel32.dll | `GetModuleHandleExW` | MISSING | BCGNR | x x x x x x x | 40 | MOD_NOT_FOUND |
| kernel32.dll | `GetModuleHandleW` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 151 | MOD_NOT_FOUND |
| kernel32.dll | `GetProcAddress` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 5003 | PROC_NOT_FOUND |
| kernel32.dll | `IsDebuggerPresent` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 106 | returned 0/NULL 106 x |
| kernel32.dll | `LoadLibraryExA` | MISSING | BCGMNRS | x x x x x x x | 41 | MOD_NOT_FOUND, ENVVAR_NOT_FOUND, SXS_KEY_NOT_FOUND |
| kernel32.dll | `LoadLibraryExW` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 183 | INVALID_PARAMETER, MOD_NOT_FOUND |
| kernel32.dll | `LoadLibraryW` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 64 | returned 0/NULL 8 x |
| kernel32.dll | `OutputDebugStringW` | MISSING | B | x x x x x x x | 1 |  |
| kernel32.dll | `RtlCaptureStackBackTrace` | MISSING | BCGMNRS | x x x x x x x | 526 |  |
| kernel32.dll | `RtlLookupFunctionEntry` | claimed (7zip) | BGNR | x x x x x x x | 8115 | returned 0/NULL 3 x |
| kernel32.dll | `SetLastError` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 26383 |  |
| kernel32.dll | `SetUnhandledExceptionFilter` | claimed (7zip in tree) | BCGMNRS | x x x x x x x | 24 | returned 0/NULL 8 x |
| kernel32.dll | `WerRegisterRuntimeExceptionModule` | MISSING | BGMNRS | x x x x x x x | 8 | returned 0/NULL 8 x |
| ws2_32.dll | `WSAGetLastError (#111)` | claimed (impl-winsock in tree) | BN | x x x x x x x | 638 | returned 0/NULL 273 x |

### file system  (45 APIs: claimed 28, MISSING 17)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| imm32.dll | `ImmGetIMEFileNameW` | MISSING | B | x x x x x x x | 3 | returned 0/NULL 3 x |
| kernel32.dll | `CreateDirectoryW` | claimed (kernel32 in tree) | BCN | x x x x x x x | 97 | ALREADY_EXISTS |
| kernel32.dll | `CreateFileW` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 481 | FILE_NOT_FOUND, PATH_NOT_FOUND, ALREADY_EXISTS |
| kernel32.dll | `DeleteFileW` | claimed (kernel32 in tree) | BN | x x x x x x x | 15 |  |
| kernel32.dll | `DeviceIoControl` | claimed (7zip in tree) | B | x x x x x x x | 59 | 31 |
| kernel32.dll | `FileTimeToSystemTime` | claimed (kernel32-2 in tree) | B | x x x x x x x | 163 |  |
| kernel32.dll | `FindClose` | claimed (kernel32 in tree) | BN | x x x x x x x | 107 |  |
| kernel32.dll | `FindFirstFileExW` | MISSING | BN | x x x x x x x | 144 | FILE_NOT_FOUND, PATH_NOT_FOUND |
| kernel32.dll | `FindNextFileW` | claimed (kernel32 in tree) | BN | x x x x x x x | 386 | NO_MORE_FILES |
| kernel32.dll | `FlushFileBuffers` | claimed (kernel32 in tree) | BGNS | x x x x x x x | 234 |  |
| kernel32.dll | `GetCurrentDirectoryW` | claimed (kernel32 in tree) | B | x x x x x x x | 3 |  |
| kernel32.dll | `GetDiskFreeSpaceExW` | claimed (7zip in tree) | BN | x x x x x x x | 4 | PATH_NOT_FOUND |
| kernel32.dll | `GetFileAttributesExW` | MISSING | BN | x x x x x x x | 117 | FILE_NOT_FOUND |
| kernel32.dll | `GetFileAttributesW` | claimed (kernel32 in tree) | BMNRS | x x x x x x x | 570 | FILE_NOT_FOUND, PATH_NOT_FOUND |
| kernel32.dll | `GetFileInformationByHandle` | claimed (kernel32-2 in tree) | BN | x x x x x x x | 5 |  |
| kernel32.dll | `GetFileInformationByHandleEx` | MISSING | BCGR | x x x x x x x | 2 |  |
| kernel32.dll | `GetFileSize` | claimed (kernel32 in tree) | BN | x x x x x x x | 257 | returned 0/NULL 232 x |
| kernel32.dll | `GetFileSizeEx` | claimed (kernel32 in tree) | BGMNRS | x x x x x x x | 119 |  |
| kernel32.dll | `GetFileType` | claimed (kernel32 in tree) | BCN | x x x x x x x | 17 | INVALID_HANDLE |
| kernel32.dll | `GetFinalPathNameByHandleW` | MISSING | B | x x x x x x x | 1 |  |
| kernel32.dll | `GetFullPathNameW` | claimed (kernel32 in tree) | BGMNRS | x x x x x x x | 42 |  |
| kernel32.dll | `GetLongPathNameW` | MISSING | BN | x x x x x x x | 112 | ACCESS_DENIED |
| kernel32.dll | `GetSystemDirectoryW` | MISSING | BGN | x x x x x x x | 5 |  |
| kernel32.dll | `GetSystemTimeAsFileTime` | claimed (kernel32 in tree) | BCNR | x x x x x x x | 70 |  |
| kernel32.dll | `GetSystemTimePreciseAsFileTime` | claimed (kernel32 in tree) | BGMNRS | x x x x x x x | 8 |  |
| kernel32.dll | `GetTempPathW` | claimed (kernel32 in tree) | B | x x x x x x x | 12 |  |
| kernel32.dll | `LockFileEx` | MISSING | BGNS | x x x x x x x | 203 |  |
| kernel32.dll | `MoveFileW` | claimed (7zip in tree) | BN | x x x x x x x | 20 | returned 0/NULL 3 x |
| kernel32.dll | `ReadFile` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 3922 | HANDLE_EOF, BROKEN_PIPE, IO_PENDING |
| kernel32.dll | `RemoveDirectoryW` | claimed (kernel32 in tree) | B | x x x x x x x | 5 | returned 0/NULL 2 x |
| kernel32.dll | `ReplaceFileW` | MISSING | BN | x x x x x x x | 19 | FILE_NOT_FOUND, ENVVAR_NOT_FOUND |
| kernel32.dll | `SetCurrentDirectoryW` | claimed (kernel32 in tree) | BGMNRS | x x x x x x x | 8 |  |
| kernel32.dll | `SetEndOfFile` | claimed (kernel32 in tree) | BGNS | x x x x x x x | 81 |  |
| kernel32.dll | `SetFileInformationByHandle` | MISSING | BN | x x x x x x x | 19 |  |
| kernel32.dll | `SetFilePointer` | claimed (kernel32 in tree) | BN | x x x x x x x | 58 | returned 0/NULL 58 x |
| kernel32.dll | `SetFilePointerEx` | claimed (kernel32 in tree) | BGNS | x x x x x x x | 99 |  |
| kernel32.dll | `SystemTimeToFileTime` | claimed (kernel32-2 in tree) | BN | x x x x x x x | 128 |  |
| kernel32.dll | `UnlockFileEx` | MISSING | BNS | x x x x x x x | 203 | NOT_LOCKED |
| kernel32.dll | `WriteFile` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 6932 | INVALID_HANDLE, NO_DATA, IO_PENDING |
| shell32.dll | `SHGetFolderPathW` | MISSING | B | x x x x x x x | 6 | NO_TOKEN |
| shell32.dll | `SHGetKnownFolderPath` | MISSING | B | x x x x x x x | 2 | FILE_NOT_FOUND, NO_TOKEN |
| shell32.dll | `SHGetSpecialFolderPathW` | MISSING | B | . . . . x . . | 1 | NO_TOKEN |
| shlwapi.dll | `PathMatchSpecW` | MISSING | B | x x x x x x x | 164 |  |
| version.dll | `GetFileVersionInfoSizeW` | MISSING | B | x x x x x x x | 2 |  |
| version.dll | `GetFileVersionInfoW` | MISSING | B | x x x x x x x | 2 |  |

### registry  (19 APIs: claimed 16, MISSING 3)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| advapi32.dll | `RegCloseKey` | claimed (advapi32) | BCGMNRS | x x x x x x x | 270 | returned 0/NULL 270 x |
| advapi32.dll | `RegCreateKeyExW` | claimed (advapi32) | B | x x x x x x x | 26 | returned 0/NULL 25 x |
| advapi32.dll | `RegDeleteKeyExW` | claimed (advapi32 in tree) | B | x x x x x x x | 1 | returned 0/NULL 1 x |
| advapi32.dll | `RegDeleteValueW` | claimed (advapi32) | B | x x x x x x x | 4 | returned 0/NULL 2 x |
| advapi32.dll | `RegDisablePredefinedCache` | MISSING | GN | . x . . . . . | 1 | returned 0/NULL 1 x |
| advapi32.dll | `RegEnumKeyExW` | claimed (advapi32) | BN | x x x x x x x | 102 | returned 0/NULL 102 x |
| advapi32.dll | `RegEnumValueW` | claimed (advapi32) | B | x x x x x x x | 37 | returned 0/NULL 37 x |
| advapi32.dll | `RegNotifyChangeKeyValue` | MISSING | BNR | x x x x x x x | 24 | returned 0/NULL 24 x |
| advapi32.dll | `RegOpenKeyExA` | claimed (advapi32) | R | x . x x x x x | 8 | returned 0/NULL 8 x |
| advapi32.dll | `RegOpenKeyExW` | claimed (advapi32) | BCGMNRS | x x x x x x x | 339 | returned 0/NULL 266 x |
| advapi32.dll | `RegQueryInfoKeyW` | claimed (advapi32) | BN | x x x x x x x | 29 | returned 0/NULL 28 x |
| advapi32.dll | `RegQueryValueExA` | claimed (advapi32) | GR | x . x x x x x | 8 | returned 0/NULL 8 x |
| advapi32.dll | `RegQueryValueExW` | claimed (advapi32) | BCGMNRS | x x x x x x x | 276 | returned 0/NULL 188 x |
| advapi32.dll | `RegSetValueExW` | claimed (advapi32) | B | x x x x x x x | 13 | returned 0/NULL 13 x |
| ntdll.dll | `NtCreateKey` | claimed (syscall in service table) | BGMNRS | x x x x x x x | 40 | returned 0/NULL 40 x |
| ntdll.dll | `NtOpenKeyEx` | claimed (syscall in service table) | BCGMNRS | x x x x x x x | 18 | returned 0/NULL 8 x |
| ntdll.dll | `NtQueryValueKey` | claimed (syscall in service table) | BCGMNRS | x x x x x x x | 24 | returned 0/NULL 8 x |
| ntdll.dll | `NtSetValueKey` | claimed (syscall in service table) | B | x x x x x x x | 8 | returned 0/NULL 8 x |
| ntdll.dll | `RtlFormatCurrentUserKeyPath` | MISSING | BCGMNRS | x x x x x x x | 8 | returned 0/NULL 8 x |

### console / process environment  (11 APIs: claimed 11)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| kernel32.dll | `AttachConsole` | claimed (console in tree) | G | x x . x . x . | 1 | INVALID_HANDLE |
| kernel32.dll | `ExpandEnvironmentStringsW` | claimed (kernel32 in tree) | B | . x x . x . x | 27 |  |
| kernel32.dll | `FreeEnvironmentStringsW` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 24 |  |
| kernel32.dll | `GetCommandLineA` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 24 |  |
| kernel32.dll | `GetCommandLineW` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 64 |  |
| kernel32.dll | `GetEnvironmentStringsW` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 24 |  |
| kernel32.dll | `GetEnvironmentVariableW` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 49 | ENVVAR_NOT_FOUND |
| kernel32.dll | `GetStartupInfoW` | claimed (7zip in tree) | BCGMNRS | x x x x x x x | 32 |  |
| kernel32.dll | `GetStdHandle` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 72 | returned 0/NULL 72 x |
| kernel32.dll | `SetConsoleCtrlHandler` | claimed (console in tree) | BCGMNRS | x x x x x x x | 8 |  |
| kernel32.dll | `SetEnvironmentVariableW` | claimed (kernel32 in tree) | B | x x x x x x x | 1 |  |

### NLS / text  (16 APIs: claimed 8, MISSING 8)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| kernel32.dll | `FormatMessageW` | claimed (kernel32 in tree) | B | x x x x x x x | 5 |  |
| kernel32.dll | `GetACP` | claimed (kernel32-2 in tree) | BCGMNRS | x x x x x x x | 24 |  |
| kernel32.dll | `GetCPInfo` | claimed (kernel32-2 in tree) | BCGMNRS | x x x x x x x | 48 |  |
| kernel32.dll | `GetGeoInfoW` | MISSING | BR | x x x x x x x | 2 |  |
| kernel32.dll | `GetLocaleInfoEx` | MISSING | BGMNRS | x x x x x x x | 8 |  |
| kernel32.dll | `GetStringTypeW` | MISSING | BCGMNRS | x x x x x x x | 24 |  |
| kernel32.dll | `GetUserDefaultLangID` | MISSING | BGN | x x x x x x x | 4 |  |
| kernel32.dll | `GetUserDefaultLCID` | MISSING | GN | . x . . . . . | 1 |  |
| kernel32.dll | `GetUserDefaultLocaleName` | MISSING | GN | . x . . . . . | 1 |  |
| kernel32.dll | `GetUserGeoID` | MISSING | BR | x x x x x x x | 2 |  |
| kernel32.dll | `IsValidCodePage` | claimed (kernel32-2 in tree) | BCGMNRS | x x x x x x x | 24 |  |
| kernel32.dll | `lstrcmpiA` | MISSING | GMNRS | x x x x x x x | 128 |  |
| kernel32.dll | `lstrcmpW` | claimed (kernel32-2 in tree) | B | x x x x x x x | 14 | INVALID_PARAMETER |
| kernel32.dll | `MultiByteToWideChar` | claimed (kernel32-2 in tree) | BCGMNRS | x x x x x x x | 774 |  |
| kernel32.dll | `WideCharToMultiByte` | claimed (kernel32-2 in tree) | BCGMNRS | x x x x x x x | 5690 |  |
| user32.dll | `CharUpperW` | claimed (7zip in tree) | B | x x x x x x x | 14230 |  |

### time  (14 APIs: claimed 5, MISSING 9)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| api-ms-win-core-realtime-l1-1-1.dll | `QueryUnbiasedInterruptTimePrecise` | MISSING | B | x x x x x x x | 40 |  |
| kernel32.dll | `GetDynamicTimeZoneInformation` | MISSING | BR | x . x x x x x | 1 |  |
| kernel32.dll | `GetLocalTime` | claimed (kernel32-2 in tree) | BGMNRS | x x x x x x x | 329 |  |
| kernel32.dll | `GetSystemTime` | claimed (kernel32-2 in tree) | BGN | x x x x x x x | 1 |  |
| kernel32.dll | `GetTickCount` | claimed (kernel32 in tree) | BGN | x x x x x x x | 70 |  |
| kernel32.dll | `GetTimeZoneInformation` | MISSING | BNR | x x x x x x x | 3 |  |
| kernel32.dll | `QueryPerformanceCounter` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 247057 |  |
| kernel32.dll | `QueryPerformanceFrequency` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 116 |  |
| kernel32.dll | `SystemTimeToTzSpecificLocalTime` | MISSING | B | x x x x x x x | 151 |  |
| kernel32.dll | `TzSpecificLocalTimeToSystemTime` | MISSING | B | x x x x x x x | 123 |  |
| user32.dll | `GetCaretBlinkTime` | MISSING | BR | x x x x x x x | 8 |  |
| user32.dll | `GetDoubleClickTime` | MISSING (name appears in tree) | B | x x x x x x x | 4 |  |
| user32.dll | `GetMessageTime` | MISSING (name appears in tree) | B | . . x . x . x | 70 |  |
| winmm.dll | `timeGetTime` | MISSING | R | x . x x x x x | 2584 |  |

### system information  (11 APIs: claimed 3, MISSING 8)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| kernel32.dll | `GetComputerNameExW` | MISSING | BN | x x x x x x x | 3 | ENVVAR_NOT_FOUND |
| kernel32.dll | `GetComputerNameW` | MISSING | B | x x x x x x x | 2 | ENVVAR_NOT_FOUND |
| kernel32.dll | `GetLogicalProcessorInformation` | MISSING | BGR | x x x x x x x | 9 | INSUFFICIENT_BUFFER |
| kernel32.dll | `GetLogicalProcessorInformationEx` | MISSING | BGMNRS | x x x x x x x | 8 |  |
| kernel32.dll | `GetNativeSystemInfo` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 40 |  |
| kernel32.dll | `GetProductInfo` | MISSING | BCGMNRS | x x x x x x x | 16 |  |
| kernel32.dll | `GetSystemInfo` | claimed (kernel32 in tree) | BGNRS | x x x x x x x | 26 |  |
| kernel32.dll | `GetSystemPowerStatus` | MISSING | B | x x x x x x x | 4 |  |
| kernel32.dll | `GetVersionExW` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 16 |  |
| kernel32.dll | `VerifyVersionInfoW` | MISSING | BGMNRS | x x x x x x x | 15 |  |
| kernel32.dll | `VerSetConditionMask` | MISSING | BGMNRS | x x x x x x x | 45 |  |

### security: tokens / ACLs / SIDs  (43 APIs: claimed 23, MISSING 20)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| advapi32.dll | `AccessCheck` | MISSING | B | . x . . . . . | 2 |  |
| advapi32.dll | `AddMandatoryAce` | MISSING | B | . x . . . . . | 1 |  |
| advapi32.dll | `AdjustTokenPrivileges` | claimed (impl-7zip-trace) | B | . x . . . . . | 10 |  |
| advapi32.dll | `BuildExplicitAccessWithNameW` | MISSING | B | x x x x x x x | 1 |  |
| advapi32.dll | `BuildSecurityDescriptorW` | MISSING | B | x x x x x x x | 1 | INSUFFICIENT_BUFFER |
| advapi32.dll | `BuildTrusteeWithSidW` | MISSING | B | x x x x x x x | 59 |  |
| advapi32.dll | `CheckTokenMembership` | claimed (advapi32) | B | x x x x x x x | 17 |  |
| advapi32.dll | `ConvertSidToStringSidA` | claimed (advapi32 in tree) | B | x x x x x x x | 1 |  |
| advapi32.dll | `ConvertSidToStringSidW` | claimed (advapi32 in tree) | BGMNRS | x x x x x x x | 8 |  |
| advapi32.dll | `ConvertStringSecurityDescriptorToSecurityDescriptorW` | MISSING | B | x x x x x x x | 3 |  |
| advapi32.dll | `ConvertStringSidToSidW` | claimed (advapi32 in tree) | B | x x x x x x x | 16 |  |
| advapi32.dll | `CreateRestrictedToken` | MISSING | B | . x . . . . . | 22 |  |
| advapi32.dll | `DuplicateTokenEx` | MISSING | B | . x . . . . . | 17 |  |
| advapi32.dll | `EqualSid` | claimed (advapi32) | B | . x . . . . . | 27 | returned 0/NULL 24 x |
| advapi32.dll | `FreeSid` | claimed (advapi32) | B | . x . . . . . | 1 | returned 0/NULL 1 x |
| advapi32.dll | `GetAce` | claimed (advapi32) | B | x x x x x x x | 19 |  |
| advapi32.dll | `GetLengthSid` | claimed (advapi32) | BGMNRS | x x x x x x x | 406 |  |
| advapi32.dll | `GetNamedSecurityInfoW` | MISSING | B | x x x x x x x | 22 | INVALID_PARAMETER, NO_TOKEN |
| advapi32.dll | `GetSecurityDescriptorControl` | claimed (advapi32) | B | x x x x x x x | 23 |  |
| advapi32.dll | `GetSecurityDescriptorDacl` | claimed (advapi32) | B | x x x x x x x | 23 |  |
| advapi32.dll | `GetSecurityDescriptorGroup` | claimed (advapi32) | B | x x x x x x x | 23 |  |
| advapi32.dll | `GetSecurityDescriptorOwner` | claimed (advapi32) | B | x x x x x x x | 23 |  |
| advapi32.dll | `GetSecurityDescriptorSacl` | claimed (advapi32) | B | x x x x x x x | 23 |  |
| advapi32.dll | `GetSecurityInfo` | MISSING | B | x x x x x x x | 5 | INSUFFICIENT_BUFFER |
| advapi32.dll | `GetTokenInformation` | claimed (advapi32 in tree) | BGMNRS | x x x x x x x | 295 | INSUFFICIENT_BUFFER |
| advapi32.dll | `InitializeAcl` | claimed (advapi32) | BGNR | x x x x x x x | 181 |  |
| advapi32.dll | `InitializeSecurityDescriptor` | claimed (advapi32) | BGNR | x x x x x x x | 181 |  |
| advapi32.dll | `IsValidAcl` | claimed (advapi32) | B | x x x x x x x | 40 |  |
| advapi32.dll | `IsValidSecurityDescriptor` | claimed (advapi32) | B | x x x x x x x | 23 |  |
| advapi32.dll | `IsValidSid` | claimed (advapi32) | BGMNRS | x x x x x x x | 405 |  |
| advapi32.dll | `LogonUserW` | MISSING | B | . . x . x . x | 1 | 1326 |
| advapi32.dll | `LookupAccountNameW` | MISSING | B | x x x x x x x | 1 |  |
| advapi32.dll | `MapGenericMask` | MISSING | B | . x . . . . . | 2 |  |
| advapi32.dll | `OpenProcessToken` | claimed (advapi32) | BGMNRS | x x x x x x x | 36 |  |
| advapi32.dll | `OpenThreadToken` | claimed (advapi32) | B | . x . . . . . | 12 |  |
| advapi32.dll | `RevertToSelf` | MISSING | GN | . x . . . . . | 1 |  |
| advapi32.dll | `SetEntriesInAclW` | MISSING | B | x x x x x x x | 31 | returned 0/NULL 31 x |
| advapi32.dll | `SetNamedSecurityInfoW` | MISSING | B | x x x x x x x | 3 | NO_TOKEN |
| advapi32.dll | `SetSecurityDescriptorDacl` | claimed (advapi32) | BGNR | x x x x x x x | 181 |  |
| advapi32.dll | `SetSecurityInfo` | MISSING | B | x x x x x x x | 17 | returned 0/NULL 17 x |
| advapi32.dll | `SetThreadToken` | MISSING | B | . x . . . . . | 12 |  |
| advapi32.dll | `SetTokenInformation` | MISSING | B | . x . . . . . | 47 |  |
| userenv.dll | `DeriveAppContainerSidFromAppContainerName` | MISSING | B | . x . . . . . | 1 | returned 0/NULL 1 x |

### security: jobs  (5 APIs: MISSING 5)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| kernel32.dll | `AssignProcessToJobObject` | MISSING | B | x . x x x x x | 1 |  |
| kernel32.dll | `CreateJobObjectW` | MISSING | B | x x x x x x x | 12 |  |
| kernel32.dll | `QueryInformationJobObject` | MISSING | B | . x . . . . . | 12 |  |
| kernel32.dll | `SetInformationJobObject` | MISSING | B | x x x x x x x | 48 |  |
| kernel32.dll | `TerminateJobObject` | MISSING | B | . x . . . . . | 9 |  |

### security: mitigations / process policy  (8 APIs: claimed 1, MISSING 7)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| kernel32.dll | `GetProcessInformation` | MISSING | BGR | x x x x x x x | 300 |  |
| kernel32.dll | `GetProcessMitigationPolicy` | MISSING | BGMNRS | x x x x x x x | 8 |  |
| kernel32.dll | `SetDefaultDllDirectories` | claimed (impl-7zip-trace in tree) | GN | . x . . . . . | 1 |  |
| kernel32.dll | `SetErrorMode` | MISSING | BGMNRS | x x x x x x x | 24 | returned 0/NULL 1 x |
| kernel32.dll | `SetProcessInformation` | MISSING | BG | x x x x x x x | 21 |  |
| kernel32.dll | `SetProcessMitigationPolicy` | MISSING | BGN | . x . . . . . | 3 |  |
| kernel32.dll | `SetProcessShutdownParameters` | MISSING | CGMNRS | x x x x x x x | 8 |  |
| kernel32.dll | `SetThreadInformation` | MISSING | BGMNRS | x x x x x x x | 89 |  |

### COM  (20 APIs: claimed 3, MISSING 17)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| ole32.dll | `CLSIDFromString` | MISSING | B | . x . . . . . | 2 | returned 0/NULL 2 x |
| ole32.dll | `CoCreateFreeThreadedMarshaler` | MISSING | B | x x x x x x x | 11 | returned 0/NULL 11 x |
| ole32.dll | `CoCreateInstance` | MISSING | BMN | x x x x x x x | 44 | INVALID_PARAMETER, PROC_NOT_FOUND, SXS_KEY_NOT_FOUND |
| ole32.dll | `CoGetApartmentType` | MISSING | B | . x x . x . x | 2 | returned 0/NULL 2 x |
| ole32.dll | `CoGetClassObject` | MISSING | B | . x x . x . x | 1 | returned 0/NULL 1 x |
| ole32.dll | `CoInitializeEx` | MISSING | BGMN | x x x x x x x | 10 | returned 0/NULL 7 x |
| ole32.dll | `CoRegisterMessageFilter` | MISSING | B | . x x . x . x | 4 | returned 0/NULL 4 x |
| ole32.dll | `CoSetProxyBlanket` | MISSING | M | . x x . x . x | 1 | NO_TOKEN |
| ole32.dll | `CoTaskMemAlloc` | MISSING | B | x x x x x x x | 16 |  |
| ole32.dll | `CoTaskMemFree` | MISSING | B | x x x x x x x | 5 |  |
| ole32.dll | `CoUninitialize` | MISSING | BM | x x x x x x x | 5 | returned 0/NULL 4 x |
| ole32.dll | `OleInitialize` | MISSING | B | x x x x x x x | 1 | returned 0/NULL 1 x |
| ole32.dll | `OleUninitialize` | MISSING | B | x . x x x x x | 1 | returned 0/NULL 1 x |
| ole32.dll | `PropVariantClear` | MISSING | B | x x x x x x x | 10 | returned 0/NULL 10 x |
| ole32.dll | `RegisterDragDrop` | MISSING | B | x x x x x x x | 1 | returned 0/NULL 1 x |
| ole32.dll | `RevokeDragDrop` | MISSING | B | x . x x x x x | 1 | FILE_NOT_FOUND |
| ole32.dll | `StringFromCLSID` | MISSING | B | . . x . x . x | 2 | returned 0/NULL 2 x |
| oleaut32.dll | `SysAllocStringLen (#4)` | claimed (7zip in tree) | BM | . x x . x . x | 44 |  |
| oleaut32.dll | `SysFreeString (#6)` | claimed (7zip in tree) | BM | x x x x x x x | 636 |  |
| oleaut32.dll | `VariantClear (#9)` | claimed (7zip in tree) | BM | x x x x x x x | 685 | returned 0/NULL 685 x |

### DirectWrite / fonts  (1 APIs: MISSING 1)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| dwrite.dll | `DWriteCreateFactory` | MISSING | B | x x x x x x x | 2 | returned 0/NULL 2 x |

### GPU / DXGI  (3 APIs: MISSING 3)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| d3d11.dll | `D3D11CreateDevice` | MISSING | G | . . x . x . . | 1 | returned 0/NULL 1 x |
| dcomp.dll | `DCompositionCreateDevice` | MISSING | G | . . x . x . . | 1 | returned 0/NULL 1 x |
| dxgi.dll | `CreateDXGIFactory1` | MISSING | G | . . x . x x x | 8 | FILE_NOT_FOUND |

### accessibility  (5 APIs: MISSING 5)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| oleacc.dll | `CreateStdAccessibleObject` | MISSING | B | x x x x x x x | 7 | FILE_NOT_FOUND |
| oleacc.dll | `LresultFromObject` | MISSING | B | x . . x . x . | 1 |  |
| uiautomationcore.dll | `UiaDisconnectProvider` | MISSING | B | x . . x . x . | 4 | 298 |
| uiautomationcore.dll | `UiaHostProviderFromHwnd` | MISSING | B | x . . x . x . | 2 | returned 0/NULL 2 x |
| uiautomationcore.dll | `UiaReturnRawElementProvider` | MISSING | B | x . . x . x . | 5 | returned 0/NULL 4 x |

### crypto  (11 APIs: claimed 5, MISSING 6)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| bcryptprimitives.dll | `ProcessPrng` | MISSING | BR | x x x x x x x | 1 |  |
| crypt32.dll | `CertAddStoreToCollection` | MISSING | B | x x x x x x x | 19 |  |
| crypt32.dll | `CertCloseStore` | claimed (impl-crypto in tree) | B | x x x x x x x | 17 |  |
| crypt32.dll | `CertControlStore` | MISSING | B | x x x x x x x | 4 | 2148081668 |
| crypt32.dll | `CertFindCertificateInStore` | claimed (impl-crypto in tree) | B | x x x x x x x | 172 | 2148081668 |
| crypt32.dll | `CertOpenStore` | claimed (impl-crypto in tree) | B | x x x x x x x | 23 | FILE_NOT_FOUND, 2148081668 |
| crypt32.dll | `CryptProtectData` | claimed (impl-crypto in tree) | B | x x x x x x x | 1 |  |
| crypt32.dll | `CryptProtectMemory` | MISSING | BN | x x x x x x x | 9 |  |
| crypt32.dll | `CryptUnprotectData` | claimed (impl-crypto in tree) | B | x x x x x x x | 1 |  |
| crypt32.dll | `CryptUnprotectMemory` | MISSING | BN | x x x x x x x | 95 |  |
| secur32.dll | `GetUserNameExW` | MISSING | B | x x x x x x x | 2 |  |

### devices  (8 APIs: MISSING 8)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| setupapi.dll | `SetupDiCreateDeviceInfoList` | MISSING | B | . x . . . . . | 5 |  |
| setupapi.dll | `SetupDiDestroyDeviceInfoList` | MISSING | B | . x x . x . x | 16 |  |
| setupapi.dll | `SetupDiEnumDeviceInterfaces` | MISSING | B | . x x . x . x | 19 | NO_MORE_ITEMS |
| setupapi.dll | `SetupDiGetClassDevsW` | MISSING | B | . x x . x . x | 11 |  |
| setupapi.dll | `SetupDiGetDeviceInterfaceDetailW` | MISSING | B | . x x . x . x | 32 | INSUFFICIENT_BUFFER |
| setupapi.dll | `SetupDiGetDevicePropertyW` | MISSING | B | . x x . x . x | 66 | INSUFFICIENT_BUFFER, NOT_FOUND |
| setupapi.dll | `SetupDiOpenDeviceInfoW` | MISSING | B | . x . . . . . | 5 |  |
| setupapi.dll | `SetupDiOpenDevRegKey` | MISSING | B | . x . . . . . | 2 |  |

### diagnostics  (5 APIs: MISSING 5)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| pdh.dll | `PdhAddEnglishCounterW` | MISSING | B | . x x . x . x | 2 | ENVVAR_NOT_FOUND |
| pdh.dll | `PdhCloseQuery` | MISSING | B | . . x . x . x | 2 | returned 0/NULL 2 x |
| pdh.dll | `PdhCollectQueryData` | MISSING | B | . x x . x . x | 5 | returned 0/NULL 5 x |
| pdh.dll | `PdhGetFormattedCounterValue` | MISSING | B | . x . . . . . | 1 | returned 0/NULL 1 x |
| pdh.dll | `PdhOpenQueryW` | MISSING | B | . x x . x . x | 2 | returned 0/NULL 2 x |

### media  (3 APIs: MISSING 3)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| mfplat.dll | `MFStartup` | MISSING | G | x . x x x x x | 1 | returned 0/NULL 1 x |
| winmm.dll | `timeBeginPeriod` | MISSING | BGR | x x x x x x x | 585 | returned 0/NULL 585 x |
| winmm.dll | `timeEndPeriod` | MISSING | BGR | x x x x x x x | 585 | returned 0/NULL 585 x |

### misc (advapi32.dll)  (9 APIs: claimed 1, MISSING 8)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| advapi32.dll | `EventRegister` | MISSING | BGMNRS | x x x x x x x | 16 | returned 0/NULL 16 x |
| advapi32.dll | `EventSetInformation` | MISSING | BGMNRS | x x x x x x x | 16 | returned 0/NULL 16 x |
| advapi32.dll | `GetTraceEnableFlags` | MISSING | BGMNRS | x x . x . x . | 6 |  |
| advapi32.dll | `GetTraceEnableLevel` | MISSING | BGMNRS | x x . x . x . | 6 |  |
| advapi32.dll | `GetTraceLoggerHandle` | MISSING | BGMNRS | x x . x . x . | 6 |  |
| advapi32.dll | `GetUserNameW` | claimed (advapi32 in tree) | B | . x x . x . x | 2 |  |
| advapi32.dll | `RegisterTraceGuidsW` | MISSING | BGMNRS | x x x x x x x | 8 | returned 0/NULL 8 x |
| advapi32.dll | `TraceEvent` | MISSING | BGMNRS | x x . x . x . | 329 | returned 0/NULL 329 x |
| advapi32.dll | `UnregisterTraceGuids` | MISSING | BMN | x x x x x x x | 1 | returned 0/NULL 1 x |

### misc (api-ms-win-core-handle-l1-1-0.dll)  (1 APIs: MISSING 1)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| api-ms-win-core-handle-l1-1-0.dll | `CompareObjectHandles` | MISSING | G | . . . . . . x | 12 | 1656 |

### misc (api-ms-win-core-winrt-l1-1-0.dll)  (4 APIs: MISSING 4)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| api-ms-win-core-winrt-l1-1-0.dll | `RoActivateInstance` | MISSING | BR | x x x x x x x | 8 | returned 0/NULL 2 x |
| api-ms-win-core-winrt-l1-1-0.dll | `RoGetActivationFactory` | MISSING | BR | x x x x x x x | 13 | returned 0/NULL 13 x |
| api-ms-win-core-winrt-l1-1-0.dll | `RoInitialize` | MISSING | B | x x x x x x x | 26 | returned 0/NULL 26 x |
| api-ms-win-core-winrt-l1-1-0.dll | `RoUninitialize` | MISSING | B | x . x x x x . | 2 | returned 0/NULL 2 x |

### misc (api-ms-win-core-winrt-string-l1-1-0.dll)  (4 APIs: MISSING 4)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| api-ms-win-core-winrt-string-l1-1-0.dll | `WindowsCreateString` | MISSING | BR | x x x x x x x | 16 | returned 0/NULL 16 x |
| api-ms-win-core-winrt-string-l1-1-0.dll | `WindowsCreateStringReference` | MISSING | BR | x x x x x x x | 8 | returned 0/NULL 8 x |
| api-ms-win-core-winrt-string-l1-1-0.dll | `WindowsDeleteString` | MISSING | BR | x x x x x x x | 17 | returned 0/NULL 17 x |
| api-ms-win-core-winrt-string-l1-1-0.dll | `WindowsGetStringRawBuffer` | MISSING | B | . . x . x . x | 1 |  |

### misc (api-ms-win-power-base-l1-1-0.dll)  (1 APIs: MISSING 1)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| api-ms-win-power-base-l1-1-0.dll | `CallNtPowerInformation` | MISSING | B | x x x x x x x | 40 | 1300 |

### misc (api-ms-win-shcore-scaling-l1-1-1.dll)  (1 APIs: MISSING 1)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| api-ms-win-shcore-scaling-l1-1-1.dll | `GetDpiForMonitor` | MISSING | B | x x x x x x x | 8 | returned 0/NULL 8 x |

### misc (kernel32.dll)  (1 APIs: claimed 1)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| kernel32.dll | `CloseHandle` | claimed (kernel32 in tree) | BCGMNRS | x x x x x x x | 1246 |  |

### misc (ntdll.dll)  (2 APIs: claimed 2)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| ntdll.dll | `RtlFreeUnicodeString` | claimed (ntdll in tree) | BCGMNRS | x x x x x x x | 8 |  |
| ntdll.dll | `RtlInitUnicodeString` | claimed (ntdll in tree) | BCGMNRS | x x x x x x x | 70 |  |

### misc: version  (1 APIs: MISSING 1)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| version.dll | `VerQueryValueW` | MISSING | B | x x x x x x x | 13 | RESOURCE_TYPE_NOT_FOUND |

### network  (34 APIs: claimed 18, MISSING 16)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| dhcpcsvc.dll | `DhcpCApiInitialize` | MISSING | N | x x x x x x x | 1 | returned 0/NULL 1 x |
| dhcpcsvc.dll | `DhcpRequestParams` | MISSING | N | x x x x x x x | 4 | returned 0/NULL 4 x |
| iphlpapi.dll | `CancelIPChangeNotify` | MISSING | BN | x . x x x x x | 1 |  |
| iphlpapi.dll | `FreeMibTable` | MISSING | B | x x x x x x x | 1 |  |
| iphlpapi.dll | `GetAdaptersAddresses` | MISSING | BN | x x x x x x x | 9 | returned 0/NULL 9 x |
| iphlpapi.dll | `GetIfTable2` | MISSING | B | x x x x x x x | 1 | returned 0/NULL 1 x |
| iphlpapi.dll | `NotifyAddrChange` | MISSING | BN | x x x x x x x | 1 | IO_PENDING |
| netapi32.dll | `NetApiBufferFree` | MISSING | B | . . x . x . x | 1 | returned 0/NULL 1 x |
| netapi32.dll | `NetUserGetInfo` | MISSING | B | x . x x x x x | 1 | returned 0/NULL 1 x |
| winhttp.dll | `WinHttpGetIEProxyConfigForCurrentUser` | MISSING | B | x x x x x x x | 2 |  |
| ws2_32.dll | `closesocket (#3)` | claimed (impl-winsock in tree) | BN | x x x x x x x | 29 | returned 0/NULL 29 x |
| ws2_32.dll | `connect (#4)` | claimed (impl-winsock in tree) | N | x x x x x x x | 29 | 10035, 10051 |
| ws2_32.dll | `freeaddrinfo` | claimed (impl-winsock in tree) | N | x x x x x x x | 17 |  |
| ws2_32.dll | `getaddrinfo` | claimed (impl-winsock in tree) | N | x x x x x x x | 17 | 11001 |
| ws2_32.dll | `getsockname (#6)` | claimed (impl-winsock in tree) | N | x x x x x x x | 48 | returned 0/NULL 48 x |
| ws2_32.dll | `getsockopt (#7)` | claimed (impl-winsock in tree) | N | x x x x x x x | 12 | returned 0/NULL 12 x |
| ws2_32.dll | `htons (#9)` | claimed (impl-winsock in tree) | N | x x x x x x x | 107 |  |
| ws2_32.dll | `ioctlsocket (#10)` | claimed (impl-winsock in tree) | BN | x x x x x x x | 19 | returned 0/NULL 19 x |
| ws2_32.dll | `ntohs (#15)` | claimed (impl-winsock in tree) | N | x x x x x x x | 2536 |  |
| ws2_32.dll | `recv (#16)` | claimed (impl-winsock in tree) | N | x x x x x x x | 216 | 10035 |
| ws2_32.dll | `sendto (#20)` | claimed (impl-winsock in tree) | N | x x x x x x x | 357 |  |
| ws2_32.dll | `setsockopt (#21)` | claimed (impl-winsock in tree) | BN | x x x x x x x | 91 | returned 0/NULL 91 x |
| ws2_32.dll | `shutdown (#22)` | claimed (impl-winsock in tree) | N | x x x x x x x | 19 | 10057 |
| ws2_32.dll | `WSACloseEvent` | claimed (impl-winsock in tree) | BN | x x x x x x x | 47 |  |
| ws2_32.dll | `WSADuplicateSocketW` | MISSING | B | . . x . x . x | 7 | returned 0/NULL 7 x |
| ws2_32.dll | `WSAEnumNetworkEvents` | MISSING | N | x x x x x x x | 395 | returned 0/NULL 395 x |
| ws2_32.dll | `WSAEventSelect` | MISSING | N | x x x x x x x | 42 | returned 0/NULL 42 x |
| ws2_32.dll | `WSAIoctl` | claimed (impl-winsock in tree) | N | x x x x x x x | 25 | returned 0/NULL 25 x |
| ws2_32.dll | `WSALookupServiceBeginW` | MISSING | BN | x x x x x x x | 1 | returned 0/NULL 1 x |
| ws2_32.dll | `WSALookupServiceEnd` | MISSING | BN | x x x x x x x | 1 | returned 0/NULL 1 x |
| ws2_32.dll | `WSALookupServiceNextW` | MISSING | BN | x x x x x x x | 1 | returned 0/NULL 1 x |
| ws2_32.dll | `WSASend` | claimed (impl-winsock in tree) | N | x x x x x x x | 60 | returned 0/NULL 60 x |
| ws2_32.dll | `WSASocketW` | claimed (impl-winsock in tree) | BN | x x x x x x x | 29 |  |
| ws2_32.dll | `WSAStartup (#115)` | claimed (impl-winsock in tree) | BN | x x x x x x x | 1 | returned 0/NULL 1 x |

### shell  (13 APIs: MISSING 11, NOT LISTED 2)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| shell32.dll | `SHCreateAssociationRegistration` | MISSING | B | . x x . x . x | 2 | returned 0/NULL 2 x |
| shell32.dll | `SHCreateItemFromParsingName` | MISSING | B | . x . . . . . | 1 | returned 0/NULL 1 x |
| shell32.dll | `Shell_NotifyIconW` | MISSING | B | x x x x x x x | 4 |  |
| shell32.dll | `SHGetPropertyStoreForWindow` | MISSING | B | x x x x x x x | 2 | returned 0/NULL 2 x |
| shlwapi.dll | `#219` | NOT LISTED (no entry) | B | x . . x . x . | 119 | returned 0/NULL 11 x |
| shlwapi.dll | `#437` | NOT LISTED (no entry) | B | x x x x x x x | 1 | returned 0/NULL 1 x |
| userenv.dll | `GetProfileType` | MISSING | B | x x x x x x x | 1 |  |
| userenv.dll | `RegisterGPNotification` | MISSING | B | x x x x x x x | 2 |  |
| userenv.dll | `UnregisterGPNotification` | MISSING | B | . . x . x . x | 2 |  |
| wtsapi32.dll | `WTSFreeMemory` | MISSING | B | x x x x x x x | 2 | returned 0/NULL 2 x |
| wtsapi32.dll | `WTSQuerySessionInformationW` | MISSING | B | x x x x x x x | 2 |  |
| wtsapi32.dll | `WTSRegisterSessionNotification` | MISSING | B | x x x x x x x | 2 |  |
| wtsapi32.dll | `WTSUnRegisterSessionNotification` | MISSING | B | x . x x x x x | 1 |  |

### win32k: common controls  (1 APIs: MISSING 1)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| comctl32.dll | `InitCommonControlsEx` | MISSING | B | x x x x x x x | 1 |  |

### win32k: dwm  (4 APIs: MISSING 4)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| dwmapi.dll | `DwmExtendFrameIntoClientArea` | MISSING | B | . . x . x . x | 1 | returned 0/NULL 1 x |
| dwmapi.dll | `DwmGetCompositionTimingInfo` | MISSING | G | . . x . x . . | 121 | returned 0/NULL 121 x |
| dwmapi.dll | `DwmGetWindowAttribute` | MISSING | B | . . x . x . x | 55 | returned 0/NULL 55 x |
| dwmapi.dll | `DwmSetWindowAttribute` | MISSING | B | x x x x x x x | 7 | returned 0/NULL 7 x |

### win32k: gdi32  (17 APIs: claimed 4, green 3, MISSING 10)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| gdi32.dll | `CombineRgn` | claimed (spec-win-gui-b) | B | . . x . x . x | 1 |  |
| gdi32.dll | `CreateBitmap` | MISSING (name appears in tree) | B | x x x x x x x | 1 |  |
| gdi32.dll | `CreateDCW` | MISSING | B | . . x . x . x | 1 |  |
| gdi32.dll | `CreateDIBSection` | green (in tree) | B | x x x x x x x | 1 | 2012 |
| gdi32.dll | `CreateFontIndirectW` | MISSING (name appears in tree) | B | x x x x x x x | 5 |  |
| gdi32.dll | `CreateRectRgn` | claimed (spec-win-gui-b in tree) | B | . . x . x . x | 1 |  |
| gdi32.dll | `CreateRectRgnIndirect` | MISSING | B | . . x . x . x | 2 |  |
| gdi32.dll | `CreateSolidBrush` | green (in tree) | B | x x x x x x x | 4 |  |
| gdi32.dll | `DeleteDC` | MISSING (name appears in tree) | B | . . x . x . x | 1 |  |
| gdi32.dll | `DeleteObject` | claimed (impl-win-gui-a in tree) | B | x x x x x x x | 14 |  |
| gdi32.dll | `GetDeviceCaps` | MISSING (name appears in tree) | B | x x x x x x x | 3 |  |
| gdi32.dll | `GetICMProfileW` | MISSING | B | . . x . x . x | 1 | 2012 |
| gdi32.dll | `GetObjectW` | MISSING (name appears in tree) | B | x x x x x x x | 5 |  |
| gdi32.dll | `GetStockObject` | MISSING (name appears in tree) | BG | x x x x x x x | 3 |  |
| gdi32.dll | `GetTextMetricsW` | MISSING (name appears in tree) | B | x x x x x x x | 5 |  |
| gdi32.dll | `SelectClipRgn` | claimed (spec-win-gui-b in tree) | B | . . x . x . x | 1 |  |
| gdi32.dll | `SelectObject` | green (in tree) | B | x x x x x x x | 10 |  |

### win32k: user32  (117 APIs: claimed 26, green 24, MISSING 67)

| DLL | API | Poly | By | S1 S2 S3 S4 S4W S5 S5W | Max calls | Failures seen on Windows |
|---|---|---|---|---|---|---|---|---|---|---|---:|---|
| user32.dll | `BeginPaint` | claimed (spec-win-gui-b in tree) | B | . . x . x . x | 9 |  |
| user32.dll | `ClientToScreen` | claimed (spec-win-gui-b in tree) | B | . . x . x . x | 928 |  |
| user32.dll | `CreateDesktopW` | MISSING | B | . x . . . . . | 1 |  |
| user32.dll | `CreateIconIndirect` | MISSING | B | x x x x x x x | 1 |  |
| user32.dll | `CreateWindowExW` | green (in tree) | BCG | x x x x x x x | 15 |  |
| user32.dll | `CreateWindowStationW` | MISSING | B | . x . . . . . | 1 |  |
| user32.dll | `DefWindowProcW` | green (in tree) | BCG | x x x x x x x | 416 | INVALID_HANDLE |
| user32.dll | `DestroyIcon` | MISSING | B | x . x x x x x | 1 |  |
| user32.dll | `DestroyWindow` | green (in tree) | BG | x x x x x x x | 12 | IO_PENDING, INVALID_WINDOW_HANDLE |
| user32.dll | `DispatchMessageW` | green (in tree) | BC | x x x x x x x | 150 | ACCESS_DENIED, IO_PENDING |
| user32.dll | `DisplayConfigGetDeviceInfo` | MISSING | BG | . . x . x x x | 130 | returned 0/NULL 130 x |
| user32.dll | `EnableWindow` | claimed (spec-win-gui-b in tree) | BG | x x x x x x x | 1 | returned 0/NULL 1 x |
| user32.dll | `EndPaint` | claimed (spec-win-gui-b in tree) | B | . . x . x . x | 9 |  |
| user32.dll | `EnumDisplayDevicesW` | MISSING | B | x x x x x x x | 5 |  |
| user32.dll | `EnumDisplayMonitors` | MISSING | B | . . x . x . x | 4 | 2012 |
| user32.dll | `EnumDisplaySettingsW` | MISSING | B | . . x . x . x | 3 |  |
| user32.dll | `EnumThreadWindows` | MISSING | B | x x x x x x x | 26 |  |
| user32.dll | `EnumWindows` | MISSING (name appears in tree) | B | . . x . x . x | 6 |  |
| user32.dll | `FillRect` | claimed (impl-win-gui-a in tree) | B | . . x . x . x | 1 |  |
| user32.dll | `FindWindowExW` | MISSING (name appears in tree) | B | x x x x x x x | 2 | returned 0/NULL 2 x |
| user32.dll | `GetActiveWindow` | claimed (spec-win-gui-b in tree) | B | . . x . x . x | 85 | returned 0/NULL 4 x |
| user32.dll | `GetAncestor` | MISSING (name appears in tree) | B | x x x x x x x | 153 |  |
| user32.dll | `GetCapture` | claimed (spec-win-gui-b in tree) | B | . . x . x . x | 24 | returned 0/NULL 23 x |
| user32.dll | `GetClassNameW` | MISSING (name appears in tree) | B | x x x x x x x | 84 |  |
| user32.dll | `GetClientRect` | green (in tree) | B | x x x x x x x | 914 |  |
| user32.dll | `GetCursorPos` | claimed (spec-win-gui-b in tree) | B | . . x . x . x | 45 |  |
| user32.dll | `GetDC` | green (in tree) | BG | x x x x x x x | 8 |  |
| user32.dll | `GetDesktopWindow` | MISSING (name appears in tree) | BG | x x x x x x x | 10 |  |
| user32.dll | `GetDisplayConfigBufferSizes` | MISSING | BG | . . x . x x x | 130 | returned 0/NULL 130 x |
| user32.dll | `GetForegroundWindow` | claimed (spec-win-gui-b in tree) | B | . . x . x . x | 2 | returned 0/NULL 1 x |
| user32.dll | `GetKeyboardLayout` | MISSING | B | x x x x x x x | 61 |  |
| user32.dll | `GetKeyboardLayoutList` | MISSING | B | x x x x x x x | 4 |  |
| user32.dll | `GetKeyboardLayoutNameW` | MISSING | B | x x x x x x x | 1 |  |
| user32.dll | `GetKeyboardState` | claimed (spec-win-gui-b in tree) | B | . . x . x . x | 1 |  |
| user32.dll | `GetKeyState` | claimed (spec-win-gui-b in tree) | B | x x x x x x x | 1089 | returned 0/NULL 972 x |
| user32.dll | `GetLastInputInfo` | MISSING | B | x x x x x x x | 2 |  |
| user32.dll | `GetLayeredWindowAttributes` | MISSING | B | . . x . x . x | 6 |  |
| user32.dll | `GetMessageExtraInfo` | MISSING (name appears in tree) | B | . . x . x . x | 62 | returned 0/NULL 62 x |
| user32.dll | `GetMessageW` | green (in tree) | C | x x x x x x x | 4 |  |
| user32.dll | `GetMonitorInfoW` | claimed (spec-win-gui-b) | BG | x x x x x x x | 130 |  |
| user32.dll | `GetParent` | green (in tree) | B | x x x x x x x | 177 | returned 0/NULL 10 x |
| user32.dll | `GetPointerDevices` | MISSING | B | x x x x x x x | 8 |  |
| user32.dll | `GetProcessWindowStation` | MISSING | B | . x . . . . . | 2 |  |
| user32.dll | `GetPropW` | MISSING | B | . . x . x . x | 1 | FILE_NOT_FOUND |
| user32.dll | `GetQueueStatus` | green (in tree) | BG | x x x x x x x | 471 | returned 0/NULL 418 x |
| user32.dll | `GetSysColor` | MISSING (name appears in tree) | BR | x x x x x x x | 96 | returned 0/NULL 16 x |
| user32.dll | `GetSystemMenu` | MISSING | B | x x x x x x x | 1 |  |
| user32.dll | `GetSystemMetrics` | claimed (spec-win-gui-b in tree) | B | x x x x x x x | 865 | returned 0/NULL 22 x |
| user32.dll | `GetUserObjectInformationW` | MISSING | B | . x . . . . . | 44 | INSUFFICIENT_BUFFER |
| user32.dll | `GetWindow` | MISSING (name appears in tree) | B | x x x x x x x | 146 | returned 0/NULL 104 x |
| user32.dll | `GetWindowDC` | MISSING (name appears in tree) | B | . . x . x . x | 1 |  |
| user32.dll | `GetWindowDisplayAffinity` | MISSING | B | x x x x x x x | 2 |  |
| user32.dll | `GetWindowLongPtrW` | green (in tree) | BCG | x x x x x x x | 644 | returned 0/NULL 8 x |
| user32.dll | `GetWindowLongW` | MISSING (name appears in tree) | B | x x x x x x x | 138 |  |
| user32.dll | `GetWindowPlacement` | MISSING | B | . . x . x . x | 5 |  |
| user32.dll | `GetWindowRect` | green (in tree) | B | x x x x x x x | 90 |  |
| user32.dll | `GetWindowRgnBox` | MISSING | B | . . x . x . x | 34 | returned 0/NULL 28 x |
| user32.dll | `GetWindowTextLengthW` | MISSING (name appears in tree) | B | x x x x x x x | 9 | returned 0/NULL 1 x |
| user32.dll | `GetWindowTextW` | green (in tree) | B | x . x x x x x | 6 |  |
| user32.dll | `GetWindowThreadProcessId` | MISSING (name appears in tree) | BG | x x x x x x x | 651 |  |
| user32.dll | `IsChild` | MISSING (name appears in tree) | B | . . x . x . x | 4 | INVALID_WINDOW_HANDLE |
| user32.dll | `IsIconic` | MISSING (name appears in tree) | B | . . x . x . x | 606 | returned 0/NULL 600 x |
| user32.dll | `IsRectEmpty` | MISSING (name appears in tree) | B | . . x . x . x | 20 | returned 0/NULL 16 x |
| user32.dll | `IsWindow` | green (in tree) | BG | x x x x x x x | 768 | returned 0/NULL 11 x |
| user32.dll | `IsWindowArranged` | MISSING | B | . . x . x . x | 1 | returned 0/NULL 1 x |
| user32.dll | `IsWindowVisible` | MISSING (name appears in tree) | B | x x x x x x x | 657 | returned 0/NULL 520 x |
| user32.dll | `IsZoomed` | MISSING (name appears in tree) | B | . . x . x . x | 1162 | returned 0/NULL 1162 x |
| user32.dll | `LoadCursorW` | claimed (spec-win-gui-b in tree) | B | . . x . x . x | 3 |  |
| user32.dll | `LoadIconW` | MISSING (name appears in tree) | B | x x x x x x x | 1 |  |
| user32.dll | `LoadImageW` | MISSING | B | x x x x x x x | 1 |  |
| user32.dll | `MapWindowPoints` | claimed (spec-win-gui-b in tree) | B | . . x . x . x | 72 |  |
| user32.dll | `MonitorFromPoint` | claimed (spec-win-gui-b) | B | x x x x x x x | 56 |  |
| user32.dll | `MonitorFromRect` | MISSING | B | x x x x x x x | 21 |  |
| user32.dll | `MonitorFromWindow` | claimed (spec-win-gui-b) | BG | x x x x x x x | 1330 | INVALID_WINDOW_HANDLE |
| user32.dll | `NotifyWinEvent` | MISSING | B | x x x x x x x | 282 |  |
| user32.dll | `OffsetRect` | MISSING (name appears in tree) | B | . . x . x . x | 1 |  |
| user32.dll | `PeekMessageW` | green (in tree) | BGMNRS | x x x x x x x | 723 | FILE_NOT_FOUND |
| user32.dll | `PostMessageW` | green (in tree) | B | x x x x x x x | 1 |  |
| user32.dll | `PostThreadMessageW` | green (in tree) | GMNRS | x x x x x x x | 8 |  |
| user32.dll | `QueryDisplayConfig` | MISSING | BG | . . x . x x x | 130 | returned 0/NULL 130 x |
| user32.dll | `RedrawWindow` | claimed (spec-win-gui-b) | B | x x x x x x x | 2 |  |
| user32.dll | `RegisterClassExW` | green (in tree) | BG | x x x x x x x | 6 |  |
| user32.dll | `RegisterClassW` | MISSING (name appears in tree) | C | x x x x x x x | 2 |  |
| user32.dll | `RegisterDeviceNotificationW` | MISSING | B | x x x x x x x | 2 |  |
| user32.dll | `RegisterPointerDeviceNotifications` | MISSING | B | x x x x x x x | 1 |  |
| user32.dll | `RegisterPowerSettingNotification` | MISSING | B | x x x x x x x | 3 |  |
| user32.dll | `RegisterSuspendResumeNotification` | MISSING | B | x x x x x x x | 1 |  |
| user32.dll | `RegisterWindowMessageW` | MISSING | B | x x x x x x x | 1 |  |
| user32.dll | `ReleaseCapture` | claimed (spec-win-gui-b in tree) | B | . . x . x . x | 1 |  |
| user32.dll | `ReleaseDC` | green (in tree) | B | x x x x x x x | 9 |  |
| user32.dll | `RemovePropW` | MISSING | B | x x x x x x x | 10 | returned 0/NULL 2 x |
| user32.dll | `SendMessageW` | green (in tree) | B | x x x x x x x | 1 | returned 0/NULL 1 x |
| user32.dll | `SetCapture` | claimed (spec-win-gui-b) | B | . . x . x . x | 1 | returned 0/NULL 1 x |
| user32.dll | `SetCursor` | claimed (spec-win-gui-b) | B | . . x . x . x | 36 | returned 0/NULL 5 x |
| user32.dll | `SetFocus` | claimed (spec-win-gui-b) | B | . . . . x . . | 2 |  |
| user32.dll | `SetForegroundWindow` | claimed (spec-win-gui-b) | B | . . x . x . x | 2 |  |
| user32.dll | `SetKeyboardState` | MISSING | B | . . x . x . x | 1 |  |
| user32.dll | `SetParent` | MISSING (name appears in tree) | B | x x x x x x x | 8 |  |
| user32.dll | `SetProcessWindowStation` | MISSING | B | . x . . . . . | 2 |  |
| user32.dll | `SetPropW` | MISSING | B | x x x x x x x | 8 |  |
| user32.dll | `SetRect` | MISSING (name appears in tree) | B | . . . . x . . | 67 |  |
| user32.dll | `SetWindowLongPtrW` | green (in tree) | BCG | x x x x x x x | 18 | returned 0/NULL 18 x |
| user32.dll | `SetWindowLongW` | MISSING (name appears in tree) | B | x x x x x x x | 21 |  |
| user32.dll | `SetWindowPos` | green (in tree) | BG | x x x x x x x | 61 |  |
| user32.dll | `SetWindowTextW` | green (in tree) | B | x x x x x x x | 6 |  |
| user32.dll | `SetWinEventHook` | MISSING | B | . . x . x . x | 11 |  |
| user32.dll | `ShowWindow` | claimed (impl-win-gui-a in tree) | B | x x x x x x x | 30 | returned 0/NULL 11 x |
| user32.dll | `SystemParametersInfoW` | MISSING | BR | x x x x x x x | 9 |  |
| user32.dll | `ToUnicodeEx` | MISSING | B | . . x . x . x | 4108 | returned 0/NULL 3416 x |
| user32.dll | `TrackMouseEvent` | claimed (spec-win-gui-b) | B | . . x . x . x | 6 |  |
| user32.dll | `TranslateMessage` | green (in tree) | BC | x x x x x x x | 150 | returned 0/NULL 102 x |
| user32.dll | `UnhookWinEvent` | MISSING | B | . . x . x . x | 11 |  |
| user32.dll | `UnregisterClassW` | green (in tree) | B | x . x x x x x | 2 |  |
| user32.dll | `UnregisterDeviceNotification` | MISSING | B | x . x x x x x | 2 |  |
| user32.dll | `UnregisterPowerSettingNotification` | MISSING | B | x . x x x x x | 3 |  |
| user32.dll | `WindowFromDC` | MISSING | G | . . . . . x x | 2 |  |
| user32.dll | `WindowFromPoint` | claimed (spec-win-gui-b) | B | . . x . x . x | 21 |  |


## 9. Proposed batches (each one PR for an implementer; smallest first)

The gate of every batch is a check that exists or is part of the batch; the S-numbers are the ladder of the task.

| # | Batch | Content | Gate |
|---|---|---|---|
| **0** | **Export surface** (mechanical, one PR) | every name in section 8 and 5 that is MISSING exists as an export of the right DLL (`ole32`, `setupapi`, `pdh`, `shell32`, `iphlpapi`, `userenv`, `wtsapi32`, `dwmapi`, `oleacc`, `uiautomationcore`, `version`, `winmm`, `netapi32`, `dhcpcsvc`, the two `winrt` API sets, `shcore` scaling, `power-base`, `comctl32`, `imm32`, `secur32`, `bcryptprimitives!ProcessPrng` real) and returns failure with a plausible last error where section 6 allows it, success with an empty result otherwise (ETW) | `GetProcAddress`/delay-load of all 1,340 imports resolves; Chrome reaches its first window or exits with an error, not a loader crash |
| **1** | **IPC plumbing** (spec-win-ipc) | `CreateNamedPipeW`, `ConnectNamedPipe`, `DisconnectNamedPipe`, `SetNamedPipeHandleState`, `TransactNamedPipe`, `PeekNamedPipe`; `CreateIoCompletionPort`, `GetQueuedCompletionStatus`, `PostQueuedCompletionStatus`, `CancelIoEx`, `GetOverlappedResult`; `DuplicateHandle` across processes and `DUPLICATE_CLOSE_SOURCE`; `CreateFileMappingW`/`MapViewOfFile`/`FlushViewOfFile`/`K32GetMappedFileNameW`; `CreateProcessW` for the three child kinds with `STARTUPINFOEX`, `InitializeProcThreadAttributeList`/`UpdateProcThreadAttribute` (`0x20002` handle list, `0x20007`, `0x2000E`), handle inheritance; `OpenProcess`, `GetExitCodeProcess`, `ReadProcessMemory`/`WriteProcessMemory` on itself, `GetThreadContext`, thread priority and cycle-time queries | two Poly processes talk over a named pipe with overlapped I/O on an IOCP (host test + a ring-3 test); `wintrace` style comparison of the browser's first 100 calls |
| **2** | **kernel32 remainder for S1** | files: `FindFirstFileExW`, `GetFileAttributesExW`, `GetFileInformationByHandleEx`, `SetFileInformationByHandle`, `LockFileEx`/`UnlockFileEx`, `ReplaceFileW`, `GetFinalPathNameByHandleW`, `GetLongPathNameW`, `GetSystemDirectoryW`; loader/debug: `LoadLibraryExA`, `GetModuleHandleExA/W`, `AddVectoredExceptionHandler`, `RtlCaptureStackBackTrace`, `WerRegisterRuntimeExceptionModule`, `OutputDebugStringW`, `SetThreadDescription`; memory: `DiscardVirtualMemory`, `HeapSetInformation`, `PrefetchVirtualMemory`, `VirtualAlloc2`/`MapViewOfFile3`/`UnmapViewOfFile2` (renderer); sync: `RegisterWaitForSingleObject`, `UnregisterWaitEx`, `SwitchToThread`; NLS/time/system: `GetLocaleInfoEx`, `GetStringTypeW`, `CompareStringEx`, `LCMapStringEx`, `GetUserGeoID`, the time-zone functions, `GetLogicalProcessorInformation(Ex)`, `GetProductInfo`, `VerifyVersionInfoW`/`VerSetConditionMask`, `GetComputerNameExW`, `IsWow64Process2` | S1 files and registry names of section 5 of the traces open as on Windows |
| **3** | **S1: advapi32 + registry + ETW** | `RegNotifyChangeKeyValue`, `RtlFormatCurrentUserKeyPath`, the security-descriptor building the browser does for pipes and sections (`InitializeAcl`, `ConvertStringSecurityDescriptor...`, `BuildSecurityDescriptorW`, `BuildExplicitAccessWithNameW`, `SetEntriesInAclW`, `GetNamedSecurityInfoW`, `SetNamedSecurityInfoW`, `GetSecurityInfo`, `LookupAccountNameW`), ETW stubs (`EventRegister`, `EventSetInformation`, `TraceEvent`, `RegisterTraceGuidsW`, `UnregisterTraceGuids`, `GetTrace*`) | (with spec-win-sec) |
| **4** | **S1: windows without a screen** | the ~60 user32/gdi32 calls of the headless browser (hidden message windows, DPI, `SystemParametersInfoW`, `GetSysColor`, `RegisterWindowMessageW`, `SetPropW`/`RemovePropW`, `GetWindowThreadProcessId`, `EnumThreadWindows`, `GetClassNameW`, keyboard layout, `GetStockObject`/`CreateFontIndirectW`/`GetTextMetricsW`/`GetDeviceCaps`) on top of ADR 0131's window manager; COM and WinRT entry points (`CoInitializeEx`, `CoCreateInstance`, `Ro*`, `Windows*String*`) as working-enough stubs | **S1 passes**: `chrome.exe --headless --no-sandbox --disable-gpu --dump-dom` prints the 440 bytes (a boot test like the 7-Zip one, `windows.json` has the digest) |
| **5** | **S1: the network service and crash handling** | `iphlpapi` (`GetAdaptersAddresses`, `NotifyAddrChange`, `CancelIPChangeNotify`, `GetIfTable2`, `FreeMibTable`), `dhcpcsvc`, `ws2_32` remainder (`WSAEventSelect`, `WSAEnumNetworkEvents`, `WSALookupService*`), `netapi32`, `secur32!GetUserNameExW`, `crypt32` (`CertAddStoreToCollection`, `CertControlStore`, `CryptProtectMemory`), `winhttp!WinHttpGetIEProxyConfigForCurrentUser`, `dbghelp`/crashpad's calls | the network service of S1 stays up |
| **6** | **S2: the sandbox** (spec-win-sec) | section 4 in full: alternate window station + desktop, `CreateRestrictedToken`, `DuplicateTokenEx`, `SetTokenInformation(IntegrityLevel, DefaultDacl)`, token impersonation on a thread, jobs with extended limits + UI restrictions + completion port, `CreateProcessAsUserW` with a restricted token and attribute lists, mitigation policies, AppContainer SID derivation, the target side (lowering the token, closing handles); and **access checks that *deny* the sandboxed child** (file, registry, named objects) the way the restricted token must | S2 prints the 440 bytes with the sandbox on and a restricted child cannot open a file the browser can (kernel test) |
| **7** | **S3: a visible window** (spec-win-gui) | the S3 delta of section 8 (22 user32 calls, `GetWindowDC`, `EnumDisplayMonitors`, `EnumDisplaySettingsW`, `QueryDisplayConfig`, `SetWinEventHook`, `ToUnicodeEx`, `dwmapi` frame calls, 4 gdi32 DC/region calls) and the message sequence of section 7 delivered to a `Chrome_WidgetWin_1` window, `WM_PAINT`/`WM_ERASEBKGND`, mouse and key messages, `WM_CLOSE`; fonts for the page (Georgia, Arial, Consolas in the screenshot are Windows'; Poly's own fonts will differ: the comparison is the text drawn, not the pixels) | S3: the title changes to `... clicks=1 keys=poly moved=1 ...` through Poly's own input, and `WM_CLOSE` ends the browser with 0 |
| **8** | **DirectWrite** (spec-win-dwrite) | `dwrite!DWriteCreateFactory` (renderers) and what Skia calls on the factory: font collections, glyph metrics, rasterisation; `DWriteCoreCreateFactory` is also tried | text renders in S3 |
| **9** | **S4: network** | real TCP and TLS through `ws2_32` (S4's APIs are the S1 ones: only `WSADuplicateSocketW`, `NetApiBufferFree`, `SHGetSpecialFolderPathW`, `SetRect` are new); name resolution (`GetAddrInfoW`, `dnsapi`), the proxy/connection checks `winhttp` and `iphlpapi` make; certificate verification (`crypt32`) | S4 loads `http://example.com` and an https page |
| **10** | **S5: GPU** | `d3d11!D3D11CreateDevice`, `dxgi!CreateDXGIFactory1/2`, `dcomp!DCompositionCreateDevice(2,3)`, `DXCoreCreateAdapterFactory`, `D3DCompile` (ANGLE's D3D11 path); a software adapter is enough | optional; lowest priority |

## 10. Coverage entries added

`coverage/windows/api`: new entries for what Chrome calls and the map did not list (source `chrome-win-trace`): `kernelbase.dll`
`AppContainerRegisterSid`, `CreateAppContainerToken`, `FlsGetValue2`, `IsEnclaveTypeSupported`, `IsUserCetAvailableInEnvironment`;
`userenv.dll` `DeriveAppContainerSidFromAppContainerName`; `dnsapi.dll` `DnsIsZtEnabled`; `mdmregistration.dll`
`IsDeviceRegisteredWithManagement`. (Not entries: `shlwapi` `#219` and `#437` are ordinals of names the map already has.)

## 10a. Per-API specs added by `spec-chrome-win-2`

Twelve entries that the sandbox broker or S1 call and `spec-win-sec` left out (sheets `specs/windows/<dll>/<Symbol>.yaml`, tests
`<Symbol>.tests/`, split about 70 % visible / 30 % held out; Windows reference `windows.json` beside the visible tests):
advapi32 `BuildExplicitAccessWithNameW`, `BuildTrusteeWithSidW`, `BuildTrusteeWithNameW`, `SetEntriesInAclW`, `GetExplicitEntriesFromAclW`,
`GetNamedSecurityInfoW`, `SetNamedSecurityInfoW`, `LookupAccountSidW`, `LookupAccountNameW`; kernel32 `GetProcessMitigationPolicy`,
`SetProcessMitigationPolicy`; userenv `DeriveAppContainerSidFromAppContainerName`. `GetSecurityInfo` was already done by `spec-win-sec`.
Test programs import `advapi32.dll` and `userenv.dll` (`tools/pipeline/src/windows.rs` allows both). Highlights an implementer needs: the
ACE order and merge rules of `SetEntriesInAclW`, the inheritance propagation rules of `SetNamedSecurityInfoW` (control bits 0x1404 /
0x404, ACE flag 0x10), the fixed names and sizes of `LookupAccount*`, the exact lengths each mitigation policy accepts and the policies that
cannot be set (DEP 50, CFG and shadow stack 5, strict handle checks need both flags), and the hash-based, case-insensitive,
64-character-limited AppContainer SID.

## 11. Cross-references and open problems

* spec-win-sec: tokens, ACLs, SDs, jobs. spec-win-ipc: pipes, IOCP, attribute lists, `DuplicateHandle`, sections.
  spec-win-dwrite: DirectWrite. spec-win-gui-a/b: the window manager's calls and message rules. This map orders their
  entries, it does not duplicate them.
* **Open** (state after `spec-chrome-win-2`): (1) *closed*: sandboxed renderers are traced (section 4a). What is still not seen from
  inside a target: the work done under a call by Windows, calls through pointers, and anything after the target has closed its
  trace handle; the Microsoft-signed-only mitigation of the renderers is switched off for the trace (event 3033 documents what it
  refuses). (2) *closed for S1*: per-API fault tolerance (section 6a, `POLYTRACE_FAULT`); S2 to S5 and combinations of failures are
  not injected, and `TAIL`/`NORETURN` thunks and `UpdateProcThreadAttribute`/`CreateProcessAsUserW` cannot be skipped. (3) S3 on the
  owner's PC (its desktop is shared); S3 and S4 were recorded on homeserver2 only. (4) S5 as a visible window with the default GPU
  flags. (5) `poly-chrometrace/1` has no Poly-side comparer yet: `tools/chrometrace.py` reads both sides' runs; `tools/sandboxdiff.py`
  compares sandboxed and unsandboxed runs; `cargo xtask win chrome-trace` is the follow-up. (6) The sandbox's *own* object accesses
  (named pipes, sections, window station and desktop ACLs) are in the browser's calls (section 4); the broker's decision of which
  token level a given process type gets is observed only through the creation-time mitigation words (section 4a), not through the
  token (`GetTokenInformation` of a live target needs a Windows tool; not done). (7) `SetProcessMitigationPolicy` of the *targets*
  (section 4a lists that they call it) is not decoded: the policy structures are not logged.
* Nothing of Chrome, Windows or any other third-party binary was read, decompiled or disassembled; import and export
  *names* and the traces' observations only.

## 12. Hand-back record

* Branch `spec-chrome-win`, pushed (`origin/spec-chrome-win`); the head is in the final reply. Nothing merged, no PR opened.
* New: `specs/windows/apps/chrome/` (README, `chrome.yaml`, `gap-map.md` = this file, pages, drivers, tools, 7 reduced traces,
  `windows.json`), `tools/wintrace/polytrace-agg/` (the counting logger; added to `ci lint`'s `fmt (wintrace)` list),
  `tools/xtask/src/wintrace.rs` (`trace-kit` with several programs, `--agg`, `--skip`; the Chrome signature table
  `CHROME_SIGNATURES`; the generated `polytrace-preload.txt`), 8 coverage entries and the source `chrome-win-trace`
  (`coverage/index.json`), README progress section regenerated.
* Commands and results: `cargo xtask win trace-kit chrome.exe chrome.dll chrome_elf.dll --agg` (built natively on Windows,
  63 proxies, about 6 minutes; the default kit's `polytrace.dll` is byte-identical before and after, checked);
  `cargo test -p xtask wintrace` 9 passed, 0 failed; `cargo xtask coverage check` ok (845,418 entries);
  `cargo xtask coverage readme` updated; `polyd 'cargo xtask ci lint'`: passed (1,748 s; it failed once on a dead-code warning in `wintrace.rs`, fixed).
  Recording: `drivers/run_stage.ps1` (S1, S2, S4, S5 headless, 2 runs each, traced and untraced), `drivers/remote_run.ps1`
  through `interactive_task.ps1` on homeserver2 (S3, S4 window, S5 window: untraced and traced, 2 runs each), `drivers/ablate.ps1`
  (28 runs). Screenshot of S3 on homeserver2 (sandbox on, untraced):
  `C:/Users/Marc.Blattmann/poly-dev/handoff/spec-chrome-win-s3-window-1.png`.
* Never run: anything of Chrome on Poly (it does not start yet), `polyd` boot tests (no kernel change), the implementer-side
  comparison of `poly-chrometrace/1` (no comparer yet).

### Round 2 (`spec-chrome-win-2`)

* Branch `spec-chrome-win-2` (from `origin/main`), pushed; nothing merged, no PR opened.
* New: `tools/wintrace/proxy/common_agg.rs`, the `ctl` module of `tools/wintrace/polytrace-agg` (fault injection, mitigation probe/relax,
  inherited trace sections, `OutputDebugString` diagnostics), `tools/sandboxdiff.py`, `traces/s2-sandboxed.trace.json`,
  `traces/s2-sandbox-targets.json`, `traces/s1-fault-injection.csv`, sections 4a, 6a and 10a above, 12 spec sheets with tests under
  `specs/windows/{advapi32.dll,kernel32.dll,userenv.dll}/` (27 tests: 14 visible, 13 held out), `tools/pipeline/src/windows.rs` allows
  `advapi32.dll` and `userenv.dll` imports (the line also changes in `spec-win-sec`: merge by taking the union).
* Machine: homeserver2 for everything (Windows 11 Home 25H2, 26200.9711, elevated administrator over SSH); the owner's PC only to build the kits.
  The homeserver2 sshd reset connections for about an hour while the 220-run fault batch was running (it is the batch's load).

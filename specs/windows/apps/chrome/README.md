# Chrome for Windows on Poly: scenarios, recorded traces, gap map

Spec side (agent `spec-chrome-win`) of the roadmap goal "Chrome for Windows runs on Poly with its sandbox on".
Everything here is **observed behaviour** of the real application on real Windows through Poly's API tracer
(ADR 0090); no Microsoft or Google binary was decompiled or disassembled (CLEAN_ROOM.md). The map that tells an
implementer what to build is [`gap-map.md`](gap-map.md) in this directory (the same text is the hand-off
`spec-chrome-win.md`).

## The pinned application

| | |
|---|---|
| Product | Chrome for Testing, Stable channel |
| Version | 157.0.8103.38, win64 |
| URL | `https://storage.googleapis.com/chrome-for-testing-public/157.0.8103.38/win64/chrome-win64.zip` |
| SHA-256 of the zip | `ae0deef27d449e1d7529df7705d609bab102de48464da13567ed3dd2ee1a0586` (210,600,182 bytes) |
| Layout | flat: `chrome.exe` (4.1 MB), `chrome.dll` (311 MB), `chrome_elf.dll`, `chrome_proxy.exe`, `libEGL.dll`, `libGLESv2.dll`, `D3DCompiler_47.dll`, `dxcompiler.dll`, `vk_swiftshader.dll`, `vulkan-1.dll`, `*.pak`, `locales/` ... |

It is downloaded at build time and checked against the SHA-256; it is never committed or redistributed.

## The ladder

Each stage is a scenario with observable results. The pages are in `pages/` (SHA-256 in `windows.json`).

| Stage | Command (after unpacking the zip) | Observable result on Windows |
|---|---|---|
| **S1** | `chrome.exe --headless --no-sandbox --disable-gpu --user-data-dir=<fresh> --dump-dom file:///<dir>/pages/s1.html` | stdout is the serialised DOM, 440 bytes, SHA-256 `7b79f8b4...813b` (below), exit code 0, in 3-5 s. Stderr has two `external_registry_loader_win.cc` lines (a missing registry value), nothing else. |
| **S2** | S1 without `--no-sandbox` | the same stdout (same bytes), exit code 0, in 2-4 s |
| **S3** | `chrome.exe --disable-gpu --no-first-run --user-data-dir=<fresh> --window-position=40,40 --window-size=900,500 file:///<dir>/pages/s3.html`, driven by `drivers/s3_driver.ps1` | a top-level `Chrome_WidgetWin_1` window titled `poly-s3 clicks=0 keys= moved=0 - Google Chrome for Testing`; after a mouse move, one click and the keys p-o-l-y: `poly-s3 clicks=1 keys=poly moved=1 - ...`; the screenshot shows the page's text (Georgia, Arial bold, Consolas, German umlauts); `WM_CLOSE` to the window ends the browser with exit code 0 |
| **S4** | S3 with `http://example.com/`, then `https://www.iana.org/` through the address bar (`Ctrl+L`); headless: `pages/s4.html` frames both | titles `Example Domain - ...` then `Internet Assigned Numbers Authority - ...`; headless DOM `<p id="r">http=1 https=1</p>` (831 bytes) |
| **S5** | headless without `--disable-gpu` (default GPU flags); and the S3 window without it | the same DOM / title as before; only the GPU process differs (see the gap map) |

S1's DOM (CRLF line ends, as Windows' console/pipe writes them):

```
<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>Poly Chrome Test</title></head>
<body>
<h1 id="h">Hello Chrome</h1>
<p id="p">script-ran 42</p>
<div id="b" style="width:200px;height:50px">box 200</div>
<script> ... </script>

</body></html>
```

## What was traced, and how

`cargo xtask win trace-kit <chrome.exe> <chrome.dll> <chrome_elf.dll> --agg --out <dir>` (this branch: ADR 0090's
kit extended, `tools/xtask/src/wintrace.rs`, `tools/wintrace/polytrace-agg`):

* the three programs get the import names of their 63 system DLLs changed to proxies (`pt_<dll>.dll`, 1,303
  functions: every static **and delay-loaded** import of Chrome's three binaries); only `chrome_elf.dll` (Chrome's
  own), `bthprops.cpl` and `winspool.drv` are left alone;
* the logger is the *counting* form: Chrome makes millions of calls and starts about 15 processes, so a call is
  written in full only for a function's first four calls and for every new string it is given (paths, key names,
  DLL names, pipe names, command lines); everything else is a counter per function (calls, first call, results
  zero/not zero/-1, the first distinct last errors and results); one `polytrace-<pid>.jsonl` per process, the
  trace lives in a memory-mapped view (no file handle for the sandbox's handle closer);
* proxies of delay-loaded DLLs are loaded at start-up of child processes (`polytrace-preload.txt`), because Chrome
  narrows the DLL search path to System32 in them;
* `tools/chrometrace.py run` reduces a run to its processes (type from `--type=`), `merge` keeps **only what two
  runs share** (a function called in both, with the minimum and maximum of its counts; the objects named in
  both), `canon` writes the committed form below. Run each stage at least twice; the stable facts only are kept.

**What this cannot see**, honestly: calls made through pointers (`GetProcAddress` results, vtables, COM) are
invisible, but the *names* resolved with `GetProcAddress` are recorded (the gap map lists them); direct system
calls (`syscall` instructions in Chrome's own code) are not seen; the NT syscalls under `kernel32` are Windows'
business, not Chrome's, and are not interesting for Poly (Poly provides `kernel32` itself).

**The sandbox and the tracer.** With the sandbox on, the traced kit's *renderers* die at start (the browser
launches them with `CreateProcessAsUserW`, they never reach the logger, Chrome retries and stalls): the tracer's
unsigned proxy DLLs are loaded into a process Chrome confines (restricted token, job, mitigations: which of them
refuses the image is not known; an ACL for `ALL (RESTRICTED) APPLICATION PACKAGES` and `RESTRICTED` on the kit
directory did not help). So **the sandboxed stage S2 has the browser (the sandbox's broker: every token, job,
desktop, mitigation and `CreateProcessAsUser` call), the GPU process, the network service and crashpad traced,
but no renderer**; S2's renderer API set is S1's (same code, same DLL) plus the sandbox target services
(token lowering, handle closing), whose functions are named by the strings the children pass to
`GetProcAddress`. The scenario results of S2 (stdout, exit code) are from the untraced program.

## Files

| Path | What |
|---|---|
| `gap-map.md` | **the gap map** (also the hand-off `spec-chrome-win.md`): APIs by area and DLL against `coverage/windows`, batches |
| `chrome.yaml` | the spec sheet (`windows/api/apps/chrome`): oracle, scenarios, expected results |
| `pages/` | `s1.html` (DOM + script + layout), `s3.html` (text, mouse, keys), `s4.html` (http + https frames) |
| `drivers/run_stage.ps1` | S1, S2, S4 (headless), S5: one run, traced or not |
| `drivers/s3_driver.ps1`, `interactive_task.ps1` | S3, S4 (visible): the window, `SendInput`, screenshot, `WM_CLOSE`; the second runs it in the interactive session through a scheduled task, as `poly-oracle.ps1 -Interactive` does |
| `drivers/ablate.ps1` | which delay-loaded DLLs Chrome can do without (S1) |
| `drivers/remote_run.ps1` | runs one S3/S4 round on homeserver2 (after `scp` of the kit and the zip; see below) |
| `tools/chrometrace.py`, `covmap.py`, `gapmap.py`, `gapmap_md.py` | reduce, merge, join with `coverage/windows`, write the tables |
| `traces/s<N>.trace.json` | the committed reduced traces (`poly-chrometrace/1`, below) |
| `traces/s2-sandboxed.trace.json` | S2 with **every** process type traced, sandbox on (`spec-chrome-win-2`, gap map section 4a) |
| `traces/s2-sandbox-targets.json` | what the confined children do/are refused, S2 against S1 (`tools/sandboxdiff.py`) |
| `traces/s1-fault-injection.csv` | per-API fault tolerance of S1, 220 APIs (gap map section 6a) |
| `tools/sandboxdiff.py` | `reduce <S1 runs> --sandboxed <S2 runs> <out.json>`: denied / failed / gone / new / outcome per process type |
| `traces/s1-chrome-elf-init.trace.json`, `...warm.trace.json` | the call-site trace of chrome_elf.dll's and chrome.exe's start-up in the S1 browser process, cold and warm registry (`poly-callsite-trace/1`, `spec-chrome-win-3`, below) |
| `tools/callsites.py` | `reduce` (raw runs -> the trace), `compare` (a Poly run against it, call by call), `flow` (which call's result a later call uses), `show` |
| `drivers/run_s1_callsites.ps1` | one S1 run of the `polytrace-sync` kit (chrome_elf.dll + chrome.exe only), browser trace in `<work>\tr\<tag>-run<n>\polytrace.jsonl` |
| `windows.json` | the recorded scenario results: exit codes, output digests, titles, machine facts |

**Window titles** have an EN DASH (U+2013) before `Google Chrome for Testing`; `windows.json` writes it as `-`.
**Where each stage was recorded**: S1, S2, S4 (headless), S5 on the owner's PC (Windows 11 Enterprise 25H2, 26200.9711);
S3 and S4 (window) on homeserver2 (Windows 11 Home 25H2, 26200.9711), interactive session through a scheduled task, as ADR 0075/0097 do.
The traces of S3 and S4 (window) were taken with `--no-sandbox` (the traced renderers do not run sandboxed); their untraced
runs (sandbox on) give the same titles and exit codes (`windows.json`).
The kit's file hashes in each trace are those of the recorded build (the logger source was reformatted afterwards).

## `poly-chrometrace/1`

The `wintrace` boot group compares a call-by-call sequence (ADR 0110). Chrome's sequences are millions of calls
per process, interleaved over dozens of threads and processes: not comparable. The committed form keeps what is
stable, per stage and per process type (`browser`, `renderer`, `renderer:extension`, `renderer:webui`,
`gpu-process`, `utility:network.mojom.NetworkService`, `utility:storage.mojom.StorageService`,
`crashpad-handler`):

```json
{"format":"poly-chrometrace/1","stage":"s1","recorded":{...},"runs":2,
 "process_types":{"browser":{"processes_per_run":[1,1],
   "calls":[["kernel32.dll","CreateFileW",n_min,n_max,"ok|mixed|never-ok",[last errors seen],rank], ...],
   "objects":[["CreateFileW","<profile>\\Default\\History"], ...],
   "unstable_calls":0,"unstable_objects":174}}}
```

`calls` are the functions called in both runs ordered by first call (`rank`: position among the process's
functions, not a position in a sequence); `n_min`/`n_max` the call counts over the two runs; `ok` = every result
was non-zero and non-(-1), `never-ok` = never, `mixed`; `objects` are the strings given to file, registry,
named-object, module and process functions, normalised (`<profile>`, `<app>`, `<n>`, `<hex>`).
A comparison on Poly (once Chrome runs there) is: every `calls` entry is called at least once
(`n >= 1`), a function that is `ok` on Windows returns non-failure on Poly, a process type that exists on Windows
exists on Poly, every `objects` entry that names an object Poly must provide (not `<profile>` files Chrome
creates) is opened. `tools/chrometrace.py` reads both sides' raw runs; porting the comparison into `cargo xtask
win chrome-trace diff` is the follow-up.

## Opt-in tracer modes (counting logger, `spec-chrome-win-2`)

Environment variables read by `polytrace.dll` of the `--agg` kit at start (`tools/wintrace/polytrace-agg`, module `ctl`); none is set by default and
the 7-Zip kit (`tools/wintrace/polytrace`, `proxy/common.rs`) is unchanged. The counting kit's proxies use `proxy/common_agg.rs`, the same thunk
plus one test that skips the real call.

| Variable | Effect |
|---|---|
| `POLYTRACE_FAULT=<fn>:<ret>:<err>[:<after>[:<count>]];...` | the named API is not called after its first `<after>` calls: it returns `<ret>` and sets the last error `<err>`; a record `{"inject":1,...}` is written for the first 8 hits per process |
| `POLYTRACE_FAULT_IN=<text>` | rules apply only in a process whose command line contains `<text>` (`browser`: no `--type=`) |
| `POLYTRACE_RELAX=<mask0>[,<mask1>]` | bits cleared from the creation-time mitigation words (`UpdateProcThreadAttribute` attribute `0x20007`) the program passes; `0x300000000000` clears "Microsoft-signed binaries only" so that a sandboxed renderer accepts the unsigned proxies. The words (`{"mitigation":1,...}`) and the first bytes of every other attribute (`{"attr":1,...}`) are logged either way |
| `POLYTRACE_INHERIT=1` | every child made with `CreateProcessAsUserW` and a handle list gets its trace file as an inherited file-backed section (`polytrace-c<n>.jsonl`, handle value in `POLYTRACE_HANDLE`); needed where a confined child cannot create a file |
| `OutputDebugStringA` | a child that cannot make its trace file says why (`polytrace[<pid>]: step=<n> error=<code>`; 1 create, 2 mapping, 3 view, 4 inherited section, 5/6 the browser's section) |

A sandboxed run: `POLYTRACE_RELAX=0x300000000000 POLYTRACE_INHERIT=1 chrome.exe --headless --disable-gpu --user-data-dir=<fresh> --dump-dom file:///.../s1.html`
from the kit overlaid on a copy of the application, then `tools/chrometrace.py run`, `merge`, `canon` and `tools/sandboxdiff.py reduce`. Do not use
`taskkill /IM`; the drivers in `drivers/` kill only the tree of the process they started.

## Call-site trace of chrome_elf's start-up (`spec-chrome-win-3`)

Why: on Poly S1 dies in the browser process inside chrome_elf.dll's crash-reporting set-up, reading `[NULL+0x10]` with no
API call before the fault. `traces/s1.trace.json` cannot say what Windows does *between* two calls (it has ranks, no sequence and
no call sites), so these traces are the sequence of **every call that `chrome_elf.dll` and `chrome.exe` make through their imports
from process start to the first `AddVectoredExceptionHandler`** (the end of chrome_elf's crash-reporting set-up), each with its
**caller as module+offset**, its arguments, result, last error and what it left in memory.

| File | Registry state of the run | Calls |
|---|---|---|
| `traces/s1-chrome-elf-init.trace.json` | **cold**: `HKCU\Software\Chrome for Testing` and `HKCU\Software\Google\Chrome for Testing` do not exist (a machine that never ran Chrome for Testing; Poly's registry at start) | 705 (433 from chrome_elf.dll, 272 from chrome.exe) |
| `traces/s1-chrome-elf-init.warm.trace.json` | **warm**: both keys exist (written by an earlier run: the blocklist beacon, the ThirdParty status codes, `usagestats`) | 853 (581 + 272) |

The state matters: with the beacon key present chrome_elf goes through a 148-call detour that a cold start does not take (an extra
`VerifyVersionInfoW`, the registry writes, `CreateMutexW`, the `ReadProcessMemory`/`WriteProcessMemory` pair on an ntdll function, 15 more
`NtCreateKey`). Compare Poly with the **cold** file; the warm one is what a second run on the same registry does.

How: `cargo xtask win trace-kit chrome_elf.dll chrome.exe --sync --out <dir>` (only the two programs are traced, as `cargo xtask
chrome-win run --kit chrome_elf.dll,chrome.exe` does on Poly: the same logger, `tools/wintrace/polytrace-sync`, the same signature table),
overlaid on a copy of the application; `drivers/run_s1_callsites.ps1` runs S1 (`--headless --no-sandbox --disable-gpu --dump-dom`,
`POLYTRACE_SYNC=1`; `-Cold` first saves and deletes the two keys and puts them back afterwards) and the browser process writes
`polytrace.jsonl`. Every run printed the 440-byte DOM with SHA-256 `7b79f8b4...813b`, as without the kit. Machines: homeserver2 and the
owner's PC (both Windows 11 25H2, build 26200.9711); the `recorded` header of each file says which runs were where. The 7-Zip logger
(`tools/wintrace/polytrace`) is untouched.

The logger (`polytrace-sync`, extended in this change) adds to each record, when `POLYTRACE_SYNC` is set (`POLYTRACE_DUMP=0` turns it
off): `rm`/`ro` the module and offset of the return address; `ac` the class of every pointer and handle argument; `rc` the class of a
returned pointer or handle; and for about 100 functions `out`, what the call left in memory: strings and counted strings
(`GetModuleFileNameW`, `GetEnvironmentVariableW`, `GetCommandLine*`, `K32GetMappedFileNameW`, ...), `UNICODE_STRING` and
`OBJECT_ATTRIBUTES` (`RtlInitUnicodeString`, `NtOpenKeyEx`, `NtCreateKey`, `RtlFormatCurrentUserKeyPath`), registry data
(`NtQueryValueKey`, `NtSetValueKey`, `RegQueryValueExW`: type, length, data), structures (`SYSTEM_INFO`, `OSVERSIONINFOEXW`,
`STARTUPINFOW`, `MEMORY_BASIC_INFORMATION`, `RTL_CRITICAL_SECTION`, `VS_FIXEDFILEINFO`, `CPINFO`), out handles and pointers
(`DuplicateHandle`, `NtOpenKeyEx`, `ConvertStringSecurityDescriptorToSecurityDescriptorW`), version resources (`GetFileVersionInfo*`,
`VerQueryValueW`), the bytes `ReadProcessMemory`/`WriteProcessMemory` move (the committed files keep how many, not which: they are a system DLL's code), and the frames of `RtlCaptureStackBackTrace`. Memory is read
through `ReadProcessMemory` on the own process, which reports a bad pointer instead of faulting. Pointers and handles are **classes**,
never values: `null`, `-1`, `stack`, `private`, `mapped`, `handle`, `int:<n>`, `image:<module>[+0x<offset>]` (the offset only for
chrome.exe, chrome_elf.dll, chrome.dll).

`poly-callsite-trace/1`: a header (`recorded`, `window`, `how_to_compare`, `landmarks`, `markers`, `noise_sites`) and `calls`, one line
per call:

```json
{"n":685,"fn":"ConvertStringSecurityDescriptorToSecurityDescriptorW","dll":"advapi32.dll","at":"chrome_elf.dll+0x8e315",
 "args":[{"w":"D:(A;;GA;;;SY)(A;;GWGR;;;S-1-15-2-1)S:(ML;;;;;S-1-16-0)"},1,"stack","null"],"ret":1,"out":[{"a":2,"c":"private"}]}
```

`at` is the key (the caller as module+offset; the same on Poly because the kit's copy of a program has its code at the same RVAs).
`"private<-685"` as an argument means: the value call 685 returned or wrote through an out parameter (so a NULL where Windows has
`private<-685` is the divergence, and every later use of it shows). A field that differs between the runs is `"<var>"` (process ids,
string lengths that hold a path, the machine's `SYSTEM_INFO`, the first bytes of an ntdll function); a `<var>` field is not compared.
**Not in `calls`**: `TryAcquireSRWLockExclusive`/`AcquireSRWLockExclusive`/`ReleaseSRWLockExclusive` (about 1,450 per cold run, a dozen
more or fewer from run to run) and the allocator's `VirtualAlloc`/`VirtualFree` (where they fall between the other calls differs from
machine to machine), and a `GetLastError` right after a failed `VirtualAlloc`; they are summarised per call site in `noise_sites`.
The other calls are the same functions from the same sites in the same order in every run.

Compare a Poly run: `cargo xtask chrome-win run --stage 1 --release --kit chrome_elf.dll,chrome.exe` (writes
`target/chrome-win/stage1.ptrace.jsonl`), then `python -I specs/windows/apps/chrome/tools/callsites.py compare
specs/windows/apps/chrome/traces/s1-chrome-elf-init.trace.json target/chrome-win/stage1.ptrace.jsonl`. It drops the calls above,
aligns call n with call n, and prints the first calls that differ (a different function or caller; a result, last error, argument class,
link or out value) and where Poly's sequence ends. `callsites.py flow <trace>` lists the results later calls use (the places where a
NULL on Poly would be a pointer on Windows); `show <trace> <first> <last>` prints calls.

What it cannot see: calls through pointers (the functions chrome_elf gets with `GetProcAddress` are visible only by their names), the
code between calls, direct system calls, and the registers.

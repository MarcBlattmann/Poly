# 0155: Chrome for Windows on Poly: the bring-up tool, stubs for what Poly lacks, images read a page at a time, address space as runs

**Status:** Accepted. Works toward the owner's goal "Chrome for Windows runs on
Poly with its sandbox on", the Windows counterpart of
[0130](0130-chromium-from-pinned-debian-packages.md) (Chromium for Linux).
Builds on [0055](0055-microsoft-redistributables-run-apps-own-copies.md)
(apps bring their own runtimes), [0071](0071-our-ntdll-a-real-pe-dll-built-from-rust.md)
(our `ntdll` and its loader), [0073](0073-windows-file-system-layout.md) (`C:`),
[0079](0079-our-kernelbase-and-kernel32.md) (`kernelbase`),
[0087](0087-7-zip-command-line-on-poly.md) (the import gap tool, the pinned
download), [0114](0114-7-zip-trace-matched-privileges-processor-groups-and-last-errors.md)
and [0128](0128-windows-gui-win32k-in-the-nt-personality-and-the-desktop-link.md)
(the GUI), and [0124](0124-native-desktop-alpine-shell-look-and-builds-on-windows-and-linux.md)
(everything builds on Windows and Linux hosts).

## Context

The owner wants Chrome itself, not the Debian Chromium build, running on the
NT personality with its sandbox on. Chrome is the hardest Windows program the
roadmap names: `chrome.dll` is 310 MB and imports from some fifty DLLs Poly
does not have and about six hundred functions of DLLs it does; it reserves
address space in hundreds of gigabytes; its sandbox needs restricted tokens,
job objects, integrity levels, named pipes and handle duplication across
processes (specs in progress: `spec-win-sec`, `spec-win-ipc`). The spec agent
`spec-chrome-win` records the same stages on real Windows. This ADR is the
implementer's side of the first step: a way to run a stage and read what
failed, a rule for what an unimplemented import does, and the first two
limits the stages hit in Poly itself.

Clean room: Chrome is run as a black box on Poly. Only its PE import and
export *names* are read (CLEAN_ROOM.md); the Windows behaviour implemented
comes from Microsoft Learn and the public SDK headers named in the code.

## Decision

### 1. The program and the tool

**Chrome for Testing 157.0.8103.38, win64** (the stable channel's, from
Google's public bucket), pinned by URL, size (210,600,182 bytes) and SHA-256
(`ae0deef2…0586`). Google publishes no hash for it; the SHA-256 is of the
bytes the first download gave over HTTPS and every later download must equal
it. It is never committed or redistributed: `cargo xtask chrome-win fetch`
downloads it into `<main checkout>/target/chrome-win/<version>/` (shared by
worktrees) and unpacks it with the pure-Rust ZIP reader (`unzip.rs`, which
gained `for_each`), so the command is the same on Windows and Linux hosts
(ADR 0124). (A debug build of `xtask` unpacks the 200 MB in about 4 minutes,
once.)

`cargo xtask chrome-win`:

| Command | Does |
|---|---|
| `fetch` | download, verify, unpack |
| `imports [--markdown FILE]` | every `.exe` and `.dll` of the package against our DLLs (and its own), rolled up: missing DLLs, missing functions, static or delay-load |
| `disk [--fresh-disk]` | `target/poly-chrome-win-disk.img`: Chrome under `C:\Program Files\Chrome`, the launcher, `polystub.dll`, the stages; rebuilt when one of those changed |
| `run [--stage N] [--release] [--timeout S] [--trace]` | boot a stage with no window, collect the serial log, print what it says (loader reasons, stubs hit, faults, Chrome's exit code, whether the expected output appeared) |

The disk is `poly-ext2`'s format with `poly/nt/c` as `C:` and
`poly/nt/registry` beside it (ADR 0073); the kernel mounts it at boot
(`poly.mount=virtio0:/mnt/root`), so the registry hives and `C:` persist on
it as on a real install.

**The launcher.** A Linux program cannot `exec` a PE (the Linux personality
answers `ENOEXEC` for NT images), and the kernel command line cannot carry a
path with a space. So the kernel's first program (`poly.init=`) is
`C:\poly-test\chrome_launch.exe` (`tests/windows/chrome_launch.rs`, built by
`winapp.rs`), which reads `stage<N>.cmd` beside it (program, command line,
directory, seconds), runs Chrome with `CreateProcessW` the way a session
would, waits, prints Chrome's standard output and error between markers and
`LAUNCH-EXIT code=0x…`. The stages are:

| Stage | Command line |
|---|---|
| S1 | `--headless --no-sandbox --disable-gpu --dump-dom file:///C:/poly-test/dom.html` (a script writes `poly 42` into the DOM; the DOM must show it) |
| S2 | S1 without `--no-sandbox` |
| S3–S5 | a visible window, http/https, GPU: later |

### 2. What an unimplemented import does: `polystub.dll`

Chrome's `chrome_elf.dll` and `chrome.exe` alone import 300 functions Poly's
DLLs lack and twenty DLLs it does not have. A static import that does not bind
ends the process before its first line runs, with one `NTSTATUS` and no word
about which of a thousand imports it was; implementing a thousand functions to
find out which of them matter is the wrong order.

**Decision.** Poly's loader binds an import (static or delay-load) that it
cannot bind, because the DLL or the function is missing, to an export of a
generated DLL, **`polystub.dll`**, when that DLL is on the search path, and
logs it. Without `polystub.dll` nothing changes: every normal boot loads and
fails as before.

- The export for `(dll, function)` is named `<dll, lower case, without
  .dll>$<function>` (`<dll>$#<n>` for an import by ordinal); the key is the
  DLL as the *importing module* spells it (`poly_ldr::stub_key`), so
  `KERNEL32.dll` and `api-ms-win-core-file-l1-1-0.dll` are different stubs of
  the same function; no API-set resolution is needed to generate the DLL.
- A stub, when called, logs `stub: dll!name` on the kernel log
  (`PolyDebugPrint`, below) and fails the way Microsoft documents for a
  function a system does not provide: `FALSE`/`NULL`/0 with
  `ERROR_CALL_NOT_IMPLEMENTED` (120) for the Win32 API, `E_NOTIMPL` for COM and
  DirectX style DLLs (`dxgi`, `d3d11`, `ole32`, `dwrite`, … per DLL, in
  `poly-stubs`), `STATUS_NOT_IMPLEMENTED` for `ntdll`. A per-function kind is
  possible.
- **Only static and delay-load binding falls back**, because only those are a
  program's *requirement*. `LoadLibrary` of a missing DLL and `GetProcAddress`
  of a missing function fail truthfully (`ERROR_MOD_NOT_FOUND`,
  `ERROR_PROC_NOT_FOUND`), because programs probe optional features that way
  and take another path when they fail (Chrome loads and probes dozens of
  system DLLs).
- The DLL is **generated from the app's own import tables** by `xtask`
  (`crates/poly-stubs`, which builds a PE with `poly_ldr::builder`): one
  32-byte stub per unresolved import, three small common routines (one per
  kind). 2,009 stubs, 104 KiB for Chrome. It is installed on the disk, in
  `C:\Windows\System32`, not embedded in the kernel. Real behaviour replaces a
  stub simply by being implemented in a real DLL, which the loader prefers; no
  list of stubs has to be kept in step.
- The coverage map is not touched: a stubbed function stays `missing` there.
  `chrome-win imports` reports what is stubbed.

**Loader diagnostics.** The loader keeps the reason the last load failed
(`Loader::take_note`: which module could not load which DLL, which `dll!name`
was not found) and which imports it bound to stubs (`take_stubbed`); `ntdll`
logs them through a new Poly service **`PolyDebugPrint(text, length)`**
(`TABLE_POLY` + 2; logs up to 512 bytes as `nt: pid N: text`; exported from
`ntdll` for the stub DLL; a debugging aid, not an interface) at process start
failure, `LdrLoadDll` failure, delay-load failure and after every load that
bound stubs. The serial log of a stage therefore names every missing piece.

### 3. Images are read a page at a time

The first hard limit: the kernel's heap hands out at most 4 MiB at once
(`kernel/src/heap.rs`, one buddy block), and the NT personality kept an
image as one `Arc<[u8]>` laid out at its RVAs. `chrome.exe`'s image is
4.3 MB; `chrome.dll`'s is 310 MB.

**Decision.** `ImageSection.image` is an `Arc<dyn ImageBytes>`
(`personalities/nt/src/image.rs`). An image up to 1 MiB is laid out in memory
as before (`Held`). A bigger one is `Lazy`: when its section is made only its
headers are read (through `ImageFile`: a VFS file's pages, or bytes), and the
kernel's `PageSource` (`ImagePages`) asks `read_at(rva, …)` when a view first
touches a page. A page is made from the file (zero past a section's raw
data) and, in a view not at the preferred base, with the base relocations of
that page applied (`DIR64` and `HIGHLOW`; a value across a page boundary is
fixed whole; the header's `ImageBase` is stored). The relocation table is
checked whole when the view is made (`poly_pe::relocate`'s rule) and indexed
by page. `SectionSource::Image` and `ViewSource::Image` carry the
`ImageBytes`. `poly_pe::Pe::load_plan_in(file_len)` checks the sections
against the file's length when only the headers are at hand.

A host test makes a 16 KiB image with relocations (one across a page boundary)
and checks that a lazy image and a held one read the same bytes, at the
preferred base and at three other bases, in whole and in pieces. A lazy image
shares the page cache with every process that maps it.

Known limit: the executable of a process is still handed over as `&[u8]`
(`start_process_from`), copied once (a file over about 4 MiB cannot be); it
fits Chrome today (`chrome.exe` is 4.13 MB) and should become a VFS file
like DLLs.

### 4. The state of an allocation's pages is a run list

The second: `VirtualMemory::allocate` made a `Vec<PageState>`, 8 bytes per
4 KiB page, for every reservation; Chrome's allocator and V8 reserve tens to
hundreds of GiB (2 MiB of vector per GiB; the first reservation, 4 GiB, asked
the heap for 8 MiB). **Decision.** `pagemap::PageMap` keeps a sorted list of
runs of equal state; a reservation is one run, a commit splits and merges
runs. `Allocation.pages` and `NewView.pages` are `PageMap`s; the operations the
model used on slices (`fill`, `contains`, `get`, runs of equal state) are its
methods. A host test checks it against a flat list over random fills.

### 5. First implementations, in the order the stages fail

`bcryptprimitives.dll` (`ProcessPrng`, which Chromium's base library asks for
before anything else and stops the process without); `kernelbase`'s
`LoadLibraryExA`, `LoadLibraryA`, `OutputDebugStringA/W` (no debugger: does
nothing, as Learn says), the user's locale and string classification
(`GetUserDefaultLCID`, `GetUserDefaultLocaleName`, `IsValidLocale`,
`GetLocaleInfoW` for English (United States), `GetStringTypeW` `CT_CTYPE1`,
`LCMapStringW`/`Ex` for lower and upper case, `CompareStringW`/`Ex` ordinal
after case folding; rules in `poly_kernelbase::locale`, host-tested), and
`ntdll`'s `RtlFormatCurrentUserKeyPath`. The running table of stage, first
failure, fix and commit is in the hand-off file.

## To be confirmed by the spec agent against real Windows

1. The `GetLocaleInfoW` values for en-US (`LOCALE_SSHORTDATE`, `LOCALE_IMEASURE`,
   `LOCALE_IFIRSTDAYOFWEEK`, …), `GetStringTypeW`'s flags for non-ASCII
   characters, and that `CompareStringW`'s linguistic order differs from the
   ordinal one Poly uses.
2. `ProcessPrng`'s return value and what it does with `cbData` 0.
3. The status of a failed `LoadLibraryExA` for a missing DLL (Chrome's own delay-load
   helper raised `0xC06D007E`, `VcppException(ERROR_SEVERITY_ERROR,
   ERROR_MOD_NOT_FOUND)`, when `LoadLibraryExA` failed) and `GetLastError` after it.
4. Whether `OutputDebugString` with no debugger touches the last error.

## Consequences

- `cargo xtask chrome-win run` gives every later step a one-command loop: a
  stage boots in under 20 s on WHPX once built; the serial log says what is
  missing. A normal boot is unchanged: the stub fallback needs a
  `polystub.dll` in `System32`.
- The stub mechanism is general: the next big app gets `polystub.dll` the
  same way from its own import tables (`poly_stubs::build`).
- Every NT image mapping, not only Chrome's, now goes through `ImageBytes`;
  an image above 1 MiB (7-Zip's `7za.exe` is 1.3 MB) is read lazily.
- Memory: a lazy image's pages are private copies made on touch; a
  relocated view re-reads and re-fixes its pages (no copy of the image is
  kept), so a 310 MB `chrome.dll` costs the pages the process touches.
- The 4 MiB heap limit remains for everything else; other places that hold a
  whole file in a `Vec` (`Backend::read_file`, `NtSystem::read_all` for
  `NtCreateUserProcess`) will meet it with big executables.

## Addendum: batches 3 and 5, the network service and crash handling (`chrome-win-b35`)

Chrome's network service and crash handler need a second set of system DLLs
beyond the loader fallback of section 2. They are real DLLs, not stubs,
because Chrome calls them with arguments that must produce plausible
results. Clean room as above: Learn's pages, the public SDK headers, the
export and import *names* of the Chrome package, and the spec agent's
traces (`specs/windows/apps/chrome/traces`).

**ETW with no listener** (`poly-etw`, `advapi32`). Chrome registers a dozen
providers per process (`EventRegister`, `RegisterTraceGuidsW`,
`EventSetInformation`) and writes through them (`EventWrite*`, `TraceEvent`).
Poly has no trace sessions, so no provider is ever enabled: registrations
succeed with a handle, `EventEnabled`/`EventProviderEnabled` say `FALSE`, a
write returns `ERROR_SUCCESS` and logs nothing, the enable callback is never
called. Argument checks follow Learn (null provider, null handle,
`ERROR_INVALID_HANDLE` for an unknown handle, 128 data descriptors at most).
`EventActivityIdControl` keeps the thread's activity id in a TLS slot.

**`RegNotifyChangeKeyValue`** (`kernelbase`, forwarded by `advapi32`) is
`NtNotifyChangeKey`. Because Learn lets the call return before the change,
the kernel writes the status block later, so asynchronous requests use a
static pool of 256 status blocks (free again once the kernel has written one).

**The interface list** (`poly-iphlp`, `iphlpapi.dll`). The kernel knows its
interfaces: the loopback inside the stack and, with a NIC, the virtio-net
device with the boot setting's address, gateway, DNS server and link-local
IPv6 address (`kernel/src/nt/afd.rs`). A new `\Device\Afd` request,
`IF_LIST`, hands them to user space (`poly_iphlp::wire`, Poly's own
protocol like the others on that device). `iphlpapi` fills the exact x64
layouts of `IP_ADAPTER_ADDRESSES_LH` (448 bytes) and its lists,
`MIB_IF_TABLE2` (rows of 1352), `IP_ADAPTER_INFO` (704) and
`IP_INTERFACE_INFO` from it, with Windows' naming (adapter GUID in
braces, `Ethernet`, `Loopback Pseudo-Interface 1`, `IfType` 6 and 24).
`NotifyAddrChange` stays pending until `CancelIPChangeNotify` completes it
as cancelled: Poly's addresses are the boot setting and never change.

**`ws2_32` remainder.** (1) Chrome's `chrome.dll` binds 23 `ws2_32` functions by
*ordinal* (`#3` is `closesocket`, `#111` `WSAGetLastError`, `#115`
`WSAStartup`); our DLL had none, so those imports would have bound to
stubs. `poly_winsock::exports::ORDINALS` lists the classic numbers and the
DLL's `build.rs` pins them with `/EXPORT:name,@n` (lld keeps the first
`/EXPORT` of a name, so the ordinal wins over rustc's own). (2)
`WSAEventSelect`/`WSAEnumNetworkEvents`: the rules of what is recorded when
are a host-tested state machine (`poly_winsock::events`: `FD_ACCEPT` once
until `accept` runs, `FD_READ` once until `recv` runs, `FD_WRITE` after a
send blocked, `FD_CONNECT` with its error, `FD_CLOSE` after the data was
read or with the error of a reset). A helper thread of the process,
started by the first `WSAEventSelect`, polls the selected sockets with the
device's `POLL` request (20 ms at most, and only for what can still be
recorded, so it does not spin) and sets the events; the socket calls tell
it when they re-enable an event. (3) `WSADuplicateSocketW` duplicates the
endpoint handle into the target process and fills a 628-byte
`WSAPROTOCOL_INFOW` with Poly's provider id and the handle's value;
`WSASocketW(FROM_PROTOCOL_INFO)` takes it up (needs cross-process
`DuplicateHandle`). (4) `WSALookupService*`: the query is accepted and
finds nothing (`WSA_E_NO_MORE`), as the trace shows.

**Small DLLs.** `netapi32` (`NetUserGetInfo` levels 0/1/10/20,
`NetGetJoinInformation`, `DsRoleGetPrimaryDomainInformation`,
`NetGetAadJoinInformation`) answers as a workgroup computer with the one
account `poly`; `secur32!GetUserNameExW` gives `COMPUTER\poly` (SAM
compatible) and the account name (display), the formats that need a
directory are `ERROR_NONE_MAPPED`, and the SSPI/LSA imports fail as a system
without security packages does (`SEC_E_SECPKG_NOT_FOUND`);
`dhcpcsvc!DhcpRequestParams` succeeds with no option data (the addresses
are static); `dnsapi` has `DnsIsZtEnabled` (false); `dbghelp` finds no
symbols, ends `StackWalk64` at once and `MiniDumpWriteDump` fails
(`ERROR_NOT_SUPPORTED`, which crashpad survives). Their logic is in
`poly-netapi` and `poly-iphlp`; `poly-win32-netrt` shares the heap
allocator and the `\Device\Afd` requests.

**`crypt32` and `winhttp`.** `CERT_STORE_PROV_COLLECTION`,
`CertAddStoreToCollection` (the collection holds its own reference to the
sibling; additions land in the first sibling with
`CERT_PHYSICAL_STORE_ADD_ENABLE_FLAG`), `CertRemoveStoreFromCollection`,
`CertControlStore` (resync/commit succeed; `NOTIFY_CHANGE` hands back an
event that is never set), `CryptProtectMemory`/`CryptUnprotectMemory`
(AES-256-CBC in place; the key is random per process for
`SAME_PROCESS`, derived from the user's master key for the other two flags;
`poly_wincrypt::dpapi::protect_memory`), and
`WinHttpGetIEProxyConfigForCurrentUser` (no proxy at all).

**To be confirmed by the spec agent against real Windows.**
1. The results of `TraceEvent` and `GetTraceLoggerHandle` when no session
   is enabled (Poly: `ERROR_SUCCESS`; `INVALID_HANDLE_VALUE` for a missing
   `WNODE_HEADER`).
2. `GetUserNameExW(NameDisplay)` for a local account without a full name
   (Poly: the account name), and which formats Chrome asks for.
3. `NetUserGetInfo`'s other levels, `NetGetAadJoinInformation`'s result for a
   machine that is not joined, `DhcpRequestParams`'s result for an adapter
   without DHCP.
4. Whether `WSAEnumNetworkEvents` clears an event that was recorded again
   between the helper's poll and the call (Poly resets and takes in one step).
5. The metrics, link speed (10 Gbit/s) and description strings Windows gives
   a virtio NIC.
## Addendum: batch 4, windows without a screen (`chrome-win-b4`)

What the headless browser asks of `user32`, `gdi32`, COM and the Windows
Runtime, on top of the window manager of ADR 0131 and 0136. The decisions:

1. **`ole32.dll` is new and hosts COM and the Windows Runtime.** On Windows
   `combase.dll` has them and `ole32.dll` forwards; ours is one module,
   `ole32.dll`, and the loader's API-set table sends `api-ms-win-core-winrt-*`
   and `api-ms-win-core-com-*` there (`poly_ldr::apiset`). Chrome imports
   `RoInitialize` and the `Windows*String*` functions through those contracts.
   The logic is `crates/poly-com` (host-tested): apartment bookkeeping, GUID
   text, `HRESULT`s, `HSTRING`s.
2. **Apartments are per thread, in a TLS slot** (`poly_com::apartment`):
   `CoInitializeEx`, `OleInitialize` and `RoInitialize` share one state: the
   first call is `S_OK`, the same model again `S_FALSE`, another model
   `RPC_E_CHANGED_MODE`; `CoUninitialize` balances.
3. **No class is registered.** `CoCreateInstance[Ex]` and `CoGetClassObject`
   clear their out parameters and fail with `REGDB_E_CLASSNOTREG`
   (`CO_E_NOTINITIALIZED` on a thread with no apartment);
   `RoGetActivationFactory` and `RoActivateInstance` fail with
   `REGDB_E_CLASSNOTREG`. Small stand-ins succeed without effect
   (`CoInitializeSecurity`, `CoRegisterMessageFilter`, `RegisterDragDrop`);
   `CoCreateFreeThreadedMarshaler` returns an object that is only `IUnknown`.
4. **`HSTRING`s are real.** A handle is a pointer to a 24-byte header
   (`sizeof(HSTRING_HEADER)`); a string from `WindowsCreateString` is a
   reference-counted heap block that starts with that header, a fast-pass
   string (`WindowsCreateStringReference`) is the caller's header and
   characters; the empty string is `NULL`.
5. **Window properties live in `user32`, per process**
   (`poly_win32k::props`): the table is keyed by window and by name
   (case-insensitive) or atom; properties of destroyed windows are dropped on
   a later `SetPropW`. Properties of another process's windows are not seen.
6. **`RegisterWindowMessageW` is win32k's** (`Request::RegisterMessage`):
   the same atom table as window classes, one number per name for the whole
   session.
7. **`SystemParametersInfoW` answers the `SPI_GET*` queries** from a table of
   a default Windows 10 session at 96 DPI (`poly_win32k::spi`, including
   `NONCLIENTMETRICSW` with Segoe UI 9 pt); `SPI_SET*` fail with
   `ERROR_INVALID_PARAMETER`.
8. **DPI awareness is remembered, not applied.** `SetProcessDpiAwarenessContext`
   works once per process; every window, monitor and DC stays at 96 DPI.
9. **The gate of stage 1 is the spec's.** The stages run
   `specs/windows/apps/chrome/pages/s1.html` (copied to `C:\poly-test`), the
   launcher prints the length and SHA-256 of Chrome's standard output
   (`LAUNCH-DIGEST` / `LAUNCH-SHA256`), and a stage passes only when Chrome
   exited with 0 and that digest is the one in `windows.json`. The boot test
   group `chrome-win` runs stage 1 and is skipped without the cached
   package; it only reports until `S1_REQUIRED` is set by the change that
   makes stage 1 pass.

Tests: `crates/poly-com`, `poly_win32k::{spi,props}` and the
`RegisterMessage` request on the host; `s1test.exe` end to end in the kernel
self-test (`nt::test_s1`).

## Addendum: the Poly-side call tracer and the comparer (`chrome-win-b4`, second step)

Chrome's first S1 crash (`chrome_elf.dll`, read of address 0x10, no stub hit) could not be read off
the serial log, so Poly gets its own tools, none of which read Chrome's code:

1. **`poly.nt.trace`** (kernel argument; `=all` adds the noisy services): one `nt-trace:` line per NT
   system call with the first four arguments, the name an `OBJECT_ATTRIBUTES` or value name carries
   and the status (`poly_personality_nt::trace`, host-tested). `cargo xtask chrome-win run --trace`.
2. **The unhandled-exception report** adds the general registers, 64 stack quadwords, the TEB/PEB
   fields and the thread's TLS array.
3. **`cargo xtask chrome-win run --kit chrome_elf.dll[,chrome.exe...]`** runs a stage with the
   Windows API tracer's kit (ADR 0090) laid over Chrome's directory: the named programs import
   through proxies that log every call (`polytrace.dll`, now with `POLYTRACE_SYNC`: each record is
   written when its call returns and carries the caller's return address `ra`, so a process that dies
   leaves its trace). The launcher sends `polytrace.jsonl` to the kernel log with `PolyDebugPrint`
   (the console wraps and scrolls long lines) and xtask collects it in
   `target/chrome-win/stage<N>.ptrace.jsonl`.
4. **`cargo xtask win chrome-trace [file]`** compares that trace with the committed Windows
   reduction (`traces/s1.trace.json`): calls that fail on Poly where Windows' never did (or the
   reverse for status codes), last errors and names Windows never saw, and functions Windows called
   earlier that Poly never reached (`tools/xtask/src/chrometrace.rs`, host-tested).
5. **Stage 9** is a probe (`tests/windows/chrome_probe.rs`): what Chrome's start-up asks of
   `version.dll`, on Chrome's own files.

What the tools found: `NtCreateKey` on `\REGISTRY\USER\<sid>\` (one trailing `\`) failed
(`STATUS_OBJECT_NAME_INVALID`; every `NtCreateKey` of the browser on Windows succeeded), the first
`FlsAlloc` returned index 0 (Windows' never did), `HKLM\...\CurrentVersion` lacked `UBR`,
`DisplayVersion` and `ReleaseId`, `ReadProcessMemory`/`WriteProcessMemory` were stubs, and
`PEB.ImageBaseAddress` and the PEB's version, processor, subsystem and heap fields were 0.

## Addendum: the loader lock in the PEB, `winmm` and `shell32` (`wip/chrome-win-b4`, step 6)

What stage 1 needed next, each with a host test and a ring-3 check in `s1test`:

1. **`PEB+0x110` points to the loader lock** (spec-chrome-elf-null): Chrome's crash reporter reads
   `PEB -> LoaderLock -> OwningThread` without calling anything, so a null field faulted at
   `chrome_elf+0x8e9b4`. `ntdll` stores the address of its loader-lock critical section there
   before any initialiser runs (`poly_rtl::peb::LOADER_LOCK`); the same section is taken by
   `LdrLockLoaderLock` and held around every `DllMain` and TLS callback, including those of
   `LdrLoadDll`, `LdrUnloadDll`, forwarder and delay loads and thread and process exit. It shows
   what a Windows process shows: free `LockCount` -1, `RecursionCount` 0, `OwningThread` 0; held
   once -2, 1, the owner's thread id; `SpinCount` 0x4000000 (`RTL_CRITICAL_SECTION_FLAG_STATIC_INIT`)
   and a `DebugInfo` record that points back. `LockCount` of every critical section now counts
   down from -1 (`poly_rtl::critsec`); the values with waiters are Poly's own continuation.
2. **`winmm.dll`**: `timeGetTime` (milliseconds since the system started, wrapping as a `DWORD`),
   `timeBeginPeriod` and `timeEndPeriod` (`poly_kernelbase::winmm`). Chrome takes this clock on
   Poly where it takes `QueryPerformanceCounter` on the Windows machines of the trace. The
   reason is the CPU QEMU shows, not Poly: its default model (`qemu64`) has no invariant TSC
   (CPUID.80000007H:EDX bit 8; Intel SDM vol. 3B, "Invariant TSC"; Learn, "Acquiring
   high-resolution time stamps"), and Chrome tests that bit with CPUID itself.
   `QueryPerformanceFrequency` is 10 MHz on both. With `POLY_QEMU_CPU=host,+invtsc` under KVM
   (`tools/xtask/src/accel.rs`) the guest has the bit, Chrome calls `QueryPerformanceCounter`
   and never `timeGetTime`, and the 705 calls of the cold trace are the same on Poly.
3. **`shell32.dll`**: `CommandLineToArgvW` and `SHGetFolderPathW` (`poly_kernelbase::shell`).
   Without the DLL Chrome's crash handler got no arguments, and its delay-load of `SHELL32.dll`
   ended it with a breakpoint.
4. **`RtlFormatCurrentUserKeyPath`** takes the user's SID from `poly_security` (the literal still
   named the SID from before ADR 0171, so `NtCreateKey` below it failed).

To be confirmed by the spec agent against real Windows: what two quotation marks in a row inside a
quoted part mean to `CommandLineToArgvW`; `timeBeginPeriod(0)` (Poly: `TIMERR_NOCANDO`);
`SHGetFolderPathW` for the folders Poly has no path for yet (Poly: `E_FAIL`); and
`FileNameInformation` of a named-pipe handle (Poly returns an empty name; Chrome's crash handler
builds `\\.\pipe` + that name for its next pipe instance, so it must be `\<pipe name>`).

## Addendum: pipe names, the crash handler child, and delay loads (`chrome-win-s8`, step 8)

1. **Pipe names** (sheets of spec-chrome-win-4): the named pipe file system remembers the name the
   creator of a pipe's first instance spelled; `FileNameInformation` of either end is `\<that name>`
   (the root `\`, an anonymous pipe the empty name, the device itself `STATUS_INVALID_PARAMETER`),
   and `NtQueryObject(ObjectNameInformation)` of a file object is its device's name and its path
   there, with the sheet's rules for buffers that are too small (`objects::file_name_answer`).
   A disk file's device was Poly's own (`\Device\PolyFs<N>`) then; since ADR 0185 it is `\Device\HarddiskVolume<N>`.
2. **`BuildSecurityDescriptorW`** (`poly_security::buildsd`): its merge differs from
   `SetEntriesInAclW`'s (a new allowed entry goes behind the leading denied entries, or last when
   the old list is not in that order).
3. **Child traces.** The kit's logger writes `<file>.N` for a process that finds the trace file
   taken; the launcher prints those files after the browser's, each behind a `{"child":N}` record,
   and `chrome-win run` writes `stage<N>.ptrace.jsonl.<child>`. `.1` is the crash handler.
4. **What Chrome treats as fatal that a stub cannot give.** A function Chrome reaches through its
   delay-load helper must be an export of a DLL that loads: a missing DLL (`LoadLibrary` error 126)
   or a missing export (`GetProcAddress` error 127) ends the process with a breakpoint at
   `chrome.dll+0x8e84bbf`, even where a *failing call* would be tolerated (the fault-injection
   table of the gap map). Static imports are different: the loader binds those to polystub. Added
   for that reason: `GetThreadPreferredUILanguages` (a FALSE is fatal), userenv's
   `RegisterGPNotification` / `UnregisterGPNotification` (policy never changes on Poly: the event is
   never set), and the API set `api-ms-win-downlevel-shell32` (hosted by shell32.dll here, by
   shcore.dll on Windows).
5. **Tracing `chrome.dll` itself deadlocks**: the browser's stack sampler suspends the main thread
   while it holds the logger's lock. Keep the kit to `chrome_elf.dll,chrome.exe`; to see why
   `chrome.dll` stops, read Chrome's own message from the stack at the breakpoint.

To be confirmed by the spec agent: `GetThreadPreferredUILanguages` for `MUI_UI_FALLBACK` (Poly:
"en-US", "en"), for no merge flag and for `MUI_THREAD_LANGUAGES`; `RegisterGPNotification` with a
null event (Poly: FALSE, `ERROR_INVALID_PARAMETER`); `DuplicateHandle` of a read-only section handle
asking for `SECTION_MAP_WRITE` (Windows in the handler's trace: FALSE, `ERROR_ACCESS_DENIED`; Poly
grants it).

## Addendum: the missing DLL batch, whole-process suspension, the exception log (`chrome-win-s9`, step 9)

Step 8 ended at the browser's delay load of `SHLWAPI.dll`, the first of a batch the gap map names
(`specs/windows/apps/chrome/gap-map.md`, the S1 process types and the fault-injection table of
section 6a). This step adds the batch at once instead of one DLL per ten-minute run. Which names:
every `(DLL, function)` the S1 trace calls (`traces/s1.trace.json`) that `cargo xtask chrome-win
imports` reports as missing, plus the other names `chrome.dll` delay-loads from the new DLLs, so
that a delay load of any of them binds.

1. **Seven new DLLs** (`personalities/nt/win32`, logic in `poly_kernelbase::{shlwapi, session}`):

   | DLL | Works | Fails cleanly (the export exists) |
   |---|---|---|
   | `shlwapi.dll` | `PathMatchSpecW`, `PathFindExtensionW`, `PathFindFileNameW`, `PathRemoveExtensionW`, `QISearch` (ordinal 219), `IsOS` (ordinal 437) | `AssocQueryStringW` (no association), `SHCreateMemStream` (ordinal 12: NULL) |
   | `wtsapi32.dll` | `WTSRegisterSessionNotification`, `WTSUnRegisterSessionNotification`, `WTSQuerySessionInformationW`, `WTSFreeMemory`: one session, at the console, that never changes state | |
   | `comctl32.dll` | `InitCommonControlsEx` accepts the structure | the control classes are not there: `CreateWindowExW` of one fails |
   | `oleacc.dll` | | `CreateStdAccessibleObject`, `AccessibleObjectFromWindow`, `AccessibleChildren`, `WindowFromAccessibleObject`, `LresultFromObject`: `E_NOTIMPL`, out-pointers cleared |
   | `uiautomationcore.dll` | no client listens: raising an event and disconnecting a provider succeed, `UiaClientsAreListening` is FALSE | `UiaHostProviderFromHwnd`, the two reserved-value functions: `E_NOTIMPL`; `UiaReturnRawElementProvider`: 0 |
   | `mfplat.dll` | | `MFStartup`, `MFShutdown`: `E_NOTIMPL` |
   | `powrprof.dll` | `CallNtPowerInformation` (`ProcessorInformation`, `SystemBatteryState`), `PowerDeterminePlatformRole[Ex]` (a desktop on mains power) | `PowerGetActiveScheme`, `PowerReadACValue`, `PowerReadDCValue`: `ERROR_FILE_NOT_FOUND` |

   `powrprof.dll` hosts the API sets `api-ms-win-power-*` (`poly_ldr::apiset`); Chrome reaches
   `CallNtPowerInformation` through `api-ms-win-power-base-l1-1-0.dll`. Ordinals are pinned in
   `shlwapi`'s `build.rs` like `ws2_32`'s. A small DLL that links a crate using `alloc` takes the
   process heap as its allocator with `poly_win32_rt::process_heap_allocator!()`.
2. **In DLLs Poly had** (`poly_kernelbase::{procinfo, knownfolder, propstore}`):
   `QueryProcessCycleTime` (the CPU time the kernel counted, as cycles at a nominal 3 GHz),
   `GetProcessInformation` / `SetProcessInformation` (memory priority, power throttling, leap
   seconds, protection level, machine type, application memory; kept per process, nothing acts on
   them yet), `SetProcessShutdownParameters` / `GetProcessShutdownParameters` (kept; levels 0x100
   to 0x3FF), `K32GetMappedFileNameW` (for the images of the calling process: the object
   manager's name of the image's file), `shell32!SHGetKnownFolderPath` (the 122 folders of
   Learn's KNOWNFOLDERID page, each from its default path and the environment),
   `SHGetPropertyStoreForWindow` (an `IPropertyStore` per window handle, in the process),
   `Shell_NotifyIconW` (FALSE: no notification area), `userenv!GetProfileType` (a local profile).
   `NtQueryInformationProcess(ProcessTimes)` now takes a handle with
   `PROCESS_QUERY_LIMITED_INFORMATION` too (Learn, `GetProcessTimes`).
3. **`NtSuspendProcess` / `NtResumeProcess`** (services 0x098 and 0x099). Chrome's crash handler
   resolves them from `ntdll` by name and calls them around the snapshot of a crashed client; with
   the export missing it called address 0 and every crash on Poly cost the browser's 60 s wait.
   Only the names and the access right `PROCESS_SUSPEND_RESUME` are public. Poly's reading of the
   names: each thread of the process is suspended (resumed) once, as `NtSuspendThread`
   (`NtResumeThread`) does one thread; a thread that was created suspended and has not started
   keeps the count its creator will take off.
4. **`poly.nt.exceptions[=N]`**, a kernel argument (off by default; `cargo xtask chrome-win run
   --exceptions[=N]`): every exception handed to a process's user-mode dispatcher is logged as
   `nt-exc:` lines: the code and where it was raised as `module+offset`, the parameters, the
   general registers, N quadwords of the stack (256 by default, 4096 at most), the stack values
   that point into a module and the text on the stack. Chrome's failed checks leave their reason
   there (a failed delay load: the DLL's name and the error). The formatting is
   `poly_personality_nt::diag::exception_report`, host-tested; the hook is
   `NtKernel::exception_delivered`.
5. **`oleaut32` by ordinal.** The first run with the batch stopped at the same breakpoint with
   `OLEAUT32.dll` and `182` (`ERROR_INVALID_ORDINAL`) on the stack: `chrome.dll` delay-loads
   fourteen more `oleaut32` functions by ordinal. Added (`poly_kernelbase::safearray`): safe
   arrays (`SafeArrayCreateVector` 411, `SafeArrayDestroy` 16, `SafeArrayGetDim` 17,
   `SafeArrayGetUBound` 19, `SafeArrayGetLBound` 20, `SafeArrayAccessData` 23,
   `SafeArrayUnaccessData` 24, `SafeArrayGetElement` 25, `SafeArrayPutElement` 26,
   `SafeArrayGetVartype` 77), `SysStringByteLen` 149, `SysAllocStringByteLen` 150, `VarBstrCmp`
   314 (the order `CompareStringW` gives) and `LoadRegTypeLib` 162 (`TYPE_E_LIBNOTREGISTERED`: no
   type library is registered). The name of each ordinal is the export table's. An array Poly
   makes keeps its `VARTYPE` in a block in front of the descriptor (Poly's own layout); the
   element functions take one-dimensional arrays.
6. **The coverage map**: the functions that work are `claimed` by `impl-chrome-s9`; the exports
   that only fail are `stub`.

### Step 9, continued: what the runs asked for after the batch

Each stage 1 run named the next stop; the exception log (item 4) and a new loader line made
every one of them readable without a temporary patch.

7. **Every delay-loaded name binds.** A delay load of a name is fatal for Chrome in two ways: the
   DLL does not load (error 126), or it loads and lacks the export (127, by ordinal 182). After
   `oleaut32` (item 5) the same check fired for `user32!GetShellWindow` and then for
   `UxTheme.dll`, one name per run. So the whole class is closed at once, from `chrome.dll`'s
   delay-import table against our DLLs' export tables:
   * `ntdll` logs a failed `LdrGetProcedureAddress` (`ldr: LdrGetProcedureAddress(dll!name)
     failed`), as it logs a failed `LdrLoadDll`.
   * `poly_win32_rt::fail_exports!` makes exports that log `stub: dll!name` (the line
     `polystub.dll` logs) and fail the way section 2 fixes for a function the system does not
     provide, per function: 0 with last error 120, `(DWORD)-1`, `E_NOTIMPL`,
     `STATUS_NOT_IMPLEMENTED`, or the error value itself where the function returns one (so no
     `HRESULT` or error-code function reports success by returning 0). 209 names in `advapi32`,
     `dwmapi`, `gdi32`, `kernelbase`, `mfplat`, `ole32`, `shell32`, `user32`, `userenv`,
     `winhttp` and `wininet` (`src/delayed.rs` of each). This does not change the loader rule:
     `GetProcAddress` of a name no DLL exports still fails; these names are ones a program
     *requires*, and they are `stub` in the coverage map.
   * Real: `user32!GetShellWindow` (NULL: no Windows shell process), `shell32!ILFree` (ordinal
     155), `kernel32!ProcessIdToSessionId` (the one session).
   * **`uxtheme.dll`**: no visual style is active. `OpenThemeData` is NULL, the functions that
     take a theme handle answer `E_HANDLE`, `IsThemeActive` and `IsAppThemed` are FALSE;
     `DrawThemeBackgroundEx` is also ordinal 47.
   * DLLs `chrome.dll` delay-loads that Poly still lacks (none was reached in stage 1):
     `bthprops.cpl`, `cfgmgr32`, `comdlg32`, `credui`, `cryptui`, `d3d11`, `d3d12`, `dcomp`,
     `dxgi`, `esent`, `hid`, `mf`, `mfreadwrite`, `mmdevapi`, `ncrypt`, `ndfapi`, `pdh`,
     `propsys`, `setupapi`, `tbs`, `urlmon`, `wevtapi`, `winspool.drv`, `wintrust`, `winusb`.
8. **A read-only handle of shared memory cannot be made writable.** Chrome's children ended at a
   check after `NtQueryObject` (type `Section`), `NtQuerySection` and `NtDuplicateObject(self, h,
   self, .., SECTION_MAP_WRITE)`: the duplicate succeeded. Two causes. `CreateFileMappingW`
   ignored `lpFileMappingAttributes` (Learn: its descriptor is the new mapping's, its
   `bInheritHandle` the handle's); both reach `NtCreateSection` now. And `NtDuplicateObject`
   checked every request without `DUPLICATE_SAME_ACCESS` against the object's descriptor: now a
   request for no more than the source handle has is granted as it is, one for more is checked
   against the descriptor (the sheet `kernel32.dll/DuplicateHandle`), and a file handle never
   yields more than it has (the sheet's rule `files`).
9. **Version resources.** The browser reads the system's version from the version resources of
   `kernelbase.dll` and `kernel32.dll` (Learn, "Getting the System Version") and ends itself when
   neither has one. `poly_kernelbase::verinfo` builds a `VS_VERSIONINFO` and the `.res` file that
   holds it; the two DLLs' build scripts hand it to lld-link. The version is the Windows version
   Poly reports (10.0.19045.2006); the strings are Poly's.
10. **`poly.nt.exceptions` for a stack overflow** counts every code address on the whole stack
    and logs the most frequent ones: a recursion's cycle shows as a few return addresses a few
    thousand times each, which the top of an 8 MB stack does not show.
11. **The sheets of `spec-chrome-win-5`** arrived during the step. Made to agree with them:
    `PathMatchSpecW` (rewritten to the sheet's rules), `SetProcessShutdownParameters` (every
    level is accepted, a flag other than `SHUTDOWN_NORETRY` is error 641),
    `SHGetKnownFolderPath` (`system32`, the refused flags), `GetProcessInformation` (the caller's
    `Version` 1 for power throttling, `ProcessInPrivateInfo`), `K32GetMappedFileNameW` (the small
    buffer), `CallNtPowerInformation` (no buffer, `SystemPowerCapabilities`), the `wtsapi32`
    functions (registrations are kept, `WTSSessionInfoEx`, error 87 for an unknown class),
    `NtSuspendProcess` / `NtResumeProcess` (a process that has exited), `DuplicateHandle` (NULL in
    the caller's handle on failure). What still differs is in the hand-off file.

**Where stage 1 stands.** The browser gets through its start-up, starts the crash handler, the
GPU process and utility processes, and writes Chrome's own log lines to standard error. It then
recurses on its main thread until the stack is full (`STATUS_STACK_OVERFLOW`; about 5,200 rounds
of the same eight `chrome.dll` frames). Each round calls `SystemParametersInfoW`, `GetDC` and
`CreateFontIndirectW`: Chrome asks for the system's message font, makes it, and something about
the font it gets sends it round again. What Windows answers there for a machine without Segoe UI
(the face name `SPI_GETNONCLIENTMETRICS` reports on Poly, which has no such font) is a question
for the text sheets (`spec-win-text`), not for this step.

**Doc-derived, not observed on Windows** (what no sheet covers yet; the functions named in item 11
follow their sheets now). In particular, to be confirmed:

* `PathMatchSpecW`: several patterns separated by `;`, leading spaces of each ignored (Learn says
  it of `PathMatchSpecExW`'s `PMSF_MULTIPLE`); case-insensitive beyond ASCII.
* `PathFindExtensionW`: a `.` before a space does not start an extension.
* `IsOS`: TRUE for `OS_NT`, `OS_WIN2000ORGREATER`, `OS_PROFESSIONAL`, `OS_PERSONALTERMINALSERVER`;
  FALSE for `OS_FASTUSERSWITCHING` and everything else.
* `QISearch`: `E_INVALIDARG` for a null object, table or interface id; `E_POINTER` for a null `ppv`.
* `WTSQuerySessionInformationW`: which classes Chrome asks; Poly answers the session id, the user
  and "domain" (the computer's name), `Console`, `WTSActive`, protocol type 0, not remote, and
  `ERROR_NOT_SUPPORTED` for the other classes; `ERROR_INVALID_PARAMETER` for another session id.
* `InitCommonControlsEx`: FALSE for a `dwSize` that is not 8.
* `CallNtPowerInformation`: the levels Chrome asks; `STATUS_NOT_SUPPORTED` for the ones Poly has
  no answer for; the MHz of a processor (Poly: 3000).
* `QueryProcessCycleTime`: only that it grows with the CPU time used is meaningful.
* `GetProcessInformation` / `SetProcessInformation`: `ERROR_BAD_LENGTH` for a wrong size,
  `ERROR_INVALID_PARAMETER` for a class Poly does not know or that cannot be set; the values of a
  new process (memory priority 5, no throttling); another process's state reads as a new one's.
* `SetProcessShutdownParameters`: `ERROR_INVALID_PARAMETER` for a level outside 0x100 to 0x3FF or
  a flag other than `SHUTDOWN_NORETRY`.
* `K32GetMappedFileNameW`: `ERROR_FILE_INVALID` for an address in no image and for another
  process; the device name is `\Device\HarddiskVolume<N>` (ADR 0185; `\Device\PolyFs<N>` before); a name longer than the buffer is cut.
* `SHGetKnownFolderPath`: `E_INVALIDARG` for an unknown id, `E_FAIL` for a folder without a path,
  `HRESULT_FROM_WIN32(ERROR_PATH_NOT_FOUND)` when the folder's variable is not set; flags other
  than `KF_FLAG_CREATE` change nothing.
* `SHGetPropertyStoreForWindow`: the window handle is not checked; `VT_LPWSTR` and the scalar
  types are kept, the others are `E_INVALIDARG`; `GetAt` past the end is `E_INVALIDARG`.
* Safe arrays: `SafeArrayDestroy(NULL)` (Poly: `S_OK`), which element types
  `SafeArrayCreateVector` refuses, `SafeArrayGetVartype` of an array without a type flag (Poly:
  `E_INVALIDARG`), `SafeArrayAccessData` and the lock count, the order of the bounds in a
  descriptor of several dimensions (Poly follows Learn: the left-most first), and
  `SafeArrayGetElement` / `SafeArrayPutElement` on arrays of several dimensions (Poly:
  `E_INVALIDARG`). `VarBstrCmp`: the result for a failed comparison and for `lcid` 0.
* `NtSuspendProcess` / `NtResumeProcess`: the statuses for a handle without
  `PROCESS_SUSPEND_RESUME` (Poly: `STATUS_ACCESS_DENIED`) and for a process that has ended.

## Addendum: the renderer's start-up, placeholders, dynamic function tables (`chrome-win-s11`, step 11)

After step 10 (ADR 0181) the browser stayed up, but each of its three or four renderers ended
about 16 s in on a breakpoint inside `ChromeMain`. Watching only the renderers made the reasons
readable, one per run; nothing here comes from reading Chrome's code.

1. **Tools: trace only some processes, keep only their last calls.** The call tracer
   (`polytrace-sync`) and the kernel learn three things, all off by default:
   * `POLYTRACE_ONLY=<text>` (`chrome-win run --trace-only TEXT`): the logger traces only a process
     whose command line contains the text (`--type=renderer`); in the others it opens no file and
     every thunk calls straight through, so the browser's stack sampler can no longer stop a
     thread inside the logger (the step 8 reason `chrome.dll` could not be in the kit).
   * `POLYTRACE_RING=N` (`--trace-ring N`): keep the last N records in memory, without a lock, and
     write them when the process meets a breakpoint or an error exception (a vectored handler that
     lets the exception go on), or N seconds after start with `POLYTRACE_RING_AT` (`--ring-at`).
   * `POLYTRACE_FIRST=1` (`--trace-first`): write a record only the first time a function returns
     with a given outcome (zero or not, and the last error it set): which functions a process
     reaches and how they answer, in a few hundred records, the shape of the Windows reference's
     per-function rows (`traces/s1.trace.json`).
   * Kernel: `poly.nt.only=<text>` limits `poly.nt.trace` and the new `poly.nt.trail[=N]` (each
     process keeps its last N NT calls, all services, written as `nt-trail:` lines when a
     breakpoint or error exception is handed to it) to processes whose command line has the
     text; `poly.nt.waits` lines now name the process (`p<pid>`) and list the command lines.
     (`--trace-only` sets `poly.nt.only` too; `--trail N`.) The ring and selection logic is
     `poly_personality_nt::trace` (`Trail`, `selects`, `trail_len`), host-tested.
   * `chrome-win run --chrome-arg ARG` adds a switch to Chrome's command line and
     `--launch-seconds N` shortens the launcher's wait, so a debugging run prints Chrome's
     standard error before QEMU is stopped. The launcher's environment line takes several
     `NAME=VALUE` words.
2. **Placeholders** (Learn: "VirtualAlloc2", "VirtualFree", "MapViewOfFile3", "UnmapViewOfFile2").
   The renderer's first stop: `VirtualFree(0x1e000000000, 64 GiB, MEM_RELEASE |
   MEM_PRESERVE_PLACEHOLDER)` failed with error 87 (`kernelbase` accepted only `MEM_RELEASE` or
   `MEM_DECOMMIT`), then V8's check fired; the text on the stack was `v8-sandbox`. V8 reserves its
   sandbox as one placeholder and splits it. `kernelbase` had treated a placeholder as an ordinary
   reservation, which cannot be split. Now the kernel's address-space model has placeholders
   (`poly_personality_nt::vm::Placeholder`): `MEM_RESERVE | MEM_RESERVE_PLACEHOLDER` makes one
   (`PAGE_NOACCESS` only, nothing can be committed in it); `MEM_RELEASE | MEM_PRESERVE_PLACEHOLDER`
   splits off part of one at 64 KiB boundaries, or frees a private allocation that replaced one
   back to it; `MEM_RELEASE | MEM_COALESCE_PLACEHOLDERS` joins neighbouring placeholders named by
   exactly their whole range; `MEM_RESERVE | MEM_REPLACE_PLACEHOLDER` (private memory) and
   `NtMapViewOfSection` with `MEM_REPLACE_PLACEHOLDER` (a data view) take exactly one placeholder's
   range; the new service `NtUnmapViewOfSectionEx` (0x04A; Learn `ZwUnmapViewOfSectionEx`) with
   `MEM_PRESERVE_PLACEHOLDER` leaves a placeholder where the view was. `VirtualAlloc2`,
   `VirtualFree`, `MapViewOfFile3` and `UnmapViewOfFile2` pass the types through; `VirtualAlloc`
   refuses the placeholder types (Learn: they are `VirtualAlloc2`'s). Host test:
   `vm::tests::placeholders_split_replace_preserve_and_coalesce`.
3. **Function tables of code made at run time** (Learn: "RtlAddFunctionTable",
   "RtlDeleteFunctionTable", "RtlAddGrowableFunctionTable", "RtlGrowFunctionTable",
   "RtlDeleteGrowableFunctionTable"). The second stop: the renderer looked up
   `ntdll!RtlAddGrowableFunctionTable`, got NULL (Poly's `ntdll` had none of these), and called
   it: `0xC0000005` executing address 0. `poly_seh::dynamic::Registry` holds up to 256 tables
   (host-tested); `ntdll` exports the five functions and its function-entry lookup
   (`RtlLookupFunctionEntry`, exception dispatch, unwinding) searches the registered tables for an
   address in no image. `kernel32` forwards `RtlAddFunctionTable` and `RtlDeleteFunctionTable`.
4. **`CreateSemaphoreA`, `CreateEventA`, `CreateMutexA`** (Learn: the ANSI forms). With the
   renderers alive, the first-call trace showed `CreateSemaphoreA(NULL, 0, 0x7FFFFFFF, NULL)` =
   NULL with error 120 in every renderer (it was a stub), where Windows' renderers always got a
   handle. They are now the W functions with the name converted from the ANSI code page.

**Where stage 1 stands.** The renderers no longer die; the browser, the crash handler, the GPU,
network and storage processes and three renderers start, and everything then goes quiet: the
browser logs `AddKeepAlive(Default, kHeadlessCommand)` about 12 s in and nothing else, the page's
renderer wakes every ~100 ms and goes back to sleep, and the launcher's 300 s timeout ends the run
without the DOM. The first-call comparison against `traces/s1.trace.json` finds no single call
that fails on Poly and not on Windows before that point; what still differs is in the hand-off
`chrome-win-s11.md`.

**Doc-derived, to be confirmed against Windows:** the statuses of every placeholder failure
(a split off granule boundaries, a range past the placeholder, a replacement of a range that is
not exactly one placeholder, coalescing a range that is not whole placeholders, committing in a
placeholder); that a split naming the whole placeholder (or size 0) succeeds and changes nothing;
that `UnmapViewOfFile2(MEM_PRESERVE_PLACEHOLDER)` of a view that replaced no placeholder leaves one
(Poly) rather than failing; `RtlAddGrowableFunctionTable`'s status for a full table
(`STATUS_NO_MEMORY`) and for a NULL handle pointer (`STATUS_INVALID_PARAMETER`); that
`RtlGrowFunctionTable` ignores a count below the current one or above the maximum; which of two
overlapping tables wins (Poly: the first registered).

## Addendum: executables bigger than 4 MiB, and running out of memory (`nt-big-images`)

The first survey of popular Windows programs (`windows-app-survey`) found that starting any
executable over about 4 MiB panicked the kernel: `NtCreateUserProcess` read the whole file into
one `Vec<u8>` (`IoState::read_all`), and the kernel heap hands out at most one 4 MiB buddy block.
Notepad++, WinMerge, KeePassXC, WinSCP, Sumatra PDF, XnView MP, Audacity, Node.js and VS Code
(8 to 238 MB) all died there, and a failed allocation is a panic, not a status.

- **No executable is read whole.** `create_user_process` opens the image path, takes the file's
  pages through `ImageSection::from_reader` (the lazy image of this ADR: the headers, then each
  page when a view touches it, from the page cache) and hands the `ImageSection` to the kernel in
  `ProcessCreate`. `exec::start_process_listed` and `start_child` take an `ImageSection`; the
  byte-slice entry points (`start_process`, `start_process_from`) stay for small, kernel-made
  images. The shell's `run` and the first program (`program::prepare`) build the image the same
  way, instead of reading the file into a `Vec`. `IoState::read_all` is gone.
- **Linux and Mach-O.** A Linux ELF was already loaded from its file's pages (`read_headers`
  reads a page, then at most 64 KiB, with `try_reserve`); Mach-O is refused before any load.
  Nothing changed there. The user-mode loader in `ntdll` reads a DLL into the process's own heap
  (user memory, not the kernel's), so a DLL over 4 MiB loads (Chrome's 310 MB `chrome.dll` does);
  it costs the file's size twice in that process and is not a kernel panic.
- **Memory that runs out is a status.** Everything an image makes in proportion to its size now
  goes through `try_filled` / `try_push` (`Vec::try_reserve_exact`): the page-protection table
  and its scratch, the header read, the base-relocation block list (16 bytes per page of
  relocations: a relocated image of about 1 GiB or more fails with `STATUS_NO_MEMORY` where it
  used to panic), the eager copy of a small image, the section's page lists. The caller gets
  `STATUS_NO_MEMORY` (`SpawnError::Image`); the process record is torn down as for any other
  failed start.
- **Tests.** Host: a counting file proves a 64 MiB image reads only its headers and one page per
  page touched, `NtCreateUserProcess` starts a 24 MiB executable, a big non-image file is
  `STATUS_INVALID_IMAGE_FORMAT`, `try_filled` reports `STATUS_NO_MEMORY`. Boot
  (`nt::test_bigimage`): a generated 64 MiB executable (`poly_ldr::builder::append_section` pads
  a small program with a data section that exists only as the file's length) starts and exits
  with its status; an image whose relocation table needs more than one heap block fails with
  `STATUS_NO_MEMORY` and the kernel goes on.

## Addendum: stage 1 passes (`chrome-win-s12`, step 12)

After step 11 every process of stage 1 lived, and about 12 s in the whole tree went quiet (the
browser had just logged the start of its `--dump-dom` command). Found with Poly's own wait report,
not with Chrome's code:

1. **The wait report shows who holds what.** `poly.nt.waits` now lists, per blocked wait, the
   processes and handle values that hold each object; per named-pipe instance the processes
   holding each end, the unread bytes and messages and the reads waiting in each direction; the
   completion ports that hold packets; the I/O requests waiting per process and the completions
   not yet delivered; the win32k thread queues (posted messages, wake bits, the queue event's
   state); and, with `poly.nt.trail`, each selected thread's last calls. The trail is kept per
   thread (a busy thread no longer pushes a quiet one's calls out), `poly.nt.trail.skip=<names>`
   leaves services out (`--trail-skip`), and `poly.nt.only` takes `a|b`. `ObjectManager::holders`
   (diagnostics), `IoState::{np_debug_lines, port_debug_lines}`, `Wm::queue_debug_lines`.
   A completion whose data or status block cannot be written into the requester is now logged.
2. **`UnregisterWaitEx` signals its completion event** (Learn: "If the function succeeds, or if the
   function fails with ERROR_IO_PENDING, the caller should always wait until the event is
   signaled"). The browser's UI thread unregisters a wait it had registered on a child process
   with an event it creates for the purpose, and waits for that event; Poly signalled the event
   only when a callback was still running, so the UI thread waited forever (the report: thread 2
   waits on an event only the browser holds; its trail: `NtCreateEvent`, `NtSetEvent` of the
   wait's cancel event, `NtWaitForSingleObject` of the new event). Now the wait's thread signals
   the event when it ends, and `UnregisterWaitEx` signals it itself when that thread had ended
   already (a `WT_EXECUTEONLYONCE` wait that fired). `poly_kernelbase::waitreg::unregister`
   (host test) and a `k32test` case.
3. **Nine more delay-loaded DLLs** (the step 9 rule: the DLL must exist, its exports may fail).
   With the UI thread free, the browser delay-loads `SETUPAPI.dll`, `pdh.dll` and `tbs.dll`, and
   in some runs `ncrypt.dll` (`0xC06D007E` and a breakpoint without them). `setupapi`, `pdh`,
   `tbs`, `cfgmgr32`, `wintrust`, `ncrypt`, `propsys`, `wevtapi` and `hid` export every name
   `chrome.dll` delay-loads from them and fail with the value each function's page or header
   gives: `INVALID_HANDLE_VALUE`, `FALSE` or `NULL` with `ERROR_CALL_NOT_IMPLEMENTED` (`setupapi`,
   the `wintrust` catalog functions, `wevtapi`, `HidD_*`), `PDH_NOT_IMPLEMENTED` (`pdh`),
   `TBS_E_TPM_NOT_FOUND` (`tbs`: no TPM), `CR_CALL_NOT_IMPLEMENTED` (`cfgmgr32`),
   `TRUST_E_PROVIDER_UNKNOWN` (`WinVerifyTrust`), `NTE_NOT_SUPPORTED` (`ncrypt`), `E_NOTIMPL`
   (`propsys`), `HIDP_STATUS_NOT_IMPLEMENTED` (`HidP_*`). Coverage: `stub`. Still missing (none
   reached): `bthprops.cpl`, `comdlg32`, `credui`, `cryptui`, `d3d11`, `d3d12`, `dcomp`, `dxgi`,
   `esent`, `mf`, `mfreadwrite`, `mmdevapi`, `ndfapi`, `urlmon`, `winspool.drv`, `winusb`.
4. **A satisfied wait is not a timeout.** The object manager retires a satisfied wait and queues
   its wake; the waiting thread learns of it when the signaller drains the wakes after its call.
   A waiter whose timeout (or APC) came in between cancelled a wait that no longer existed and
   reported `STATUS_TIMEOUT` (or `STATUS_USER_APC`) although the object was taken (an auto-reset
   event reset, a semaphore count, a mutant), and the late wake became the result of the thread's
   next wait. Now the waiter takes a still-queued wake of its own wait first
   (`ObjectManager::take_wake`, host test), and a recorded completion carries its wait's id so a
   later wait never takes it.
5. **Memory.** In the 4 GiB guest the kernel ran out of frames in about half of the stage 1 runs
   once the browser got this far (about ten Chrome processes): "memory allocation of 1232 bytes
   failed" (an exception's `CONTEXT`, the first allocation after the frames were gone; the kernel
   heap takes its pages from the frame allocator). The stages now run with 8 GiB
   (`chromewin::MEMORY`); why Poly needs that much more than Windows for the same processes is
   open.
6. **The summary reads the launcher's output whole.** The kernel's log lines land in the middle of
   the launcher's console lines and the console wraps the SHA-256 line with a cursor move, so the
   stage's digest was never found; `console_stream` cuts the kernel lines out first.

**Stage 1 passes**: Chrome exits with 0 after about 50–70 s and prints the page's DOM, 440 bytes,
SHA-256 `7b79f8b4…3807b`, the digest the spec recorded on Windows: 8 of the last 10 runs. The
other two ended `0xffff7002`; with `poly.nt.exceptions` one of them showed a failed check (int3) on
a browser thread at `chrome.dll+0xc62f809`, no text on the stack. `S1_REQUIRED` stays `false`
until stage 1 passes every time.

**Doc-derived, to be confirmed against Windows:** the failure values of the nine stub DLLs
(`PDH_NOT_IMPLEMENTED`, `TBS_E_TPM_NOT_FOUND` with no TPM, `CR_CALL_NOT_IMPLEMENTED`,
`TRUST_E_PROVIDER_UNKNOWN`, `NTE_NOT_SUPPORTED`, `HIDP_STATUS_NOT_IMPLEMENTED`), and that `UnregisterWaitEx` with an event signals it once the wait's
thread is done even when it returns `TRUE`.

## Addendum: stage 1 every time, in 4 GiB (`chrome-win-s13`, step 13)

Step 12 left two problems: the kernel ran out of frames in the 4 GiB guest (so the stages ran
with 8 GiB), and about one run in five failed: a browser thread broke with `int3` at
`chrome.dll+0xc62f809` and Chrome ended with `0xffff7002`, or the launcher's 300 s ran out.

1. **Memory: DLLs are image section views ([ADR 0188](0188-dlls-are-image-section-views-and-share-their-clean-pages.md)).**
   A new report (`poly.nt.mem`, `chrome-win run --mem`) showed every Chrome process owning about
   77,300 pages (302 MB, `chrome.dll`'s `SizeOfImage`) from its start: `ntdll`'s loader read each
   DLL file whole and copied the image into private memory. DLLs are now mapped as `SEC_IMAGE`
   views and relocated in place, and every view of the same image file shares the pages nobody
   wrote. User pages went from 3.3 GiB (then the kernel panicked) to about 360 MiB; stage 1 takes
   15-25 s instead of 50-70 s on a quiet machine. `chromewin::MEMORY` is 4 GiB again. A kernel
   heap allocation that fails while an exception's `CONTEXT` is made now ends that process with
   `STATUS_NO_MEMORY` instead of panicking the kernel.
2. **The failures were one bug in Poly's `InterlockedPushEntrySList`.** With per-thread trails
   carrying their time (and the counter values `NtQueryPerformanceCounter` returned), every
   failing run showed a browser thread-pool worker that read `C:\poly-test\s1.html`
   (`NtReadFile`) and then made no system call again. The wait report's new columns said why:
   that thread used 100 % of a CPU in user mode, at `kernelbase.dll+0xdcfe` in every run (the
   kernel records a thread's user RIP when an interrupt returns to it while the report is on).
   That is Poly's own `InterlockedPushEntrySList`: it swapped the new low quadword into RBX by
   hand around `LOCK CMPXCHG16B` but let the compiler choose the register for the `SETE` result;
   the compiler chose BL, which the swap back overwrote, so the loop tested the caller's RBX. A
   caller whose RBX ended in a zero byte looped for ever, and each pass linked its entry to itself.
   Whether a call hung depended on what the caller had in RBX, hence one run in five. When the
   worker spun long enough, another browser thread broke on a failed check (the `int3`, apparently
   Chrome noticing the stuck work);
   otherwise the launcher timed out. Now every operand is a fixed register other than RBX and the
   flag is read after RBX is back; `k32test` calls push and pop with such an RBX (the old code
   hangs it).
3. **Diagnostics (off by default).** Trail lines end with ` @<µs>us` (and ` = <value>` for
   `NtQueryPerformanceCounter`); at a breakpoint the other threads of the process show their calls
   of the last 3 s; the wait report lists suspended threads (count, where they parked) and every
   thread that used more than 100 ms of CPU with its user/system split and where it last was in
   user mode (`module+offset`).

With `poly.nt.waits` on, the kernel's lines can land inside the launcher's SHA-256 line and the
summary misreads a correct digest; runs without that diagnostic are not affected.

| Runs (4 GiB) | Tree | Passed | Failures |
|---|---|---|---|
| m1 | step 12 | 0/1 | kernel out of memory (3.3 GiB of user pages) |
| m2, b1-b7 (71 runs) | + image views, diagnostics on | 53 | 11 failed checks (`int3`), 5 timeouts: each one examined (b2-b7) a browser thread spinning after reading the page, at `kernelbase.dll+0xdcfe` where sampled; 2 more printed the right DOM but `poly.nt.waits` garbled the digest line |
| f1-f12 | + the `InterlockedPushEntrySList` fix, no diagnostics | **12/12** | none (11-39 s each) |

**`S1_REQUIRED` is `true`**: the `chrome-win` boot test group now fails when stage 1 does not pass.

## Addendum: the DLLs and exports the popular Windows programs import (`win-apps-exports`, step 10)

The survey (`windows-app-survey`) ran 19 popular Windows programs: none got a window, and many stopped
for a name or a DLL, not for behaviour. This step makes every system DLL and export they import (statically,
delay-loaded, or looked up at run time in the first runs) exist, so that the next blockers are behaviour.
Nothing here reads a program's code: the inputs are PE import tables (names), our own export tables and the
loader's own "failed" log lines.

- **Fail-only exports in the DLLs Poly has.** 1,739 names in 34 DLLs (`src/apps.rs` of each crate, from
  `poly_win32_rt::fail_exports!`, the macro of step 9; new kinds `invalid_handle` and `error(code, value)`), 83 in
  `ntdll`. Each logs `stub: dll!name` and fails as Learn documents: `FALSE`/`NULL`/0 with `ERROR_CALL_NOT_IMPLEMENTED`
  (the default of the Win32 API); `(HANDLE)-1` or `-1` for the functions whose failure value that is
  (`CreateFile2`, `DialogBoxParamW`, `ImageList_Add`, `GetMessageA` ...); `E_NOTIMPL` for COM, shell and theme
  functions; `STATUS_NOT_IMPLEMENTED` for `Nt*` and the `NTSTATUS` functions of `ntdll` and `bcrypt`;
  the function's own error value where it returns one directly (`ERROR_CALL_NOT_IMPLEMENTED` from `Reg*`, `Cred*`,
  `SH*Value`; `ERROR_NOT_SUPPORTED` from `iphlpapi` and `netapi32`; `MMSYSERR_NOTSUPPORTED` from `winmm`;
  `SEC_E_UNSUPPORTED_FUNCTION` from `secur32`; `WSAEOPNOTSUPP` and `SOCKET_ERROR` from `ws2_32`;
  `APPMODEL_ERROR_NO_PACKAGE` (15700) from the package-identity functions, which is also what a program without
  identity gets on Windows; `E_HANDLE` from the `uxtheme` functions that take a theme handle, of which Poly
  has given none out). Exports by ordinal only (`#2557` of `user32`, the ordinals the programs import from
  `shell32`, `oleaut32`, `comctl32`, `ws2_32`, `shlwapi`) are `ord_<n>` with an `/EXPORT:ord_<n>,@<n>` link argument.
  The generated lists are the `src/apps.rs` files; they carry no knowledge of what the functions do.
- **Forwarders.** `kernelbase` forwards the names `kernel32`, `version.dll` and `advapi32` hold (`GlobalAlloc`,
  `VerQueryValueW`, `EventRegister` ...), and `kernel32` and `ucrtbase` forward `__C_specific_handler` to `ntdll`;
  `msvcrt` forwards three accessors to `ucrtbase`. `ucrtbase` exports the "optimized" `_o_<name>` entry points
  as aliases of `<name>` (the same function; the names are from the import tables).
- **The DLLs Poly lacks, generated.** `poly-stubs` (the crate of `polystub.dll`) got a `system` module that makes a
  whole DLL from a table (`sysdlls.rs`, 65 DLLs, about 1,900 exports): `comdlg32`, `rpcrt4`, `mpr`,
  `winspool.drv`, `msimg32`, `usp10`, `gdiplus`, `wldap32`, `urlmon`,
  `windowscodecs`, `xmllite`, `authz`, `msi`, `tdh`, `cabinet`, `httpapi`, the DirectX and Media
  Foundation set (`d3d9`, `d3d11`, `d3d12`, `dxgi`, `dcomp`, `d2d1`, `dsound`, `mf`, `mfreadwrite`, `mfplay`, `evr`,
  `dxva2`, `mmdevapi`, `d3dcompiler_47`), `opengl32`, `mscoree`, `riched20`, `riched32`, `mlang`, `winsta`,
  `cryptbase`, `vulkan-1`, `icu` and the rest (the table is the list). No toolchain is involved: `xtask win32`
  writes them next to the DLLs it builds, names them in `stub-dlls.txt`, and the kernel's `build.rs` embeds that
  list as `nt::STUB_DLLS`, which `layout` installs in `System32` (one place for all of them). A DLL's exports are
  the names the programs import plus, for the small DLLs, the names the coverage map lists. Each failure is the
  documented one: `DXGI_ERROR_UNSUPPORTED` from `D3D11CreateDevice`, `D3D12CreateDevice` and `CreateDXGIFactory*` (no
  adapter), `NULL` from `Direct3DCreate9`, `D3DERR_NOTAVAILABLE` from `Direct3DCreate9Ex`, `DSERR_NODRIVER` from
  `dsound`, `NTE_NOT_SUPPORTED` from `ncrypt`, `TRUST_E_PROVIDER_UNKNOWN` from `WinVerifyTrust`,
  `PDH_NOT_IMPLEMENTED` from `pdh`, `LDAP_NOT_SUPPORTED` from `wldap32`, `Status` 6 (`NotImplemented`) from `gdiplus`,
  `SCARD_E_NO_SERVICE` from `winscard`, `CDERR_DIALOGFAILURE` from `CommDlgExtendedError` (the dialog functions
  return FALSE). `vulkan-1.dll` is the loader with no driver: `vkEnumerateInstanceExtensionProperties` and
  `vkEnumerateInstanceLayerProperties` report 0 items, `vkEnumerateInstanceVersion` 1.0, `vkGetInstanceProcAddr`
  and `vkGetDeviceProcAddr` NULL, and `vkCreateInstance` (and every function that needs an instance)
  `VK_ERROR_INCOMPATIBLE_DRIVER` (-9), as the specification says for "no conformant implementation". A few are
  forwarders to what Poly has: `psapi` to `kernel32!K32*` (Learn names the pairs), `wsock32` to `ws2_32`, `shfolder`
  to `shell32`, `combase` to `ole32`, `sspicli` to `secur32`, `cryptbase!SystemFunction036` to `advapi32`.
  The report tool (`cargo xtask win-app imports`) counts these DLLs, and names exports by ordinal only as `#<n>`.
- **API sets.** `api-ms-win-ntuser-*`, `api-ms-win-rtcore-ntuser-*` (`user32`), `api-ms-win-shcore-*` (`shcore`),
  `api-ms-win-shell-*` (`shell32`), `api-ms-win-appmodel-*` (`kernelbase`) and `api-ms-win-ro-*` (`ole32`) are hosted;
  the `ext-` ones stay absent (programs probe for them and fall back; ADR 0155 step 9).
- **Doc-derived implementations** (from Learn, not observed on Windows; host tests in `poly_kernelbase::apps`,
  `poly_crt::math`, `poly-stubs`): `MulDiv` (a tie rounds away from zero, which Learn does not say),
  `EncodePointer` / `DecodePointer` (a rotation and an exclusive-or with a per-process cookie: `EncodePointer(NULL)`
  is not NULL), `SetErrorMode` / `GetErrorMode` (the four `SEM_*` flags Learn lists), `InterlockedIncrement` /
  `InterlockedDecrement`, `IsEqualGUID`, `DisableThreadLibraryCalls` (the loader's
  `LdrDisableThreadCalloutsForDll`), `VerSetConditionMask` in `ntdll` (the code of `kernelbase`'s, moved to
  `poly_rtl::version`), `memcpy` / `memmove` / `memset` / `memcmp` in `ntdll` (`compiler_builtins`'), and in `ucrtbase`
  the C99 functions that only round, scale or compare, so the answer is exact: `ceilf floorf truncf roundf rint
  nearbyint lrint llrint lround llround scalbn ilogb nextafter nexttoward fmax fmin copysign nan imaxdiv` and their
  `float` / `long double` forms (`ceilf` also in `msvcrt`). To be confirmed on Windows: the `MulDiv` tie; the cookie
  scheme (only the round trip is documented); `lrint` of a NaN or an out-of-range value (Learn: unspecified); the
  ordinals `wsock32!#101` to `#116` and `#151`, which forward to the same ordinals of `ws2_32`.
- **Left to `impl-win-apps` (spec-win-apps-1).** The kernel32 ANSI variants, `shlwapi!Path*`, `PathCch*`, the resource
  APIs, `RtlGetNtVersionNumbers` and `LoadStringW` are fail-only here, in a separate `fail_exports!` group at the end
  of each `src/apps.rs` ("The real versions are written by the implementer track"): merging that branch deletes
  those names from the groups. Menus, dialogs, comctl32 controls and comdlg32 dialogs stay fail-only until their
  implementer round (spec-win-apps-2).
- **Coverage.** The names in the map are `stub` (3,591 entries); the doc-derived ones are claimed by `impl-win-apps`
  (56; they have no sheet, so they stay `claimed`). Names the programs import that the map does not list (275, most
  of them ordinals) are exports without an entry.
- **Where the programs stop now** is in the hand-off `win-apps-exports.md`: the per-program table, run on the final
  head, and the ranked next blockers.
- **After `chrome-win-s12` (step 12)** hand-wrote nine fail-only DLLs for Chrome (`setupapi`, `pdh`, `tbs`, `cfgmgr32`,
  `wintrust`, `ncrypt`, `propsys`, `wevtapi`, `hid`), the generator no longer makes `setupapi`, `pdh`, `cfgmgr32`,
  `wintrust`, `ncrypt`, `propsys`, `wevtapi` and `hid`: one implementation per DLL, and Chrome stage 1 depends on their exact
  failure values. The generator reproduces their kinds but not all of their values (`pdh` answers `0xC0000BD3`, `cfgmgr32`
  `0x34`, `HidP_*` `0xC0110020`, none of which the generic table has), so theirs stay. The names the programs import
  that those crates lacked are added to each crate as `src/apps.rs`, fail-only with the failure value of the crate's
  own family (`pdh`, `cfgmgr32`: its value; `ncrypt`: `NTE_NOT_SUPPORTED`, `BCrypt*` a status; `HidP_*`: its status,
  `HidD_*` FALSE; `propsys`: `E_NOTIMPL`; `setupapi`: FALSE, `(HANDLE)-1` for `SetupDiGetClassDevs*`; `wevtapi`: FALSE;
  `wintrust`: FALSE).
- **Only the 64-bit modules.** The 32-bit modules of ADR 0186 (i686 `ntdll`, `kernelbase`, `kernel32`, `win32u`, `gdi32`,
  `user32`) carry none of the fail-only exports or doc-derived functions of this step (`#[cfg(target_arch = "x86_64")]`,
  and `build.rs` skips the ordinals, forwarders and aliases there): a stub without parameters is wrong for `stdcall`, and the
  i686 `ntdll` has its own `RtlUnwind`.
## Addendum (2026-11-15): S3 hunt, the browser window stays up

Chrome's S3 window (`--disable-gpu --no-sandbox`, Poly's desktop, `cargo xtask win-app run --mode
desktop`) died in every run: either before its window (an access violation in
`KiUserExceptionDispatcher` after a stack overflow of the browser's main thread) or 60–90 s after
it (a breakpoint `0x80000003` at `chrome.dll+0xc62f809` on a browser thread-pool worker, the
address the S1 flake had). Found with Poly's own diagnostics only (the per-thread trail of the
breaking thread, the other threads' calls beside it, the wait report's suspended threads and CPU
columns, the kernel's `stub:` lines); Chrome's own log (`--enable-logging --log-file`, read back
from the disk image with `debugfs`) had no message for either.

1. **The stack overflow: no fonts on the `win-app` disk.** Each level of the recursion made the
   same three win32k requests (`Desktop`, twice from user32 and once from gdi32's
   `GetDeviceCaps`), the pattern of a screen-DPI query around a font set-up: the failure step 9
   recorded for stage 1 (Chrome's UI font set-up finds no face and starts over until the stack is
   full). `chrome-win`'s disk carries Poly's fonts in `C:\Windows\Fonts`; `win-app`'s did not.
   It does now (`winapps::disk`), and the `win-app` desktop file escapes `%` as `%%` (the Desktop
   Entry Specification's field codes ate the `%20` and `%23` of a `data:` URL).
2. **`GetThreadContext` of a suspended thread that waits in the kernel did not answer.** Chrome's
   stack sampler (a browser thread, one in each renderer) suspends a thread every 50 ms, reads its
   context and resumes it. Poly's `NtGetContextThread` waited (2 s, yielding) for the target to
   park at its return to user mode, which a thread blocked in `NtRemoveIoCompletion` or
   `NtWaitForMultipleObjects` does not reach: the wait report showed `p2 t24 suspend count 1
   running` and the sampler with 19 s of system time; every sample kept its target suspended up to
   2 s. A thread that does not run now has its user registers in the trap frame at the top of its
   kernel stack (its last entry from ring 3); the kernel reads them there under the thread table's
   lock (`thread::user_regs_of`, `arch::user::regs_at`), as Windows answers at once for a
   suspended thread (Learn, `GetThreadContext`). `k32test`: a thread suspended in an infinite wait
   gives its context in under a second and does not end while suspended.
3. **The breakpoint: `K32GetModuleInformation` and `K32GetProcessMemoryInfo` were fail-only
   stubs.** The breaking worker's trail ended, each time, in a batch of `stub:` prints 38 bytes
   long, right after the UI thread posted it a task; the browser called
   `kernel32!K32GetModuleInformation` 900 times in a run (and `K32GetProcessMemoryInfo`). Both are
   implemented from Learn: `MODULEINFO` from the loader's module list and the module's PE headers
   (`AddressOfEntryPoint`; another process's modules answer `ERROR_PARTIAL_COPY` for now),
   `PROCESS_MEMORY_COUNTERS[_EX[2]]` from a new `NtQueryInformationProcess(ProcessVmCounters)`
   that reports the kernel's page counts (working set, its peak, the process's own pages as its
   commit and private usage; Poly counts no reserved address space, so the virtual sizes are the
   working set's). With them the browser ran through every following run (table below).
4. **The other constantly hit stubs, from Learn** (`poly_kernelbase::priority`,
   `kernelbase/src/sched.rs`): `Set/GetThreadPriority`, `Set/GetThreadPriorityBoost`,
   `Set/GetProcessPriorityBoost`, `Set/GetPriorityClass` (kept per process and read back, every
   documented value accepted, the background modes only for the caller; Poly's scheduler has no
   priorities yet), `SetThreadInformation` (sizes checked), `GetThreadTimes` (new
   `NtQueryInformationThread(ThreadTimes)` from the kernel's per-thread CPU time; the thread's
   creation time is its process's), `QueryThreadCycleTime` (its kernel + user time in 100 ns
   units stands in for cycles). The table they keep takes no lock and allocates nothing, because
   the sampler asks while the thread it reads is suspended. `GetThreadContext`, `SetThreadContext`
   and `TerminateThread` are the same wrappers as `win-git-bash`'s (ADR 0194), placed identically.

**Memory.** With ADR 0188's shared image pages (merged from `main`) a desktop run peaks at about
500 MiB of user pages, 200 MiB of page cache and 80 MiB of kernel heap (seven renderers, a GPU and
two utility processes); the default 4 GiB is enough (`win-app` keeps its 4 GiB default; the
`--memory=6G` of the first S3 runs is not needed).

## Addendum (2026-11-15): S3 hunt 2, the page paints the way it does on Windows

**Context.** Round 1 saw Chrome's browser UI but a white page. Every run of round 2, round 1's
code included, took another path: the GPU process created its "Intermediate D3D Window" (a
layered `WS_CHILD` of the browser's window, as on Windows, `s3-windows.md` step 4), win32k
refused it, and the GPU process stopped on a CHECK three times, after which the browser exited
("GPU process isn't usable"). Round 1's runs never reached that window and drew through GDI
(`SetWorldTransform` 7 000-10 000 times a run, 0 times in round 2), most likely because of state
in its long-lived profile. The path of round 2 is Windows' own: Chrome's software compositor
(`--disable-gpu`) rasterises each frame itself, copies it into a Direct3D 11 texture, and shows
it with a DXGI composition swap chain that DirectComposition puts into that child window.

**Decision.** Poly provides that path from public documentation and the public SDK headers
(interface identifiers, vtable orders, structure layouts of `d3d11.h`, `dxgi.h`, `dxgi1_2.h`,
`dcomp.h`; `dcomp.h` is C++ only, its overloaded methods take their slots in reverse order of
declaration as the Microsoft C++ ABI lays them out):

1. **Layered child windows** for a process whose manifest declares Windows 8 or later
   (`supportedOS`; Learn, "Layered Windows"). Without one the window is still refused without a
   last error (`SetLayeredWindowAttributes.yaml` `children`). `user32` tells win32k once
   (`Request::LayeredChildren`) when it is asked for such a window.
2. **`dxgi.dll`**: a factory (`IDXGIFactory`, `1`, `2`) with one software adapter
   (`DXGI_ADAPTER_FLAG_SOFTWARE`, the basic render driver's ids) whose one output is the screen;
   swap chains are made by `d3d11.dll` (`PolyD3D11CreateSwapChain*`, found with
   `GetProcAddress` so the two DLLs do not import each other).
3. **`d3d11.dll`**: a software device. 2D textures live in memory; `Map`/`Unmap`, `CopyResource`,
   `CopySubresourceRegion`, `UpdateSubresource`, `ClearRenderTargetView`, render target views,
   `IDXGIDevice1` (its adapter is dxgi's), `ID3D11Multithread`. Drawing with shaders is not
   implemented (Chrome's software path does not draw); every method that is not implemented logs
   `stub: d3d11: <interface>::<method>` the first time and fails, so the next program that needs
   one says which. Swap chains present their back buffer into their window with GDI
   (`SetDIBitsToDevice`); the first three presents are logged (`d3d11: present ...`).
4. **`dcomp.dll`**: devices (`IDCompositionDevice`, `IDCompositionDesktopDevice`), targets for an
   HWND, visuals. `Commit` binds each swap chain that is the content of a target's visual to the
   target's window (`PolyD3D11BindSwapChain`); offsets, transforms and clips are accepted and not
   applied (Chrome's visual is at the window's origin).
5. **A child window draws into its top-level window's surface** (ADR 0128 addendum below), also
   from another process: the GPU process's present reaches the browser window's surface.
6. **gdi32 world transformations** (`SetGraphicsMode`, `Set/Get/ModifyWorldTransform`,
   `CombineTransform`; drawing, clipping and `BitBlt` follow them), `StretchBlt`,
   `StretchDIBits`, `SetDIBitsToDevice`, `Set/GetStretchBltMode`, and `CreateDIBSection` over the
   section it is given (it ignored `hSection`).

**Consequences.** The page paints: green with its heading (`chrome-win-s3-hunt-2-page-paints-green.png`),
also after a maximize (`...-maximized-by-caption-dclick.png`). The `0xc62f809` breakpoint of the
browser still comes in some runs (once 35 s after the window without input, once around a
maximize and restore); in runs with a slow start (window after 30 s, Chrome's services restarted)
the page stays "Loading". Both are open (hand-off `chrome-win-s3-hunt-2.md`). The trace now names
the win32k request of each `PolyUserCall` (`win32k#0x0(...) "CreateWindow { ... }"`).

## Addendum (2026-11-15): S3 hunt 3, what is behind the browser's breakpoint at `chrome.dll+0xc62f809`

**Context.** The browser of Chrome for Testing 157.0.8103.38 stopped with a breakpoint at
`chrome.dll+0xc62f809` in every run that lived long enough: stage 1 runs that hung (step 13), S3
window runs 35 to 90 s after the window. Three hunts each fixed a real Poly bug that changed the
timing and the breakpoint came back. A failed check of Chrome's own writes no log line, so the
log and the system call trail did not say what Chrome had found wrong.

**Decision.**

1. **The exception log shows what the program was holding** (`poly.nt.exceptions.mem[=N]`, with
   `poly.nt.exceptions`; `poly_personality_nt::diag`): N bytes of the memory each register points
   to (`mem` lines, hexadecimal and text); the text that values on the stack point to inside a
   module (`text the stack points to`: a program keeps the function and file a task was posted
   from as pointers to constant strings); the code addresses left below the stack pointer
   (`code addresses below the stack pointer (its last calls)`, with the text they point to and
   the nearest quadwords): the return addresses of the calls the thread made last. For that the
   log is written before the exception frame is written to the thread's stack (the frame
   overwrites what is below the stack pointer).
2. **`ole32` names the class a program asks COM for** when it answers `REGDB_E_CLASSNOTREG`:
   `stub: ole32: CoCreateInstance: class {...} is not registered (interface {...})`, like the
   fail-only exports' `stub:` lines.

**What the runs showed** (hand-off `fable-c62f809.md`).

- The stack of the breaking thread holds the record of the task it runs: posted from
  `DoStartDownload` in `components\update_client\background_downloader_win.cc`. It is the
  component updater starting a download through BITS on its COM worker thread, about 35 s after
  the first page has loaded. Nothing in it is a race: a slow start whose page never loads never
  gets there, a run that lives long enough always does.
- The task's method makes one call and then stops at its check; that call leaves nothing on the
  stack and makes no call of its own, and no `CoCreateInstance` comes before the check.
- With `TEB.ReservedForOle` (`winternl.h`; 0x1758 in the 64-bit TEB) pointing to memory that
  cannot be read, the thread instead faults reading that pointer + 0x14; with a block whose 32
  bits at +0x14 are all set, the call returns 1 instead of 0 and the same check still fails.
  Chrome reads its thread's COM apartment from the block `TEB.ReservedForOle` points to, not
  with `CoGetApartmentType`, and wants one particular kind. Poly's `ole32` keeps the apartment
  in a TLS slot and leaves that TEB field null.
- With Chrome's `--disable-component-update` the window stays up (240 s, page painted, no
  exception).

**Consequences.** The fix is small and is not made here: `ole32` has to keep a block per thread
in `TEB.ReservedForOle` whose 32-bit value at +0x14 is what Windows has there for a thread with
no apartment, a single-threaded and a multithreaded one. That value is in no public document, so
it has to come from a spec sheet (observed on Windows: the field before `CoInitializeEx`, after
it with each model, after `CoUninitialize`, for `OleInitialize` and `RoInitialize`). Until then
S3 window runs that have to stay up pass `--disable-component-update`.

## Addendum (2026-11-16): browsing in the window (`chrome-win-browse`)

**Context.** With the page painted and the window up (ADR 0207), Chrome was driven on the
desktop as a user would: type a URL into the address field, load `http://example.com/` and
`https://www.iana.org/` over the network, click a link, scroll with the wheel and the keyboard,
go back, open and close a tab, resize the window by its borders, minimize and restore it. The
network, the omnibox, links, keyboard scrolling, back and tabs worked at once. What did not:

| Step | What Poly did | Fix |
|---|---|---|
| typing a URL | `http://` arrived as `httpÖ--`: win32k read keys with the German layout while the desktop's was US | ADR 0209 |
| the wheel | the compositor's `Axis` record was dropped by win32k | `Wm::link_wheel`: a `MOUSEEVENTF_WHEEL` event (Learn, "MOUSEINPUT"), `WM_MOUSEWHEEL` to the window under the pointer |
| restore from the window switcher | win32k never sent `State` (minimized), the switcher showed the stale picture and win32k still thought the window minimized; the switcher also listed Chrome's status bubble (an owned popup) and switched to it | win32k sends `State`; the compositor asks win32k for `Request::Restore` / `Minimize` when the shell shows or hides an application window itself, win32k posts `WM_SYSCOMMAND` `SC_RESTORE` / `SC_MINIMIZE` (Learn, "WM_SYSCOMMAND"); owned internal windows are not in the switcher |
| resize by a border | the compositor ignored `BeginResize` for Windows surfaces; a window that draws its own frame keeps its sizing borders outside its surface, where no pointer reached it; `DefWindowProc` answered `HTBORDER` on a sizable window's border | below |

**Decision (resizing a Windows window by its border).**

1. The compositor runs the resize: `Server::internal_start_resize` (a grab like an xdg
   toplevel's resize) asks the owner for each new content rectangle
   (`InternalEvent::Configure`), never smaller than 64x32; the scene keeps the old picture until
   the app has drawn the new size.
2. win32k gives a new size to the window's own thread: `Wm::configure_from_desktop` hands it a
   message sent from no thread (`DESKTOP_SET_POS`, never seen by the app), on which `user32`
   calls `SetWindowPos` there, as Windows' sizing loop does in the window's thread (the app gets
   `WM_WINDOWPOSCHANGING`, `WM_NCCALCSIZE`, `WM_WINDOWPOSCHANGED`, `WM_SIZE` and draws). A newer
   size replaces one the thread has not taken; a move alone is still applied at once.
   `WM_ENTERSIZEMOVE`/`WM_SIZING`/`WM_EXITSIZEMOVE` are not sent (no program needed them yet).
3. A window that draws its own caption tells the compositor its own borders around the surface
   (the new link record `Margins`, skipped by a compositor that does not know it); the pointer
   there is the window's (`Server::set_internal_margins`) where no window above it is, so its
   `WM_NCHITTEST` decides.
4. `DefWindowProc`'s `WM_NCHITTEST` gives Learn's sizing codes on a `WS_THICKFRAME` window's
   border (`HTLEFT`, `HTRIGHT`, `HTTOP` above the caption, `HTBOTTOM`, the corners), `HTBORDER`
   only for a window without a sizing border (Learn, "WM_NCHITTEST").

**The kernel stack on the first NT use.** The new boot check first failed with a kernel stack
overflow in the debug kernel: the compositor's first write to the desktop link is the first NT
use, and `make_state` built the whole NT state (6 KiB in a debug build) in the frame that then ran
the folder and VFS set-up (about 8 KiB deeper), at the edge of the 16 KiB stack. The state is now
built by a function of its own whose frame is gone before the set-up runs.

**Consequences.** Every step of the flow works in one run (hand-off `chrome-win-browse.md`).
`boot-tests chrome-win` stage 3 now also types a `data:` URL into the address field after the
green page and checks its blue background. `win-app` has the steps `wheel:X|Y|N`, `move:X|Y`,
`wtype:TEXT` (win32k's German positions) and lets a typed text contain commas.

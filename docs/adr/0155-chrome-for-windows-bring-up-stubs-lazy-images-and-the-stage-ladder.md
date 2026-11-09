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

# 0071: Our `ntdll`: a real PE DLL built from Rust, the loader and `Rtl*` logic in host-tested crates

**Status:** Accepted. Builds on [ADR 0029](0029-pe-loader-logic-in-poly-pe.md)
(`poly-pe`: PE parsing, relocations, import binding), [ADR 0055](0055-microsoft-redistributables-run-apps-own-copies.md)
(Poly runs the apps' own Microsoft runtimes, so our loader must run real
third-party DLLs unchanged), [ADR 0063](0063-nt-processes-object-manager-and-memory.md)
(NT processes, Poly's own service numbers, "`cargo xtask` will generate our
`ntdll`'s stubs from this table"), [ADR 0065](0065-nt-threads-synchronisation-and-apcs.md)
(the thread-start and APC trampolines) and [ADR 0068](0068-nt-structured-exception-handling-and-x64-unwinding.md)
(`poly-seh`, kernel-to-user exception dispatch, `PolySetExceptionDispatcher`).

## Context

The Phase 5 item is "Our `ntdll`: PE loader (imports, relocations, TLS,
delay-load), heap, `Rtl*`, loader lock". Until now an NT process ran one
image with no imports, started through per-process trampolines (ADR 0065)
and, for exceptions, a dispatcher it registered itself (ADR 0068). Every
real program imports from `ntdll`, `kernel32` and a C runtime, and the
runtimes apps bring (ADR 0055: `vcruntime140`, `msvcp140`, D3DX ...) are
ordinary third-party PE DLLs with relocations, TLS, forwarders, API-set
imports, delay-load imports and `DllMain`s that expect the documented
loader behaviour. So our `ntdll` is the first DLL that real code links
against, and its loader is the piece everything else stands on.

How the DLL itself is built and delivered was open. The options:

- **(a) A Rust `no_std` crate compiled for a Windows target**, linked by
  `rust-lld` in its `lld-link` flavour into a real PE32+ DLL with an
  export table, `.pdata` unwind data for every function and base
  relocations.
- **(b) Generated with the repo's PE writer** (`personalities/nt/src/pe_writer.rs`)
  from Rust or assembly compiled for another target (ELF objects converted
  to PE sections).
- **(c) The loader mostly in the kernel**, with `ntdll` a thin export shim
  over Poly system services.

What was checked in this container (no network): the pinned toolchain
(1.97.0) has the `rust-src` component, whose `library/vendor` holds every
crate the standard library's build needs, and `rust-lld`. No Windows
target's prebuilt standard library is installed (`rustup target list
--installed`: `x86_64-unknown-linux-gnu`, `x86_64-unknown-none`), and
adding one to `rust-toolchain.toml` would make every agent's and CI's
toolchain download it. But `cargo -Zbuild-std=core,alloc,compiler_builtins
--target x86_64-pc-windows-msvc` with `crates-io` replaced by
`rust-src/library/vendor` builds `core` and `alloc` for the target offline,
and `-Clinker=rust-lld -Clinker-flavor=lld-link` links a working DLL (an
experiment with three exports produced a PE32+ DLL with the export table
and `.pdata` `llvm-objdump` shows). `-Zbuild-std` is unstable, so the build
sets `RUSTC_BOOTSTRAP=1`, which the pinned compiler makes reproducible.

## Decision

**Option (a): `ntdll.dll` is a Rust `no_std` + `alloc` crate,
`personalities/nt/ntdll`, compiled for `x86_64-pc-windows-msvc` and linked
by `rust-lld` (lld-link flavour) into a real PE DLL.** It is its own Cargo
workspace (like `kernel/`), excluded from the host workspace.

Why not (b): converting ELF objects to PE would mean writing and
maintaining a linker, and the result would have no `.pdata` — x64 SEH
walks *through* `ntdll`'s own frames (a `DllMain` that throws, a TLS
callback that faults, `RtlUnwindEx` called from a handler unwinding its own
caller), and a frame without unwind data is taken for a leaf, which breaks
the walk. The MSVC target makes LLVM emit `.pdata`/`.xdata` for every
function. Why not (c): applications and the redistributables call `Rtl*`
and `Ldr*` *in process*, by the thousand, and `DllMain`/TLS callbacks run in
the process under the loader lock; doing that in the kernel would mean an
upcall for every callback and the kernel parsing every DLL. It would also
contradict ADR 0003 (the kernel stays thin). The kernel keeps only what it
must do: map the executable and `ntdll`, and start the first thread in
`ntdll`.

**The logic is in host-tested crates; `ntdll` is the glue.**

- `crates/poly-ldr` (new, `no_std` + `alloc`): the loader. It maps an image
  (sections at their RVAs, per-section protections), relocates it
  (`poly_pe::relocate`: `ABSOLUTE`, `HIGHLOW`, `DIR64`, and for PE32 `HIGH`,
  `LOW`, `HIGHADJ`), resolves imports by name (with the hint) and by
  ordinal, follows forwarders (also to API sets) with a depth limit, keeps
  bound imports whose binding is still valid, resolves delay-load imports on
  first call (`LdrResolveDelayLoadedAPI`), assigns static TLS indices and
  builds every thread's TLS array and blocks, orders `DllMain` and TLS
  callbacks (dependencies first; detach in reverse), counts references
  (`LdrLoadDll`/`LdrUnloadDll`/`LdrAddRefDll`, pinned process-startup
  imports), applies the documented DLL search order, and keeps the PEB's
  loader lists in process memory. It touches the process only through a
  `Process` trait (read a file, reserve/commit/protect/free memory, read and
  write memory, allocate loader structures), so the tests run it against a
  fake address space with PE images they build themselves, including
  malformed ones (it never panics).
- `crates/poly-rtl` (new, `no_std`, no `alloc`): the run-time library: the
  NT heap, counted strings (`UNICODE_STRING`, `ANSI_STRING`, code page
  1252), `RtlNtStatusToDosError`'s table, critical sections, and the version
  `RtlGetVersion` reports.
- `crates/poly-seh` (ADR 0068) does the unwinding and the two dispatch
  passes; `ntdll` drives it over the real stack with real handler routines.
- `personalities/nt/ntdll`: the exports (`#[unsafe(no_mangle)] extern
  "system"`), the system-call stubs, the process heap as the global
  allocator, the loader lock, `LdrInitializeThunk`,
  `KiUserExceptionDispatcher`, `KiUserApcDispatcher`. No third-party crates;
  it depends only on the repo's crates and `core`/`alloc`.

**Re-entrancy: the loader decides, the caller calls out.** `DllMain` and TLS
callbacks may call back into the loader (`GetProcAddress`, `LoadLibrary`,
`GetModuleHandle` from `DllMain` are allowed, Learn: "Dynamic-Link Library
Best Practices"), so `poly-ldr` never calls user code while it holds its own
state. A load maps and binds everything, then returns the list of
*callouts* (TLS callback or entry point, module base, reason, reserved) in
the order to run them; `ntdll` runs each one with the loader lock held but
no borrow of the loader state, and reports a `DllMain` that returned
`FALSE`, for which the loader returns the clean-up (the failed DLL gets
`DLL_PROCESS_DETACH` and is unloaded, `LoadLibrary` fails with
`STATUS_DLL_INIT_FAILED`; at process start the process ends with that
status). This is the pattern of ADR 0065 ("the dispatcher decides; the
kernel blocks").

**The loader lock** is an `RTL_CRITICAL_SECTION` (`winnt.h` layout) in
`ntdll`, recursive like every critical section: the thread that holds it
may load and unload from `DllMain`. Every `Ldr*` export takes it, process
and thread start and exit take it while callouts run, and
`LdrLockLoaderLock`/`LdrUnlockLoaderLock` expose it.

**Build and delivery.** The kernel's `build.rs` builds `ntdll` (release,
`opt-level = "s"`, its own target directory `target/ntdll`) with the pinned
toolchain's `cargo`, `-Zbuild-std` from `rust-src` and `rust-lld`, and the
kernel embeds the DLL (`include_bytes!`), as it embeds the vDSO (ADR 0044).
So every boot, test or not, has the same `ntdll`, with no file system
needed to start an NT process; it is the counterpart of Windows' `KnownDlls`
section for `ntdll.dll`. `cargo xtask ci` adds `fmt` and `clippy` for the
`ntdll` workspace. The system-call stubs are generated by `ntdll`'s
`build.rs` from `personalities/nt/src/services.rs` (every `(Service::X,
number, "NtX")` row becomes `NtX` and `ZwX`: `mov r10, rcx; mov eax,
number; syscall; ret`), so the numbering stays in one place (ADR 0063).

**Process start (the kernel's side).** For an executable that has an
import directory, `exec::start_process` also maps the embedded `ntdll` as
an image section at its preferred base (`0x7FF0_0000_0000`, relocated if
taken), writes `RTL_USER_PROCESS_PARAMETERS` with the image path and
command line, and points the process's thread start at `ntdll`'s
`LdrInitializeThunk`, its APC trampoline at `KiUserApcDispatcher` and its
exception dispatcher at `KiUserExceptionDispatcher` (the export addresses).
Executables without imports keep the ADR 0063/0065/0068 path (their own
trampolines, `PolySetExceptionDispatcher`), which the earlier ring-3 tests
use; once those tests move onto `ntdll` every PE gets it, as on Windows.

The kernel–`ntdll` start convention is Poly's own (both sides are ours):
every thread of an `ntdll` process starts at `LdrInitializeThunk` with
RDI = start routine, RSI = its argument and RDX = the executable's base;
the first thread's start routine is the executable's entry point and its
argument the PEB. `LdrInitializeThunk` initialises the process the first
time (process heap, loader, imports, TLS, `DLL_PROCESS_ATTACH` callouts),
or the thread (TLS, `DLL_THREAD_ATTACH`), then calls
`RtlUserThreadStart(routine, argument)`, which calls the routine and ends
the thread with its return value through `RtlExitUserThread`
(`DLL_THREAD_DETACH`, then `NtTerminateThread`). `RtlExitUserProcess` runs
`DLL_PROCESS_DETACH` (reverse initialisation order) and then
`NtTerminateProcess`. `KiUserExceptionDispatcher` is entered as ADR 0068's
dispatcher (`RCX` = record, `RDX` = context); it runs vectored handlers,
then `poly_seh::dispatch_exception` over the loaded modules' `.pdata`, and
ends with `NtContinue` or a second-chance `NtRaiseException`.
`KiUserApcDispatcher` takes ADR 0065's APC entry registers.

### Structures in process memory

Only documented layouts and fields (`winternl.h`, `winnt.h`, `ntdef.h`):

| Structure | Field | Offset (x64) | Source |
|---|---|---|---|
| `PEB` | `Ldr` | 0x18 | `winternl.h` |
| `PEB` | `ProcessParameters` | 0x20 | `winternl.h` |
| `RTL_USER_PROCESS_PARAMETERS` | `ImagePathName`, `CommandLine` | 0x60, 0x70 | `winternl.h` |
| `PEB_LDR_DATA` | `InMemoryOrderModuleList` | 0x20 | `winternl.h` |
| `LDR_DATA_TABLE_ENTRY` | `InMemoryOrderLinks` | 0x10 | `winternl.h` |
| `LDR_DATA_TABLE_ENTRY` | `DllBase`, `EntryPoint` | 0x30, 0x38 | `winternl.h` (Learn's `PEB_LDR_DATA` page) |
| `LDR_DATA_TABLE_ENTRY` | `FullDllName` | 0x48 | `winternl.h` |
| `LDR_DATA_TABLE_ENTRY` | `CheckSum`, `TimeDateStamp` | 0x78, 0x80 | `winternl.h` |
| `TEB` | `ProcessEnvironmentBlock` | 0x60 | `winternl.h` |
| `TEB` | `ThreadLocalStoragePointer` | 0x58 | the TLS access sequence our own compiler emits (`mov rdx, gs:[0x58]; mov rcx, [rdx + 8*_tls_index]`), the x64 counterpart of the PE Format's "FS:[0x2C]" |
| `TEB` | `TlsSlots[64]` | 0x1480 | `winternl.h` |
| `RTL_CRITICAL_SECTION` | all six fields | | `winnt.h` |
| `UNICODE_STRING`, `ANSI_STRING` | | | `ntdef.h` |
| `RTL_OSVERSIONINFOEXW` | | | `wdm.h` |

Poly's own use of reserved space, which a program must not rely on and
the spec agent should compare with Windows: the in-load-order and
in-initialisation-order links of each `LDR_DATA_TABLE_ENTRY` sit in
`Reserved1` (0x00) and `Reserved2` (0x20), its image size in `Reserved3`
(0x40) and its base name as a `UNICODE_STRING` at 0x58; `PEB_LDR_DATA`'s
in-load-order list head is in `Reserved2` (0x10) and the
in-initialisation-order list head follows the documented structure at
0x30. Nothing else of `PEB_LDR_DATA` is written. `ntdll`'s per-thread data
(the last Win32 error, the last status, the loader's thread record) lives
past the end of the documented TEB, at TEB + 0x1840 (our TEBs are two pages,
ADR 0063).

### API sets

Names starting with `api-` or `ext-` are API-set contracts (Learn: "Windows
API sets", "API set loader operation"), resolved before the search order.
A contract name is matched without its `.dll` and without its last
hyphenated version number (`api-ms-win-core-file-l1-2-4` →
`api-ms-win-core-file-l1-2`), case-insensitively, against **Poly's
mapping table** (`poly_ldr::apiset`), longest prefix first:

| Contract family | Host DLL |
|---|---|
| `api-ms-win-core-rtlsupport-` | `ntdll.dll` |
| `api-ms-win-crt-` | `ucrtbase.dll` |
| `api-ms-win-core-`, `api-ms-win-security-base-`, `api-ms-win-eventing-provider-` | `kernelbase.dll` |
| `api-ms-win-security-`, `api-ms-win-eventing-` (others) | `advapi32.dll` |
| `ext-ms-` | none: extension sets are optional; the load fails with `STATUS_DLL_NOT_FOUND` and `ApiSetQueryApiSetPresence` reports them absent |

The host DLLs are Poly's own (ADR 0055); which contract Poly hosts where is
our choice, since only the names are an interface. `kernelbase`,
`ucrtbase` and `advapi32` are later Phase 5 items; until they exist an
import of their contracts fails to load with `STATUS_DLL_NOT_FOUND`.

### Search order

The standard order for unpackaged applications with safe DLL search mode
(Learn: "Dynamic-link library search order"): a name with a path is
loaded from that path only; otherwise `.dll` is appended when the name has
no extension (a trailing `.` means "no extension", Learn: `LoadLibraryW`);
then API sets, the loaded-module list (by base name, case-insensitively),
the known DLLs (`ntdll`, `kernel32`, `kernelbase`, ... always from
`System32`), the application's directory, `C:\Windows\System32`,
`C:\Windows\System`, `C:\Windows`, the current directory and the `PATH`
directories. `LdrLoadDll`'s search path argument, when given, replaces the
directories after the known DLLs.

## To be confirmed by the spec agent against real Windows

1. TLS callbacks and the entry point on detach: ours runs a module's TLS
   callbacks before its `DllMain` for attach *and* detach; and the order of
   `DLL_THREAD_DETACH` across modules (ours: reverse initialisation order).
2. The `lpReserved` value for static `DLL_PROCESS_ATTACH` and for
   `DLL_PROCESS_DETACH` at process exit (ours: 1).
3. Whether `LdrUnloadDll` of a module the executable imports statically is
   ignored (ours: those are pinned, the call succeeds and nothing happens).
4. The `NTSTATUS` of each load failure (ours: `STATUS_DLL_NOT_FOUND`,
   `STATUS_INVALID_IMAGE_FORMAT`, `STATUS_ENTRYPOINT_NOT_FOUND` for a
   missing named import, `STATUS_ORDINAL_NOT_FOUND`,
   `STATUS_PROCEDURE_NOT_FOUND` from `LdrGetProcedureAddress`,
   `STATUS_DLL_INIT_FAILED`, `STATUS_CONFLICTING_ADDRESSES` for an image
   without relocations whose base is taken) and the exit status of a
   process whose static imports fail (ours: that status).
5. Which of Windows' heap behaviours programs rely on: `RtlSizeHeap`
   returning the requested size (ours does), `RtlFreeHeap(NULL)` (ours:
   `TRUE`), the alignment (ours: 16 bytes), what freeing twice reports
   (ours: `FALSE`, nothing changes).
6. `RtlNtStatusToDosError` for codes outside the published table (ours:
   facility `NTWIN32` codes give their low 16 bits, everything else
   `ERROR_MR_MID_NOT_FOUND` as Learn documents).
7. The in-memory order of `InMemoryOrderModuleList` (ours: load order) and
   the reserved-field use above.
8. API-set resolution details (the version-number rule) and which
   contracts Windows hosts where; whether a missing `api-` contract fails
   differently from an `ext-` one.

## Consequences

- Every Poly DLL after this one (`kernelbase`, `kernel32`, `ucrtbase`,
  `advapi32`) is built the same way, from the same workspace pattern.
- The build needs `RUSTC_BOOTSTRAP=1` for `-Zbuild-std`, confined to the
  `ntdll` build (the kernel and host crates stay on stable features). If a
  Windows target's standard library is ever installed with the toolchain,
  the build can drop it.
- The first kernel build compiles `core`, `alloc` and `compiler_builtins`
  for the Windows target once (about a minute); later builds are
  incremental. The DLL is about 200 KB.
- The ring-3 test (`kernel/src/nt/test_ldr.rs`) runs a self-built
  executable importing from our `ntdll` and from a self-built DLL with
  TLS, `DllMain` and relocations that loads at a non-preferred base, plus a
  delay-loaded DLL, end to end.
- ADR 0065's trampolines and ADR 0068's `PolySetExceptionDispatcher` remain
  for executables without imports and are retired once the older ring-3
  tests run on `ntdll`.
- The coverage entries of the exports implemented here are claimed by
  `ntdll`; they turn green when the Windows oracle and held-out tests exist.

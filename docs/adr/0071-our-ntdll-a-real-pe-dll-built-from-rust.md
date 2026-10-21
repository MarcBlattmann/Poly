# 0071: Our `ntdll`: a real PE DLL built from Rust, the loader and `Rtl*` logic in host-tested crates

**Status:** Accepted; amended when the kernel started mapping it (build and delivery, process start). Builds on [ADR 0029](0029-pe-loader-logic-in-poly-pe.md)
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

**Build and delivery** (amended when the kernel started using the DLL;
the first version had the kernel's `build.rs` run the build). `cargo xtask`
builds `ntdll` (release, `opt-level = "s"`, `-Zbuild-std` from `rust-src`,
`rust-lld`) **once per source version**: before every kernel build or
`clippy` it hashes everything the DLL is built from (the `ntdll` crate, the
repo crates it reaches through `Cargo.toml` path dependencies, the service
table its stubs come from, `rust-toolchain.toml` and the build flags; FNV-1a,
`tools/xtask/src/ntdll.rs`) and looks for `ntdll-<hash>.dll` in a cache
shared by every worktree of the checkout (`<main checkout>/target/ntdll-cache`,
found through `git rev-parse --git-common-dir`; the newest eight are kept).
Only a miss builds, in `target/ntdll`, verifies the exports and deletes the
build-std artifacts again (`POLY_NTDLL_KEEP_BUILD=1` keeps them for
incremental work on `ntdll`). `xtask` names the cached file in
`POLY_NTDLL_DLL`, and the kernel's `build.rs` copies it to `$OUT_DIR`,
where `include_bytes!` embeds it (`nt::NTDLL`), as the vDSO is (ADR 0044).
A bare `cargo build` in `kernel/` without `xtask` embeds an empty file
(with a warning): that kernel boots but refuses to start NT processes.

Measured in the agent container (the pinned 1.97.0 toolchain): a cold
build of the DLL takes **29 s** and **140 MB** of build-std artifacts,
which are deleted right after; a cache hit costs about **0.3 s** (hashing
the ~150 input files); what stays is the **105 KB** DLL in the cache and the
same 105 KB in the kernel image. The cached DLL is reproducible: a rebuild
from the same sources gave the same bytes. CI checks `ntdll`'s formatting
(`cargo fmt --check`); `clippy` for the Windows target would need its own
build-std pass and is left out.

Options weighed: having the kernel's `build.rs` build the DLL (the first
version of this ADR) makes every kernel build that touches `build.rs`
inputs pay the 30 s and keeps 140 MB per worktree; putting the DLL only on
the root disk would leave boots without a disk (every boot test) without
`ntdll`. Embedding plus the content-hash cache costs the build once per
change of `ntdll`'s sources, across all worktrees.

**Where it lives.** The kernel embeds `ntdll` (the counterpart of Windows'
`KnownDlls` section for `ntdll.dll`), so every boot, with or without a
disk, can start an NT process. ADR 0073's layout step also writes the same
bytes to `C:\Windows\System32\ntdll.dll` at the first NT use of a boot
(`layout::create::install_file`, rewritten only when the bytes differ, so
a replaced file is restored), where programs and the loader's search order
find it. The kernel maps its embedded copy, which is by construction what
`System32` holds.

**Process start (the kernel's side).** Every NT process gets `ntdll`, as
on Windows. `exec::start_process` maps the executable and then `ntdll` as
image sections, each at its preferred base or relocated if that is taken
(`ntdll` is linked with `/BASE:0x7FF000000000` and `/NOENTRY`: it has no
`DllMain`), reads the RVAs of `LdrInitializeThunk`, `KiUserApcDispatcher`
and `KiUserExceptionDispatcher` from its export table (a DLL without them is
`STATUS_INVALID_IMAGE_FORMAT`), and makes the PEB, the first TEB and stack;
the kernel then writes `RTL_USER_PROCESS_PARAMETERS` (ADR 0073). Every
thread, the first and those of `NtCreateThreadEx`, starts at
`LdrInitializeThunk`; user APCs are delivered at `KiUserApcDispatcher` and
exceptions at `KiUserExceptionDispatcher`. ADR 0065's per-process
trampoline page and ADR 0068's `PolySetExceptionDispatcher` (service 0x3010,
now retired; the number is not reused) are gone: the kernel tests that used
them run on `ntdll` now.

The kernel–`ntdll` start convention is Poly's own (both sides are ours):
every thread starts at `LdrInitializeThunk` with RDI = start routine, RSI =
its argument and RDX = the executable's base, and RSP 8 bytes below the
16-byte aligned top of its stack, pointing at a zero return address (as if
called), so a stack walk ends there; the first thread's start routine is
the executable's entry point and its argument the PEB.
`LdrInitializeThunk` is a small assembly stub with unwind data (`.seh_proc`)
that moves those registers into the x64 argument registers and calls the
Rust side, which initialises the process the first time (process heap, the
loader with the search-order directories from the process parameters:
the executable's directory from `ImagePathName`, `System32`, `System`,
`Windows`, the current directory, which is the executable's until
`kernelbase` keeps one, and `Path` from the environment block; `PEB.Ldr`,
`ntdll` and the executable registered under their DOS paths, imports, TLS,
`DLL_PROCESS_ATTACH` callouts, all under the loader lock), or the thread
(TLS, `DLL_THREAD_ATTACH`, under the loader lock), then calls
`RtlUserThreadStart(routine, argument)`, which calls the routine and ends
the thread with its return value through `RtlExitUserThread`
(`DLL_THREAD_DETACH`, then `NtTerminateThread`). `RtlExitUserProcess` runs
`DLL_PROCESS_DETACH` (reverse initialisation order) and then
`NtTerminateProcess`. A static import that cannot be bound ends the
process with the loader's status; a `DllMain` that refuses
`DLL_PROCESS_ATTACH` gets `DLL_PROCESS_DETACH` and the process ends with
`STATUS_DLL_INIT_FAILED`. `KiUserExceptionDispatcher` is entered as ADR
0068's dispatcher, an ordinary function `KiUserExceptionDispatcher(record,
context)` with a zero return address; it runs the vectored handlers
(`RtlAddVectoredExceptionHandler`, first to last, with the documented
`EXCEPTION_POINTERS`; `EXCEPTION_CONTINUE_EXECUTION` resumes the context
they changed), then `poly_seh::dispatch_exception` over the loaded
modules' `.pdata`, and ends with `NtContinue` or a second-chance
`NtRaiseException`. `KiUserApcDispatcher` takes ADR 0065's APC entry
registers (R11 = the routine, RCX/RDX/R8 = its arguments, RSP 16-byte
aligned; the kernel keeps the interrupted context): an assembly stub calls
the routine and ends with `PolyResumeApc`, whose stub `ntdll`'s `build.rs`
generates from the service table like the `Nt*` stubs but does not export
(Poly's own services are not an interface).

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
- A change to `ntdll`'s sources (or the crates it uses) costs one 30 s
  build-std compile, once for all worktrees of the checkout; every other
  kernel build pays 0.3 s for the hash. The DLL is about 105 KB, in the
  cache and in the kernel image.
- The ring-3 test (`kernel/src/nt/test_ldr.rs`, `nt_ldr_*`) runs a
  self-built executable that imports heap, string, thread and process
  functions from our `ntdll` and three functions from a self-built DLL with
  two TLS callbacks, a TLS template, a `DllMain` and relocations, which
  prefers the executable's base and so loads elsewhere; the executable has
  a TLS callback and delay-loads a second DLL through
  `LdrResolveDelayLoadedAPI`, starts a thread and ends with
  `RtlExitUserProcess`. Every callout logs itself, and the order is
  checked at each step and, last, by the first DLL's `DllMain` at
  `DLL_PROCESS_DETACH`: attach `TLS₁ TLS₂ DllMain`(DLL) `TLS`(exe), then
  the delay-loaded DLL's `DllMain`, `DLL_THREAD_ATTACH` in initialisation
  order and `DLL_THREAD_DETACH` and `DLL_PROCESS_DETACH` in reverse.
  Every ring-3 NT test also boots through `ntdll` now.
- ADR 0065's trampolines and ADR 0068's `PolySetExceptionDispatcher` are
  retired: every ring-3 NT test runs on `ntdll` (the SEH test registers a
  vectored handler through an import of `RtlAddVectoredExceptionHandler`).
  The personality's host tests start processes with a stand-in `ntdll`
  built by `poly_ldr::builder` that exports the three entry points.
- The coverage entries of the exports implemented here are claimed by
  `ntdll`; they turn green when the Windows oracle and held-out tests exist.

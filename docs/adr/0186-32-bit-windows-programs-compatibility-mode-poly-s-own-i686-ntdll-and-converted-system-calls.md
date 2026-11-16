# 0186: 32-bit Windows programs: compatibility mode, Poly's own i686 `ntdll`, converted system calls

**Status:** Accepted. Builds on [ADR 0063](0063-nt-processes-object-manager-and-memory.md) (NT
processes, Poly's own service numbers), [ADR 0068](0068-nt-structured-exception-handling-and-x64-unwinding.md)
(exception dispatch), [ADR 0071](0071-our-ntdll-a-real-pe-dll-built-from-rust.md) (our `ntdll`,
built from Rust, the loader in `poly-ldr`), [ADR 0073](0073-windows-file-system-layout.md) (the
layout of `C:`), ADR 0079 (`kernelbase`/`kernel32`) and [ADR 0155](0155-chrome-for-windows-bring-up-stubs-lazy-images-and-the-stage-ladder.md)
(image sections read from the file).

## Context

A 32-bit Windows executable (PE32, machine `IMAGE_FILE_MACHINE_I386`) was refused with
`STATUS_INVALID_IMAGE_FORMAT` (`ERROR_BAD_EXE_FORMAT`, 193). The Windows app survey hit it with
WinSCP, and many installers and older programs are 32-bit only.

On 64-bit Windows such a program runs under WOW64 (Learn: "Running 32-bit Applications",
"WOW64 Implementation Details"): the process has a 32-bit view (its own 32-bit `ntdll`, PEB and
TEBs, a 2 or 4 GiB address space), `%windir%\System32` is redirected to `%windir%\SysWOW64`
("File System Redirector"), parts of the registry to `WOW6432Node` ("Registry Redirector"),
`IsWow64Process` says TRUE, and `GetSystemInfo` describes an x86 machine while
`GetNativeSystemInfo` describes the real one. Microsoft documents what a program sees, not how
the layer underneath is built, and Poly does not have to copy it: both `ntdll` and the kernel are
ours, so the simplest correct arrangement is open.

Inputs used (CLEAN_ROOM.md): the Intel SDM (IA-32e compatibility mode, segment descriptors,
`SYSCALL`/`SYSRET`, interrupt gates, `IRET`), Microsoft Learn (the WOW64 pages above,
`IsWow64Process`, `IsWow64Process2`, `GetNativeSystemInfo`, "Memory Limits for Windows",
`NtQueryInformationProcess(ProcessWow64Information)`, `RtlUnwind`, "Structured Exception
Handling"), the public SDK/WDK headers for the 32-bit layouts (`winnt.h`'s `_X86_` `CONTEXT`,
`EXCEPTION_RECORD`, `NT_TIB`, `SLIST_HEADER`; `winternl.h`; `ntpebteb.h`; `ntddk.h`), the PE/COFF
specification. No Wine, ReactOS or Microsoft binary was looked at.

## Decision

### A 32-bit process: nothing 64-bit runs in it

A PE32 executable (`I386`, magic 0x10B) starts a **32-bit process**. The kernel maps it and
**Poly's own i686 `ntdll.dll`** (built from the same crate as the x64 one) as image sections,
writes a 32-bit PEB, TEBs and `RTL_USER_PROCESS_PARAMETERS`, and runs every thread in **IA-32e
compatibility mode**. Unlike Windows there is no 64-bit `ntdll`, `wow64.dll` or 64-bit stack in
the process: the conversion between the 32-bit and the 64-bit world is the kernel's, in the NT
personality, where it is host-tested. Nothing is mapped above 4 GiB in a 32-bit process.

- **Address space** (`wow64::address_end`): `[0x10000, 0x7FFF0000)`, or `[0x10000, 0xFFFF0000)`
  for an executable with `IMAGE_FILE_LARGE_ADDRESS_AWARE` (Learn: 2 GB / 4 GB for a 32-bit
  process on 64-bit Windows, highest address `0x7FFEFFFF` / `0xFFFEFFFF`). `KUSER_SHARED_DATA`
  stays at `0x7FFE0000`, inside both. The PEB and first TEB are placed bottom up (top down would
  take `KUSER_SHARED_DATA`'s place before the kernel maps it).
- **Layouts** (`poly_rtl::layout`): one table of the TEB, PEB and process-parameter offsets per
  pointer width, `X64` (the values used before) and `X86`, with `NATIVE` for code compiled for
  either. The 32-bit offsets are the documented ones with 4-byte pointers (`TEB.Self` 0x18,
  `ThreadLocalStoragePointer` 0x2C, `ProcessEnvironmentBlock` 0x30, `TlsSlots` 0xE10;
  `PEB.ImageBaseAddress` 0x08, `Ldr` 0x0C, `ProcessParameters` 0x10, `ProcessHeap` 0x18, the OS
  version at 0xA4..; `ImagePathName` 0x38, `CommandLine` 0x40); Poly's own uses of reserved
  space (ADR 0073, 0080, 0089) sit in the same `Reserved` slots or after the documented fields.
  The loader's lists (`PEB_LDR_DATA`, `LDR_DATA_TABLE_ENTRY`) have their 32-bit offsets in
  `poly_ldr::LdrLayout::X86`; the loader takes the process's `Bitness` at run time
  (`poly_ldr::Config::bitness`) and loads only images of its own machine.
- **The thread's state**: the GDT gets a 32-bit user code segment (slot 7, selector `0x3B`: base
  0, limit 4 GiB, D=1, L=0, DPL 3). Every return to ring 3 of a 32-bit thread uses it (a
  per-thread flag in `ThreadState`, mirrored per CPU on each switch, so `IRETQ` of a new thread,
  of a loaded context and of every interrupt keeps the thread in compatibility mode). DS, ES and
  FS hold the user data selector (`0x1B`, DPL 3) from boot on: in compatibility mode an access
  through a null selector is a #GP, and `IRETQ` to ring 3 nulls any selector whose DPL is below 3;
  64-bit code ignores DS/ES, and FS's base stays `IA32_FS_BASE`, which the kernel switches per
  thread. A 32-bit thread's FS base is its TEB (`NtCurrentTeb` reads `fs:[0x18]`), its GS unused.
- **Start convention** (Poly's own; both sides ours): a thread starts at the i686 `ntdll`'s
  `LdrInitializeThunk` as if called (`cdecl`) with the start routine, its argument and the
  executable's base on the stack under a zero return address (`wow64::start_esp`). The
  executable's entry point (`cdecl`, no arguments) and a thread routine (`stdcall`, one) are both
  called through a stub that restores ESP.

### System calls: `int 0xF4`, converted by a signature table

Intel CPUs refuse `SYSCALL` in compatibility mode (#UD), and `SYSRET` can only return there with a
selector layout Poly does not have; `SYSENTER` is refused in long mode on AMD. So the i686 `ntdll`
enters with **`int 0xF4`**, a DPL 3 interrupt gate (a vector above every device, IPI and APIC
vector and outside the legacy PIC's range), with **EAX = the service number** (Poly's numbering,
ADR 0063) and **EDX = the address of the arguments on the caller's stack**; its stubs are
`stdcall` (`ret 4·n`), generated from the service table like the x64 ones. The gate's entry builds
the same trap frame as `syscall`'s, runs the same handler and leaves by `IRETQ`. `IA32_CSTAR`
(where AMD CPUs take `SYSCALL` in compatibility mode) points at an entry that treats it the same
way, so the kernel never jumps to an unset address.

The personality converts each call (`wow64::thunk`): **`wow64::sigs::SIGNATURES`** gives every
service's 32-bit arguments a kind — a value (zero extended), a handle (sign extended:
`NtCurrentProcess()` is -1 in both widths), a pointer to data laid out the same in both widths,
`HANDLE*`, `SIZE_T*`/`PVOID*`, `OBJECT_ATTRIBUTES*`, `UNICODE_STRING*`, `IO_STATUS_BLOCK*`,
`CLIENT_ID*`, a handle array, `CONTEXT*`, `EXCEPTION_RECORD*`, or "not converted yet" (a non-null
pointer of that kind answers `STATUS_NOT_IMPLEMENTED`). The same table gives the i686 stubs their
argument counts, so it is the one source of both. The 64-bit copies of a call's structures live in
a **scratch window** at `0x7000_0000_0000`, which a wrapper kernel (`Thunked`) serves from a
buffer that lives for the one call; the services run unchanged against it, and the results go back
in the 32-bit layout (a handle's or size's low half, `IO_STATUS_BLOCK`'s two fields, a
`UNICODE_STRING`'s lengths). Information classes whose output has pointer-sized fields are
converted per class: `SystemBasicInformation` (with the 32-bit process's own address range),
`MemoryBasicInformation`, `ProcessBasicInformation`, `ProcessWow64Information`,
`ThreadBasicInformation`; other classes pass through (their output layout is the same, or is a
later stage's).

### Exceptions

A 32-bit thread's handlers are frame-based (`fs:[0]`, `EXCEPTION_REGISTRATION_RECORD`), not
table-based. The kernel delivers a CPU exception, or `NtRaiseException`'s, to the i686 `ntdll`'s
`KiUserExceptionDispatcher(record, context)` with the x86 `EXCEPTION_RECORD` and `CONTEXT`
(`wow64::context`: `winnt.h`'s `_X86_` layouts; the x87/SSE state as the `FXSAVE` image in
`ExtendedRegisters`) and a call frame on the thread's stack. The dispatcher runs the vectored
handlers, then each registration record's handler (`cdecl` `EXCEPTION_ROUTINE`), stopping at a
frame outside the thread's stack; `ExceptionContinueExecution` resumes the context through
`NtContinue` (an x86 `CONTEXT`, converted), refused for a non-continuable exception
(`STATUS_NONCONTINUABLE_EXCEPTION`, nested); nobody handling it, the top-level filter, then a
second-chance `NtRaiseException` that ends the process. `RtlRaiseException` dispatches in the
process; `RtlUnwind` calls the handlers of the frames above its target with `EXCEPTION_UNWINDING`,
unlinks them and continues at the target address. User APCs are not delivered to 32-bit code yet.

### What the program sees

- `IsWow64Process`: TRUE for a 32-bit process, from `ProcessWow64Information` (nonzero — Poly:
  the 32-bit PEB's address — for a 32-bit process, 0 for a 64-bit one, asked about itself or
  another process). `IsWow64Process2`: `IMAGE_FILE_MACHINE_I386` / `IMAGE_FILE_MACHINE_UNKNOWN`,
  the native machine `IMAGE_FILE_MACHINE_AMD64`.
- `GetSystemInfo` in a 32-bit process: the 36-byte x86 `SYSTEM_INFO`, `PROCESSOR_ARCHITECTURE_INTEL`,
  `PROCESSOR_INTEL_PENTIUM`, the process's own address range; `GetNativeSystemInfo`: the same with
  `PROCESSOR_ARCHITECTURE_AMD64` (8664).
- **File-system redirection** (`wow64::redirect_system32`): a file service's full path under
  `%windir%\System32` becomes the one under `%windir%\SysWOW64`, except the directories Learn
  exempts (`catroot`, `catroot2`, `driverstore`, `drivers\etc`, `logfiles`, `spool`), in the
  conversion of the `OBJECT_ATTRIBUTES` of `NtCreateFile`, `NtOpenFile`, `NtDeleteFile` and the
  attribute queries. The i686 loader's system directory is `SysWOW64`.

### Poly's 32-bit DLLs and the build

`ntdll`, `kernelbase` and `kernel32` build for `i686-pc-windows-msvc` from their x64 sources
(**`cargo xtask wow64`**, `tools/xtask/src/wow64.rs`): `no_std` Rust, `-Zbuild-std` from
`rust-src`, `rust-lld` (lld-link), no C toolchain, cached by content hash in `target/wow64-cache`
beside `ntdll`'s and the Win32 modules' (CI restores it like theirs). The kernel embeds them
(`nt::NTDLL32`, ...) and the layout writes them to `C:\Windows\SysWOW64`. What is
architecture-specific in the sources is behind `cfg(target_arch)`: the TEB access (`gs:[0x30]` /
`fs:[0x18]`), the entry stubs, `RtlCaptureContext`, the x64 unwinder (x64 only) and the x86
frame-based dispatcher (x86 only), the interlocked lists (`CMPXCHG16B` / the 8-byte x86
`SLIST_HEADER`), the `cpuid` intrinsic path. Every `ntdll` export is `extern "system"` (`NTAPI`,
`stdcall` in 32-bit code; the same ABI as `C` on x64), the `raw-dylib` imports name undecorated
symbols on x86 (`import_name_type = "undecorated"`). Structures that hard-coded 64-bit layouts
(`IO_STATUS_BLOCK`, `OVERLAPPED`, `EXCEPTION_RECORD`, the thread and process basic information
ids) are pointer-sized now. The compiler's 32-bit helper routines (`__chkstk` with its 32-bit
contract — it moves ESP itself —, `_alldiv`, `_aulldiv`, `_allrem`, `_aullrem`, `_allmul`, the
shifts), which MSVC's C runtime would bring, are `personalities/nt/msvc32` (`poly-msvc32`),
hand-written from their documented conventions.

## What runs

`tests/windows/wow64/hello32.rs`, a 32-bit console program importing `kernel32` (and `RtlUnwind`
from `ntdll`), prints `hello` and ends with 0 after: `SetLastError`/`GetLastError`;
`IsWow64Process` TRUE and its stack below 4 GiB; a file written, read back, sized and deleted;
a second thread with its own id, argument and last error, waited for, its exit code read; an
access violation and a `RaiseException` caught by an `fs:[0]` frame (what `__try` compiles to)
and continued, and `RtlUnwind` of an inner frame. The ring-3 kernel test
`nt_wow64_32_bit_program_runs_in_compatibility_mode` runs it.

## Staged plan

1. **More of the 32-bit `kernel32`/`kernelbase`** (next): go through the remaining hard-coded
   64-bit layouts in `kernelbase` (`GetStartupInfo`'s parameters, `FormatMessage`'s `va_list`,
   the `CreateProcessW` parameter block, `CONTEXT` users), and give the remaining services with
   pointer-carrying structures their conversion: `NtGetContextThread`/`NtSetContextThread`
   (x86 `CONTEXT` both ways), `NtCreateUserProcess` (a 32-bit parent's parameter block, and a
   64-bit child from a 32-bit parent and back: the per-process layout already chooses),
   `NtQueryObject(ObjectNameInformation)`, `NtQueryDirectoryObject`, the token classes with
   SIDs (`TOKEN_USER`, `TOKEN_GROUPS`), absolute security descriptors, `NtRemoveIoCompletionEx`'s
   entries, `FILE_RENAME_INFORMATION`/`FILE_LINK_INFORMATION`'s root handle, the key information
   classes. Asynchronous I/O: a 32-bit `IO_STATUS_BLOCK` written at completion (the request must
   keep the caller's address and width, not the scratch window's). User APCs: a 32-bit
   `KiUserApcDispatcher` frame. Needs: per-class converters in `wow64::thunk`, a width in the IRP.
2. **Registry redirection** (`WOW6432Node`, Learn: "Registry Redirector", "Registry Keys
   Affected by WOW64"): for a 32-bit caller, `HKLM\SOFTWARE` (and the redirected `Classes`
   subkeys) resolve under `WOW6432Node` except the shared keys; `KEY_WOW64_64KEY` /
   `KEY_WOW64_32KEY` choose the view in either kind of process; the default hives get the
   `WOW6432Node` keys. Needs: the caller's bitness in `registry::services`' name resolution.
   Also `Wow64DisableWow64FsRedirection` / `Wow64RevertWow64FsRedirection` (a per-thread flag the
   file-name conversion consults) and `%windir%\Sysnative`.
3. **The C runtime and console programs built with MSVC**: i686 `ucrtbase.dll` / `msvcrt.dll`
   and the `vcruntime140` apps bring (ADR 0055: apps' own redistributables run unchanged, so
   their 32-bit `vcruntime140.dll` needs only what it imports), `_except_handler3/4`'s
   `__try`/`__except` (works on the dispatcher above; needs `RtlUnwind` with a real target IP
   from `_global_unwind2` and the security-cookie checks), SafeSEH (`/SAFESEH` handler tables in
   the load configuration: a dispatcher that checks them), C++ exceptions (`_CxxThrowException`
   on x86 raises `0xE06D7363` through the same path). Needs: the i686 builds of `ucrtbase`,
   `msvcrt` in `cargo xtask wow64`, the x86 parts of `poly-crt`.
4. **GUI programs**: i686 builds of `user32`, `gdi32`, `win32u` and the small DLLs; the win32k
   table's 32-bit signatures (`poly_win32k::services`: window handles stay 32-bit values, the
   message structures — `MSG`, `WNDCLASSEX`, `CREATESTRUCT`, `PAINTSTRUCT` — are converted, and
   window procedures are called back in 32-bit code: a 32-bit callback frame like the APC one);
   `comctl32`, `uxtheme`, `shell32` follow. Needs: win32k argument kinds beside `sigs`, callback
   delivery in compatibility mode.
5. **Installers**: NSIS and Inno Setup installers are 32-bit GUI programs (stage 4) that need
   `shell32`'s folders, the registry views (stage 2), `CreateProcess` of 64-bit children
   (stage 1) and `MoveFileEx` delay; MSI needs `msiexec`'s service. WinSCP (the survey's case)
   needs stage 4 and its own redistributables.
6. **Performance and corners**: the scratch window allocates per call (a per-thread buffer would
   not); `int` is slower than `syscall` (a 64-bit gate page reachable by a far call could use
   `syscall`, at the cost of a 64-bit code page in every 32-bit process); programs that switch
   to the 64-bit code segment themselves ("Heaven's gate", selector `0x23` on Poly, not `0x33`)
   get Poly's 64-bit system-call path with the 32-bit argument conversion; x87 state in
   `FloatSave` (`FNSAVE` format) for programs that read it from a `CONTEXT`; a 32-bit thread that
   reloads FS loses its TEB base.

## To be confirmed by the spec agent against real Windows

1. `ProcessWow64Information`'s value (Poly: the 32-bit PEB's address; Learn only says nonzero).
2. What `GetNativeSystemInfo` puts in the 4-byte address fields of a 32-bit caller (Poly: the
   process's own range), and `GetSystemInfo`'s `dwActiveProcessorMask` beyond 32 processors.
3. Whether `IsWow64Process2`'s process machine is `I386` for every 32-bit process.
4. The 32-bit TEB's initial `ExceptionList` (Poly: -1), and the start frame a 32-bit thread sees.
5. The redirection of `System32` paths given relative to a root handle (Poly: not redirected),
   and of `\\?\` and `\??\GLOBALROOT` forms.
6. Which status a 32-bit `NtQuerySystemInformation(SystemBasicInformation)` with a 64-byte
   buffer answers (Poly: success, 44 bytes).

## Consequences

- A 32-bit console program that imports `kernel32` runs; the survey's 32-bit apps need the later
  stages.
- `cargo xtask wow64` is one more cached build (about two minutes cold, once per source version,
  for all worktrees); every kernel build waits for it as for `ntdll`'s.
- `int 0xF4` is reserved: no device may get vector 0xF4 (it is above the device range).
- DS, ES and FS are the user data selector in ring 0 too; nothing in the kernel uses them.
- Changing a service's arguments now means changing its row in `wow64/sigs.rs` (a host test
  checks every service has exactly one).

## Addendum: stage 2 (registry and file-system views)

Stage 2 of the plan is done, with host tests. Inputs: Learn's "Registry Redirector", "Registry
Keys Affected by WOW64" (the table's column for Windows 7 and newer), "Accessing an Alternate
Registry View", "File System Redirector", `Wow64DisableWow64FsRedirection`,
`Wow64RevertWow64FsRedirection`, `Wow64EnableWow64FsRedirection`.

### Registry views

- **The rule** is one pure function over NT paths, `registry::wow64_view(path, wow64, access)`
  (`personalities/nt/src/registry/wow64.rs`). A call's view is the 32-bit one for a 32-bit
  process, or when `KEY_WOW64_32KEY` (0x0200) is in `DesiredAccess`, in either
  kind of process; `KEY_WOW64_64KEY` (0x0100) gives the native view to a 32-bit process. Both
  flags together mean nothing to the kernel (the process's own view); `RegCreateKeyEx`,
  `RegOpenKeyEx` and `RegDeleteKeyEx` refuse them with `ERROR_INVALID_PARAMETER`, as Learn says.
  The view bits are not rights: `registry::key_access` already dropped them from the access
  check, and `kernelbase` now passes them to `NtOpenKey`, `NtOpenKeyEx` and `NtCreateKey`
  instead of dropping them.
- **In the 32-bit view**, `HKLM\SOFTWARE` and everything below it lives under
  `HKLM\SOFTWARE\Wow6432Node`, except the keys Learn lists as shared (`registry::wow64::SHARED`,
  with its source in the table's "Windows 7 and Newer" column: `Classes` and its subkeys,
  `Clients`, `Policies`, `RegisteredApplications`, and the `Microsoft\...` keys from `COM3` to
  `Windows NT\CurrentVersion\Time Zones`; every subkey inherits). Below `Classes`, the five
  subkeys Learn marks redirected (`CLSID`, `DirectShow`, `Interface`, `Media Type`,
  `MediaFoundation`) live under `Classes\Wow6432Node`. `HKCU\SOFTWARE` is shared and only those
  five `Classes` subkeys are redirected (`\REGISTRY\USER\<sid>\Software\Classes\...` and the
  `\REGISTRY\USER\<sid>_Classes` hive form). Learn's page lists `Microsoft\Shared Tools\MSInfo`
  with a doubled `SOFTWARE\Microsoft\` prefix, a typo of the page; the key meant is used. The
  symbolic links Learn lists for programs that hard-code `Wow6432Node` paths are not made.
- **Where it applies**: the name resolution of `NtCreateKey`, `NtOpenKey`, `NtOpenKeyEx`
  (`registry::apply_view`), on the *full* path: the root handle's path plus the relative name, or
  the absolute name. `kernelbase` opens `HKLM` (`\REGISTRY\MACHINE`) as a handle and the rest
  relative to it, so the same code handles `RegOpenKeyEx(HKLM, "SOFTWARE\X")` and
  `NtOpenKey("\Registry\Machine\Software\X")`. A path already below `Wow6432Node` is physical and
  not moved again (`RegCreateKeyEx` creates one component after the other); below it a shared key
  is mapped to its one physical key, so a relative open from a handle of the 32-bit view reaches
  the same key as the one-step open. `NtDeleteKey` takes a handle and needs no view; handles keep
  the physical key they opened (Learn: "all subsequent operations on child registry keys must
  explicitly use the same flag"). `RegEnumKey` on `HKLM\SOFTWARE` lists `Wow6432Node` for a
  64-bit caller, never for a 32-bit one.
- **The default hive** has an empty `HKLM\SOFTWARE\Wow6432Node` (`poly_registry::defaults::install`
  makes it after mounting the software hive, also for a hive loaded from an older file).
- **Not done**: Learn's rewriting of `%ProgramFiles%` and `%commonprogramfiles%` in `REG_SZ` and
  `REG_EXPAND_SZ` values a 32-bit process writes (to `%ProgramFiles(x86)%`, ...), the
  `Wow6432Node` compatibility links, `HKEY_CLASSES_ROOT`'s merge of the two views beyond what the
  paths give, and `RRF_SUBKEY_WOW64xxKEY` of `RegGetValue`.

### File-system views

- **Per-thread switch.** `Wow64DisableWow64FsRedirection(&old)`, `Wow64RevertWow64FsRedirection(old)`
  and `Wow64EnableWow64FsRedirection(enable)` are in `kernelbase` (`kernel32` forwards them
  through the `api-ms-win-core-wow64-l1-1-0` contract like `IsWow64Process`). The state is one
  `u32` in the **calling thread's TEB** at `poly_rtl::layout::TEB_FS_REDIRECTION_OFF` (0x1FF0, in
  both widths' two-page TEB, far above `ntdll`'s data at `TEB_POLY`), zero in a new thread. Both
  sides own it: `kernelbase` stores it (no system call), `wow64::thunk::object_attributes` reads
  it for the calling thread before it redirects a file name, so it is per thread by
  construction (Learn: "Disabling file system redirection affects only operations made by the
  current thread"). `OldValue` is the flag's previous value, so nested pairs restore what was
  there (the rules are `poly_kernelbase::wow64`, host-tested). In a 64-bit process the three
  functions fail with `ERROR_INVALID_FUNCTION` (Learn names no error; to be confirmed below).
- **`%windir%\Sysnative`** (`wow64::sysnative_to_system32`): a path under it names the same path
  under `System32`, whether redirection is on or off (Learn: "a special alias used to indicate
  that the file system should not redirect the access"). A 64-bit process gets no alias: only
  the 32-bit conversion looks for it.
- **Redirection** now also covers the other two rows of Learn's table:
  `%windir%\lastgood\system32` (to `lastgood\SysWOW64`) and `%windir%\regedit.exe` (to
  `SysWOW64\regedit.exe`).

### Checks of stage 1's calls against Learn

`IsWow64Process2` writes `IMAGE_FILE_MACHINE_I386` for a 32-bit process and
`IMAGE_FILE_MACHINE_UNKNOWN` for a 64-bit one in `pProcessMachine`, `IMAGE_FILE_MACHINE_AMD64` in
the optional `pNativeMachine` (Learn); `GetSystemInfo` and `GetNativeSystemInfo` in a 32-bit
process use the 36-byte x86 `SYSTEM_INFO` (`system_info_32`, host-tested: architecture,
page size, address range, processor mask and count, `dwProcessorType`, granularity, level), the
native call with `PROCESSOR_ARCHITECTURE_AMD64`. No change was needed.

### To be confirmed by the spec agent (additions to the list above)

7. What the three `Wow64*FsRedirection` functions do in a 64-bit process (Poly: fail with
   `ERROR_INVALID_FUNCTION`, `Wow64EnableWow64FsRedirection` returning FALSE).
8. Whether `NtOpenKey` with both `KEY_WOW64_64KEY` and `KEY_WOW64_32KEY` is an error (Poly: the
   process's own view; the Win32 functions refuse it).
9. What a relative open below a handle opened with the other view does in the kernel (Poly: the
   32-bit view of the full path, which Learn calls "unexpected behavior").
10. Whether a created key below a shared key whose parents exist only in the other location
    works (Poly: the parents must exist at the shared location).

## Addendum: stage 1 completed — every conversion the plan listed (`nt-wow64-2`)

Inputs as above (Learn, the SDK headers' `_X86_` and 32-bit layouts), nothing else.

- **Status blocks of requests that finish later.** A 32-bit caller's `IO_STATUS_BLOCK*` is no
  longer copied into the scratch window: the thunk passes the caller's own address with bit 62 set
  (`io::irp::IOSB32`, not a canonical address, so a write that forgot to decode it faults instead of
  landing elsewhere). Every status-block write decodes it (`io::irp::iosb_write`: the 16-byte block,
  or the 8-byte one — `Status`, a 4-byte `Information` — at the caller's address): the services'
  `write_iosb`, the kernel's completion drain, the registry's change notifications,
  `NtFlushVirtualMemory`. The request keeps the marked value, so an asynchronous `ReadFile` or
  `WriteFile` writes the 32-bit `OVERLAPPED.Internal`/`InternalHigh` when it completes; an APC
  routine gets the plain address; `NtCancelIoFileEx`/`NtCancelSynchronousIoFile` name the request
  by the same marked value (kind `IR`).
- **APCs in 32-bit code.** The routine (a 32-bit code address, kind `V`) and its three arguments are
  written below the interrupted ESP (`wow64::apc_frame`, Poly's own frame) and the thread continues
  at the i686 `ntdll`'s `KiUserApcDispatcher`, which pops the routine, calls it (`stdcall`) and then
  `PolyResumeApc`, which restores the interrupted context as for a 64-bit thread (a routine that
  takes fewer arguments leaves the rest; the restore drops them). `QueueUserAPC`, timer APCs and
  I/O completion routines all take this path; a wait that returns `STATUS_USER_APC` runs every
  queued APC as before.
- **I/O completion entries.** `NtRemoveIoCompletion`'s outputs were already pointer-sized slots;
  `NtRemoveIoCompletionEx`'s `FILE_IO_COMPLETION_INFORMATION` array (kind `CE`) is run against
  32-byte entries in the window and written back as 16-byte ones (at most 1024 per call).
- **`NtGetContextThread` / `NtSetContextThread`.** `CX` (in) and `CO` (in and out): the x86
  `CONTEXT` becomes a 64-bit one for the service; `NtGetContextThread` writes back only the groups
  its x86 `ContextFlags` names (`wow64::context::merge_context32`), the others stay the caller's.
- **`NtCreateUserProcess` from a 32-bit parent.** The parameter block is read in the caller's own
  layout (`params::read_for` with `poly_rtl::layout::X86`: 4-byte pointers, 8-byte strings); the
  attribute list is Poly's own all-64-bit format and the token handle a value. The child is 32-bit
  or 64-bit by its image, and its parameters are written in its own layout, so a 32-bit parent
  starts a 64-bit child and back. `kernelbase` builds the block (`child_parameters_in`) and reads
  `STARTUPINFO` (`SiLayout`: 104 bytes x64, 68 in 32 bits, `GetStartupInfoW/A`) in the process's own
  layout; `PROCESS_INFORMATION` and `SECURITY_ATTRIBUTES` are pointer-sized. Every other
  structure `kernelbase` reads or writes in the caller's memory with pointer-sized fields is laid
  out in the process's own width too (`MEMORY_BASIC_INFORMATION`, the affinity masks,
  `FormatMessage`'s argument slots, `OVERLAPPED_ENTRY`, `FILE_RENAME_INFO`, the token groups,
  `MEM_EXTENDED_PARAMETER`, ...; `nt-wow64-2c`).
- **Information classes laid out in the caller's width** (kind `P`; the service asks the caller's
  layout): `NtQueryDirectoryObject` (`OBJECT_DIRECTORY_INFORMATION`: two 8-byte strings),
  `NtQueryObject` `ObjectNameInformation` / `ObjectTypeInformation`, `NtQueryVirtualMemory`
  `MemoryMappedFilenameInformation` (8-byte `UNICODE_STRING` headers), `NtQueryInformationToken`
  (`Token::query_width`: 4-byte pointers, 8-byte `SID_AND_ATTRIBUTES`, `TOKEN_GROUPS` without
  padding; `TokenAccessInformation` is `STATUS_NOT_IMPLEMENTED` for a 32-bit caller),
  `NtSetInformationToken` (the owner, primary group, default DACL and label pointers). The key
  information classes have no pointer-sized fields: unchanged. `NtSetInformationFile`'s
  `FileRenameInformation`, `FileLinkInformation` (and their `Ex` forms) and
  `FileCompletionInformation` are widened from the 32-bit layout by the thunk.
- **Kernel stack.** A 32-bit `NtCreateUserProcess` overflowed the 16 KiB kernel stack (the thunk's
  inlined helpers held an x86 and a 64-bit `CONTEXT` while the deep process creation ran):
  `prepare` and `write_back` are kept out of line.
- **Tests.** Host (`wow64::tests`, table-driven where the signature table allows): contexts both
  ways, APC frame and routine, completion packets and `Ex` entries, the marked status block,
  directory entries, `TOKEN_USER` and `ObjectNameInformation`, the rename/link/completion inputs, a
  32-bit parameter block and `STARTUPINFO` round trip (`params`, `poly-kernelbase`). Boot:
  `hello32.exe` gained `apc` (two `QueueUserAPC`s in an alertable `SleepEx`), `iocp` (overlapped
  `WriteFile`/`ReadFile` on a completion port) and `child` (itself as a 32-bit child checking its
  68-byte `STARTUPINFOW`, and the 64-bit `k32test.exe --child4`).

## Addendum: stage 4 started — 32-bit GUI (`nt-wow64-2`)

- **No callbacks from the kernel are needed.** Poly's `user32` calls window procedures itself (ADR
  0128: the window manager is reached through `PolyUserCall` with `poly_win32k::wire` records and
  blocking happens in `user32`), so a 32-bit window procedure is called by the i686 `user32` in the
  same process. Table 1 has two services; `wow64::sigs::WIN32K_SIGNATURES` gives their 32-bit
  rows (`PolyUserCall`: the reply length is a `SIZE_T*`; `PolyUserCommitSurface`), which also give
  `win32u`'s i686 stubs (`int 0xF4`, `ret 20`). The wire records are fixed-width, so a 32-bit
  `user32` sends the 64-bit one's bytes.
- **i686 `win32u`, `gdi32`, `user32`** are built by `cargo xtask wow64` from the same sources and
  installed in `SysWOW64`. What had a 64-bit layout baked in: the stubs' `u64` arguments (now
  pointer-sized), `poly_win32k`'s `CREATESTRUCTW`, `WINDOWPOS`, `MSG` and `PAINTSTRUCT` mirrors
  (pointer-sized fields are `usize`), `user32`'s `WNDCLASS(EX)W`, `GUITHREADINFO`,
  `TRACKMOUSEEVENT`, `ICONINFO`, `CURSORINFO`, `HIGHCONTRASTW` (`user32::cstructs`, with size
  assertions for both widths).
- **Painting needs a display.** The window manager invalidates and paints only on an interactive
  display (a headless kernel test sends no `WM_PAINT`, for 64-bit programs too), so the boot test
  calls `BeginPaint` itself and fills a memory DC; `gui32 --show --paint` checks `WM_PAINT` on
  Poly's desktop (`cargo xtask win-app run --mode desktop`).
- **Tests.** Host: a 32-bit `PolyUserCall` round trip. Boot: `gui32.exe`
  (`nt_wow64_32_bit_window_program_paints_and_gets_messages`): `RegisterClassExW` (48-byte class),
  `CreateWindowExW` with its 32-bit window procedure seeing `WM_CREATE` and its `lpParam`,
  `GetWindowTextW`, `GetClientRect`, `BeginPaint` (the 64-byte `PAINTSTRUCT`), `FillRect` and
  `GetPixel` on a memory DC, `SendMessageW`, `PostMessageW`, `GetMessageW`/`DispatchMessageW`,
  `DestroyWindow`, `PostQuitMessage`.
- **A 32-bit `polystub.dll`** (`poly_stubs::build_x86`): a missing import of a 32-bit program
  binds to a stub as on x64 (ADR 0155), but a 32-bit stub cannot return: the function is `stdcall`
  and pops its own arguments, and the import does not say how many. So the stub logs
  `stub: dll!name` and ends the process with `STATUS_NOT_IMPLEMENTED`; the load succeeds and the
  first missing function the program really calls is named. `cargo xtask win-app` builds it for a
  PE32 program against the i686 modules and puts it in `SysWOW64`.
- **WinSCP 6.7.1 (portable, 32-bit, C++ Builder)**: every import of its 32 DLLs binds (496 to the
  32-bit stubs: 26 of its DLLs have no i686 build yet), its start-up code runs, and it ends at its
  first missing function, `kernel32!SetThreadLocale`; behind that are the registry, COM, shell and
  common-control functions of the DLLs that are still x64-only (`advapi32`, `ole32`, `oleaut32`,
  `shell32`, `comctl32`, `comdlg32`, ...). The next step is building those for i686 as this stage
  did for `user32` and `gdi32`.

## Addendum: stage 3 — the other system DLLs, SafeSEH, and where WinSCP stands (`nt-wow64-3`)

### i686 builds of the other system DLLs

`cargo xtask wow64` now also builds `advapi32`, `ole32`, `oleaut32`, `shell32`, `shlwapi`,
`comctl32`, `version`, `winmm`, `ws2_32`, `imm32`, `uxtheme`, `dwmapi`, `shcore`, `secur32`,
`crypt32`, `winhttp`, `wininet`, `iphlpapi`, `wtsapi32`, `oleacc`, `powrprof`, `userenv`, `bcrypt`,
`bcryptprimitives`, `ncrypt`, `netapi32`, `setupapi`, `propsys`, `wintrust`, `cfgmgr32` and `dbghelp`
(`wow64::SYSTEM_MODULES`) from the sources of the x64 ones. The kernel's `build.rs` embeds every
`*.dll` of the directory (`WOW64_DLLS`, generated), the layout installs them in `SysWOW64`, and
`verify` checks the set (imports only between our modules, no decorated names, every imported name
exported). What the x64 sources assumed:

- **Imports.** A `raw-dylib` link of 32-bit code binds `_Name@N` unless it says
  `import_name_type = "undecorated"`: no DLL exports that, so the load failed with
  `0xC0000139`. Every link of the Win32 workspace now has the two `cfg_attr` forms, `cargo xtask
  ci lint` checks it (`wow64::check_links`), and the build repeats the check, so a new link
  without a 32-bit form fails with a file and line, not in a guest.
- **Layouts.** Found by `sys32.exe` (below) and fixed by pointer width, with host tests for both
  widths: `TOKEN_USER` (`GetUserNameW` skipped 16 bytes of header), the `SAFEARRAY` descriptor
  (`pvData` at 12 and `rgsabound` at 16, not 16 and 24), `CommandLineToArgvW`'s block (4-byte
  pointers), `URL_COMPONENTS` (60 bytes), the PEB read of `wtsapi32` (`fs:[0x30]`).
- **Not done**: the x64-only pieces of these DLLs that are not on a tested path (`inet`'s and
  `netrt`'s structures beyond `URL_COMPONENTS`, `dwrite`, `crypt32`'s certificate structures) have
  not been run in 32 bits.

`tests/windows/wow64/sys32.rs` (`nt_wow64_32_bit_program_uses_the_system_dlls`) runs 11 checks
through these DLLs in a 32-bit program: the registry, a token and its SID, COM and GUIDs, `BSTR`,
`VARIANT` and `SAFEARRAY`, `CommandLineToArgvW` and `SHGetFolderPathW`, `shlwapi` paths, a version
resource (`VS_FIXEDFILEINFO`), `timeGetTime` and `InitCommonControlsEx`, a TCP connection over the
loopback interface (the AFD structures in 32 bits), `BCryptGenRandom` and SHA-256, and
`GetUserNameExW`.

### The 32-bit `polystub.dll` returns when it knows the signature

`poly_stubs::x86_signatures.txt` lists, from the prototypes on Learn, how many 4-byte stack slots
the arguments of a missing function take (about 290 functions that WinSCP, 7-Zip, PuTTY and
Process Explorer import and Poly lacks). A function in the table gets a stub that logs
`stub: dll!name`, fails the way the x64 stub does (0 and `ERROR_CALL_NOT_IMPLEMENTED`,
`E_NOTIMPL`, `STATUS_NOT_IMPLEMENTED`) and returns with the right `ret 4n` (a plain `ret` for the
`cdecl` `wsprintfA`); any other still ends the process. A program therefore runs past its first
missing function, and a run lists every stub it reached (`stub calls reached`), which is what
makes WinSCP's remaining list readable. A wrong count would corrupt the stack, so an entry is
added from the function's page when a program reaches it, not guessed.

### Start-up calls and ANSI forms

`GetThreadLocale`/`SetThreadLocale` (one locale), `GetUser/SystemDefaultUILanguage`,
`GetThreadUILanguage`, `SetThreadPreferredUILanguages`, `SetHandleCount` (returns its argument),
`MulDiv` (`poly_kernelbase::muldiv`, halves away from zero), `GetEnvironmentStrings(A)`,
`FreeEnvironmentStringsA`, `GetModuleFileNameA`, `GetCurrentDirectoryA`; the i686 `kernel32` also
forwards `RtlUnwind` to `ntdll`. `poly.nt.trace` reads the calls of 32-bit threads (arguments at
`EDX`, 4-byte `UNICODE_STRING`/`OBJECT_ATTRIBUTES`), so a 32-bit program's file and registry names
show in the trace like a 64-bit one's.

### SafeSEH (stage 4 of the staged plan, "SafeSEH checks")

Learn ("/SAFESEH (Image has Safe Exception Handlers)", "PE Format: The Load Configuration
Structure"): a 32-bit image linked with `/SAFESEH` carries `SEHandlerTable` (the VA of a sorted
table of RVAs) and `SEHandlerCount` in its load configuration, and the system calls no other
handler of that image; `IMAGE_DLLCHARACTERISTICS_NO_SEH` says the image has none at all.
`poly_pe::handler_is_safe` decides from a mapped image: a handler in a table is safe, one not in
it is not (an empty table allows none), `NO_SEH` allows none, an image without a table (linked
without `/SAFESEH`, as Borland's linker and old Visual C++ do) allows any of its code, and code
outside every image (a thunk made at run time) is not judged. The i686 `ntdll`'s dispatcher asks
before it calls a frame's handler; a refused handler raises `STATUS_INVALID_EXCEPTION_HANDLER`
(`0xC00001A5`, non-continuable, naming the first exception), which ends the process.

lld marks an x86 image `NO_SEH` when it has no safe handler table, so the test programs, which
register their own frames, are linked `/safeseh:no` (legacy images); `safeseh32.exe` is linked
`/safeseh` with a `.safeseh` handler and checks both ways
(`nt_wow64_safe_exception_handlers_are_checked`): the listed handler continues, the unlisted one
is refused with `0xC00001A5`.

### The C runtime (stage 3 of the staged plan): not started

`poly-win32-crt`, `ucrtbase` and `msvcrt` do not build for i686 and need more than layout fixes:
`setjmp`/`longjmp` (x64 `RtlUnwindEx`, `gs:[8]`; x86 has its own `_JUMP_BUFFER` and unwinds the
`fs:[0]` chain), `LegacyInfo` (size and offsets asserted for 64 bits), `FILE`/`_iobuf` and
`va_list` (the formatter reads 8-byte slots; x86 slots are 4 bytes, 8 for `double` and 64-bit
integers), start-up (`__argc`, `_environ`), and `_except_handler3/4` with `_global_unwind2` on the
frame dispatcher. None of WinSCP, Inno Setup or NSIS needs it (Borland's runtime and NSIS's own
code import `kernel32` directly); Visual C++ programs built with `/MD` do, together with
`vcruntime140` (ADR 0199, which shares its C++ exception code with the 64-bit one).

### WinSCP 6.7.1

With the i686 DLLs, the GUI branch's menus, dialogs and `MessageBox` (not yet in `main` at the
time of writing, `impl-win-apps-gui`), the signature table and the start-up calls above, WinSCP
runs its Borland start-up: the runtime shows its own message box (the first window of a 32-bit
program that is not ours) while `GetEnvironmentStrings` was a stub, then gets through the
runtime's initialisation, the `Variants`/locale/registry probes, window-class registration and a
probe for `Msctf.dll`, and ends in a read past the end of a buffer in its own code
(`WinSCP.exe+0x3a90ca`, address `0x1811000`). It shows no main window yet. The next stoppers, in
the order the run reaches them: that crash (an unterminated or mis-sized value some call returned:
the `A` forms of `Char*Buff`, `GetLocaleInfoA`, `GetStringTypeA` and `LCMapStringA` are the
candidates: they are stubs that return 0), the `oleaut32` ordinals the Delphi `Variants` unit
probes (`VariantChangeTypeEx`, `VarNeg`), then the comctl32 controls (`SysListView32`,
`SysTreeView32`, toolbars, status bars) that its forms create.

## Addendum: stage 4 — WinSCP runs its start-up; what the hunt found (`nt-wow64-4`)

Stage 3 ended with WinSCP (32-bit, C++ Builder) crashing at `WinSCP.exe+0x3a90ca`. This stage found why, and
the next dozen reasons it did not get further. Every one is a place where a function written for the 64-bit
build had a layout or a width in it that a 32-bit program sees differently; each now has a test in
`tests/windows/wow64/ansi32.rs` (boot test `nt_wow64_32_bit_program_uses_the_ansi_and_locale_calls`) or a host test.

### The tool that found them: `poly.nt.apitrace`

A 32-bit program that fails inside its own code leaves no system call to read. `poly.nt.apitrace=<text>` starts
at the first system call whose object name contains `<text>` (`msctf`: the program's probe for `Msctf.dll`) and
single-steps that thread (`RFLAGS.TF`; the `#DB` of a traced thread is handled in `trap_dispatch`, `kernel/src/nt/apitrace.rs`).
The kernel logs every entry into an export of the process's other modules, read from Poly's own export tables
(`nt-api: kernel32.dll!Name ret=... args ...`, and `-> result` when the call returns), the string a program looks a
function, a DLL or a resource up by, and, at an exception, the last 64 instructions' registers. It reads no byte of
the program's code. `poly.nt.apitrace.after=<name>` then logs the registers of the first 400 steps after the
named call returns, and every 2,000,000th. The exception reports of 32-bit threads now name modules
(`WinSCP.exe+0xc004b0`), print the stack in 32-bit words and dump what the registers and the parameters point at
(`diag::modules32`, `exception_report_words`).

### What was wrong

| Symptom | Cause | Fix |
|---|---|---|
| The crash at `+0x3a90ca`: a hash loop over 4 GiB | The `RTL_CRITICAL_SECTION` was the 64-bit layout (40 bytes): `RtlInitializeCriticalSection` of a Delphi object's embedded section zeroed the 16 bytes after its 24, the `Word` field that held the resource manager's data size | `poly_rtl::critsec::CriticalSection` has pointer-sized fields (24 bytes in 32 bits); the section's offsets are the 32-bit ones |
| Calls through a NULL pointer after `WSAStartup` | `WSADATA` was written in the 64-bit layout (408 bytes) into a 400-byte structure, over the caller's saved registers | `write_wsadata` knows both layouts |
| Every `FindResourceW`, `LoadStringW`, `LoadIconW` failed (`EResNotFound` for the main form) | `kernelbase`'s resource functions read `VirtualQuery`'s answer as a 48-byte `MEMORY_BASIC_INFORMATION` | `resource.rs` reads the 28-byte one in a 32-bit process |
| `GetObject` of a bitmap, brush or DIB section gave a 64-bit structure | `BITMAP` is 24 bytes (`bmBits` at 20), `LOGBRUSH` 12 and `DIBSECTION` 84 in 32 bits | `Gdi::object_bytes_for_width` |
| Delay loads and `GetProcAddress` probes ended in a NULL call or `0xC06D007F` | The 32-bit modules export no fail-only stubs (a fail-only function takes no arguments and `stdcall` would leave them) | The functions that were reached are real, with their prototype's argument count: `uxtheme` (`BufferedPaint*`, the `GetTheme*` family), `SHGetSpecialFolderLocation`/`SHGetPathFromIDListW`/`ILCreateFromPathW`/`SHGetMalloc`/`CoGetMalloc` (`IMalloc`), `CryptAcquireContext*`/`CryptGenRandom`, `SetClassLong*`, `GetNearestColor`, `CreateBrushIndirect`, `CreatePenIndirect`, `SetROP2`, `UnrealizeObject`, `WSAAsyncSelect` (a helper thread and `PostMessage` over the `WSAEventSelect` machinery), `wsprintfA/W` (`poly_kernelbase::wsprintf`), `GetShortPathNameW`, `CreateToolhelp32Snapshot`/`Process32FirstW`/`Process32NextW` (the calling process only), `FindFirstFileA`/`FindNextFileA`/`FindFirstFileExA` |
| `shfolder.dll` not found | A redistributable whose functions live in `shell32` | The loader maps the name to `shell32.dll` (`poly_ldr::apiset::resolve`), with `SHGetFolderPathA` |
| Pointer encoding broke on a 32-bit cookie | `EncodePointer` rotated 64 bits and cut the result | `encode_pointer32` |

The ANSI and locale family the stage-3 notes suspected (`GetLocaleInfoA`, `GetStringType*`, `LCMapStringA`, `CompareStringA`,
`GetCPInfoEx*`, `EnumCalendarInfo*`, `GetCalendarInfoW`, `CharUpperBuffW` and the other `Char*`/`IsChar*` functions, `MulDiv`)
was implemented too, from Learn, with host tests and `ansi32.exe`; it was not the cause of the crash. `msi.dll` functions fail
with a non-zero code (`HRESULT_DLLS`); `msi!#205` (`MsiEnumRelatedProductsW`) has its signature.

### Where WinSCP stands

WinSCP runs its Borland start-up and its VCL unit initialisation, creates its application windows, loads
`TGlyphsModule`, opens the cryptographic random source and Winsock, and reaches the code that builds its image
lists. It then faults in its own code at `WinSCP.exe+0xc004b0` (a write at address 1) after `gdi32!GetDIBits`,
which is not implemented yet (the 32-bit `polystub.dll` fails it): the bitmaps of its glyph image lists come back
empty. No window yet. Next stoppers, in order: `GetDIBits`/`SetDIBits`/`CreateDIBitmap`; the `oleaut32` `Var*`
functions the Delphi `Variants` unit probes; `ws2_32`'s `addrinfo`, `hostent` and `WSABUF` structures, which are still
the 64-bit layouts in the 32-bit module (not run: WinSCP does not connect before its dialog); the comctl32 controls
(`SysListView32`, `SysTreeView32`) its forms create.

### The C runtime

Not started in this stage; the blockers of stage 3 stand (setjmp/longjmp, `LegacyInfo`, `FILE`, `va_list`, start-up).

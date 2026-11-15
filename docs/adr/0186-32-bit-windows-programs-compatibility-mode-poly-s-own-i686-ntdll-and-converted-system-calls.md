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

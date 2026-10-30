# 0089: `CreateProcessW`: handle inheritance, `STARTUPINFO` and its standard handles, `CREATE_SUSPENDED`

**Status:** Accepted. Completes what [ADR 0079](0079-our-kernelbase-and-kernel32.md)'s
second slice left of `CreateProcessW`; builds on [ADR 0080](0080-console-host.md)
(console and standard handles in the process parameters), [ADR 0030](0030-nt-object-manager-and-dispatcher-in-a-crate.md)'s
handle tables (`poly-ntobj`, which already copied inheritable handles), [ADR 0065](0065-nt-threads-synchronisation-and-apcs.md)
(suspension) and [ADR 0087](0087-7-zip-command-line-on-poly.md) (its `GetStartupInfoW`
returned `cb` and zeros).

## Context

The roadmap's `kernelbase` / `kernel32` item had two things left:
`CREATE_SUSPENDED` and handle inheritance in `CreateProcessW`. The
7-Zip milestone needs more than the flag: a driver runs `7za.exe` through
`CreateProcessW` with its standard output and error redirected to files
(`STARTUPINFOW` with `STARTF_USESTDHANDLES` and inheritable handles),
waits for it and reads its exit code; 7-Zip asks `GetFileType` and
`GetConsoleMode` whether its output is a console. Until now the child got
its creator's descriptors 0–2 as standard handles whatever it was given,
no handles were inherited, `CREATE_SUSPENDED` was `ERROR_NOT_SUPPORTED`,
and the `STARTUPINFO` was dropped.

Sources (clean room): Learn's pages of `CreateProcessW`, `CreateProcessA`,
`STARTUPINFOW`, `GetStartupInfoW`, `GetStartupInfoA`, `PROCESS_INFORMATION`,
`SECURITY_ATTRIBUTES`, `SetHandleInformation`, `GetHandleInformation`,
`DuplicateHandle`, `ResumeThread`, "Handle Inheritance", "Inheritance
(Processes and Threads)", "Creating a Child Process with Redirected Input
and Output", "Creation of a Console", "Console Handles"; the layouts and
constants of the public headers (`processthreadsapi.h`: `STARTUPINFOW`,
`STARTF_USESTDHANDLES`; `winbase.h`: `CREATE_SUSPENDED`,
`CREATE_UNICODE_ENVIRONMENT`, `HANDLE_FLAG_*`; `winnt.h`: `DUPLICATE_*`;
`minwinbase.h`: `SECURITY_ATTRIBUTES`; `ntdef.h`: `OBJ_INHERIT`;
`winerror.h`); export names from the coverage map. No Windows binary,
Wine or ReactOS source was looked at.

## Decision

### Inheritable handles, set and read

A handle's flags were already the object manager's (`poly_ntobj::HandleFlags`:
inherit, protect from close), set by `OBJ_INHERIT` at creation and by
`NtDuplicateObject`'s attributes. New:

- **`NtSetInformationObject`** (service `0x011`, its own table
  `HANDLE_SERVICES`, so it merges beside other new services) and a new
  `NtQueryObject` class: **Poly's own `ObjectHandleFlagInformation`**,
  class 4, two `BOOLEAN`s `Inherit`, `ProtectFromClose`. No public header
  defines the class or the structure; to be confirmed by the spec agent.
  A wrong length is `STATUS_INFO_LENGTH_MISMATCH`, a pseudo handle
  `STATUS_INVALID_HANDLE`.
- `kernelbase` (`handles.rs`): **`GetHandleInformation`** (the two
  `HANDLE_FLAG_*` bits), **`SetHandleInformation`** (Learn: `dwMask` picks
  the flags that change, `dwFlags` gives them; unknown mask bits are
  `ERROR_INVALID_PARAMETER`, a guess), **`DuplicateHandle`**
  (`bInheritHandle` is `OBJ_INHERIT`; the options pass through). A handle
  protected from close fails `CloseHandle` (`STATUS_HANDLE_NOT_CLOSABLE`
  → `ERROR_INVALID_HANDLE`).
- `SECURITY_ATTRIBUTES.bInheritHandle` was honoured by `CreateFileW`, the
  synchronisation objects and registry keys already; `CreateProcessW`'s
  two now make the returned process and thread handles inheritable.

### `bInheritHandles`: the creator's inheritable handles at the same values

`kernelbase` passes it to `NtCreateUserProcess` as **Poly's own
`ProcessFlags` bit `0x4`** (`PROCESS_CREATE_FLAGS_INHERIT_HANDLES`; the
flags are in no public header). The kernel then creates the child's
handle table from the creator's (`ObjectManager::create_process(child,
Some(creator))`, ADR 0030: every inheritable handle at the same index,
same access, same flags) **before any other handle of the child is
made** (`exec::start_process_from`), so the values are free; the child's
console and standard handles come after, in the free slots. Learn: "An
inherited handle refers to the same object as the original handle ...
has the same value and access privileges".

### The `STARTUPINFO` goes into the child's parameters

`winternl.h` places nothing of `STARTUPINFO` in
`RTL_USER_PROCESS_PARAMETERS`, so **Poly's own placement**, after ADR
0079's fields (to be confirmed by the spec agent): `cb` at + 0xA0 (0:
none was given), `dwX` … `cbReserved2` exactly as `STARTUPINFOW` lays
them out at + 0xA4 (`dwFlags` lands at + 0xC0), `lpDesktop`, `lpTitle`,
`lpReserved` as `UNICODE_STRING`s at + 0xC8, + 0xD8, + 0xE8 (a null
`Buffer` keeps a null pointer apart from an empty string), and the
`lpReserved2` bytes' address at + 0xF8 (the C runtime's way of passing
descriptors; `cbReserved2` of them). `poly_kernelbase::process`
(`Startup`, `child_parameters`, host-tested) writes them; the NT side
(`params::Startup`, `params::read` / `image`) copies them into the child
as they are.

- **`STARTF_USESTDHANDLES`**: `kernelbase` puts `hStdInput`, `hStdOutput`,
  `hStdError` in the standard-handle slots ADR 0080 defined (+ 0x20, 0x28,
  0x30) and the kernel gives them to the child **as they are**
  (`Parameters::given_std_handles`): values in the child's table, which
  is why Learn requires them inheritable and `bInheritHandles` TRUE. A
  console program still attaches to a console (for `CONOUT$`), but the
  console's own input/output handles are closed for the slots given.
  Without the flag, ADR 0080's rule stands (the creator's descriptors,
  else the console: Learn's default "is the keyboard buffer" / "the
  console window's buffer").
- **`GetStartupInfoW`** reads it back (`poly_kernelbase::process::startup_info`):
  the fields, its parameters' own copies of the strings, and the standard
  handles when `dwFlags` has `STARTF_USESTDHANDLES`. A process the kernel
  started has none and gets `cb` and zeros, as before.
- **`GetStartupInfoA`** is `kernel32`'s own (no `kernelbase` export of
  that name in the coverage map): `GetStartupInfoW`'s structure with the
  three strings converted to the ANSI code page once, kept for the life
  of the process.

### The child's console and the standard handles the kernel opens

- A child with no terminal among its creator's descriptors (a
  `CreateProcessW` child has none) attaches to **its creator's console**
  (`IoState::console_of_process`) instead of making a new hidden one —
  Learn: a console process "inherits its parent's console" unless asked
  otherwise (`CREATE_NEW_CONSOLE` and `DETACHED_PROCESS` are accepted and
  not acted on yet).
- **The standard handles the kernel opens for a process (console handles
  and redirected files) are inheritable** — Poly's choice, as a shell's
  redirections are: so a program can pass its own standard handles to a
  child with `STARTF_USESTDHANDLES` (the usual pattern) and they are
  valid there. To be confirmed by the spec agent.

### `CREATE_SUSPENDED`

`kernelbase` passes `THREAD_CREATE_FLAGS_CREATE_SUSPENDED`. The kernel
creates the process completely (memory, image, parameters, handles, the
creator's handles to it) but does not start its first thread: it
records the thread's start with suspend count 1 and the new process and
memory (`ThreadCtl::deferred_process`). `NtResumeThread` on the creator's
thread handle drops the count, and at zero starts the thread
(`sched::resume`; it also finds a thread's own process now, so suspending
and resuming another process's threads reach them). `ResumeThread`
returns 1 the first time, as Learn's suspend count says. Until then the
process is `STILL_ACTIVE` and its handle unsignalled.

### `CreateProcessA`

Converts `lpApplicationName`, `lpCommandLine`, `lpCurrentDirectory` and
`STARTUPINFOA`'s strings from the ANSI code page and runs the same code
as `CreateProcessW`; the environment block is passed as it is (Learn: ANSI
unless `CREATE_UNICODE_ENVIRONMENT`).

### The console/file distinction (already so, now tested)

A redirected standard handle is an ordinary file handle (ADR 0080):
`WriteFile` writes the file, `GetFileType` is `FILE_TYPE_DISK`, and
`GetConsoleMode` fails with `ERROR_INVALID_HANDLE` (the console ioctl is
`STATUS_INVALID_DEVICE_REQUEST` on a file). The child test checks all
three on its inherited handles.

## To be confirmed by the spec agent against real Windows

1. Where `RTL_USER_PROCESS_PARAMETERS` keeps the `STARTUPINFO` fields
   (Poly: + 0xA0 … + 0x100) and the standard handles (ADR 0080's slots);
   what `GetStartupInfoW` returns for the standard handles without
   `STARTF_USESTDHANDLES` (Poly: zero) and for a process not started by
   `CreateProcess`.
2. Whether a process's initial console and redirected standard handles
   are inheritable (Poly: yes).
3. `SetHandleInformation` with unknown mask bits (Poly:
   `ERROR_INVALID_PARAMETER`), on a pseudo handle (Poly:
   `ERROR_INVALID_HANDLE`); `GetHandleInformation` of a pseudo handle
   (Poly: 0 flags); `CloseHandle` of a protected handle without a debugger
   (Poly: FALSE, `ERROR_INVALID_HANDLE`).
4. Whether an inherited handle keeps `HANDLE_FLAG_INHERIT` in the child
   (Poly: yes, the flags are copied).
5. `STARTF_USESTDHANDLES` with `bInheritHandles` FALSE (Poly: the values
   are passed as they are, valid or not).
6. The `NtCreateUserProcess` flag Poly uses for inheritance (bit 2 of
   `ProcessFlags`) and the `ObjectHandleFlagInformation` class number.

## Consequences

- Host tests: `cargo test -p poly-kernelbase process` (the `STARTUPINFO`
  in and out of the parameters, the handle-flag rules) and `cargo test -p
  poly-personality-nt` (the parameters round trip with a `STARTUPINFO`,
  `NtSetInformationObject` / `NtQueryObject` handle flags and close
  protection, a child inheriting exactly the inheritable handles at their
  values and nothing without the flag, a suspended child resumed through
  its thread handle, a creator's console found for its child).
- The ring-3 test (`k32test.exe`) gains `inheritance_and_startup`: the
  handle flags through `SECURITY_ATTRIBUTES`, `SetHandleInformation`,
  `GetHandleInformation`, protection from close and `DuplicateHandle`;
  a child (`--child3`) created suspended and inheriting, with standard
  output and error redirected to two files: it has not run before
  `ResumeThread`, then checks its `GetStartupInfoW`, that its standard
  handles are disk files and not consoles, writes both files, sets the
  creator's event through an inherited duplicate and finds a
  non-inheritable handle absent; the creator reads both files back and
  checks the exit code. A second child (`--child4`) is made by
  `CreateProcessA` with an ANSI environment block and checks
  `GetStartupInfoA`.
- Edits to shared files are additive: a service table and a dispatch arm,
  a `NtQueryObject` class, two `ProcessCreate` fields, `Parameters`
  fields with `..Default::default()` at the two other constructions,
  `exec::start_process_from` (`start_process` unchanged), a `start_child`
  argument (the fake kernel's only caller), `console::setup`'s
  `FromCreator`, a `ThreadCtl` field, `IoState::console_of_process`,
  `KERNEL32_FORWARDS` / `KERNEL32_OWN`, a new `kernelbase` module.
- Not done: `CREATE_NEW_CONSOLE` / `DETACHED_PROCESS` /
  `CREATE_NO_WINDOW` (accepted, ignored), `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`
  (`STARTUPINFOEX`), `TerminateProcess` of a process whose first thread
  never started (it stays until resumed), anonymous pipes (`CreatePipe`)
  for redirection to the creator.
- The coverage entries claimed by `impl-createprocess`: `CreateProcessA`,
  `DuplicateHandle`, `GetHandleInformation`, `SetHandleInformation` in
  `kernel32.dll` and `kernelbase.dll`, `GetStartupInfoA` in
  `kernel32.dll`, `NtSetInformationObject` / `ZwSetInformationObject` in
  `ntdll.dll`.

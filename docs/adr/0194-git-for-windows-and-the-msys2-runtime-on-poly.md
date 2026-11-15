# 0194: Git for Windows and the MSYS2 runtime on Poly

**Status:** Accepted. Track `win-git-bash`: Git for Windows (MinGit and
PortableGit 2.56.1) and git-bash's runtime, MSYS2's `msys-2.0.dll` behind
`usr/bin/sh.exe`. Inputs: Microsoft Learn's reference pages, the public SDK
headers (`winnt.h`, `ntstatus.h`, `iptypes.h`, `wdm.h`) and what the programs
did on Poly (their output and Poly's own traces: `poly.nt.trace`,
`poly.nt.trail`, `poly.nt.waits`, `poly.nt.exceptions`). The programs are black
boxes; no Git, MSYS2, Cygwin, Wine or ReactOS source was read.

## What was there

`cmd/git.exe` started `ucrt64/bin/git.exe` (ADR 0155's lazy images) and that
stopped on `fatal: could not open '/dev/null'`, printed to the console rather
than to the wrapper's output. `sh.exe` ran into `STATUS_GUARD_PAGE_VIOLATION` on
its own stack and then spun until the time-out. Behind those, in the order they
came up: `rand_s`, `NtAllocateLocallyUniqueId`, `AddVectoredContinueHandler`,
`iphlpapi!GetNetworkParams`, `ReadProcessMemory` of another process, and
`TerminateThread` were missing or fail-only.

## Decisions

### 1. `NUL` is a device

`\Device\Null`, with the symbolic link `\??\NUL`, is the device the reserved
name `NUL` (Learn: "Naming Files, Paths, and Namespaces") reaches through
`\\.\NUL`. Every create disposition opens it (`FILE_OPENED`), a directory open
is `STATUS_NOT_A_DIRECTORY` and a name below it `STATUS_OBJECT_NAME_NOT_FOUND`.
A write takes every byte and drops it; a read is at the end of the file
(`STATUS_END_OF_FILE`, nothing read). `FileFsDeviceInformation` says
`FILE_DEVICE_NULL` (0x15, `wdm.h`), so `GetFileType` is `FILE_TYPE_CHAR`, as
Learn's "Console Handles" says of `NUL`. Git opens `/dev/null` as `nul`.

### 2. A child inherits its creator's standard handles

Learn ("Console Handles"): "the parent's standard handles are inherited by any
child process"; `GetStdHandle`: a console program's handles are filled "if the
parent didn't already fill the standard handle table by inheritance".
`CreateProcessW` without `STARTF_USESTDHANDLES`, with `bInheritHandles` and
without `CREATE_NEW_CONSOLE`, `DETACHED_PROCESS` or `CREATE_NO_WINDOW` puts the
creator's three standard handles where ADR 0089 keeps chosen ones in the
parameters; the kernel keeps each that is a handle in the child's table after
inheritance and gives the child its console's handle for the others, as before.
`GetStartupInfoW` does not report them (the flag is not set). Git's wrapper
`cmd/git.exe` relies on it: its child's output reached the console instead of
the wrapper's pipe.

Not decided, left as before (the child gets its console's handles): the
same call with `bInheritHandles` FALSE, and with a new or no console. The spec
agent is to record what Windows passes there.

### 3. A stack the TEB names grows like a stack Poly made

ADR 0091 grows a thread's stack through its guard page for stacks Poly
allocated. The MSYS2 runtime runs its main thread on memory it reserved itself
and points `NT_TIB.StackBase` (`winnt.h`) at it. A guard-page fault in the
allocation that holds the faulting thread's `StackBase - 1`, below `StackBase`,
now grows that stack (the page below becomes the guard page, `StackLimit`
moves) instead of raising `STATUS_GUARD_PAGE_VIOLATION`; a guard page anywhere
else still raises it. `Backend::read` lets the personality read the TEB.

### 4. Another process's memory

`NtReadVirtualMemory` and `NtWriteVirtualMemory` (services 0x04B and 0x04C, the
`ntdll` export names) are `ReadProcessMemory` and `WriteProcessMemory` of
another process: page by page through the target's address space, and a page
that cannot be read or written ends the copy with `STATUS_PARTIAL_COPY`
(`ERROR_PARTIAL_COPY`) and the bytes done (Learn: "the entire area ... must be
accessible"). The handle needs `PROCESS_VM_READ`, or `PROCESS_VM_WRITE` and
`PROCESS_VM_OPERATION`. `NtAllocate/Free/Protect/Query/FlushVirtualMemory`
take a handle to another process too (through the same `with_process` as
ADR 0169's views), so `VirtualAllocEx`, `VirtualFreeEx`, `VirtualProtectEx` and
`VirtualQueryEx` exist. The MSYS2 runtime's child copies its parent's runtime
heap this way.

### 5. Smaller ones

- `NtAllocateLocallyUniqueId` (service 0x143) takes the next id of the
  counter tokens' ids come from: unique for the boot, never 0.
- `AddVectoredContinueHandler` / `RemoveVectoredContinueHandler`
  (`RtlAdd/RemoveVectoredContinueHandler`) keep a list of their own. Learn
  does not say when continue handlers run, so Poly does not call them yet.
- `rand_s` reads the kernel's random device, as `RtlGenRandom` does.
- `GetNetworkParams` fills `FIXED_INFO` (`iptypes.h`, 600 bytes on x64): the
  host name, no domain, the Ethernet adapters' DNS servers once each,
  `BROADCAST_NODETYPE`, DNS on; a short or missing buffer is
  `ERROR_BUFFER_OVERFLOW` with the size.
- `TerminateThread`, `GetThreadContext` and `SetThreadContext` call the
  kernel's services (`sh.exe` ends a waiter thread with them).

### 6. A check that keeps it working

`cargo xtask win fetch-git` downloads the pinned MinGit 2.56.1 zip (size and
SHA-256 checked) into `target/mingit/<version>` and unpacks it with the pure-Rust
ZIP reader, so it works on Windows and Linux hosts (ADR 0124). The launcher of
`chrome-win` runs `stage<N+1>.cmd` after a stage whose line 7 is `next`
(`cargo xtask win-app run --then "ARGS"`), so one boot runs several programs on
one disk. The boot group `git-win` (shard `sevenzip-b`; CI keeps MinGit in a
cache, the group is skipped without it) runs `sh.exe -c "echo hi && /usr/bin/ls
/cmd"`, `git --version`, `init`, `config`, `add`, `commit` and `log --stat`
and checks their output and exit codes.

## Consequences

- MinGit's `git.exe` works for local repositories; MSYS2's `sh.exe -c` runs
  builtins and one program.
- `fork()` in the MSYS2 runtime does not work yet: the parent writes to its
  signal pipe (`\\.\pipe\msys-<id>-<pid>-sigwait`) and waits on an event that
  the runtime's signal thread would set, and that thread is never created
  (no `CreateThread` call is made). Which Poly answer makes the runtime skip
  it needs the program observed on Windows; the hand-back says what to record.
- `IMAGE_SCN_MEM_SHARED` sections (MSYS2's `.cygwin_dll_common`) are still
  private to each process.

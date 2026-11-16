# MSYS2 pipelines: why `sh.exe -c "ls / | head"` hangs on Poly (observation note)

Observed by spec agent `spec-win-apps-6`, 2026-11-16. Behaviour only: the calls of the unmodified `sh.exe` into `kernel32` and
`ntdll` as the repository's call tracer (`cargo xtask win trace-kit`, ADR 0090) records them on Windows, Poly's own
`poly.nt.trace` of the same program, and test programs of our own. Nothing of MSYS2, Cygwin or Windows was read or disassembled.
Continues [fork.md](fork.md).

## What hangs

PortableGit 2.56.1, `usr\bin\sh.exe -c "/usr/bin/ls / | /usr/bin/head -n 3"` (full paths: `sh -c` without a login shell has an empty `PATH`).

- **Windows** (this PC, interactive session; homeserver2 session 0 gives the same call order): exit 0, `LICENSE.txt README.portable bin`.
- **Poly** (main plus the merge of the `impl-win-apps-4` branch; `poly.nt.trace`): the process runs until the launcher's 60 s limit. The whole
  trace holds **one** process (`sh.exe`, two threads): no fork, no `CreateProcess`. After start-up the main thread calls
  `NtOpenFile("\Device\NamedPipe\")` (success) and then **`NtCreateNamedPipeFile` 8,850 times, every call `STATUS_INVALID_DEVICE_REQUEST`
  (0xC0000010)**: the runtime's `pipe()` retries with a new name after every failure and never gives up. The second thread (the signal
  thread, since impl-win-apps-4) waits in `NtReadFile` on the `sigwait` pipe, as it should. `sh.exe -c "/usr/bin/true; echo hi"` has no
  `|`, so it never makes this call and works.

## The call, as Windows makes it (parent, in `pipe()`)

| step | call | answer on Windows |
|---|---|---|
| 1 | `NtOpenFile(&dir, 0x100080 = SYNCHRONIZE \| FILE_READ_ATTRIBUTES, "\Device\NamedPipe\", ...)` | success; `IoStatus.Information` = `FILE_OPENED` (1) |
| 2 | `RtlInitUnicodeString(&name, "<install-id>-<pid>-pipe-nt-0x1")` (a name **relative** to `dir`, no backslash) | |
| 3 | `NtCreateNamedPipeFile(&rd, 0x80100100 = GENERIC_READ \| SYNCHRONIZE \| FILE_WRITE_ATTRIBUTES, {RootDirectory = dir, name}, ...)` | success, `FILE_CREATED` (2); the read end (inbound server end of a byte pipe) |
| 4 | `NtOpenFile(&wr, 0x40100080 = GENERIC_WRITE \| SYNCHRONIZE \| FILE_READ_ATTRIBUTES, {RootDirectory = dir, same name}, ...)` | success, `FILE_OPENED`; the write end |
| 5 | `NtSetInformationFile(rd and wr, ..., 8 bytes, FilePipeInformation)` | success |
| 6 | `NtQueryObject(rd, ObjectNameInformation)` | `\Device\NamedPipe\<name>` |
| 7 | `CreateMutex`es `cygpipe.input.mutex.<pid>-0x1` / `.output.` and a semaphore, `DuplicateHandle(self, self, self, &h, 0x100158, TRUE, 0)` | success |

The same name again with `FILE_CREATE` gives `STATUS_INSTANCE_NOT_AVAILABLE` (0xC00000AB) (one instance), which is what the runtime's name loop
reacts to by taking the next number. Poly answers 0xC0000010 for the first call, so no name ever works. What Poly does support is
`NtCreateNamedPipeFile` with an absolute name (`\??\pipe\...`, which `CreateNamedPipeA` and the `sigwait` pipe use): the visible tests of
`NtCreateNamedPipeFile` pass. The tests for the relative form are in the `NtOpenFile` entry (`pipe_root_pair`, `pipe_root_ops`).

## Data and close semantics the pipeline needs

The same tests record: data written to the write end is read from the read end; `FilePipeLocalInformation` of the read end (type byte, config
inbound, max 1, current 1, state connected 3, end server 1; the client end reports end 0); a read after the last write handle closes is
`STATUS_PIPE_BROKEN` (0xC000014B) (this is the pipe's end of file); a write after the read end closed is `STATUS_PIPE_CLOSING` (0xC00000B1)
(what the runtime turns into `SIGPIPE`); a second write handle (a duplicate) keeps the pipe open; in no-wait mode an empty pipe gives
`STATUS_PIPE_EMPTY` (0xC00000D9). Reading from the write end or writing to the read end is `STATUS_ACCESS_DENIED`.

## Other calls the two children make that Poly stubs (not the cause of the hang)

- `kernel32!QueryDosDeviceW`: every MSYS2 process calls `QueryDosDeviceW(NULL, buf, 65536)` (the whole list: about 55,000 units on this PC),
  then once per name. Poly's stub fails it; the runtime goes on. The `QueryDosDeviceW` sheet gives the list and the per-name answers.
- `kernel32!GetCommState` (the C runtime's check of each standard handle for a serial port): Poly's stub fails with 120; Windows fails with
  `ERROR_INVALID_FUNCTION` (1) for a file or device and `ERROR_INVALID_HANDLE` (6) for a bad handle and clears the DCB.
- `ntdll!RtlSetProcessPlaceholderCompatibilityMode`: probed through `GetProcAddress`; Poly has no such export (the runtime copes).
- "Couldn't compute FAST_CWD pointer": printed by the runtime on Poly; not on Windows. What it reads to compute it is not visible from the
  outside (it is not one of the calls above); it is left open here.

## Recipe

`C:\Users\Marc.Blattmann\poly-generated\spec-win-apps-6\msys2\run-local.ps1 -Cmd '...' -Label x` runs the traced tree on a Windows PC (kit of
`sh.exe` and `msys-2.0.dll`; tracing `ls.exe` and `head.exe` as well makes their fork children die with "couldn't create signal pipe", so only
`sh.exe` is traced). On Poly: `cargo xtask win-app run --dir <PortableGit> --exe usr/bin/sh.exe --args '-c "..."' --kernel "poly.nt.trace"`.

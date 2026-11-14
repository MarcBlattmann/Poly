# 0186: What Chrome's sandbox needs of the NT personality: window stations and desktops as objects, mitigation policies per process, known DLLs under a token

**Status:** Accepted. Works toward stage S2 of the Chrome-for-Windows ladder
([0155](0155-chrome-for-windows-bring-up-stubs-lazy-images-and-the-stage-ladder.md)),
Chrome with its sandbox on. Builds on the security model
([0171](0171-windows-security-model-tokens-descriptors-and-access-checks.md): tokens,
descriptors, restricted tokens, integrity labels) and on the process attributes
([0169](0169-named-pipes-completion-ports-jobs-and-cross-process-services.md): handle list, job
list, mitigation). The rules are the sheets of `spec-chrome-win-s2`
(`specs/windows/user32.dll/CreateWindowStationW.yaml` and its neighbours, `kernel32.dll/UpdateProcThreadAttribute.yaml`,
`advapi32.dll/RegOpenKeyExW.yaml`, `kernel32.dll/LoadLibraryExW.yaml`); nothing here was read from
Chrome or from Microsoft's binaries.

## Context

Chrome's broker makes, for each confined child, an alternate window station and desktop, a restricted
primary token lowered to Untrusted or Low integrity, an impersonation token for the child's
main thread, and a list of creation attributes (mitigation words, child-process policy, job). The child
then starts on `station\desktop`, lowers its own token (`RevertToSelf`), and from then on cannot open
what its restricted token cannot. Poly had the tokens and the access checks; what it lacked was
the objects the broker names (no window station or desktop existed), the creation-time policies
(a child's `GetProcessMitigationPolicy` read the *parent's* statics), and the three places where a
confined process meets the file system and the registry at start-up.

## Decision

### 1. Window stations and desktops are objects

`WindowStation` and `Desktop` are object types of the object manager with Learn's generic
mappings and rights (`WINSTA_*`, `DESKTOP_*`). A registry
(`poly_win32k::winsta::Registry`) finds them by name (case-insensitive, a desktop in its station).
`user32`'s twelve functions send one `Request::Winsta` through `PolyUserCall`; the NT personality
answers it (`personalities/nt/src/winsta.rs`) because it makes the handles and reads the caller's
descriptor. The window manager does not model several desktops: the names, descriptors, handles and
lifetimes are real, the windows of every process stay on the one desktop.

- **Lifetime.** The object manager deletes a station or desktop with its last reference: handles,
  the processes on a station, the threads on a desktop, and a desktop's hold on its station. The
  registry forgets the name then (`WinSta::deleted`). `WinSta0` and `Default` are made on first use and
  never go.
- **Security.** A new object takes its creator's default descriptor with the creator's integrity as a
  no-write-up label (the same label processes, tokens and jobs carry), unless the call gave one. The
  creator of a *station* gets the access it asked for whatever the descriptor says (the sheet: every
  mask gives a handle with exactly those rights, access 0 included); the creator of a *desktop* has to
  be granted it by the descriptor, else the call fails with 5 (the sheet: a descriptor naming only
  SYSTEM and the AppContainer group makes `CreateDesktopW` fail for its caller).
- **Process and thread.** A process is on one station and its handle (`GetProcessWindowStation` is
  always the same handle; `SetProcessWindowStation` makes exactly that handle the answer and the
  handle then cannot be closed, error 170); each thread is on one desktop (`GetThreadDesktop` the
  same handle every time, the desktop a thread is on cannot be closed through its handle).
- **Start.** A process started with `lpDesktop` names its station and desktop. `user32`'s `DllMain`
  connects: both have to exist, and the process's token (the thread's impersonation token first) has
  to be granted all specific rights on both, or `user32` does not initialise and the process ends with
  `STATUS_DLL_INIT_FAILED` (0xC0000142). Processes that were not told inherit their creator's station
  and desktop and are not checked, as on Windows (a Untrusted child started on the creator's desktop
  runs). The consequence the sheet records follows from the label: a Low or Untrusted token only
  starts on objects whose descriptor has a DACL for it and a label at its level or lower.
- **Information.** `GetUserObjectInformationW` answers `UOI_FLAGS`, `UOI_NAME`, `UOI_TYPE`,
  `UOI_USER_SID` (0 bytes), and for desktops `UOI_HEAPSIZE` and `UOI_IO`; `SetUserObjectInformationW`
  sets the flags. The order of the checks, the sizes and the errors are the sheet's
  (`poly_win32k::winsta::object_info`, host-tested).

### 2. Mitigation policies belong to the process, in the kernel

`poly_kernelbase::mitigation::State` holds one flags word per `PROCESS_MITIGATION_POLICY` and
is the kernel's per-process state. A child's creation attributes (the 16-byte
`PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY` of two 64-bit words, `PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY`)
set it before the child runs a line: the 2-bit fields of `winbase.h` become the flags each
`GetProcessMitigationPolicy` reads (`ExtensionPointDisable`, `FontDisable`, `ImageLoad`,
`SystemCallDisable` with the win32k bit 0 and the FSCTL bit 2, `SideChannelIsolation`, `Signature` 5 for
Microsoft-signed-only, `ChildProcess`, `DynamicCode`; the shadow-stack policy reads
`CetDynamicApisOutOfProcOnly` by default). `kernelbase` reads and writes it through
`NtQueryInformationProcess` / `NtSetInformationProcess` class 52 (before, it kept per-process statics that a
child never saw). The kernel enforces the policies that act on it: dynamic code and win32k (every
table-1 call is refused; `user32`'s `DllMain` takes the refusal as a failed initialisation, so
`LoadLibraryW("user32.dll")` fails with `ERROR_DLL_INIT_FAILED`, 1114). The `0x2001A` attribute
(component filter) is accepted and has no visible effect. `CreateProcessW` that succeeds sets the last
error to 0 (the sheet).

### 3. Known DLLs under a token

Windows opens a known DLL's section from `\KnownDlls` with the caller's token; every other DLL is
found by opening files. After a process lowered its token a library that is not loaded can
be refused two ways: a known one (`msctf.dll`) with `ERROR_ACCESS_DENIED`, any other
(`version.dll`) is just not found (126). `poly_ldr::path::is_windows_known` lists the names Windows
11 has as known; for such a name the loader asks the process (`Process::known_dll_access`) before it looks
for a file, and `ntdll` answers by opening `System32` with the thread's token: a restricted token that
matches nothing there gets `STATUS_ACCESS_DENIED`. A process whose main thread has no impersonation
token cannot even start (its first import is `kernel32`: 0xC0000022); a DLL named by its path that the
token may not open is refused with the status too. `LoadLibraryExW` leaves the last error 0 after
mapping a module (the sheet).

### 4. Impersonating above the process token

An impersonation token whose integrity level is higher than the process token's is only an
identification token when the process holds no `SeImpersonatePrivilege` (Learn: "Impersonation Levels",
"Mandatory Integrity Control"): the object manager's access check then answers
`STATUS_BAD_IMPERSONATION_LEVEL` (`ObjectSecurity::impersonation_downgraded`), which is how an
Untrusted target with a Low token on its main thread ends with 0xC00000A5.

### 5. The predefined registry keys

`HKEY_LOCAL_MACHINE`, `HKEY_CURRENT_USER` and the others are opened with `MAXIMUM_ALLOWED` and
cached: a token that may only read gets a handle that can read, and the keys below are checked when
they are opened. A cached root explains the sheet's "a missing key still gives 2 after
`RevertToSelf`, an existing one 5".

## To be confirmed by the spec agent

- The rights a process needs on its station and desktop at start (Poly: every specific right of
  both); the sheet only shows Low and Untrusted outcomes.
- Whether a thread that owns windows may leave its desktop (Poly: it may; Windows: error 170).
- `GetThreadDesktop` of a thread of another process (Poly: no handle).
- The `UOI_HEAPSIZE` value (Poly: 3072 KB).
- Which names the known-DLL list holds beyond those the sheets show.

## Consequences

- Chrome's broker runs its window-station, desktop, token and attribute sequence against real
  objects, and its confined children see what they would on Windows: the policies they read, the
  refusals they meet, the failures at start.
- The coverage map claims the 31 entries for `impl-chrome-s2`; nothing is `green` until the
  held-out tests have run.

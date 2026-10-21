# 0072: The Windows file system layout: `C:` on the hive disk, a host-tested layout table, the registry kept in step, and a default environment in the process parameters

**Status:** Accepted. Builds on [ADR 0067](0067-nt-file-io.md) (NT file I/O:
`\??\C:` is the VFS directory `/nt/c`, case-insensitive lookup over the
case-sensitive VFS, NT attributes in `user.poly.ntattr`),
[ADR 0066](0066-nt-registry-hives-in-polys-own-format.md) (the registry, its
default hives, Poly's single user `S-1-5-21-0-0-0-1000`, where hive files
live), [ADR 0065](0065-nt-threads-synchronisation-and-apcs.md) (the first
thread gets the PEB as its argument), [ADR 0055](0055-microsoft-redistributables-run-apps-own-copies.md)
(apps bring their own Microsoft runtimes) and the `ntdll` milestone's ADR
0071 (our `ntdll` and its DLL search order, being written alongside).

## Context

The Phase 5 roadmap item is "Windows file system layout: `C:\Windows`,
`System32`, `Program Files`, user profile". Windows programs and their
installers assume a drive with known folders: they write to
`%ProgramFiles%`, keep settings in `%APPDATA%`, create temporary files in
`%TEMP%`, look for DLLs in `System32`, and read where each folder is from
environment variables, the registry (`ProgramFilesDir`, `ProfileList`,
`Shell Folders`) or `SHGetKnownFolderPath`. Before this ADR `C:` was an
empty directory, nothing created the folders, no process had environment
variables (there were no process parameters at all), and the registry
named folders that did not exist.

Sources (clean room, [CLEAN_ROOM.md](../../CLEAN_ROOM.md)): Microsoft Learn's
"KNOWNFOLDERID" reference (each folder's default path, `CSIDL` and legacy
name), "Recognized environment variables", "Environment Variables" and
"Changing Environment Variables" (the block format and its sort order),
`ExpandEnvironmentStringsW`, "WOW64 Implementation Details" (the
program-files variables and registry values a 64-bit process sees), "File
System Redirector", the user profile functions (`GetProfilesDirectory`,
`GetDefaultUserProfileDirectory`, `GetAllUsersProfileDirectory`), "Naming
Files, Paths, and Namespaces", "Parsing C++ command-line arguments"
(`CommandLineToArgvW`'s rules) and the `winternl.h` / `ntdef.h` layouts of
`PEB`, `RTL_USER_PROCESS_PARAMETERS` and `UNICODE_STRING`. No Windows
installation, binary, Wine or ReactOS source was looked at. There is still
no Windows oracle VM, so what Learn leaves open is a labelled guess.

## Decision

### What `C:` is

**`C:` lives on the disk that keeps the registry hives.** ADR 0066 puts the
hives on the first writable disk whose root has a `poly` directory
(`<disk>/poly/nt/registry`). `C:` goes beside them, in `<disk>/poly/nt/c`,
and `/nt/c` (ADR 0067's path for `\??\C:`) becomes a symbolic link to it.
So installed programs, their settings and the user's documents survive a
reboot together with the registry that describes them, ADR 0067's mapping
(`\Device\PolyFs1` on `/nt/c`) stays exactly as it was (the volume's root
is looked up through the link), and nothing in the file I/O code changes.
With no such disk at the first NT use, `C:` is the directory `/nt/c` on
the boot file system and lasts one boot, like the in-memory registry. The
choice is made once per boot, with the hives' (`kernel/src/nt/layout.rs`).
If `/nt/c` is already something other than an empty directory or the right
link, it is left alone and `C:` stays on it (logged).

**One VFS, visible to Linux.** `C:` is an ordinary directory tree: Linux
programs outside the disk-root chroot see it at `/nt/c` (and at
`/mnt/root/poly/nt/c`), and inside the chroot of ADR 0059 at
`/poly/nt/c`. Copying a program into `/nt/c/Program Files/App` from a Linux
shell installs it for Windows. File names keep their case; NT finds them
without case (ADR 0067). `Z:` remains the whole VFS.

`cargo xtask root-disk` makes `poly/nt/c` on the persistent disk (an empty
directory, beside `poly/nt/registry`); the kernel fills it.

### The layout table

`personalities/nt/src/layout` (host-tested) holds `FOLDERS`: every folder
Poly makes, machine-wide (below `C:\`) or below the user's profile, each
with the `KNOWNFOLDERID` that names it, the environment variables whose
value it is, its legacy shell-folder value name, and whether it is hidden.
Paths are Learn's defaults:

| Folder | Known folder | Variables | Notes |
|---|---|---|---|
| `C:\Windows` | `FOLDERID_Windows` | `SystemRoot`, `windir` | |
| `C:\Windows\System32` | `FOLDERID_System` | | **Poly's own system DLLs** (below) |
| `C:\Windows\System32\drivers\etc` | | | for the `hosts` file programs look for |
| `C:\Windows\SysWOW64` | `FOLDERID_SystemX86` | | empty; Poly runs 64-bit programs only, so nothing is redirected (WOW64 file system redirection is left for 32-bit support) |
| `C:\Windows\Fonts`, `Temp`, `WinSxS` | `FOLDERID_Fonts` | | `Temp` is the system `TEMP` (ADR 0066's `%SystemRoot%\TEMP`); `WinSxS` for side-by-side runtimes (ADR 0055) |
| `C:\Program Files`, `...\Common Files` | `FOLDERID_ProgramFiles`, `..Common` | `ProgramFiles`, `ProgramW6432`, `CommonProgramFiles`, `CommonProgramW6432` | **where applications are installed** |
| `C:\Program Files (x86)`, `...\Common Files` | `FOLDERID_ProgramFilesX86`, `..CommonX86` | `ProgramFiles(x86)`, `CommonProgramFiles(x86)` | exists because 64-bit Windows has it and installers read the variable |
| `C:\ProgramData` (hidden) and its `Microsoft\Windows\Start Menu\Programs\StartUp`, `Templates` | `FOLDERID_ProgramData`, `..CommonStartMenu`, `..CommonPrograms`, `..CommonStartup`, `..CommonTemplates` | `ProgramData`, `ALLUSERSPROFILE` | |
| `C:\Users` | `FOLDERID_UserProfiles` | | |
| `C:\Users\Public` and `Desktop`, `Documents`, `Downloads`, `Music`, `Pictures`, `Videos` | `FOLDERID_Public`, `..PublicDesktop`, ... | `PUBLIC` | |
| `C:\Users\Default` (hidden) | (`GetDefaultUserProfileDirectory`) | | the profile a new user would be copied from |
| `C:\Users\poly` | `FOLDERID_Profile` | `USERPROFILE` (`HOMEDRIVE` + `HOMEPATH`) | **Poly's single user** |
| `...\poly\Desktop`, `Documents`, `Downloads`, `Music`, `Pictures`, `Videos`, `Favorites` | `FOLDERID_Desktop`, `..Documents`, ... | | |
| `...\poly\AppData` (hidden), `AppData\Roaming`, `AppData\Local`, `AppData\LocalLow` | `FOLDERID_RoamingAppData`, `..LocalAppData`, `..LocalAppDataLow` | `APPDATA`, `LOCALAPPDATA` | |
| `...\poly\AppData\Local\Temp` | | `TEMP`, `TMP` | the user's temporary directory (`GetTempPath` reads `TMP`, `TEMP`) |
| `...\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\StartUp`, `SendTo`, `Templates`, `Recent` | `FOLDERID_StartMenu`, `..Programs`, `..Startup`, `..SendTo`, `..Templates`, `..Recent` | | |

**The default user.** ADR 0066's `HKU\S-1-5-21-0-0-0-1000` is the user
`poly`; its profile is `C:\Users\poly` (Learn's `%SystemDrive%\Users\
%USERNAME%`). One user until tokens exist (the `advapi32` milestone); the
table is written per user (`User { name, sid }`), so a second user is a
second call.

**Hidden**: `ProgramData`, `AppData` and `Users\Default` get
`FILE_ATTRIBUTE_HIDDEN` (in `user.poly.ntattr`, ADR 0067) when Poly makes
them, never afterwards (to be confirmed by the spec agent).

### Who makes the folders, and how

The kernel makes them at the first NT use of every boot, before file I/O
attaches `C:` (`layout::create::create`, from `kernel/src/nt/layout.rs`).
Each folder is walked from `C:`'s root one component at a time with ADR
0067's lookup: an entry that matches without case is used in its own case
(a `windows` a Linux program made *is* the Windows directory; no `Windows`
is made beside it); of names that differ only in case the one NT opens is
used (the exact name, else the smallest as bytes); a missing component is
made (mode 0755) in the table's case. Something that is not a directory
where a folder belongs is reported and left alone with everything below it:
Poly never deletes a user's file to make its layout. Running it again
changes nothing, so it is cheap to run at every boot and repairs a folder
someone deleted.

### Poly's system DLLs and where programs go

- **`C:\Windows\System32` holds Poly's own system DLLs** (`ntdll`,
  `kernelbase`, `kernel32`, `ucrtbase`, ... as they are written). It is the
  loader's system directory in ADR 0071's search order (known DLLs from
  `System32`, then the application's directory, `System32`, `System`,
  `Windows`, the current directory and `PATH`), and `Path` starts with it.
  Our DLLs are built into the kernel (ADR 0071 embeds `ntdll`, the
  counterpart of Windows' `KnownDlls`); once that lands, the kernel also
  writes each one into `System32` after making the layout, with
  `layout::create::install_file` (here and host-tested; one call per DLL
  in `kernel/src/nt/layout.rs::prepare`), which rewrites a file only when
  its bytes differ, so a system DLL that an installer or a user
  replaced is restored at the next boot (Poly's small "resource
  protection"). The known DLLs are always loaded from the kernel's copy, so
  a changed file there never runs.
- **Applications go in `C:\Program Files\<App>`** with their own DLLs and
  the Microsoft runtimes they ship ("app-local", ADR 0055), found first by
  the application-directory step of the search order.
- **Redistributable installers** (`vc_redist.exe`, `DXSETUP.exe`, ADR 0055)
  write Microsoft's runtimes into `System32` and `WinSxS`; both are
  writable, so they can. A file with the name of one of Poly's own DLLs is
  the exception above.
- Per-user data goes where the known folders say (`%APPDATA%`,
  `%LOCALAPPDATA%`), machine-wide data in `%ProgramData%`.

### The registry kept in step

`layout::sync` runs at every boot after the hives are loaded and writes a
value only when it is missing or (for "fixed" values) different, so a boot
with nothing to change leaves every hive clean. The values are Poly's own
(none copied from a Windows installation); "name only" means Learn names
the key or value without defining it.

| Key (`HKLM` = `\REGISTRY\MACHINE`, `HKU\<sid>` the user) | Values | Policy | Source |
|---|---|---|---|
| `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion` | `SystemRoot` = `C:\Windows` | fixed | name only |
| `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion` | `ProgramFilesDir`, `ProgramW6432Dir`, `ProgramFilesDir (x86)`, `CommonFilesDir`, `CommonW6432Dir`, `CommonFilesDir (x86)` | fixed | Learn: "WOW64 Implementation Details" (names) |
| `HKLM\...\Windows NT\CurrentVersion\ProfileList` | `ProfilesDirectory`, `Default`, `Public`, `ProgramData` (`REG_EXPAND_SZ`, below `%SystemDrive%`) | fixed | name only (the profile functions return these folders) |
| `...\ProfileList\S-1-5-21-0-0-0-1000` | `ProfileImagePath` = `C:\Users\poly` (`REG_EXPAND_SZ`) | fixed | name only |
| `HKLM\SYSTEM\<current control set>\Control\Session Manager\Environment` | `TEMP`, `TMP` = `%SystemRoot%\TEMP`, `windir` = `%SystemRoot%` | if missing | Learn: "Environment Variables" |
| `HKU\<sid>\Environment` | `TEMP`, `TMP` = `%USERPROFILE%\AppData\Local\Temp` | if missing | Learn: "Environment Variables" |
| `HKU\<sid>\Volatile Environment` (volatile) | `USERPROFILE`, `HOMEDRIVE`, `HOMEPATH`, `APPDATA`, `LOCALAPPDATA`, `USERNAME` | fixed | Learn: "Recognized environment variables" |
| `HKLM\...\CurrentVersion\Explorer\User Shell Folders`, `HKU\<sid>\Software\...\Explorer\User Shell Folders` | each folder's legacy name (`Personal`, `My Music`, `AppData`, `Local AppData`, `Common AppData`, `Common Documents`, ...) = its path below `%USERPROFILE%`, `%ProgramData%` or `%PUBLIC%` (`REG_EXPAND_SZ`) | if missing (a user may redirect a folder) | Learn: "KNOWNFOLDERID" (the legacy names); the keys: name only |
| `...\Explorer\Shell Folders` (both) | the same names, expanded (`REG_SZ`), from whatever `User Shell Folders` now says | fixed | name only |

The default hives of ADR 0066 already held the program-files values,
`SystemRoot`, the `Environment` values and the volatile environment; the
host tests check that the sync finds them equal (the two tables agree), so
`crates/poly-registry/src/defaults.rs` only gained a pointer to this ADR.

### A new NT process's environment and parameters

**The environment** (`layout::env::build`, at every process start, from
the registry under the NT lock), each step overriding the previous one's
variables of the same name (names compared without case):

1. the machine's variables a 64-bit process sees: `SystemDrive`,
   `SystemRoot`, `ProgramFiles`, `ProgramW6432`, `ProgramFiles(x86)`,
   `CommonProgramFiles`, `CommonProgramW6432`, `CommonProgramFiles(x86)`
   (from the `CurrentVersion` values above), `ProgramData`,
   `ALLUSERSPROFILE`, `PUBLIC` (from `ProfileList`), `COMPUTERNAME` (the
   `ComputerName` key of the control set `Select\Current` names) and
   `NUMBER_OF_PROCESSORS` (the CPUs the scheduler runs on);
2. the system variables, `Session Manager\Environment` (`ComSpec`, `OS`,
   `Path`, `PATHEXT`, `PROCESSOR_ARCHITECTURE`, `TEMP`, `TMP`, `windir`);
3. the user's logon variables, `HKU\<sid>\Volatile Environment`
   (`USERPROFILE`, `HOMEDRIVE`, `HOMEPATH`, `APPDATA`, `LOCALAPPDATA`,
   `USERNAME`, `USERDOMAIN`, `LOGONSERVER`);
4. the user's variables, `HKU\<sid>\Environment`; a user `Path` is
   appended to the system's with `;`.

`REG_EXPAND_SZ` values are expanded against what is set so far
(`ExpandEnvironmentStringsW`: an undefined `%name%` stays as it is); other
value types are skipped. Where a registry value is missing the layout's
value stands in, so every process has every variable. The block is
`Name=Value\0` strings ended by one more `\0`, sorted by name without case
in UTF-16 code-unit order (Learn: "Changing Environment Variables").

**The process parameters** (`personalities/nt/src/params.rs`). The kernel
gives every NT process an `RTL_USER_PROCESS_PARAMETERS` in one private
read-write allocation and points `PEB.ProcessParameters` (PEB + 0x20,
`winternl.h`) at it:

| Offset | Field | Source |
|---|---|---|
| + 0x60 | `ImagePathName` (`UNICODE_STRING`) | `winternl.h` |
| + 0x70 | `CommandLine` | `winternl.h` |
| + 0x80 | the environment block's address | **Poly's own**: the pointer right after the documented fields |
| + 0x88 | the environment block's size in bytes | **Poly's own** |

The structure takes 0x400 bytes, zero elsewhere; the image path and the
command line (each with a NUL that `MaximumLength` counts) and the block
follow it in the same allocation. `winternl.h` declares only the two
strings and reserved space, so the environment's place is Poly's choice;
our `ntdll`/`kernelbase` (`GetEnvironmentStringsW`,
`GetEnvironmentVariableW`, `GetCommandLineW`) read it from there, and the
`ntdll` milestone (ADR 0071) writes the same structure for processes it
starts. The image path is the executable's DOS path: below the directory
behind `C:` it is `C:\...`, anywhere else on the VFS `Z:\...`; an image the
kernel built itself (a test) is named `C:\Windows\System32\poly-image.exe`.
The command line is the image path (quoted when it has a space) and the
arguments quoted so that `CommandLineToArgvW`'s documented rules give them
back (host-tested against a parser written from those rules). The initial
current directory and the standard handles stay with the `kernelbase` and
console milestones.

## To be confirmed by the spec agent against real Windows

1. Which folders a fresh Windows has that programs rely on and Poly lacks;
   the hidden/system attributes of `ProgramData`, `AppData`, `Users\Default`
   and the profile's junctions (Poly makes no junctions such as
   `Application Data` or `My Documents`).
2. The exact `ProfileList` values and types (`ProfilesDirectory` as
   `REG_EXPAND_SZ` below `%SystemDrive%`), and the full set of `Shell
   Folders` / `User Shell Folders` value names.
3. The environment's construction: the order of the steps, whether a user
   `Path` is appended (ours) or replaces the system's, which variables come
   from where (`USERDOMAIN`, `LOGONSERVER`, `PROCESSOR_IDENTIFIER`,
   `PROCESSOR_LEVEL`, `PROCESSOR_REVISION`, `PSModulePath`, `DriverData`,
   `OneDrive` and others Poly does not set), and the case a name keeps when
   a later step sets it in another case (ours: the first one's).
4. `ExpandEnvironmentStringsW` on `%%` and an unterminated `%` (ours: kept
   as they are, a closing `%` of an undefined name may open the next).
5. Where the environment block sits and where its address is kept in
   `RTL_USER_PROCESS_PARAMETERS` (ours: + 0x80), and whether programs read
   it directly.
6. Whether `System32` holds a file for every DLL `KnownDlls` names, and how
   Windows reacts to a replaced system file.

## Consequences

- The table, the creation (idempotence, case-insensitive reuse, names that
  differ only in case, a file in the way, installing a system file), the
  registry sync (agreement with ADR 0066's defaults, idempotence with clean
  hives, fixed values restored, user redirections kept and mirrored), the
  environment (every documented variable, the sort order, `Path` merging,
  expansion) and the process parameters (layout, command-line quoting) are
  host-tested in `personalities/nt` (`cargo test -p poly-personality-nt
  layout params`). The kernel side is one `kernel_test!`
  (`nt::test_layout`): a self-assembled PE finds `TEMP` in its PEB's
  environment, opens `\??\c:\windows\SYSTEM32`, finds `System32` in its own
  case by enumerating `C:\Windows`, and creates, enumerates and deletes a
  file in its `TEMP` directory; the kernel then checks every folder exists
  and `ProfileImagePath` was synced.
- Edits to shared files are small and additive: `lib.rs` gains two
  modules; `kernel/src/nt/mod.rs` one call before `io::attach`, a
  `Launch` (path and arguments) with `start_with` / `spawn_pe_with` beside
  the unchanged `start` / `spawn_pe`, and the parameters written after
  `exec::start_process`; `program.rs` passes the path and arguments;
  `rootdisk.rs` makes `poly/nt/c`.
- Every NT process start now builds its environment from the registry
  (a few dozen value reads under the NT lock) and one more allocation.
- What is left: WOW64 redirection (with 32-bit support), junctions for the
  legacy profile names, more users, `SHGetKnownFolderPath` (the `shell32`
  milestone of Phase 7) over this table, and the initial current
  directory.
- The coverage map is unchanged: nothing here is an exported symbol.

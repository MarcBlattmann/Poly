# 0230: Windows installers through Poly's front door

**Status:** Accepted. Track `impl-chrome-installer-frontdoor`, batch B5 of
`docs/windows/chrome-installer-plan.md`. Roadmap Phase 8, "Windows installers (`.exe`, `.msi`) ... go
through the same installer" (the `.exe` half). Builds on [ADR 0132](0132-installing-linux-apps-of-every-package-format.md)
(one installer for every format), [ADR 0173](0173-installing-a-deb-polys-installer-over-debians-own-apt-and-dpkg.md)
(the `.deb` front door: desktop file, window, the desktop opens it), [ADR 0180](0180-uninstalling-apps-and-installing-appimages.md)
(uninstall from Applications), [ADR 0216](0216-chrome-for-windows-on-the-desktop-to-try-by-hand.md) (an app's
icon from its PE resources), ADR 0066/0073 (the registry's hive files and `C:` on disk) and the owner's decisions of
2026-11-17 (the plan's section 4). Inputs (clean room): the PE format specification, Microsoft's **[MS-SHLLINK]**
(shell links) and **[MS-CFB]** (compound files), Learn's "VS_VERSIONINFO", "StringFileInfo", "VS_FIXEDFILEINFO",
"Resource File Formats" (icon groups), "Parsing C++ command-line arguments", "Uninstall Registry Key", "KNOWNFOLDERID",
the Desktop Entry Specification. No other program's source was read.

## Context

Files sniffed `MZ` as `application/x-msdownload` and opened it with whatever claimed that type: nothing did, so a
downloaded `ChromeSetup.exe` could not be started by a double click (the survey's stop 6). The owner wants Google's
installer downloaded in Poly's Chromium, double-clicked, run with its own windows, and Chrome in the launcher and in
Settings → Apps afterwards. Batches B0 to B4 make the installer itself run; this is the door around it.

## Decisions

### 1. A Windows program is a format the installer knows

`poly_pkg::format` knows two more formats by their content:

- **`WindowsProgram`**: `MZ`, and `PE\0\0` where the DOS header's `e_lfanew` points (or that offset is past the bytes
  read). Installed by running it (below).
- **`WindowsInstaller`**: a compound file (`D0 CF 11 E0 A1 B1 1A E1`). It is a Windows Installer package only if the
  root storage's class is `{000C1084-0000-0000-C000-000000000046}` ([MS-CFB] 2.6: the first directory entry; the
  header gives the sector size and the first directory sector), else it is no package (a `.doc` is a compound file
  too). An `.msi` is **recognised and refused** with a sentence pointing at the app's `.exe`: MSI is batch B6, deferred
  by the plan (Google's consumer installers do not use it).

`poly-installer.desktop` claims `application/x-msdownload`, `application/vnd.microsoft.portable-executable` (what
`file` says, which `xdg-open` asks), `application/x-ms-dos-executable` (the older shared-MIME name) and
`application/x-msi`, and the Chromium disk's `mimeapps.list` makes it their default. So Chromium's "open" of a
download, a double click in Files and `poly-installer FILE` all reach the installer, through the desktop as for a
`.deb` (ADR 0173, decision 5). The desktop's initramfs carries the desktop file too.

### 2. What the window shows, from the file

`poly_pkg::windows::program` reads, at the offsets it needs and never the whole file (the offline installer is 170 MB,
nearly all one resource): the headers, the resource directory, then single resources.

- **Name**: `ProductName`, else `FileDescription`, else the file's name without `.exe`; the line under it is
  `FileDescription` (or `CompanyName`). **Version**: `ProductVersion` (`1, 2, 3, 4` written with dots), else
  `FileVersion`, else the fixed version. **Size**: the file's. **Kind**: "Windows installer (32-bit)" or "Windows
  program (64-bit)".
- **Publisher**: the Authenticode signer, read by `poly_x509::authenticode::publisher_of` (the headers, the
  certificate table, the first signer's certificate subject), the same function the consent prompt of ADR 0222
  uses, and worded by the same `publisher_label`: "Google LLC (signature not verified)" in both places for Google's
  installer. Else "Unknown publisher" ("(signed)" when the file has a certificate table whose signer Poly cannot
  read). The signature itself is not verified (ADR 0222 says the same of its prompt).
- **Icon**: the largest image of the program's first icon group; a PNG image as it is, a DIB (1-, 4-, 8-, 24- or
  32-bit, transparency from its alpha or its AND mask) written as a PNG file (stored deflate blocks), because the
  launcher reads PNG files.
- **Installer or program**: a file whose names (the file's, `FileDescription`, `ProductName`, `OriginalFilename`,
  `InternalName`) say "setup" or "install" is an installer: **Cancel / Install**. Anything else is a plain program:
  **Cancel / Run**, which starts it where it is (with the desktop's busy pointer, ADR 0167) and closes the window.
  Nothing is installed. A DLL is refused ("a library, not a program").

### 3. Install runs the installer unchanged, with its windows, and waits for all of it

**Install** starts the `.exe` from where it is, on the NT personality, as the desktop's user: the same way the launcher
starts a Windows app, so its windows are the desktop's windows (win32k's link). Poly does not elevate it and does not
treat the click as consent: by the owner's decision 1 elevation is a **separate prompt** (B0's consent dialog, ADR
0222), which the installer gets when it asks (`runas`, a manifest). Its standard output and error go into the window's
log (shown in a failure's report).

The window is in a **running** state until the installer **and every process it started** have ended: Google's stub
starts the updater, which starts Chrome's setup. The installer finds them in `/proc`: for that the kernel now records
an NT process's creator as its parent (`Process::ppid`, read by `/proc/<pid>/status`'s `PPid`; before, an NT process
had none). Every tick the window adds the processes whose parent is in the tree; once the installer itself has ended,
the window says how many it still waits for and offers **Stop waiting** (an installer may leave a program running,
Chrome itself for one: `chrome.exe --from-installer`).

### 3a. Started as the shell opens it: the manifest is honoured (addendum, track `frontdoor-manifest`)

On Windows a double click is the shell's `ShellExecuteEx`: the program's manifest is read and a
`requireAdministrator` program (or `highestAvailable`, for an administrator) gets the consent prompt **before** it
runs. The installer window is a native program and cannot call `ShellExecuteExW`, and the kernel's exec reads no
manifest; so programs ran un-elevated and only installers that ask for themselves (`runas`, as Google's does) got
the rights. Now the window (**Install**, **Run**, an app's **Uninstall**) starts every Windows program through
**`polyopen.exe`**, Poly's own small program in `C:\Windows\System32` (`personalities/nt/win32/polyopen`): it calls
`ShellExecuteExW` with the default verb, which is ADR 0222's path (`CreateProcessW` refuses with
`ERROR_ELEVATION_REQUIRED`, `ShellExecuteExW` then asks the consent broker and starts the program with the full
token), waits for the program and for everything it started, and ends with the program's exit code. That wait
is exact: `polyopen.exe` puts itself into a job object before it opens the program, every process of the tree joins
it (ADR 0169), and it waits until it is the job's only process, saying `polyopen: the program has ended; waiting
for N it started` while it does. The window then waits for `polyopen.exe` alone; the `/proc` tree of decision 3
remains for a disk without it. (Watching `/proc` lost short-lived processes in between, such as an elevated copy
that starts a helper and ends within one look; the kernel now also hands an ending NT process's children to its
parent.) The program is still a child of the window's process
tree, so the wait of decision 3 holds. If the shell does not start it, `polyopen.exe` writes `polyopen:
ShellExecuteExW failed: <error>` and the window says so: for `ERROR_CANCELLED` (1223, the prompt answered No) "...
did not run: it asks for administrator rights, and they were not given." On a disk without `polyopen.exe` the
program is started directly, as before.

Windows' *installer detection* (32-bit images without a manifest whose names say "setup" or "install" are elevated)
is **not** done: it has no spec sheet (ADR 0222 lists it as missing). The front door's own "installer by its names"
only chooses between **Install** and **Run**; it never asks for rights.

### 4. Afterwards: shortcuts become launcher entries

Before the installer starts, the window notes the shortcut files (path and a checksum of their bytes) in the common
Start menu (`C:\ProgramData\Microsoft\Windows\Start Menu\Programs`), the user's Start menu, the public desktop
(`C:\Users\Public\Desktop`) and the user's desktop, with their sub-folders, and the `Uninstall` keys in the registry.
Afterwards, every shortcut that is new or changed is read by **`poly-lnk`** (ADR 0227), the one reader of shell
links Poly has, shared with `shell32`'s `ShellLink` object ([MS-SHLLINK]):

- the target: `poly_lnk::Link::target` (the `LinkInfo` path, else the ID list's path as Poly's `ShellLink` writes
  it, else the environment block's), else the relative path against the shortcut's folder (this module's addition:
  `poly-lnk` does not resolve a relative path alone); the arguments, working folder, description and icon as
  `poly-lnk` reads them;
- `%ProgramFiles%` and the other variables of Poly's NT environment are expanded; the Windows path is found on `C:`
  (`/nt/c`) without regard to case, each component as it is spelt on the disk.

A shortcut to a program (`.exe`) that is there becomes a **desktop file** `poly-windows-<slug>.desktop` in the user's
applications folder (where the launcher and Files' Applications read, as AppImages', ADR 0180): `Name` is the
shortcut's name, `Comment` its description, `Exec` the program's path and the shortcut's arguments (split as Windows
splits a command line, quoted as the Desktop Entry Specification says), `Path` its working folder, `Icon` a PNG file
of the program's icon (the shortcut's icon location and index if it has them, as ADR 0216 did for Chrome). A Start
menu entry and a desktop shortcut of the same program and arguments are one entry. Shortcuts named "Uninstall ..."
and shortcuts to documents are left out. The entry's program gets the execute bit: what a Windows program writes on
`C:` has none (Windows has no such bit), and the desktop's exec wants it; the installer and an uninstaller it runs
get it too (a download comes without it). Then the window asks the desktop to read its apps again (`rescan`), and
**Open** asks it for the first new entry.

### 5. The app's `Uninstall` key: Settings → Apps

The keys that were not there before are read from the registry's hive files (`<disk>/poly/nt/registry/SOFTWARE.hive`
and `USER.hive`, ADR 0066; the kernel writes them when an NT process ends): `HKLM\SOFTWARE\Microsoft\Windows\
CurrentVersion\Uninstall\*`, its 32-bit view under `WOW6432Node`, and the user's under `HKCU`; keys without a
`DisplayName` or marked `SystemComponent` are not apps. Each entry takes the key that belongs to it (the one whose
`InstallLocation` holds the program, whose `DisplayIcon` is the program, or whose uninstaller sits beside it; else the
only key the installer made) and carries it as `X-Poly-Windows-Key`, `-Uninstall`, `-Publisher` and `-Version`.

Poly's Settings has no apps page; the list of installed apps is Files' **Applications** folder (ADR 0175), with
**Uninstall** (ADR 0180). A Windows entry with an `UninstallString` is removable there: the installer's window runs
that command (its program found on `C:`, quoted or not) the same way as an installer, waits for it and what it
started, then removes the entries whose programs are gone. An entry whose program is still there stays and the
window says so (the user said no in the uninstaller). With `C:` in memory (no disk with a `poly` folder) there are no
hive files to read, and the entries have no registration.

### 6. Auto-update and Chrome for Testing

Nothing here turns Google's updater off (the owner's decision 2: on from the start). The launcher entry made from
Chrome's own shortcut is the installed Chrome's; replacing Chrome for Testing's entry follows B7 (decision 4).

## Tests

Host: `poly-pkg` (`windows/tests.rs`: formats; the version resource, names, versions, icon groups by index and by id,
DIBs made PNG files, refused files; shell links from three fixture files written by hand from [MS-SHLLINK] with a
separate script (`tests/fixtures/*.lnk`: Unicode `LinkInfo` with every string and an ID list; ANSI with a
`CommonPathSuffix`; an environment block with a relative path), read through `poly-lnk`, cut links; paths without case, variables, Windows
command lines, `Exec` quoting read back; entries, duplicates; `Uninstall` keys from hive files, matching, uninstall
commands; `installer.rs`: install, run, wait, stop waiting, failures, uninstall; `remove`: what can be uninstalled),
`poly-installer-app` (the window with a made-up system: install with waiting, run, uninstall).

Boot group **`win-installer`** (shard `desktop-apps`): the desktop with a small Windows installer in `/root/Downloads`
(`tests/windows/win_test_setup.rs`: it copies itself to `C:\Program Files`, writes a shortcut with an argument into the
common Start menu and an `Uninstall` key, starts a helper that lingers four seconds) and a disk with a `poly` folder
for `C:` and the hive files. The Terminal runs `poly-installer FILE`; the window opens, **Install** runs it. Like Google's
installer, the test installer asks for administrator rights itself: un-elevated (ADR 0222's filtered token, the
default since #313) it starts itself again with `ShellExecuteExW` `runas`; the boot test answers Poly's consent
prompt with Yes (Tab, Enter: No has the focus first), and the elevated copy installs (declined, it would install for
the user: `HKCU`, `%LOCALAPPDATA%`). The window waits for the elevated copy and the helper, the shortcut becomes the launcher entry with the key as its registration, the desktop reads its
apps again, and **Open** and the launcher start the installed app with the shortcut's argument.

Boot group `win-installer`, addendum: a second copy of the test installer with a `requireAdministrator` manifest
embedded by xtask (`with_manifest`: a new `.rsrc` section with `RT_MANIFEST` 1, host-tested by reading it back with
`poly_kernelbase::elevation`, B0's reader) installs "Poly Admin App": the consent prompt comes once, before it
runs; it runs elevated without asking itself, and its `Uninstall` key is under `HKLM`. Host: the model's words for a
refused or failed start.

## Not in this ADR

- **MSI** (B6), and running an `.msi` through `msiexec`.
- **Verifying signatures** (the image digest, the chain to a trusted root): the name is shown as not verified.
- **Shortcuts whose target is only in an ID list Poly's `ShellLink` did not write** (`poly-lnk`'s reading of the
  shell's items is Poly's own layout; the public specification does not give them).
- **Installers that elevate**: B0's consent prompt is theirs; this window only waits.
- **Google's installer end to end** (B7): it needs B1 to B4.

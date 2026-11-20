# 0256: Squirrel installers unpacked by Poly

**Status:** Accepted. Track `squirrel-frontdoor`, batch W6 of [the Windows apps survey](../windows/apps-survey.md), by
the owner's decision of 2026-11-20: **unpack, don't build .NET Framework.** Builds on
[ADR 0230](0230-windows-installers-through-polys-front-door.md) (the front door: shortcuts become launcher entries, the
`Uninstall` key makes an app Files can uninstall) and ADR 0234 (absolute paths in a command line reach an NT program
as DOS paths). Inputs (clean room): PKWARE's APPNOTE.TXT (the zip format), ECMA-376 Part 2 (Open Packaging
Conventions), Learn's ".nuspec reference", "Uninstall Registry Key" and "KNOWNFOLDERID", the PE format specification,
the public SDK headers, and Squirrel's public user documentation for the install layout. Squirrel's source was not
read.

## Context

Discord, Slack and many Electron apps ship a **Squirrel** installer: a small `Setup.exe` that carries the app as a
NuGet package and installs it with `Update.exe`, a **.NET Framework** program (its only import is
`mscoree!_CorExeMain`). Poly has no CLR, so the installer ends after three seconds having installed nothing (the
survey). A CLR is a very large project; the owner chose to have Poly do what the installer does, from the files'
formats.

## Decisions

### 1. Recognising one

A Squirrel installer is a PE program with a resource of the **named type `DATA`** whose first resource is a **zip
archive** with a top-level **`<id>-<version>-full.nupkg`** and `RELEASES` or `Update.exe` beside it
(`poly_squirrel::bundle`). The front door (`poly_pkg::windows::program::Image::squirrel`) reads only the end of
that resource for the archive's central directory (`poly_zip::central_directory`), never the 160 MB whole. Such a
program is an installer whatever its names say; the window's kind reads "Windows installer (Squirrel), installed by
Poly".

### 2. `polysquirrel.exe` installs it

**Install** does not run the installer. The window starts Poly's own NT program **`polysquirrel.exe`**
(`C:\Windows\System32`, `personalities/nt/win32/polysquirrel`) with the installer's path, through `polyopen.exe`
like every Windows program the window starts (ADR 0230). It works with Win32 calls, so the registry and the shell
are the NT personality's own:

- It maps the installer, finds the bundle (`poly_squirrel::bundle_resource`), inflates the full package
  (`poly_zip`: stored and deflated entries, CRC-32 checked, ZIP64) and reads its manifest (`<id>.nuspec`: `id`,
  `version`, `title`, `authors`, `description`).
- **Where things go** (`poly_squirrel::plan`), as Squirrel lays an app out:
  - the package's `lib/net45/` in `%LOCALAPPDATA%\<id>\app-<version>\` (OPC part names percent-decoded; `..` and
    empty components refused);
  - `Update.exe` in `%LOCALAPPDATA%\<id>\`, where the app looks for it;
  - `RELEASES` and the package in `%LOCALAPPDATA%\<id>\packages\`.
- **The app's program**: `<id>.exe`, else `<title>.exe`, else the only program in the folder that is not
  Squirrel's.
- **Shortcuts** through the shell's `ShellLink` object (`IShellLinkW`, `IPersistFile`): `<title>.lnk` in the user's
  Start menu, in a folder named for the first author, and on the user's desktop. They start the app's program
  directly: Squirrel's own shortcuts start `Update.exe --processStart`, which is the .NET program.
- **The per-user `Uninstall` key** `HKCU\...\Uninstall\<id>`: `DisplayName`, `DisplayVersion`, `Publisher`,
  `InstallLocation`, `DisplayIcon`, `EstimatedSize`, `NoModify`, `NoRepair`, and `UninstallString`
  `"C:\Windows\System32\polysquirrel.exe" --uninstall <id>`.

It writes a line per step (`polysquirrel: ...`) and ends with 0, or with 1 after `polysquirrel: failed: <why>`.
The user's folders come from the environment (`%LOCALAPPDATA%`, `%APPDATA%`, `%USERPROFILE%`), the values
`KNOWNFOLDERID` documents.

### 3. The front door does the rest

The new shortcuts and key are picked up as for any installer (ADR 0230, decisions 4 and 5): the launcher entry, its
icon, the registration. Then the window **starts the app**, as Squirrel's installer does, and stays open with
**Close** and **Open**. (`polysquirrel.exe` itself starts the app only with `--start`: started by it, the app
would be in `polyopen.exe`'s job, and the window would wait for it.)

### 4. Uninstall

Files' **Uninstall** runs the `UninstallString` (ADR 0230): `polysquirrel.exe --uninstall <id>` reads the key,
removes `%LOCALAPPDATA%\<id>` only if the key's `InstallLocation` is exactly that folder, deletes the two shortcuts
(and the publisher's Start menu folder if it is empty now) and the key. The window then drops the launcher entries
whose program is gone.

## Consequences

- Squirrel apps install without .NET Framework, and Squirrel's **own update does not work**: `Update.exe` is still
  .NET. An update is a new installer through the front door, as for other Windows apps.
- The app's own checks for `..\Update.exe` find a file. Calls the app makes to it fail as a program that does not
  start.
- `poly-zip` is Poly's zip reader for later formats (`.appx`, `.nupkg` elsewhere).

## Not in this ADR

- .NET Framework (W6's other option, deferred by the owner).
- Delta packages, `--squirrel-install` hooks the app runs on first start, and the app's own `Update.exe`
  arguments.
- Machine-wide Squirrel installs (`Program Files`), which Squirrel's MSI deployment tool makes.

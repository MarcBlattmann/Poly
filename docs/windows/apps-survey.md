# Popular Windows programs on Poly: survey and plan

Track `survey-win-apps` (2026-11-20). Goal of the owner's coordinator: with Chrome for Windows in good shape, find out
which widely used free Windows programs run on Poly, and where each one stops, to plan the next tracks.
PuTTY, 7-Zip and Git for Windows have tracks of their own and are not repeated here.

**Inputs:**
- the programs' PE import tables (names only, `cargo xtask win-app imports` and a name-only cross-check against Poly's
  fail-only export lists);
- runs of the programs on Poly in QEMU/KVM (`cargo xtask win-app run --mode desktop`, 4 GB, 4 CPUs, Poly's desktop,
  `poly.nt.exceptions`, and `poly.nt.trail` where a breakpoint ended a run), with Poly's serial log, the run summary
  and two screenshots each;
- the guest disk images afterwards (`debugfs`).

Nothing of the programs was read to learn how Windows behaves (CLEAN_ROOM.md), and nothing was decompiled or
disassembled. They were downloaded from their official sites into the build container's `target/`, **never run on
the owner's PC**, and nothing of theirs is committed. On `origin/main` 8e35cb74e.

## 1. The programs

| Program | Version | Form tried | Kind |
|---|---|---|---|
| Visual Studio Code | 1.147.0 | zip (`Code.exe`) and the user installer (`VSCodeUserSetup.exe`) | Electron; Inno Setup installer (32-bit) |
| Notepad++ | 8.9.9.1 | portable zip and the installer | Win32 (C++); NSIS installer (32-bit) |
| VLC media player | 3.0.24 | zip | Qt 5, plugins |
| Firefox | 160.0.1 | `FirefoxSetup.exe`, its inner `setup.exe` and the browser itself (`core\firefox.exe`, taken out of the 7-Zip archive the setup stub is) | Gecko; NSIS installer behind a 32-bit 7-Zip SFX stub |
| Discord | 1.0.9269 | `DiscordSetup.exe` and the app itself (`Discord.exe`, taken out of the Squirrel `.nupkg`) | Electron; Squirrel installer |
| OBS Studio | 33.0.1 | zip (`bin\64bit\obs64.exe`) | Qt 6, Direct3D 11, CEF |
| Paint.NET | 5.2 | portable zip | .NET (self-contained, apphost) |
| Steam | current `SteamSetup.exe` | the installer | NSIS installer (32-bit); the client itself not reached |

## 2. Where each one stops

| Program | First stop on Poly | What the user sees | How far it gets |
|---|---|---|---|
| **VLC** | Qt draws no text: `gdi32!GetGlyphOutlineW`, `GetFontData`, `GetCharABCWidthsFloatW` are fail stubs (reached); `CoCreateInstance` of the audio device enumerator `{BCDE0395-...}` not registered; `dsound!DirectSoundEnumerateW` stub | The main window **with its toolbar, cone, volume slider**, and the first-run "Privacy and Network Access Policy" dialog **with blank labels and an empty button** | Furthest of all: the UI is up and responsive; text and audio are what is missing |
| **Notepad++** | `comctl32!#381` (`LoadIconMetric`) is a fail stub (reached), so its icon list gets no icons | A message box "IconList::addIcon : LoadIcon() function return null. Ignore the error?" (Yes/No/Cancel) before the main window | Starts, creates windows, shows a working message box; 81 of its 633 imports are fail-only stubs (printing, drag and drop, a few user32/gdi32 drawing calls) |
| **Paint.NET** | (1) In the survey boots, "A processor (CPU) with SSE4.2 support is required": `win-app` boots QEMU's default CPU model, which has no SSE4.2 (Poly's `IsProcessorFeaturePresent` reads CPUID correctly). (2) With `POLY_QEMU_CPU=host`: the .NET host ends with status 2 after 56 ms; it reached `kernelbase!PathCchRemoveFileSpec`, `shlwapi!PathFileExistsW` and `kernel32!GetCurrentPackageId` (fail stubs) | (1) the message box; (2) nothing | The .NET apphost starts; the runtime does not |
| **Firefox** | `FirefoxSetup.exe` (32-bit) does not start: `MSVCRT.dll` has no 32-bit build on Poly, and there is no stub for `free`. The browser itself (64-bit): the launcher process exits with 0 after 3.5 s, the browser process it starts ends with `0xc0000409` parameter 7 (a fail-fast from `abort`) | Nothing | `firefox.exe` (64-bit) loads `xul.dll` and starts its browser process, which aborts; stubs reached: `CreateWaitableTimerExW`, `GetTimeZoneInformationForYear`, `K32GetPerformanceInfo`, `SetThreadStackGuarantee`, `SetDllDirectoryW`, `SetProcessDEPPolicy`, `GetCurrentPackage*`, `RtlQueryPerformanceCounter`, `RtlDuplicateUnicodeString` |
| **VS Code** (zip) | After `NtFreeVirtualMemory`, two `NtAllocateVirtualMemory` calls return `0xc000002d` (`STATUS_NOT_COMMITTED`), then a CHECK breakpoint (`0x80000003`) in `Code.exe` after 1.1 s (`poly.nt.trail`). Stubs reached before: `GetPackageFamilyName`, `AppPolicyGetThreadInitializationType`, `PowerRegisterSuspendResumeNotification`, `uxtheme!#135`; `kernelbase!GetFileInformationByName` missing | Nothing | The browser-process start of Electron. Chrome for Testing gets much further on the same stack, so this is an Electron/V8 memory pattern Poly answers differently |
| **Discord** (app) | `Discord.exe` exits with 0 after 4.7 s without a window; its Electron and GPU start-up ran (ANGLE's EGL/GLES lookups, `DXCore.dll` missing, `CoCreateInstance` of four classes not registered, `SetCurrentProcessExplicitAppUserModelID` stub). Not determined: most likely it refuses to run outside its Squirrel install layout (`..\Update.exe`) | Nothing | Electron starts and quits on its own |
| **OBS Studio** | Exits with 1 after about 150 ms, before it creates its profile (`%APPDATA%\obs-studio`); no stub reached, no exception. Its Qt 6 is bound to stubs of three DLLs Poly does not have: `msvcp140_1.dll` and `msvcp140_2.dll` (Visual C++ 2022 runtime) and `icuuc.dll` (Windows' own ICU); `QUSEREX.DLL` missing | Nothing | `obs64.exe` loads, Poly's `d3d11` loads, then it ends. Cause not traced; the missing runtime DLLs are the first suspect |
| **Notepad++ installer**, **Steam installer**, **Firefox's inner `setup.exe`** (NSIS) | `kernel32!lstrcatW` (32-bit fail stub without an argument size: the process ends). With that worked around: `kernel32!GetTempFileNameW`, the same way | Nothing | Ends within 65 ms, before any window |
| **VS Code installer** (Inno Setup) | `kernel32!SetSearchPathMode`, then (worked around) `SetProcessDEPPolicy`: 32-bit fail stubs without an argument size | Nothing | Ends within 50 ms |
| **Discord installer** (Squirrel `Setup.exe`) | Exits with 0 after 3 s **having installed nothing** (no `%LOCALAPPDATA%\Discord`); `comctl32!TaskDialogIndirect` is not exported (its error dialog). Its payload's `Update.exe` is a **.NET Framework program**: its only import is `mscoree.dll!_CorExeMain` | Nothing | The setup host starts, then gives up |

**Static picture** (`win-app imports`). Nothing is missing from the outer installer executables:
- Firefox, Discord, Steam and Notepad++ installers: 0 missing functions;
- VS Code installer: 1 (`oleaut32!SafeArrayPtrOfIndex`).

The programs miss little by name:
- Notepad++: 1 (`comdlg32!PrintDlgW`);
- VLC: 1 (`ws2_32!WSASendMsg`, in two streaming plugins);
- Paint.NET: 0;
- VS Code: 69 (64 of them `msalruntime.dll` exports, plus `ext-ms-win-uiacore` in its terminal helper);
- OBS: 6 missing DLLs and 22 functions.

What stops them are functions that **exist as fail-only stubs**. Their imports crossed with Poly's fail-only lists
(`fail-only.txt` in the kit):

| File | Imports | Fail-only |
|---|---|---|
| NSIS installers | 163–166 | 11–12, the same in all three |
| Inno installer | 146 | 6 |
| Squirrel `Setup.exe` | 109 | 2 |
| `notepad++.exe` | 633 | 81 |
| `Code.exe` / `Discord.exe` | 536 | 59 each, the same Electron set |
| Firefox's `xul.dll` | 1793 | 60 |
| `obs64.exe` | 3651 | 5 |
| `vlc.exe`, `paintdotnet.exe` | 170, 66 | 1 each |

## 3. Shared blockers

1. **32-bit fail stubs end the process** (ADR 0186: a stub without an argument size in
   `crates/poly-stubs/src/x86_signatures.txt` cannot `ret n`).
   - Every 32-bit installer here (NSIS, Inno, Squirrel, the Firefox stub) ends on the first such call, before any
     window, and only one stop shows per run.
   - The full sets, from the import tables:
     - **NSIS** (Notepad++, Steam, Firefox's inner setup): `kernel32!{lstrcatW, lstrcpyA, lstrcpyW, lstrcpynW,
       GetTempFileNameW, SearchPathW, GetPrivateProfileStringW, WritePrivateProfileStringW}`, `shell32!SHGetFileInfoW`,
       `user32!{CreatePopupMenu, ExitWindowsEx}`, `comctl32!#17` (`InitCommonControls`).
     - **Inno Setup** (VS Code): `kernel32!{GetSystemDefaultUILanguage, GetUserDefaultUILanguage, GetThreadLocale,
       SetThreadLocale, RtlUnwind}`, `user32!ExitWindowsEx`, plus `SetSearchPathMode` and `SetProcessDEPPolicy` found
       through `GetProcAddress`.
     - **Squirrel setup host** (Discord): `kernel32!{GetTempFileNameW, RtlUnwind}`,
       `kernelbase!AppPolicyGetProcessTerminationMethod`, `comctl32!TaskDialogIndirect` (not exported).
     - The 64-bit forms of most of these are fail stubs too (`lstr*` wait for `impl-win-apps`).
   - Runtime lookups add a few more that only runs show.
2. **No 32-bit C runtime DLL.** Poly builds about 40 system DLLs for WOW64 (ADR 0186 stage 3), but not `msvcrt.dll`;
   Firefox's 32-bit setup stub cannot start.
3. **Squirrel needs .NET Framework.** The `Update.exe` of every Squirrel app (Discord here; the same for Slack and many
   Electron apps' classic installers) is a .NET Framework 4.x program (`mscoree!_CorExeMain`). Poly has no CLR; Discord
   cannot be installed or started through its normal layout without one.
4. **Electron.**
   - VS Code: `NtAllocateVirtualMemory` answers `STATUS_NOT_COMMITTED` where Electron expects success, right after a
     decommit, then a CHECK. Which allocation type and range is the open question for a spec sheet: the trail has no
     arguments beyond the pointers.
   - Discord ends silently.
   - Both share 59 fail-only imports and the same stubs reached: `GetPackageFamilyName`,
     `AppPolicyGetThreadInitializationType`, `PowerRegisterSuspendResumeNotification`, `uxtheme!#135`; and
     `kernelbase!GetFileInformationByName`, missing.
   - Chrome for Testing's stack covers the rest.
5. **Text in Qt programs.** Qt's Windows font engine asks GDI for glyph outlines and font data. With
   `GetGlyphOutlineW`, `GetFontData` and `GetCharABCWidthsFloatW` as fail stubs, VLC's labels are blank. The same is
   expected for other Qt 5/6 programs that use the GDI engine.
6. **Runtime DLLs Windows has and Poly lacks.** `msvcp140_1.dll`, `msvcp140_2.dll` (the Visual C++ 2022 redistributable
   beside `msvcp140.dll`), `icuuc.dll` (Windows' ICU since Windows 10 1903) for OBS's Qt 6; `DXCore.dll` (Electron
   probes it). Programs expect them in `System32`.
7. **The .NET host.** `PathCchRemoveFileSpec` and `PathFileExistsW` are fail stubs; .NET's apphost needs them before the
   runtime starts (Paint.NET).
8. **Small Win32 gaps with visible effects.**
   - `comctl32` `LoadIconMetric` (ordinal 381) and `LoadIconWithScaleDown` (380): Notepad++'s icon error.
   - `TaskDialogIndirect`: not exported.
   - `CoCreateInstance` of the MMDevice enumerator: audio (the audio track, ADR 0244).
9. **Tooling** (both fixed in W3, ADR 0252).
   - `win-app` booted QEMU's default CPU model (no SSE4.2, no AVX). Programs that check CPU features (Paint.NET, many
     games, .NET's own checks) refused to run. Now `win-app`, the desktop and the other boots that run programs use
     `-cpu host` on a hypervisor and `max` under TCG.
   - `win-app` could not run the program an installer installed in the same boot: `--then` reruns the same
     executable. Now `--then-exe "PATH|ARGS"` runs a program by its Windows path after the previous run (and
     `launch:N` starts it on the desktop).

## 4. Recommended batches

Sizes as in `chrome-installer-plan.md`:
- **S**: one track of 10 entries or fewer;
- **M**: one track of 10 to 25;
- **L**: two tracks;
- **XL**: three or more.

"Spec" means a spec agent records sheets on the oracle (homeserver2); "impl only" needs no oracle.

| Batch | Content | Spec on the oracle | Size | Unlocks |
|---|---|---|---|---|
| **W0: 32-bit installers start** | The NSIS, Inno and Squirrel-host sets of section 3.1 as real functions (most are kernel32/user32/shell32 basics: `lstr*`, `GetTempFileNameW`, `SearchPathW`, the private-profile pair, `SHGetFileInfoW`, `CreatePopupMenu`, `ExitWindowsEx`, `InitCommonControls`, the UI-language and thread-locale calls, `RtlUnwind`, `SetSearchPathMode`, `SetProcessDEPPolicy`, `AppPolicyGetProcessTerminationMethod`, `TaskDialogIndirect`), each also built for WOW64; plus **argument sizes in `x86_signatures.txt` for every remaining name in these import tables**, so a gap fails a call instead of ending the installer | yes (most have Learn pages and are cheap to observe) | M | NSIS (Notepad++, Steam, Firefox), Inno (VS Code) installers reach their first window; then a second survey pass |
| **W1: what users see in the apps that already run** | GDI glyph outlines and font data (`GetGlyphOutlineW`, `GetFontData`, `GetCharABCWidthsFloatW`; with the test-font approach of `spec-win-text`), `LoadIconMetric`/`LoadIconWithScaleDown` | yes | M | VLC's text; Notepad++ starts clean; other Qt programs' text |
| **W2: Electron** | Pin down VS Code's `STATUS_NOT_COMMITTED` (a spec sheet on `VirtualAlloc`/`VirtualFree` sequences: decommit, re-commit, reset, placeholders), `GetFileInformationByName`, `GetPackageFamilyName` and `GetCurrentPackage*` (`APPMODEL_ERROR_NO_PACKAGE`), `AppPolicyGetThreadInitializationType`, `PowerRegisterSuspendResumeNotification`, `uxtheme` 135; then a rerun of `Code.exe` and `Discord.exe` with `poly.nt.trail` | yes | M (L if the memory question goes deep) | VS Code (zip), probably Discord's app, other Electron apps |
| **W3: the .NET host** | `PathCchRemoveFileSpec`, `PathFileExistsW` and what the next runs show; the `win-app` CPU model | small sheets | S | Paint.NET and other self-contained .NET apps get to the runtime |
| **W4: runtime DLLs** | Poly's own `msvcp140_1.dll`/`msvcp140_2.dll` (the few exports Qt 6 uses: `memory_resource` and the like) and an `icuuc.dll` (the converter calls Qt 6 imports), `DXCore.dll` as "no adapters" | yes for behaviour; the DLLs are Poly's own | M (ICU could be L) | OBS (then its own stops), Qt 6 programs |
| **W5: Firefox** | A 32-bit `msvcrt.dll` (its stub installer), then a `poly.nt.trail` of the browser process's abort, then whatever that shows | after the trace | M, then unknown | Firefox |
| **W6: .NET Framework** | A CLR for `mscoree`: the whole .NET Framework 4.x | n/a | **XL**; owner decision | Squirrel installers (Discord, Slack, ...), WinForms/WPF apps |
| **W7: Steam** | After W0, the client: 32-bit, with its own CEF (`steamwebhelper.exe`), so Chromium on WOW64 | after W0 | unknown, likely XL | Steam |

**Recommended order:**
1. **W0 and W3 first**: cheap, and they turn four installers and one app from "ends at once" into a real first
   window, which a second survey pass then measures.
2. **W1 next**: VLC and Notepad++ are close; this is the most visible gain per entry.
3. **W2.**
4. **W4 and W5 after W2**: they share the "find the first real stop" step.
5. **W6 and W7 need an owner decision on scope.**

### Owner decisions this raises

1. **.NET Framework** (W6). **Open question, deferred until the owner is back** (coordinator, 2026-11-20).
   Squirrel-installed apps need it (Discord, Slack, many classic Electron installers). Options:
   - build a CLR, which is very large;
   - support Squirrel apps by unpacking their `.nupkg` in Poly's installer (a Poly-specific route, like the
     Chrome-installer survey's "route B"; it breaks Squirrel's own update);
   - skip them.
2. **Runtime DLLs** (W4). **Decided (coordinator, 2026-11-20): yes.** Poly writes its own `msvcp140_1/_2` and ICU subset,
   clean room from spec sheets, rather than asking users to install Microsoft's redistributable.
3. **The CPU model Poly's desktop boots with.** **Decided (coordinator, 2026-11-20).** The desktop, the release image and the test
   tools boot with `-cpu host` on a hypervisor (KVM, WHPX, HVF) and QEMU's `max` under TCG, so programs that need
   SSE4.2 or AVX run (W3, ADR 0252). The kernel boot tests keep QEMU's default model.

## 5. Not covered

- **Not tried:** Spotify (not downloaded), GIMP (an Inno Setup installer, so the same blocker is expected
  as VS Code's installer), MSI-based installers.
- **Installed apps not run:** every installer stops before installing anything, so none of the installed programs
  could be started.
- **Apps' main functions only partly used:** only VLC and Notepad++ reach a window. VLC's playback, Notepad++'s
  editing and the rest wait for W1.

## 6. How to run it again

The kit is `poly-dev/scratch/survey-win-apps/`: `dl.py`, `imports.sh`, `runs.sh`, `runs2.sh`, `stubs.py`,
`workaround.py`, and the summaries, screenshots, serial logs and `fail-only.txt` of every run.

1. In the `polyd` container: `python3 dl.py` (downloads into `target/win-app/dl` and unpacks the zips into
   `target/win-app/apps/<name>`).
2. Extract the Firefox and Discord payloads with the cached `target/7zip/26.05/linux/7zz`.
3. `bash imports.sh` for the static picture, `bash runs.sh` for the runs.
4. `workaround.py` makes the second pass: minimal `lstr*` copies and three argument sizes. Never commit it; it is only
   for seeing past the first stop.

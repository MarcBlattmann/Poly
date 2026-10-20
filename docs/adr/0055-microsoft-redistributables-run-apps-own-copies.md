# 0055: Microsoft's redistributable C++ and DirectX runtimes: run the apps' own copies

**Status:** Accepted (owner decision, 2026-10-20). Resolves the ⚖️ item at the
start of Phase 5 in [ROADMAP.md](../ROADMAP.md). Adds the coverage status
`app-provided` to the statuses of [0004](0004-coverage-map-format.md) (stored
as in [0006](0006-coverage-map-one-file-per-component.md)).

## Context

The Windows half of the coverage map is seeded from Wine's `.spec` export
lists ([0004](0004-coverage-map-format.md)), which name every DLL Wine
implements, not only the DLLs that are part of Windows. Before this decision
the map had 131,958 Windows entries (1,811 NT and win32k services, 130,147
DLL exports). A large share belongs to libraries that Microsoft ships with
its developer tools, not with Windows, and that applications install
themselves:

| Family | Components | Entries | Share of Windows entries |
|---|---|---:|---:|
| Visual C++ standard library | `msvcp70` to `msvcp140` (with `msvcp120_app`, `msvcp140_1`, `_2`, `_atomic_wait`, `_codecvt_ids`) | 48,960 | 37.1% |
| Versioned Visual C runtimes | `msvcr70` to `msvcr120` (with `msvcr120_app`) | 13,914 | 10.5% |
| Visual C++ runtime support | `vcruntime140`, `vcruntime140_1` | 91 | <0.1% |
| Concurrency Runtime, OpenMP | `concrt140`, `vcomp`, `vcomp90` to `vcomp140` | 1,402 | 1.1% |
| D3DX (DirectX end-user runtime) | `d3dx9_24` to `d3dx9_43`, `d3dx10_33` to `d3dx10_43`, `d3dx11_42`, `d3dx11_43` | 8,679 | 6.6% |
| HLSL compiler | `d3dcompiler_33` to `d3dcompiler_43`, `d3dcompiler_46` | 152 | 0.1% |
| DirectX end-user runtime audio and input | `xinput1_1` to `xinput1_3`, `xaudio2_0` to `xaudio2_7`, `x3daudio1_0` to `x3daudio1_7`, `xapofx1_1` to `xapofx1_5` | 78 | <0.1% |
| **Total** | 99 components | **73,276** | **55.5%** |

(Counting every file whose name starts with `msvcp` gives about 58k entries,
44% of the Windows entries; that count includes `msvcp_win` and `msvcp60`,
which come with Windows, see below. There is no `vcamp` component in the map.)

These libraries are not part of Windows. Apps either put them next to their
`.exe` ("app-local") or run Microsoft's installer for them (`vc_redist.exe`,
the DirectX end-user runtime's `DXSETUP.exe`) from their own setup program.
Reimplementing them would be most of the Windows work in the map, for code
that every app that needs it already brings along. The roadmap left this
open as a ⚖️ decision before Phase 5.

## Decision

**Poly runs the application's own copies.** A redistributable runtime comes
from the application: app-local DLLs, or the app's own installer
(`vc_redist.exe`, `DXSETUP.exe`, a merge module or MSI package) running on
Poly. We do not reimplement these libraries now and we do not ship
Microsoft's copies.

**Poly provides what is part of Windows itself**, as its own clean-room code:

- `ntdll` (including exception dispatch and x64 unwinding), `kernelbase`,
  `kernel32` and the rest of the system DLLs in the map;
- the Universal CRT: `ucrtbase` and the `api-ms-win-crt-*` API sets (an
  operating system component since Windows 10);
- `msvcrt.dll` (an operating system component), the older runtimes that
  Windows still carries for old programs (`msvcp60`, `msvcirt`, `msvcrt20`,
  `msvcrt40`), and `msvcp_win` (Windows' own C++ library);
- Direct3D 9, 10, 11 and 12, DXGI, `d3dcompiler_47` (part of Windows since
  8.1), `d3dxof`, `xinput1_4`, `xinput9_1_0`, `xinputuap`, `xaudio2_8` and
  `xaudio2_9`.

**Coverage map.** The 99 components in the table get a new status,
`app-provided`: "supplied by the application or its installer, per ADR 0055".
The entries stay in the map so their names stay known, but they are not our
work: the README percentages and the coverage treemap leave them out (the
denominator is the entries that are ours to build), and the README table
shows them in their own column. The status always covers a whole component
(`cargo xtask coverage check` enforces it), new imports into such a
component inherit it, it cannot be claimed, and it cannot carry tests of
ours. `coverage set-status --component <os>/<layer>/<component> <status>`
sets or clears it for a whole library.

Further candidates under the same rule, not marked by this change and left
`missing` until the owner confirms them: `msvcm80` and `msvcm90` (C++/CLI
support, part of the Visual C++ 2005 and 2008 redistributables), `atl80`,
`atl90`, `atl100` and `atl110` (ATL, likewise), and `msvcrtd` (a debug C
runtime, which is not redistributable at all and comes with developer tools).

## Licensing

Microsoft's license terms for Visual Studio and the DirectX SDK let
developers redistribute the listed runtime files *with their own
applications*. Those terms are the app developer's. Poly is not an
application and has no such right: putting Microsoft's DLLs into Poly's
images, installers or repository would be redistribution we are not allowed
to do. So we never ship, download on the user's behalf, modify or patch
Microsoft's files. The user installs an application, together with whatever
runtime it brings, as they would on Windows.

The clean-room rules ([CLEAN_ROOM.md](../../CLEAN_ROOM.md)) apply in full:
running Microsoft's DLLs as black boxes is fine, but nobody disassembles,
decompiles or debugs their internals. When one of them fails on Poly, we
find out what it asked of our `ntdll`, `kernel32` or `ucrtbase` (our own
tracer at our own boundary) and what we answered, and fix our side.

## Consequences

- **Our loader and system DLLs must run Microsoft's real DLLs unchanged.**
  The PE loader follows the documented search order (the application's
  directory first), resolves `api-ms-win-crt-*` and other API sets to our
  DLLs, runs TLS callbacks and `DllMain` in the documented order under the
  loader lock, and supports delay-load imports. The Visual C++ 2005 and 2008
  runtimes (`msvcr80`, `msvcr90`, `msvcp80`, `msvcp90`) are side-by-side
  assemblies found through manifests, so the loader needs activation
  contexts and `WinSxS`-style lookup to run them.
- **Our `ucrtbase` is a hard dependency of every modern app.** `vcruntime140`
  and `msvcp140` import the Universal CRT through `api-ms-win-crt-*`, so the
  Phase 5 C runtime milestone is our `ucrtbase` and `msvcrt`; the versioned
  runtimes come with the apps.
- **C++ exceptions run through our x64 unwinding.** `vcruntime140`'s
  `_CxxThrowException` raises an SEH exception, and its frame handlers
  (`__CxxFrameHandler3`, `__CxxFrameHandler4`) are called by our
  `RtlDispatchException` / `RtlUnwindEx`, which walk the `.pdata` /
  `.xdata` unwind information as Microsoft documents it ("x64 exception
  handling"). This must match the documentation exactly; the structured
  exception handling milestone of Phase 5 carries it.
- **The kernel32 surface these runtimes use** (fiber-local storage, SRW locks
  and condition variables, `WaitOnAddress` for `msvcp140_atomic_wait`, the
  locale functions, the thread pool for `concrt140`) is part of our
  `kernelbase` / `kernel32` work and gets weight from app traces like any
  other API.
- **DirectX end-user runtime parts sit on top of our graphics, audio and
  input.** `d3dx9_*` and `d3dx10_*`/`d3dx11_*` call our Direct3D; `xaudio2_7`
  and older are COM servers registered by `DXSETUP.exe`, so they need our
  COM activation and registry; `xinput1_1` to `xinput1_3` talk to a game
  controller driver.
- **Running app installers needs Windows Installer.** `vc_redist.exe` is a
  bundle of MSI packages, and `DXSETUP.exe` unpacks cabinet files and writes
  to `System32`. Our Windows Installer (`msi.dll`, `msiexec`, part of
  Windows), cabinet support and the Windows file system layout milestone
  have to carry that. A later "install the runtime" helper may run
  Microsoft's official installer that the user downloads; it never fetches
  or bundles the files itself.
- **Apps that assume a pre-installed runtime** and neither ship nor install
  it will fail with a missing DLL until the user installs that runtime. The
  loader's error should name the DLL and say it is a Microsoft
  redistributable.
- **The Windows percentage now measures what is ours to build.** The
  denominator went from 131,958 to 58,682 entries (55.5% of Windows entries
  are app-provided); each green entry now moves the badge about 2.2 times as
  far.

## Fallback: our own `vcruntime140` and `msvcp140` subsets

The owner's words: "if it causes too many problems in the end, we can build
our own." This is recorded as an explicit fallback, with a roadmap line at
the end of Phase 5.

**Trigger.** Reopen this decision when either of these blocks target apps
we care about (a roadmap proof-of-concept app, or several apps from the
target list):

1. apps that expect a pre-installed runtime, do not ship it, and whose
   users cannot reasonably install Microsoft's runtime on Poly (for example
   because its installer cannot run yet);
2. app-shipped Microsoft DLLs that depend on undocumented OS internals we
   cannot provide clean-room (behaviour that no public documentation or
   observation at our own boundary explains).

**What we would build, in order.**

1. `vcruntime140` (and `vcruntime140_1`): about 90 exports; C++ exception
   throwing and catching, frame handlers, RTTI and `type_info`, `memcpy` /
   `memset` style intrinsics, the `__std_*` helpers. Small, and everything
   built with Visual C++ 2015 or later needs it.
2. `msvcp140` subsets, chosen by app-weighted trace data (strings, streams,
   locales, threads, filesystem), not the whole 3,758-export library.

To do this, the components go back to `missing` with
`cargo xtask coverage set-status --component windows/api/vcruntime140.dll missing`
(and so on), and enter the normal spec and implementer pipeline. One
clean-room hurdle to plan for: the C++ exception metadata the compiler emits
(throw descriptors, function and unwind tables) must be specified from
public documentation and observed behaviour only, never from Microsoft's
CRT sources or binaries.

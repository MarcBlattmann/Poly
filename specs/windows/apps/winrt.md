# Windows Runtime calls of the Qt 6 apps (observation note)

Observed by spec agent `spec-win-apps-6`, 2026-11-16. XnView MP 1.12.3 and Audacity 4.0.2 (both Qt 6) were run on Poly (main plus the merge of
`impl-win-apps-4`) with `cargo xtask win-app run --mode console --kernel "poly.nt.trail"`; Windows behaviour comes from test programs of our
own. Nothing of the apps, Qt or Windows was read or disassembled; the apps' import and call names are the only input.

## Where they stop on Poly

| App | Result | What the log shows |
|---|---|---|
| XnView MP | exit code 3 (`abort`) after about 2 s, no window | `NtOpenKeyEx("Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")` -> `STATUS_OBJECT_NAME_NOT_FOUND`; `combase.dll` is mapped; then `LdrLoadDll` of **`Windows.UI.ViewManagement.dll`**, **`Windows.UI.dll`**, **`Windows.dll`** (all absent); stubs `oleaut32!ord_200`, `oleaut32!ord_201`, `ole32!RoOriginateLanguageException`; a C++ exception (0xE06D7363) is raised and not handled; `abort` |
| Audacity | the same call sequence, exit code 3 | identical stubs, identical library probes |

The call comes from the platform plugin (`qwindows.dll`: the stack shows `qwindows.dll` -> `ole32.dll!RoGetActivationFactory` (Poly's name for the
combase export)), asking for the activation factory of a class in `Windows.UI.ViewManagement` (the probed library names are exactly that namespace
and its parents, which is what a C++/WinRT client tries when `RoGetActivationFactory` answers `REGDB_E_CLASSNOTREG`). Poly answers that for every
class, so the client falls back to those `LoadLibrary` probes, then throws `winrt::hresult_error`: `RoOriginateLanguageException`, `oleaut32`
ordinals 200 and 201 (these are **`GetErrorInfo`** and **`SetErrorInfo`**; ordinal 202 is `CreateErrorInfo`: the names are read from oleaut32's
export table) and nobody catches the exception.

The class is the one every Qt 6 theme code reads for the colour scheme: `Windows.UI.ViewManagement.UISettings` (the sheets give its identity and
what its colour getter returns). **Windows gives a working object** for it (`RoActivateInstance` and `RoGetActivationFactory` both succeed), so the apps never
reach the exception path there. To get past this stopper Poly needs a real `UISettings` class (the `RoActivateInstance` sheet specifies its
identity, its interfaces, the getters and `GetColorValue`). Independent of that, the error path itself is specified (`RoOriginateError`,
`RoOriginateLanguageException`, `GetErrorInfo`, `SetErrorInfo`, `CreateErrorInfo`), so that a failed activation of any other class ends in the
failure a client can handle rather than in a stub; whether the apps' code handles `REGDB_E_CLASSNOTREG` for this class is not known (on Windows
they never see it).

## What Windows does, in the sheets

- `combase.dll/RoGetActivationFactory`, `RoActivateInstance`: which classes work, the unregistered-class result (`REGDB_E_CLASSNOTREG`), bad names.
- `combase.dll/WindowsCreateString` and the other HSTRING sheets: every HSTRING rule the C++/WinRT client library relies on (fast-pass references,
  `NULL` as the empty string, the shared handle of a duplicate, the empty results of substring and trim).
- `combase.dll/RoOriginateError` (with `RoOriginateLanguageException`, `GetRestrictedErrorInfo`, `SetRestrictedErrorInfo`) and
  `oleaut32.dll/GetErrorInfo` (with `SetErrorInfo`, `CreateErrorInfo`): the error object a failed WinRT call leaves on the thread; Get takes it once.
- The registry key the apps read first (`HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize`) is missing on Poly and present on
  Windows (values `AppsUseLightTheme` and `SystemUsesLightTheme`, DWORD): not specified here (registry data, not a call); the apps continue without it.

## Not covered

What Qt reads after the first `UISettings` call (the colour change event, `IUISettings2`/`3` events: `add_ColorValuesChanged` takes a real handler
object); the other WinRT classes of the Qt apps (none was seen before the abort); `GetTempPath2W` and the other probes that fail harmlessly
(`kernel32!GetTempPath2W` is in the Firefox sheets).

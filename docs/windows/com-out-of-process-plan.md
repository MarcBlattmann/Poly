# COM across processes for Google's installer: scope and sub-batches (B2)

Track `spec-win-com-oop` (spec agent, 2026-11-17). Batch B2 of
[chrome-installer-plan.md](chrome-installer-plan.md): the installer drives the install through Google's update service over
out-of-process COM, so Poly needs a COM that works between processes. Today Poly's COM is in-process only
(`CoCreateInstance` of a registered DLL class). This file says exactly which behaviour the updater and the installer
depend on, splits it into sub-batches, and says what the first sub-batch (B2a) specifies.

Inputs: the survey's oracle traces (registry before/after, the updater's own log, process lists), the PE import *names* of
Google's `updater.exe` (157, 32-bit, and 159, 64-bit) and Chrome's `elevation_service.exe`, and our own test programs run on the
Windows oracle (homeserver2). No Microsoft or Google binary was decompiled or disassembled.

## 1. What the updater and the installer use

### 1.1 Calls (import names; `-` = not imported)

| Area | updater 157 (x86) / 159 (x64) | `elevation_service.exe` (x64) | Sub-batch |
|---|---|---|---|
| Activation (client) | `CoCreateInstance` | `CoCreateInstance` | B2a |
| Class objects (server) | `CoRegisterClassObject`, `CoResumeClassObjects`, `CoRevokeClassObject` | same | B2a |
| Server lifetime | `CoAddRefServerProcess`, `CoReleaseServerProcess` | same | B2a |
| Apartments | `CoInitializeEx`, `CoUninitialize`, `OleUninitialize`, `CoIncrementMTAUsage` (sheet exists: ole32 `CoIncrementMTAUsage`) | `CoInitializeEx`, `CoUninitialize` | B2a (sheet exists) |
| Initialisation spy | `CoRegisterInitializeSpy`, `CoRevokeInitializeSpy` | same | B2a-extra |
| Security | `CoInitializeSecurity`; `CoSetProxyBlanket` (159 only) | `CoInitializeSecurity` | B2c |
| Caller identity | `CoGetCallContext` | `CoGetCallContext`, `rpcrt4!I_RpcOpenClientProcess` | B2c |
| Misc | `CoCreateGuid`, `IIDFromString`, `StringFromGUID2`, `CoTaskMemAlloc/Free` | `IIDFromString`, `CoTaskMemFree` | (in-process; entries exist) |
| Automation types | `oleaut32` by ordinal: `#2 #4 #6 #7` (BSTR), `#9` (VariantClear), `#16 #17 #19 #20 #23 #24` (SAFEARRAY family), `#77 #161 #184 #411 #424` (to be named from oleaut32's public export table) | `#149 #150` | B0 (missing ones) / B2b |

Not imported by either: `CoCreateInstanceEx`, `CoGetClassObject`, `CoImpersonateClient`, `CoQueryClientBlanket`,
`LoadTypeLib*`, `RegisterTypeLib*`, `rpcrt4` binding or server calls. The type library is not loaded by the programs: COM
loads it by itself when it marshals (section 1.3).

### 1.2 Registry the install writes and COM has to read (observed on the oracle, both the 64-bit and the `WOW6432Node` view)

- `HKLM\SOFTWARE\Classes\AppID\{guid}`: 7 keys with `LocalService` = `GoogleUpdaterService<ver>` or
  `GoogleUpdaterInternalService<ver>` and `ServiceParameters` = `--com-service`; Chrome adds one with
  `LocalService` = `GoogleChromeElevationService` and no `ServiceParameters`. **No `RunAs`, `AccessPermission` or
  `LaunchPermission` value was written by either installer** (the services are LocalSystem; COM's default security applies).
- `HKLM\SOFTWARE\Classes\CLSID\{guid}`: 4 for the updater (each with an `AppID` value), one `LocalServer32`
  (`notification_helper.exe`), `GoogleUpdate.Update3WebMachine` as ProgID.
- `HKLM\SOFTWARE\Classes\Interface\{iid}`: 37 keys with `ProxyStubClsid32` = `{00020424-0000-0000-C000-000000000046}`
  (the automation marshaller, "PSOAInterface") and `TypeLib` = the updater's type library; `TypeLib\{guid}\<ver>\0\win32|win64`
  = `...\updater.exe\4` (resource number 4 of the exe).
- Started by COM: the SCM (`AppID` `LocalService`), with `ServiceParameters` as the service's arguments
  (`updater.exe --system --windows-service --service=update`; the `--com-service` value reaches `ServiceMain` as an argument).

### 1.3 What the log shows the server doing (updater log, `chrome\updater\app\server\win\com_classes.cc`)

1. The service process starts, runs `--com-service`, and registers its class objects ("Running COM server within the Windows
   Service"); COM then lets the waiting client's `CoCreateInstance` return.
2. `RuntimeClassInitialize` of each object logs "COM client for this COM server has PID: n" (the server asks who called:
   B2c).
3. The client calls methods that take callback objects (`IUpdaterCallback::Run`, `IUpdaterInternalCallback::Run`): the
   server calls back into the client (marshalled interface pointers in both directions; the client process must be able to
   receive calls while it waits).
4. WRL's module count is the server's lifetime: every task adds one, every completed task removes one ("TaskCompleted.
   Count: 5 ... Module count: 6" and so on until "COM server is shutting down" at 0): `CoAddRefServerProcess` /
   `CoReleaseServerProcess`.
5. Setup checks `AreComInterfacesPresent` by reading the registry entries of section 1.2.

## 2. The sub-batches

| Batch | Content | Entries | Size | Depends on |
|---|---|---|---|---|
| **B2a** Activation and class objects | `CoRegisterClassObject`, `CoRevokeClassObject`, `CoResumeClassObjects`, `CoSuspendClassObjects`, `CoAddRefServerProcess`, `CoReleaseServerProcess`, `CoCreateInstanceEx` (activation with `LocalServer32`, `AppID\LocalService` + `ServiceParameters`, hives, contexts), plus B2a-extra `CoRegisterInitializeSpy` / `CoRevokeInitializeSpy` when time allows. Only `IUnknown` / `IClassFactory` / `IMarshal` cross processes (marshalled by COM itself, no type library) | 7 (+2) | L | B1 for the SCM start of a LocalService (the oracle tests create and start a real service: the implementer needs the service entries, `spec-win-services`) |
| **B2b** Marshalling through a type library | `CoGetPSClsid`, `CoRegisterPSClsid`, `LoadRegTypeLib`, `LoadTypeLibEx` (path syntax `file\N` for a resource), `RegisterTypeLib(ForUser)`, `UnRegisterTypeLib`, `QueryPathOfRegTypeLib`, `ITypeLib` / `ITypeInfo` as far as the marshaller reads them; the `{00020424}` proxy and stub (PSOAInterface) for custom, dual and `IDispatch` interfaces with `BSTR`, `VARIANT`, `SAFEARRAY`, `IUnknown*`, `[out]`/`[retval]`/`[in,out]` parameters, interface pointers passed to the server (callbacks) and `HRESULT` propagation; `CoMarshalInterface` / `CoUnmarshalInterface` / `CoGetStandardMarshal`; server-side errors (`RPC_E_*`, `RPC_S_*`); test image builds its own type library with `CreateTypeLib2` | ~15 | **XL** | B2a; `rpcrt4` NDR as far as it is needed (an implementer may do it natively) |
| **B2c** Security and caller identity | `CoInitializeSecurity` (once per process: `RPC_E_TOO_LATE`), `CoSetProxyBlanket`, `CoQueryProxyBlanket`, `CoCopyProxy`, `IClientSecurity`, `CoGetCallContext` + `IServerSecurity` (`QueryBlanket`, `ImpersonateClient`, `RevertToSelf`), `CoImpersonateClient`, `CoQueryClientBlanket`, `rpcrt4!I_RpcOpenClientProcess`, AppID `RunAs` / `LaunchPermission` / `AccessPermission`, "who is the caller" (the PID the updater logs), `E_ACCESSDENIED` | ~14 | L | B0 (tokens, `spec-win-elevation`), B2a |
| **B2d** Bitness and lifetime details | 32-bit clients and servers (the updater 157 is x86): the `WOW6432Node` registry view, `CLSCTX_ACTIVATE_32/64_BIT_SERVER`, 32-bit servers started by COM; the server's idle timeout and what COM does when a server dies with objects alive (`RPC_E_DISCONNECTED`, `RPC_S_SERVER_UNAVAILABLE`); `CoDisconnectObject`, `CoLockObjectExternal`; needs 32-bit test images (`win build` builds 64-bit only) | ~8 | M | B2a to B2c; a 32-bit test build in the pipeline |

The B2a-extra pair is imported by both servers; its sheets come with B2a when there is time, else they go first into
B2d.

## 3. What B2a specifies (this run)

Each entry has a sheet in `specs/windows/ole32.dll/` and visible tests beside it; the held-out tests are in the store. Every
test is one Rust program that is client, COM server and (where needed) service in one image (`--child srv|svc <tag> ...`),
run on the oracle twice with identical output, and removed from the machine again.

| Entry | Tests | What is observed |
|---|---|---|
| `CoRegisterClassObject` | `regclass_basic`, `regclass_apartments`, `regclass_launch`, `regclass_launch_fail` | the cookie, factory reference counts, lookup in the registering apartment, `SINGLEUSE` / `MULTIPLEUSE`, the per-apartment rule for in-process and local-server registrations, a server COM starts from a `LocalServer32` command (one process or one per request), a late registration, a server that never registers, a missing server file |
| `CoRevokeClassObject` | `revoke_cookies`, `revoke_holders`, `revoke_remote` | cookie results (unknown cookies succeed), what happens to held factory pointers and to remote clients' proxies |
| `CoCreateInstanceEx` | `createex_multiqi`, `createex_local`, `createex_context`, `createex_hives`, `createex_localservice`, `createex_localservice_args` | `MULTI_QI` results, which definition of a class wins, `HKCU` over `HKLM`, the 32-bit view, `AppID\LocalService` and `ServiceParameters` (starting a real service), a missing service |
| `CoResumeClassObjects`, `CoSuspendClassObjects`, `CoAddRefServerProcess`, `CoReleaseServerProcess` | see the sheets | suspended registrations and their release, the process-wide server count and what reaching zero does |

Left for B2b to B2d: everything that needs more than `IUnknown` / `IClassFactory` across a process boundary.

## 4. Notes for the implementer

- The observable contract is COM's, not RPCSS's: tests never name RPCSS, DCOM wire formats or service names of Windows. A
  Poly design needs a machine-wide registry of class registrations by process (the "class table"), a launcher that
  waits for the registration, and per-process proxies and stubs; how is the implementer's choice (ADR).
- Two behaviours that look odd and are real: an activation of a class whose server started and ended without
  registering **keeps the caller waiting** (and later requests for the same class id wait behind it), and registrations
  for the in-process context are **per apartment**.
- The tests write class keys under `HKLM\SOFTWARE\Classes\CLSID\{B2A0xxxx-...}` and (for services) the service
  `PolyWsComSvc<n>`; Poly's registry and SCM entries (B1) have to let a test program do the same.

## 5. Status

B2a is implemented by [ADR 0220](../adr/0220-com-across-processes-an-activation-process-object-exporters-on-named-pipes-and-the-class-table.md)
(track `impl-chrome-installer-com`): the class table, `comlaunch.exe` (the activation process), proxies for
`IUnknown`/`IClassFactory` across apartments and processes, the server count and initialisation spies. 22 of the 24
visible tests pass on Poly; the two LocalService tests wait for B1's service functions. The ADR's section 6 says
where B2b's marshalling and B2c's security plug in.

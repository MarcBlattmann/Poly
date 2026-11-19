# 0220: COM across processes: an activation process, object exporters on named pipes, and the class table

**Status:** Accepted. Batch B2a of [the Chrome-installer plan](../windows/chrome-installer-plan.md) (track
`impl-chrome-installer-com`): Google's installer drives the install through its update service over out-of-process
COM, and Poly's COM was in-process only (every class unregistered but the global interface table, ADR 0207). Inputs:
the nine B2a spec sheets and their 24 visible tests (`specs/windows/ole32.dll/{CoRegisterClassObject,
CoRevokeClassObject, CoCreateInstanceEx, CoResumeClassObjects, CoSuspendClassObjects, CoAddRefServerProcess,
CoReleaseServerProcess, CoRegisterInitializeSpy, CoRevokeInitializeSpy}`, track `spec-win-com-oop`), the scope in
[com-out-of-process-plan.md](../windows/com-out-of-process-plan.md), Microsoft Learn ("CoRegisterClassObject",
"REGCLS", "CLSCTX", "CoCreateInstanceEx", "MULTI_QI", "LocalServer32", "AppID", "LocalService", "ServiceParameters",
"CoAddRefServerProcess", "IInitializeSpy", "Out-of-Process Server Implementation"), `[MS-DCOM]` for vocabulary
only, and the SDK headers named in the code. No third-party source.

## Context

What the sheets require, beyond a table of factories:

- **Machine-wide activation.** `CoCreateInstance(CLSCTX_LOCAL_SERVER)` in one process finds a class object another
  process registered; with none registered it starts the class's `LocalServer32` command (or the service its
  `AppID\LocalService` names, with `ServiceParameters` as arguments) and **waits until that process registers**,
  however long (a server that never registers keeps the caller waiting, and later requests for the class wait
  behind it). `REGCLS_SINGLEUSE` serves one request (three requests, three servers). `HKCU\Software\Classes` wins
  over `HKLM`; a class only in the 32-bit view (`WOW6432Node`) is found.
- **Proxies.** What comes back from another process, and from another apartment of the same process for a
  local-server registration, is a proxy: it answers `IMarshal`, `IClassFactory::CreateInstance` runs in the
  registering apartment (an STA's own thread while it pumps messages; for the MTA, a thread that is neither the
  registering one nor the caller), objects it makes are proxies too, a revoked factory's proxy fails with
  `RPC_E_DISCONNECTED`. Only `IUnknown` and `IClassFactory` cross a boundary in B2a.
- **The class table's states**: per-apartment in-process registrations, `REGCLS_SUSPENDED` held until
  `CoResumeClassObjects` (`0x800706C6` when nothing was waiting), `CoSuspendClassObjects` hiding local-server
  registrations for good, and the server count (`CoAddRefServerProcess`/`CoReleaseServerProcess`) whose release to 0
  hides every local-server registration permanently.
- **Initialisation spies** per thread, newest first, around every `CoInitializeEx`/`CoUninitialize`.

Windows does the machine-wide part in RPCSS and the DCOM launcher over ALPC with `[MS-DCOM]` packets. Nothing of that
is observable to a program (the sheets say so), so Poly is free to choose its own transport and processes.

## Decision

### 1. Three parts

| Part | Where | Does |
|---|---|---|
| The **class table** of a process | `poly_com::classtable` (host-tested), held by `ole32` | registrations, contexts, apartments, suspended/hidden/dead states, the server count, which registrations are *published* machine-wide |
| The **activation process** `comlaunch.exe` | `personalities/nt/win32/comlaunch`, state in `poly_com::activation` (host-tested) | the machine-wide table of published class objects, starting servers, making callers wait for a registration |
| The **object exporter** of a process | `ole32` (`oop` module), messages in `poly_com::wire` (host-tested) | a named pipe per process that serves calls on the objects the process handed out |

### 2. Transport: named pipes, one call per connection

Poly's named pipes (ADR 0169, used by Chrome's IPC) carry everything: framed messages (`u32` length, then the
payload of `poly_com::wire`), one request and one reply per connection, then the connection closes. That costs a
connect per call and needs no multiplexing, no call ids and no connection state: calls in both directions and calls
made while another is in flight (callbacks, B2b) are simply separate connections. The pipes are
`\\.\pipe\poly-com-activation` (the activation process) and `\\.\pipe\poly-com-ox-<pid>-<random>` (each process's
exporter, made on first use: when it publishes a class or hands out an object).

ALPC, Windows' own carrier for this, was the other choice. Poly has no ALPC ports; pipes are complete and carry
Chrome's IPC every day. Nothing above the `wire` module depends on the choice, so moving to ALPC (or to shared
sections for large payloads) later changes one module.

### 3. The activation process

`C:\Windows\System32\comlaunch.exe` (Poly's own file, installed by the kernel's layout like the DLLs) plays RPCSS's
endpoint-mapper and the DCOM launcher's roles: it keeps, per class id, the published registrations (process id,
cookie, exporter pipe, object id of the factory, single use), and answers three requests: **Register**, **Revoke**
and **Activate**. Activate returns a live registration (removing a single-use one), or, if a launch of that class is
pending, waits behind it; otherwise, with a launch plan, starts the server and waits until it registers. Its
decisions are `poly_com::activation::Activator`, a state machine without I/O; the exe adds the pipe, threads,
`CreateProcessW` and polling. Registrations of a process that ended are dropped (it keeps a handle of each
registering process and checks it). A launch is abandoned after 120 s with `CO_E_SERVER_EXEC_FAILURE`; that limit is
Poly's (the sheets only say the wait is longer than 6 s).

**Who starts it.** `ole32` starts it on demand, under a named mutex so only one starts: when a process publishes its
first class, and when an activation needs a launch. An activation that needs no launch (no registry entry) does not
start it: with no activation process running nothing can be published, so `REGDB_E_CLASSNOTREG` is the answer at
once. Chrome's many probes of unregistered classes (BITS among them, ADR 0207) therefore never start a process.
When B1's service host exists it can start `comlaunch.exe` at boot as a service instead; nothing else changes.

### 4. How servers are started and found

The **client's** `ole32` reads the class key (`HKCU\Software\Classes\CLSID`, then `HKLM\SOFTWARE\Classes\CLSID`, then
`HKLM\SOFTWARE\Classes\WOW6432Node\CLSID`; the first hive that has the key wins), because the in-process parts
(`InprocServer32`, the context rules) are decided there anyway, and sends a **launch plan** with Activate:

- `AppID` value → `Classes\AppID\{appid}` with `LocalService` → plan *service* (name, `ServiceParameters` split at
  spaces); `LocalService` wins over `LocalServer32`;
- else `LocalServer32` → plan *command*, the command line with ` -Embedding` appended (Learn, "Out-of-Process Server
  Implementation": COM starts a local server with `-Embedding`).

`comlaunch.exe` runs a command plan with `CreateProcessW`; a command that cannot start is `REGDB_E_CLASSNOTREG` at
once (sheet: a missing server file). `CLSCTX_ACTIVATE_32_BIT_SERVER`/`_64_BIT_SERVER` are checked against the
machine of the command's image (PE header) before anything starts.

**LocalService and B1.** A service plan goes to the service control manager: `OpenSCManagerW`, `OpenServiceW`,
`StartServiceW(args)` — B1's job (`spec-win-services`, in parallel). Until B1 lands those fail with
`ERROR_CALL_NOT_IMPLEMENTED`, and `comlaunch.exe` falls back to *start the service's executable as a plain process*:
`HKLM\SYSTEM\CurrentControlSet\Services\<name>\ImagePath` plus the arguments. A service key that does not exist is
`ERROR_SERVICE_DOES_NOT_EXIST` (`0x80070424`), as the sheet observed. When B1 lands, the fallback is deleted and the
hook is the three SCM calls; the two LocalService visible tests (`createex_localservice*`) need B1 anyway, since the
test programs create and run real services.

### 5. Proxies and the exporter (`ole32`)

- **Object references.** An object handed across a boundary is *exported*: the exporter's table maps an object id to
  the object's identity (`IUnknown`), its apartment, and the references the proxies hold; an object reference on the
  wire is (exporter pipe, object id). A registered factory is exported when its registration is published and
  disconnected when it is withdrawn (revoke, suspend, server count 0): later calls get `RPC_E_DISCONNECTED`.
- **The proxy** is one object per (exporter, object id) per process, so identity holds (`IUnknown` from any of its
  interfaces is the same pointer). It has three interfaces: `IUnknown`, `IClassFactory` (when the object has it) and
  `IMarshal` (answered by the proxy itself, as Windows' proxy manager does; its methods return `E_NOTIMPL` in B2a).
  `AddRef`/`Release` are local; the last `Release` sends one remote release. `CreateInstance` with an outer unknown
  is `CLASS_E_NOAGGREGATION` without a call.
- **Calls in the server** run in the object's apartment: an STA's calls are posted to a message-only window the STA
  thread owns (made on that thread when it first exports something), so they run when it pumps messages; the MTA's
  calls run on the exporter's connection thread.
- **The same proxy type serves other apartments of the same process**, with an in-process channel instead of a pipe:
  calls into an STA are posted to its window, calls into the MTA run on a new thread. A caller in an STA waits for
  its reply pumping only those COM windows' messages.
- **The server count during activation.** A `CreateInstance` that arrives from another process runs with the count
  raised by one (sheet: the count is already 1 inside it); that internal raise and its lowering never hide classes.

### 6. Plug points for B2b and B2c

- **B2b (type-library marshalling).** A remote `QueryInterface` for any interface but `IUnknown`/`IClassFactory` is
  answered `E_NOINTERFACE` today, because there is nothing to marshal it with. B2b adds a marshaller lookup by IID
  (`Interface\{iid}\ProxyStubClsid32` → the automation marshaller reading the type library): the proxy gains an
  interface whose methods encode their arguments into a new `wire` operation (`Invoke { iid, method, ndr bytes }`),
  and the exporter's dispatch gains the stub side. Interface pointers in arguments become object references both
  ways (callbacks into the client: the client gets an exporter too, which already exists for any process). A
  client STA waiting for a reply must then pump messages and serve incoming calls: B2b moves pipe I/O to
  overlapped reads with `MsgWaitForMultipleObjects`.
- **B2c (security).** Requests carry no identity today; the exporter and `comlaunch.exe` can learn the caller's
  process from the pipe (`GetNamedPipeClientProcessId`) and its token by impersonating the pipe client — that is where
  `CoGetCallContext`, `CoImpersonateClient`, `I_RpcOpenClientProcess` and the AppID's `LaunchPermission`/
  `AccessPermission` checks go. B2c should also make `comlaunch.exe` re-read the class from the registry itself
  instead of trusting the client's launch plan, once it runs with more rights than its clients.
- **B2d (bitness, lifetime).** A 32-bit `ole32` speaks the same `wire` messages; only the registry view changes.

### 7. The in-process rules (class table)

Kept in `poly_com::classtable`, one per process, as the sheets give them: cookies never 0 and never reused; a
registration holds one reference of its object; in-process-context registrations are per apartment;
`REGCLS_MULTIPLEUSE` with `CLSCTX_LOCAL_SERVER` also serves in-process requests of its apartment (Learn, "REGCLS");
local-server registrations are found from other apartments as proxies; a revoke from another apartment of an STA's
registration takes effect when the STA revokes it itself or ends (the visible test finds the class still served
right after such a revoke); `CoSuspendClassObjects` hides every local-server registration (a registration made for
both contexts comes back with the next `CoResumeClassObjects`); the server count's release to 0 makes every
local-server registration dead and closes the process for local-server requests; a release at 0 wraps once COM was
initialised or the count was raised. Spies (`poly_com::spy`) are per thread, newest first for every method, and end
with their thread (`DLL_THREAD_DETACH`).

### 8. What the recorded runs settled where the sheets' prose was open

- **Bad arguments write the cookie.** `CoRegisterClassObject` with an undefined flag or a NULL object returns
  `E_INVALIDARG` *and writes 0* to the cookie (the recorded output: "cookie stays 0"); only a thread without COM
  leaves it untouched.
- **Waiting behind a launch is per launch plan.** A request for a class whose server was started and never
  registered waits behind that start when it would start the same thing; with another command (the class's
  `LocalServer32` now names a missing file) it is tried at once and fails with `REGDB_E_CLASSNOTREG`. So pending
  launches are keyed by (class id, plan).
- **Withdrawing does not wait.** `CoReleaseServerProcess` reaching 0, `CoRevokeClassObject` and the end of an
  apartment return at once, as on Windows (a test server logs its count before its own polling loop notices the
  last object); the factory's export is disconnected synchronously and the activation process is told by an
  ordered sender thread (started when the process first publishes, so the withdrawal itself costs a queue entry
  and an event). A client that was served a factory in that moment finds it disconnected when it first calls it
  and asks the activation process again.
- **Stale and fresh registrations (2026-11-19, the three races).** A server can end before its withdrawal arrives,
  so `comlaunch.exe` checks a registration that existed before the request (a call on the factory that needs no
  apartment: `RPC_E_DISCONNECTED` or no pipe means it is gone, and it is dropped). A registration that arrives
  while the request waits is handed out at once, without the check, and the waiting request looks every 1 ms:
  the visible test that restarts a service after its stop gets a server that leaves on its first 10 ms poll, and
  Windows serves the caller inside that time. Both pipes always have a listening instance (the next one is made
  before a connected one is handed on, the exporter's first one before its name is published): a client takes
  "no pipe" for "no server".
- **Spies and `OleInitialize`.** The spies see the result of `OleInitialize`'s COM part (`S_FALSE` on a thread that
  already has its STA), while `OleInitialize` keeps returning its own result.

## Consequences

- `CoCreateInstance(CLSCTX_LOCAL_SERVER)` of a registered class starts a real server process and returns working
  `IUnknown`/`IClassFactory` proxies; 22 of the 24 visible tests pass on Poly (three rounds, identical); the other
  two (`createex_localservice*`) cannot load until B1 exports the service functions they import
  (`RegisterServiceCtrlHandlerExW` and the rest).
- Every process that publishes a class or hands out an object runs one listener thread and creates pipe
  instances; processes that only use in-process COM (Chrome) are unchanged: no thread, no pipe, no window.
- A new system file, `comlaunch.exe`, in `System32`, built with the Win32 modules and embedded in the kernel.
- Calls cost a pipe connection each (tens of microseconds on Poly); fine for activation and the updater's call
  rate, and the place to optimise when a program needs more.

## Addendum (B2b, 2026-11-19): type libraries and standard marshalling

Batch B2b (track `impl-chrome-installer-com-b2b`). Inputs:
- the 14 B2b sheets and their 19 visible tests (`specs/windows/oleaut32.dll/{CreateTypeLib2, LoadTypeLibEx, LoadRegTypeLib, RegisterTypeLib, UnRegisterTypeLib, QueryPathOfRegTypeLib, DllGetClassObject}`, `specs/windows/ole32.dll/{CoGetPSClsid, CoRegisterPSClsid, CoMarshalInterface, CoUnmarshalInterface, CoReleaseMarshalData, CoGetMarshalSizeMax, CoGetStandardMarshal}`, track `spec-win-com-typelib`);
- [com-typelib-plan.md](../windows/com-typelib-plan.md);
- Microsoft Learn (the pages of these functions, "IPSFactoryBuffer", "IRpcProxyBuffer", "IRpcStubBuffer", "IRpcChannelBuffer", "RPCOLEMESSAGE", "ITypeLib", "ITypeInfo", "ICreateTypeLib2", "ICreateTypeInfo", "Type Library Registration", "RegDeleteTreeW");
- `[MS-DCOM]` 2.2.18 for the `OBJREF` layout, `[MS-OAUT]` for the automation types;
- the SDK headers named in the code.

No third-party source.

### A. Type libraries (`poly_com::typelib`, `oleaut32/src/typelib/`)

- **Model and views.** A library is a host-tested model: types, functions, parameters, `TYPEDESC` trees, and imports for references to other libraries.
  - Its `HREFTYPE`s are Poly's own encoding: internal type, import, or the interface form of a dual interface.
  - A dual interface is stored once and read in two forms. The library hands out the dispatch form: `IUnknown`'s and `IDispatch`'s methods first, results folded, vtable size 56. `GetRefTypeOfImplType(-1)` gives the interface form. The marshaller uses the interface form.
- **Files.** `CreateTypeLib2` writes **Poly's own format** (`PolyTLB\0` and the model). Programs never read the bytes, and the sheet allows any form that reads back.
  - `file::read` is the single entry for loading. It routes by the first bytes.
  - The `MSFT` format of MIDL-built libraries (Google's updater) has its slot in `typelib::msft`. Until the format sheet exists (B2b-2), it answers `Unsupported`, which the loader reports as `TYPE_E_CANTLOADLIBRARY`. No layout is guessed.
  - `file\N` resource paths are read through `LoadLibraryExW(LOAD_LIBRARY_AS_DATAFILE)` and `FindResourceW("TYPELIB")`.
- **Registration.** The sheets' rules are host-tested in `typelib::reg`: decimal version keys, version pick (same major, lowest minor at least the one asked for), LCID pick (exact, then neutral), `win64` then `win32`, and which interfaces get `Interface` keys.
  - Registration writes `HKLM\SOFTWARE\Classes`. Lookups read the user's classes first, then the machine's.
- **`stdole`.** Poly's own `stdole` 2.0 is built in code. The kernel installs it as `System32\stdole2.tlb`, and the default hive registers it, together with the `Interface` keys of IDispatch, IClassFactory, IStream and IErrorInfo, and `CLSID\{00020424}` and `{00020420}` (`oleaut32.dll`).
- **`ITypeInfo::Invoke` and `DispCallFunc`.** They call vtable methods through an x64 call trampoline (`oleaut32/src/call.rs`), converting arguments with `VariantChangeType`. On other architectures the generic call and the marshaller are not available.
- **Error information.** It lives in `oleaut32`, one object per thread. `ole32`'s `CreateErrorInfo`, `SetErrorInfo` and `GetErrorInfo` are `oleaut32`'s.

### B. Standard marshalling (`ole32/src/oop/{marshal, channel, proxy, exporter}.rs`, `poly_com::marshal`)

- **`OBJREF`.** `CoMarshalInterface` writes an `OBJREF` with the signature `MEOW`.
  - `OBJREF_STANDARD` is 68 bytes: the header, a `STDOBJREF` and an empty string-binding array. Its OXID, OID and IPID carry Poly's exporter (pid and random part of the pipe name), the object's export id and a marshal-table token.
  - `MSHCTX_INPROC` writes `OBJREF_CUSTOM` with `CLSID_StdMarshal`.
- **References.**
  - A LOCAL marshal entry holds one reference of the object's export; an INPROC entry holds one of the object.
  - `NORMAL` data is used up by its first unmarshal. Table data lives until `CoReleaseMarshalData`, and an unmarshal in another process first asks for a reference of its own (`Op::AddRef` with the token; `CO_E_OBJNOTREG` once released).
  - Marshalling a proxy (recognised by its identity, so also an interface proxy) asks the object's process for one more reference (`Op::AddRef`, token 0); whoever unmarshals takes it over. Data that comes back to the object's own process yields the object itself.
- **Proxy/stub classes.** `CoGetPSClsid` reads `Interface\{iid}\ProxyStubClsid32`. Successful answers are cached per process, `CoRegisterPSClsid` overrides them, and the cache is cleared after the last apartment ends.
  - For `{00020424}` and `{00020420}`, `ole32` calls `oleaut32!DllGetClassObject` directly; other classes go through `CoGetClassObject`.
- **Proxies.** A remote `QueryInterface` for any other interface now gives an interface proxy:
  - `IPSFactoryBuffer::CreateProxy` is called with the proxy manager as the outer object. As the sheet observed, `*ppv` stays NULL, so the interface is obtained from the proxy buffer.
  - The proxy is connected to a client `IRpcChannelBuffer` whose `SendReceive` sends `Op::Invoke` (interface, method, data). For another apartment of the same process, it runs the stub there.
  - Interface proxies are cached per IID in the manager and disconnected with it.
- **Exporter.** The exporter answers `Op::Invoke` by making the interface's stub (`CreateStub`) and calling `IRpcStubBuffer::Invoke` with a server channel. It answers `Op::AddRef` too.
  - Replies carry data (`Reply.data`). `MAX_FRAME` is now 16 MiB.
- **Callbacks into an STA.** A caller on an STA thread runs its pipe transaction on a helper thread and pumps its COM window meanwhile (`apt::blocking`), so callbacks into it run on that thread. Handing out an object from an STA makes the window.
- **Pipes.** The exporter's listener now creates the next listening instance before it hands a connection on. A client that finds no instance retries for about 100 ms before it reports the server gone (`0x800706BA`). One connection per call made this window visible as spurious failures.
- **Activation.** The server is asked for `IUnknown`, and the interface comes from a query (the recorded servers log `create IUnknown`). The client re-asks the activation process for up to a second while a served class object is gone.

### C. The automation marshaller (`oleaut32/src/marshal/`)

- **`info`.** An interface's methods come from `Interface\{iid}\TypeLib` and the registered library, with each parameter's direction and wire type.
- **`codec`.** Poly's own encoding (nothing of it is visible to programs):
  - integers and floats by size; `BSTR`s by byte length (an odd length is kept);
  - `LPWSTR`s by character count (NULL is `RPC_X_NULL_REF_POINTER` in the client);
  - `VARIANT`s by type and value (`DISP_E_BADVARTYPE` for types a variant cannot have; a NULL `BSTR` inside one arrives empty; `VT_BYREF` arrives by reference to the server's copy);
  - `SAFEARRAY`s with their bounds; interface pointers as the `OBJREF`s that `CoMarshalInterface` writes.
- **`proxy`.** The vtable entries are naked thunks (one per slot) that spill the register arguments and call a dispatcher, which encodes `[in]` values, sends them and decodes the results.
  - A call that fails before it is sent leaves its pure `[out]` values empty. A failure of the channel leaves them alone.
  - Error information travels in the reply and becomes the calling thread's (or none).
- **`stub`.** Decodes, calls the method through the call trampoline, and encodes the `HRESULT`, the `[out]` values and the error information.
- **`disp`.** `IDispatch`'s four methods.
  - `GetTypeInfo` sends the type information by value: the library's bytes and the index. The client's copy answers `IMarshal` as a proxy does.
  - `GetIDsOfNames` returns the ids also on failure.
  - `Invoke` refuses a non-NULL IID in the client (`DISP_E_UNKNOWNINTERFACE`), and returns the result, the `EXCEPINFO` of `DISP_E_EXCEPTION` and the argument-error index.

### D. Also

- `kernelbase` gained `RegDeleteTreeW`, forwarded from `advapi32`. It was a fail stub, so the tests' own cleanup did nothing.
- `CoRegisterInitializeSpy` writes its cookie only after a `QueryInterface` that returned `S_OK` with a pointer.

### Results

- **B2b:** all 19 visible tests (and `LoadTypeLib`'s test in the same run) pass on Poly.
- **B2a:** 23 of its 24 visible tests pass; before, 22 passed and the two service tests could not load. The one left, `createex_localservice`, is the creation after a STOP control: the stopped service's class object stays disconnected at the activation process for more than a second. It belongs to the B1/B2a service stop path and is not changed here.

### Left for B2b-2 and later

- **The `MSFT` reader** (format sheet), which loading `updater.exe\N` needs.
- **Not done here:** `RegisterTypeLibForUser`, records (`VT_RECORD`, `IRecordInfo`), `CreateStdDispatch`/`DispInvoke`, 32-bit proxies (B2d), and reclaiming the references of a client process that ended (held-out rule `other_process`: references back within 5 s).

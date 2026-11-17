# COM marshalling through a type library for Google's installer: scope of B2b

Track `spec-win-com-typelib` (spec agent, 2026-11-17). Batch B2b of [chrome-installer-plan.md](chrome-installer-plan.md); B2a
(activation and class objects) is in [com-out-of-process-plan.md](com-out-of-process-plan.md). This file says what the
updater's interfaces need from the type-library marshaller, which entries B2b specifies, what the tests build, and what is
left for B2b-2.

Inputs: the survey's registry diffs and logs, the PE import *names* of `updater.exe` (157 and 159) and
`elevation_service.exe`, the updater's own type libraries **read through the public `ITypeLib` / `ITypeInfo` interfaces on the
oracle** (homeserver2; nothing was disassembled), and our own test programs run on the oracle. No Wine or ReactOS source was read.

## 1. Corrections to B2a's notes

- The oleaut32 ordinals the updater imports are, by the export table of the oracle's `oleaut32.dll`: `#161` **LoadTypeLib**,
  `#184` SystemTimeToVariantTime, `#424` OleLoadPicturePath, `#77` SafeArrayGetVartype, `#411` SafeArrayCreateVector,
  `#2 #4 #6 #7` SysAllocString, SysAllocStringLen, SysFreeString, SysStringLen, `#9` VariantClear, `#16 #17 #19 #20 #23 #24`
  SafeArrayDestroy, GetDim, GetUBound, GetLBound, AccessData, UnaccessData; the elevation service: `#149 #150` SysStringByteLen,
  SysAllocStringByteLen. **B2a said LoadTypeLib is not imported: it is (`#161`).**
- On the oracle `LoadTypeLib` (and `LoadTypeLibEx` with `REGKIND_DEFAULT`) **does not register** anything (test `loadtl_registers`,
  `loadex_regkind`); only `RegisterTypeLib` and `REGKIND_REGISTER` do. So the 37 `Interface` keys and the `TypeLib` keys the
  install leaves (with `updater.exe\N` as the `win64` value) come from a `RegisterTypeLib` call (ATL's registration code calls
  `LoadTypeLib` first, which is the `#161` import; `RegisterTypeLib` is not imported by name, the updater reaches it
  some other way, for example through ATL's own import of it in a helper).
  This is an inference from the import list and the registry result, not an observed call.

## 2. What the updater's type libraries contain (read on the oracle)

`updater.exe` (159.0.8140.0) holds six `TYPELIB` resources, loaded with `LoadTypeLibEx("updater.exe\N")`; 7 and above do not exist
(`TYPE_E_CANTLOADLIBRARY`):

| N | Library | Types |
|---|---|---|
| 1, 4 | `UpdaterLib` (user, system) | 2 coclasses, 4 custom `oleautomation` interfaces (`IUpdateState*`, `ICompleteStatus*`, `IUpdaterObserver*`, `IUpdaterCallback*`), 2 dual interfaces (`IUpdaterAppState`, `IUpdaterAppState*`), `IUpdaterAppStatesCallback*`, `IUpdater*`, `IUpdater2*` (11 types) |
| 2, 5 | `UpdaterInternalLib` | 2 coclasses, `IUpdaterInternalCallback*`, `IUpdaterInternal*` (4 types) |
| 3, 6 | `UpdaterLegacyLib` | 6 coclasses, about 25 dual interfaces (`IGoogleUpdate3Web`, `IAppBundleWeb`, `IAppWeb`, `IAppCommandWeb`, `IPolicyStatus`, `ICurrentState`, ...), the alias `ULONG_PTR` (29 and 33 types) |

All libraries are syskind 3 (64-bit), LCID 0, version 1.0. The full listing of resources 4 to 6 (every function with its parameter
types and flags) is the oracle observation `docs/windows/com-typelib-updater-types.md`.

What the marshaller has to do with them (patterns, counted over resources 4, 5 and 6):

- **`[in] LPCWSTR` parameters (VT_LPWSTR, 31)** are the commonest parameter type (153 of about 400); results are `HRESULT` in the
  custom interfaces; `[out, retval]` pointers to `long`, `BSTR`, **64-bit integers (VT_I8)**; **property gets** (`INVOKE_PROPERTYGET`,
  e.g. `IUpdateState::downloadedBytes`).
- **Interface pointers as parameters in both directions**: the client passes its callback objects (`IUpdaterCallback`,
  `IUpdaterObserver`, `IUpdaterAppStatesCallback`) to the server; the server calls them back and passes its own objects
  (`IUpdateState`, `ICompleteStatus`) to the client's callback.
- **A `VARIANT` parameter** (`IUpdaterAppStatesCallback::Run(VARIANT)`) that carries an array of `IDispatch` pointers (the dual
  `IUpdaterAppState` objects).
- Dual interfaces: `IDispatch::GetIDsOfNames` / `Invoke` across the process boundary, properties through `DISPATCH_PROPERTYGET`.
- The base interface of every custom interface is `IUnknown`, resolved through the standard library `stdole` (`{00020430-...}` 2.0,
  file `stdole2.tlb`): the loader needs it to read these libraries.

## 3. What B2b specifies

All entries have a sheet, visible tests and held-out tests. Each test is one image: client, a COM server it starts itself (the class is
registered for `LocalServer32` pointing to the image), and objects of its own; a **type library it builds itself** with
`CreateTypeLib2` (a callback interface, a custom `oleautomation` interface with 15 methods covering the patterns above, a dual interface),
registers with `RegisterTypeLib` and removes again. No Microsoft or Google type library is needed by the tests.

| Entry | Tests (visible + held out is set by `tests split`) | What is observed |
|---|---|---|
| oleaut32 `CreateTypeLib2` | `createlib_roundtrip`, `createlib_errors` | the library as `ITypeLib` / `ITypeInfo` read it back: attributes, names, kinds, flags, function and parameter descriptions, how a dual interface is stored; errors of the builder interfaces |
| oleaut32 `LoadTypeLibEx` | `loadex_regkind`, `loadex_resource` | `REGKIND_*`, `file\N` resource syntax (a DLL image the test writes), errors |
| oleaut32 `LoadTypeLib` | `loadtl_registers`, `loadtl_errors` | no registration, caching, file and path errors |
| oleaut32 `LoadRegTypeLib` | `loadreg_versions`, `loadreg_stdole`, `loadreg_errors` | version and LCID matching, `stdole`, unloadable registrations |
| oleaut32 `RegisterTypeLib` | `register_keys`, `register_variants` | every key and value written, flags, LCID, versions, interface registration rules |
| oleaut32 `UnRegisterTypeLib` | `unreg_keys`, `unreg_errors` | what is removed, mismatching arguments |
| oleaut32 `QueryPathOfRegTypeLib` | `querypath_basic` | the registered path, version and LCID rules, resource index, `win32` fallback |
| oleaut32 `DllGetClassObject` | `psfactory_create`, `call_errors`, `call_dispatch` | the automation marshaller class `{00020424-...}` (`IPSFactoryBuffer`, proxy and stub objects), HRESULT and `IErrorInfo` propagation, `IDispatch` across processes |
| ole32 `CoGetPSClsid`, `CoRegisterPSClsid` | `psclsid_lookup`, `regps_basic` | registry lookup order, cache, per-process override |
| ole32 `CoMarshalInterface` | `marshal_stream`, `marshal_errors`, `marshal_child`, `callbacks_sta`, `callbacks_mta` | the OBJREF in a stream, contexts and flags, an object marshalled to another process, callbacks |
| ole32 `CoUnmarshalInterface` | `call_basic`, `call_variants`, `call_arrays`, `call_objects`, `unmarshal_stream` | calls through the proxy: integers, `BSTR`, `LPWSTR`, `VARIANT`, `SAFEARRAY`, `[in, out]`, interface pointers, server death; unmarshalling rules |
| ole32 `CoReleaseMarshalData`, `CoGetMarshalSizeMax`, `CoGetStandardMarshal` | 2, 2, 1 | see the sheets |

## 4. Left for B2b-2

- `RegisterTypeLibForUser`, `UnRegisterTypeLibForUser` (per-user registration under `HKCU`), `CoMarshalInterThreadInterfaceInStream`,
  `CoGetInterfaceAndReleaseStream`, `CoMarshalHresult`, `CoUnmarshalHresult`.
- **The `MSFT` type-library file format (B2b-2, first item; owner's decision 2026-11-17).** Poly must read `updater.exe`'s libraries at run
  time (the owner downloads Google's installer; a build-time converter is out). No public document describes the format, so the
  spec agent derives a **format sheet from black-box observation**: write libraries with `CreateTypeLib2` on the oracle, vary one
  thing at a time (a name, a guid, a function, a parameter type, a flag, an inherited interface, a reference to another type,
  an alias), compare the bytes, and check every hypothesis by reading the resulting file back through `ITypeLib` / `ITypeInfo`
  and by loading Google's own libraries (`updater.exe` to ``, `docs/windows/com-typelib-updater-types.md` is the target
  content). The sheet covers what `LoadTypeLib` must read; writing the format is not needed (Poly's own `CreateTypeLib2`
  may store libraries in any form it reads back). Nothing of Wine, ReactOS or any decompiler is used. Sub-batch B2b-2 starts after
  the PR of this batch.
- `SAFEARRAY` records (`IRecordInfo`), `VT_RECORD`, `VT_CY`/`VT_DECIMAL` edge cases, `ICreateTypeInfo2` custom data, `ITypeComp`,
  `ITypeInfo::Invoke` details for server-side `IDispatch`, `CreateStdDispatch`, `DispInvoke`.
- `[out]` parameters when the server fails after writing them, `RPC_E_*` for a server that dies in the middle of a call, apartment
  edge cases (marshalling between a free-threaded and a single-threaded apartment of one process, `CoDisconnectObject`).
- Bitness: a 32-bit client and a 64-bit server (the updater 157 is x86): needs 32-bit test images (B2d).

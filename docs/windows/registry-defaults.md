# Default registry contents of a Windows machine (names and values only)

Written for seeding Poly's default `SOFTWARE` hive (`crates/poly-registry/src/defaults.rs`): what a real Windows 11 machine holds in the 64-bit and the
32-bit view of the registry before any program has made a key, for the parts that installers and COM touch. It comes from the spec job
`spec-win-wow64-defaults` (spec sheets `specs/windows/advapi32.dll/RegistryDefaults.*.yaml`, `specs/windows/ole32.dll/ComDefaults.*.yaml`,
`specs/windows/oleaut32.dll/ComDefaults.StdOleTypeLib.yaml`, each with recorded tests). Every line was read on the oracle machine by test programs
(64-bit and 32-bit builds; the values below are identical in both builds' views where it says so). Nothing here is copied from Microsoft files or from
another project: only names and the small values the behaviour needs. The oracle is an installed machine, so vendor keys (Google, Intel, Python, Git,
ODBC, ...) are left out; only keys Windows itself makes are listed.

## 1. The two views in one paragraph

The registry is stored once (the *physical* registry, which is what a 64-bit process sees). A 32-bit process sees a second view of `HKLM\SOFTWARE`: its `SOFTWARE`
is the physical `SOFTWARE\Wow6432Node`, and below `SOFTWARE\Classes` the keys `CLSID` and `Interface` are the physical `Classes\Wow6432Node\CLSID` and
`Classes\Wow6432Node\Interface`; `TypeLib` and `AppID` are shared. How a *create* or *open* picks the view (per handle, per access bit) is in the sheets of
`RegCreateKeyExW` (observed by `spec-win-wow64-registry`); this file lists what is *there*. To seed a default hive, write each key below into the
**physical** place given in section 6.

## 2. `HKLM\SOFTWARE` and `Wow6432Node`

Physical keys that exist on a fresh machine (64-bit view paths):

| Key | Default children (names) |
|---|---|
| `SOFTWARE` | `Classes`, `Clients`, `Microsoft`, `Policies`, `RegisteredApplications`, `Wow6432Node` |
| `SOFTWARE\Wow6432Node` | `Classes`, `Clients`, `Microsoft`, `Policies`, `RegisteredApplications` (no `Wow6432Node`) |
| `SOFTWARE\Wow6432Node\Microsoft` | `Windows`, `Windows NT`, `Ole`, `COM3`, `Rpc` (and many more) |
| `SOFTWARE\Wow6432Node\Microsoft\Windows\CurrentVersion` | `Uninstall`, `App Paths`, `Run`, `RunOnce`, `Policies`, `Explorer`, `Installer`, `Setup`, `Internet Settings` |
| `SOFTWARE\Wow6432Node\Microsoft\Windows NT\CurrentVersion` | (values only, section 4) |
| `SOFTWARE\Classes\Wow6432Node` | `CLSID`, `Interface`, `TypeLib`, `AppID`, `DirectShow`, `Media Type`, `MediaFoundation` |
| `SOFTWARE\Microsoft` | `Windows`, `Windows NT`, `Ole`, `COM3`, `Rpc` (64-bit side; a different key from the Wow6432Node one) |

`SOFTWARE\Wow6432Node\Wow6432Node` and `SOFTWARE\Classes\Wow6432Node\Wow6432Node` do not exist in the 64-bit view.

What is one key (64-bit view): `SOFTWARE\Wow6432Node\Classes\X` and `SOFTWARE\Classes\Wow6432Node\X` for every `X`; `Classes\Wow6432Node\TypeLib` is
`Classes\TypeLib` and `Classes\Wow6432Node\AppID` is `Classes\AppID`. In the 32-bit view, `Classes\Wow6432Node\X` and `Wow6432Node\Classes\X` name `Classes\X`
(as that view sees it); a key stored at the physical `Classes\Wow6432Node\Y` for a name other than `CLSID`, `Interface`, `TypeLib` and `AppID` is not visible there.

## 3. The Classes roots

`HKLM\SOFTWARE\Classes` (and `HKEY_CLASSES_ROOT`, its merge with the user's classes) has, in both views, at least: `CLSID`, `Interface`, `TypeLib`, `AppID`,
`Wow6432Node`, `Installer`, `MIME`, `Local Settings`, `Media Type`, `MediaFoundation`, `DirectShow`, `Drive`, `Directory`, `Folder`, `*`, `exefile`, `dllfile`,
`.exe`, `.dll`, `.tlb`, `.lnk`, `http`, `https`, `SystemFileAssociations`.

A per-user `HKCU\Software\Classes` follows the machine rules: `CLSID` and `Interface` are redirected for a 32-bit process (to `Classes\Wow6432Node\CLSID` and
`Classes\Wow6432Node\Interface` physically), `TypeLib` and `AppID` are shared; `HKCU\Software\X` is not redirected at all.

## 4. `CurrentVersion`

`HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion`: the 64-bit view and the 32-bit view (physically `SOFTWARE\Wow6432Node\Microsoft\Windows\CurrentVersion`) differ:

| Value | 64-bit view | 32-bit view |
|---|---|---|
| `ProgramFilesDir` (REG_SZ) | `C:\Program Files` | `C:\Program Files (x86)` |
| `ProgramFilesDir (x86)` (REG_SZ) | `C:\Program Files (x86)` | `C:\Program Files (x86)` |
| `ProgramW6432Dir` (REG_SZ) | `C:\Program Files` | `C:\Program Files` |
| `CommonFilesDir` (REG_SZ) | `C:\Program Files\Common Files` | `C:\Program Files (x86)\Common Files` |
| `CommonFilesDir (x86)` (REG_SZ) | `C:\Program Files (x86)\Common Files` | `C:\Program Files (x86)\Common Files` |
| `CommonW6432Dir` (REG_SZ) | `C:\Program Files\Common Files` | `C:\Program Files\Common Files` |
| `ProgramFilesPath` (REG_EXPAND_SZ) | `%ProgramFiles%` | `%ProgramFiles%` |
| `DevicePath` (REG_EXPAND_SZ) | `%SystemRoot%\inf` | same |
| `MediaPathUnexpanded` (REG_EXPAND_SZ) | `%SystemRoot%\Media` | same |

`CurrentVersion\Installer`: 64-bit view `InstallerLocation` = `C:\Windows\System32\`, `MsiExecCA32` = `C:\Windows\syswow64\msiexec.exe`, `MsiExecCA64` = `C:\Windows\System32\msiexec.exe`;
32-bit view `InstallerLocation` = `C:\Windows\SysWOW64\` only.

`HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion` (both views, same values): `CurrentVersion` = `6.3`, `CurrentMajorVersionNumber` = 10 (REG_DWORD), `CurrentMinorVersionNumber` = 0,
`InstallationType` = `Client`; present and equal to each other: `CurrentBuild`/`CurrentBuildNumber`, `SystemRoot`/`PathName`; present: `UBR`, `DisplayVersion`, `ProductName`,
`EditionID`, `CompositionEditionID`, `BuildLab`, `BuildLabEx`, `InstallDate`, `ReleaseId`. Build and edition values are machine data; Poly keeps its own.

## 5. Standard COM registrations

### 5.1 `Classes\Interface\{iid}`: 82 keys (identical in both views)

Each key's default value is the interface name. Subkeys listed are `ProxyStubClsid32` (default value = the proxy/stub class) and the others where present.

| IID | Name | ProxyStubClsid32 | Other subkeys |
|---|---|---|---|
| `{00000000-0000-0000-C000-000000000046}` | IUnknown | - | - |
| `{00000001-0000-0000-C000-000000000046}` | IClassFactory | `{00000320-0000-0000-C000-000000000046}` | - |
| `{0000000A-0000-0000-C000-000000000046}` | ILockBytes | `{00000320-0000-0000-C000-000000000046}` | - |
| `{0000000B-0000-0000-C000-000000000046}` | IStorage | `{00000320-0000-0000-C000-000000000046}` | - |
| `{0000000C-0000-0000-C000-000000000046}` | IStream | `{00000320-0000-0000-C000-000000000046}` | - |
| `{0000000D-0000-0000-C000-000000000046}` | IEnumSTATSTG | `{00000320-0000-0000-C000-000000000046}` | - |
| `{0000000E-0000-0000-C000-000000000046}` | IBindCtx | `{00000320-0000-0000-C000-000000000046}` | - |
| `{0000000F-0000-0000-C000-000000000046}` | IMoniker | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000010-0000-0000-C000-000000000046}` | IRunningObjectTable | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000012-0000-0000-C000-000000000046}` | IRootStorage | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000022-0000-0000-C000-000000000046}` | ISurrogate | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000026-0000-0000-C000-000000000046}` | IUrlMon | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000030-0000-0000-C000-000000000046}` | ISynchronize | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000035-0000-0000-C000-000000000046}` | IActivationFactory | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000036-0000-0000-C000-000000000046}` | IAsyncInfo | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000037-0000-0000-C000-000000000046}` | IWeakReference | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000038-0000-0000-C000-000000000046}` | IWeakReferenceSource | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000100-0000-0000-C000-000000000046}` | IEnumUnknown | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000101-0000-0000-C000-000000000046}` | IEnumString | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000102-0000-0000-C000-000000000046}` | IEnumMoniker | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000103-0000-0000-C000-000000000046}` | IEnumFORMATETC | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000104-0000-0000-C000-000000000046}` | IEnumOLEVERB | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000105-0000-0000-C000-000000000046}` | IEnumSTATDATA | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000109-0000-0000-C000-000000000046}` | IPersistStream | `{00000320-0000-0000-C000-000000000046}` | - |
| `{0000010A-0000-0000-C000-000000000046}` | IPersistStorage | `{00000320-0000-0000-C000-000000000046}` | - |
| `{0000010B-0000-0000-C000-000000000046}` | IPersistFile | `{00000320-0000-0000-C000-000000000046}` | - |
| `{0000010C-0000-0000-C000-000000000046}` | IPersist | `{00000320-0000-0000-C000-000000000046}` | - |
| `{0000010D-0000-0000-C000-000000000046}` | IViewObject | `{00000320-0000-0000-C000-000000000046}` | - |
| `{0000010E-0000-0000-C000-000000000046}` | IDataObject | `{00000320-0000-0000-C000-000000000046}` | - |
| `{0000010F-0000-0000-C000-000000000046}` | IAdviseSink | `{00000320-0000-0000-C000-000000000046}` | AsynchronousInterface = `{00000150-0000-0000-C000-000000000046}` |
| `{00000112-0000-0000-C000-000000000046}` | IOleObject | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000113-0000-0000-C000-000000000046}` | IOleInPlaceObject | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000114-0000-0000-C000-000000000046}` | IOleWindow | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000115-0000-0000-C000-000000000046}` | IOleInPlaceUIWindow | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000116-0000-0000-C000-000000000046}` | IOleInPlaceFrame | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000117-0000-0000-C000-000000000046}` | IOleInPlaceActiveObject | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000118-0000-0000-C000-000000000046}` | IOleClientSite | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000119-0000-0000-C000-000000000046}` | IOleInPlaceSite | `{00000320-0000-0000-C000-000000000046}` | - |
| `{0000011A-0000-0000-C000-000000000046}` | IParseDisplayName | `{00000320-0000-0000-C000-000000000046}` | - |
| `{0000011B-0000-0000-C000-000000000046}` | IOleContainer | `{00000320-0000-0000-C000-000000000046}` | - |
| `{0000011C-0000-0000-C000-000000000046}` | IOleItemContainer | `{00000320-0000-0000-C000-000000000046}` | - |
| `{0000011D-0000-0000-C000-000000000046}` | IOleLink | `{00000320-0000-0000-C000-000000000046}` | - |
| `{0000011E-0000-0000-C000-000000000046}` | IOleCache | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000122-0000-0000-C000-000000000046}` | IDropTarget | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000125-0000-0000-C000-000000000046}` | IAdviseSink2 | `{00000320-0000-0000-C000-000000000046}` | AsynchronousInterface = `{00000151-0000-0000-C000-000000000046}` |
| `{00000126-0000-0000-C000-000000000046}` | IRunnableObject | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000127-0000-0000-C000-000000000046}` | IViewObject2 | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000128-0000-0000-C000-000000000046}` | IOleCache2 | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000129-0000-0000-C000-000000000046}` | IOleCacheControl | `{00000320-0000-0000-C000-000000000046}` | - |
| `{0000012A-0000-0000-C000-000000000046}` | IContinue | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000131-0000-0000-C000-000000000046}` | IRemUnknown | `{00000320-0000-0000-C000-000000000046}` | AsynchronousInterface = `{000E0131-0000-0000-C000-000000000046}` |
| `{00000132-0000-0000-C000-000000000046}` | ILocalSystemActivator | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000134-0000-0000-C000-000000000046}` | IRundown | `{00000320-0000-0000-C000-000000000046}` | AsynchronousInterface = `{000E0134-0000-0000-C000-000000000046}` |
| `{00000135-0000-0000-C000-000000000046}` | IInterfaceFromWindowProp | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000136-0000-0000-C000-000000000046}` | ISCMLocalActivator | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000138-0000-0000-C000-000000000046}` | IPropertyStorage | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000139-0000-0000-C000-000000000046}` | IEnumSTATPROPSTG | `{00000320-0000-0000-C000-000000000046}` | - |
| `{0000013A-0000-0000-C000-000000000046}` | IPropertySetStorage | `{00000320-0000-0000-C000-000000000046}` | - |
| `{0000013B-0000-0000-C000-000000000046}` | IEnumSTATPROPSETSTG | `{00000320-0000-0000-C000-000000000046}` | - |
| `{0000013C-0000-0000-C000-000000000046}` | IRemUnknownN | `{00000320-0000-0000-C000-000000000046}` | AsynchronousInterface = `{000B013C-0000-0000-C000-000000000046}` |
| `{00000140-0000-0000-C000-000000000046}` | IClassActivator | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000141-0000-0000-C000-000000000046}` | IDLLHost | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000142-0000-0000-C000-000000000046}` | IRemoteQI | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00000143-0000-0000-C000-000000000046}` | IRemUnknown2 | `{00000320-0000-0000-C000-000000000046}` | AsynchronousInterface = `{000E0143-0000-0000-C000-000000000046}` |
| `{00000150-0000-0000-C000-000000000046}` | AsyncIAdviseSink | - | SynchronousInterface = `{0000010F-0000-0000-C000-000000000046}` |
| `{00000151-0000-0000-C000-000000000046}` | AsyncIAdviseSink2 | - | SynchronousInterface = `{00000125-0000-0000-C000-000000000046}` |
| `{00000160-0000-0000-C000-000000000046}` | IPrivDragDrop | `{00000320-0000-0000-C000-000000000046}` | - |
| `{000001A0-0000-0000-C000-000000000046}` | ISystemActivator | `{00000320-0000-0000-C000-000000000046}` | - |
| `{00020400-0000-0000-C000-000000000046}` | IDispatch | `{00020420-0000-0000-C000-000000000046}` | ProxyStubClsid = `{00020420-0000-0000-C000-000000000046}`, NumMethods = `7` |
| `{00020401-0000-0000-C000-000000000046}` | ITypeInfo | `{00020422-0000-0000-C000-000000000046}` | ProxyStubClsid = `{00020422-0000-0000-C000-000000000046}`, NumMethods = `22` |
| `{00020402-0000-0000-C000-000000000046}` | ITypeLib | `{00020423-0000-0000-C000-000000000046}` | ProxyStubClsid = `{00020423-0000-0000-C000-000000000046}`, NumMethods = `13` |
| `{00020403-0000-0000-C000-000000000046}` | ITypeComp | `{00020425-0000-0000-C000-000000000046}` | ProxyStubClsid = `{00020425-0000-0000-C000-000000000046}`, NumMethods = `5` |
| `{00020404-0000-0000-C000-000000000046}` | IEnumVARIANT | `{00020421-0000-0000-C000-000000000046}` | ProxyStubClsid = `{00020421-0000-0000-C000-000000000046}`, NumMethods = `7` |
| `{00020411-0000-0000-C000-000000000046}` | ITypeLib2 | `{00020420-0000-0000-C000-000000000046}` | NumMethods = `16` |
| `{00020412-0000-0000-C000-000000000046}` | ITypeInfo2 | `{00020420-0000-0000-C000-000000000046}` | NumMethods = `32` |
| `{1CF2B120-547D-101B-8E65-08002B2BD119}` | IErrorInfo | `{B196B286-BAB4-101A-B69C-00AA00341D07}` | - |
| `{22F03340-547D-101B-8E65-08002B2BD119}` | ICreateErrorInfo | `{B196B286-BAB4-101A-B69C-00AA00341D07}` | NumMethods = `8` |
| `{DF0B3D60-548F-101B-8E65-08002B2BD119}` | ISupportErrorInfo | `{DF0B3D60-548F-101B-8E65-08002B2BD119}` | ProxyStubClsid = `{DF0B3D60-548F-101B-8E65-08002B2BD119}` |
| `{A6EF9860-C720-11D0-9337-00A0C90DCAA9}` | IDispatchEx | `{10E2414A-EC59-49D2-BC51-5ADD2C36FEBC}` | NumMethods = `15` |
| `{B196B286-BAB4-101A-B69C-00AA00341D07}` | IConnectionPoint | `{00000320-0000-0000-C000-000000000046}` | - |
| `{B196B287-BAB4-101A-B69C-00AA00341D07}` | IEnumConnections | `{00000320-0000-0000-C000-000000000046}` | - |
| `{6D5140C1-7436-11CE-8034-00AA006009FA}` | IServiceProvider | `{A6FF50C0-56C0-71CA-5732-BED303A59628}` | - |

### 5.2 `Classes\CLSID\{clsid}`: 27 classes

Default value = the name; the `InprocServer32` key's default value is the server, `ThreadingModel` where shown. Where the two views differ, only the absolute paths do (`System32` in the
64-bit view, `SysWOW64` in the 32-bit view).

| CLSID | Name | InprocServer32 (64-bit view) | InprocServer32 (32-bit view) |
|---|---|---|---|
| `{0000002F-0000-0000-C000-000000000046}` | CLSID_RecordInfo | default = `C:\Windows\System32\oleaut32.dll`; ThreadingModel = `Both` | default = `C:\Windows\SysWOW64\oleaut32.dll`; ThreadingModel = `Both` |
| `{00000300-0000-0000-C000-000000000046}` | StdOleLink | default = `combase.dll` | same |
| `{00000303-0000-0000-C000-000000000046}` | FileMoniker | default = `combase.dll` | same |
| `{00000304-0000-0000-C000-000000000046}` | ItemMoniker | default = `combase.dll` | same |
| `{00000305-0000-0000-C000-000000000046}` | AntiMoniker | default = `combase.dll` | same |
| `{00000306-0000-0000-C000-000000000046}` | PointerMoniker | default = `combase.dll` | same |
| `{00000308-0000-0000-C000-000000000046}` | PackagerMoniker | default = `combase.dll` | same |
| `{00000309-0000-0000-C000-000000000046}` | CompositeMoniker | default = `combase.dll` | same |
| `{0000030B-0000-0000-C000-000000000046}` | DfMarshal | default = `coml2.dll` | same |
| `{00000315-0000-0000-C000-000000000046}` | Picture (Metafile) | default = `combase.dll` | same |
| `{00000316-0000-0000-C000-000000000046}` | Picture (Device Independent Bitmap) | default = `combase.dll` | same |
| `{00000319-0000-0000-C000-000000000046}` | Picture (Enhanced Metafile) | default = `combase.dll` | same |
| `{0000031A-0000-0000-C000-000000000046}` | ClassMoniker | default = `combase.dll` | same |
| `{0000031D-0000-0000-C000-000000000046}` | DCOMAccessControl | default = `combase.dll` | same |
| `{00000320-0000-0000-C000-000000000046}` | PSFactoryBuffer | default = `combase.dll`; ThreadingModel = `Both` | same |
| `{00000327-0000-0000-C000-000000000046}` | objref | default = `combase.dll`; ThreadingModel = `Both` | same |
| `{0000032E-0000-0000-C000-000000000046}` | PipePSFactory | default = `combase.dll` | same |
| `{00000355-0000-0000-C000-000000000046}` | MetadataPSFactory | default = `C:\Windows\System32\combase.dll`; ThreadingModel = `Both` | default = `C:\Windows\SysWOW64\combase.dll`; ThreadingModel = `Both` |
| `{00020420-0000-0000-C000-000000000046}` | PSDispatch | default = `C:\Windows\System32\oleaut32.dll`; ThreadingModel = `Both` | default = `C:\Windows\SysWOW64\oleaut32.dll`; ThreadingModel = `Both` |
| `{00020421-0000-0000-C000-000000000046}` | PSEnumVariant | default = `C:\Windows\System32\oleaut32.dll`; ThreadingModel = `Both` | default = `C:\Windows\SysWOW64\oleaut32.dll`; ThreadingModel = `Both` |
| `{00020422-0000-0000-C000-000000000046}` | PSTypeInfo | default = `C:\Windows\System32\oleaut32.dll`; ThreadingModel = `Both` | default = `C:\Windows\SysWOW64\oleaut32.dll`; ThreadingModel = `Both` |
| `{00020423-0000-0000-C000-000000000046}` | PSTypeLib | default = `C:\Windows\System32\oleaut32.dll`; ThreadingModel = `Both` | default = `C:\Windows\SysWOW64\oleaut32.dll`; ThreadingModel = `Both` |
| `{00020424-0000-0000-C000-000000000046}` | PSOAInterface | default = `C:\Windows\System32\oleaut32.dll`; ThreadingModel = `Both` | default = `C:\Windows\SysWOW64\oleaut32.dll`; ThreadingModel = `Both` |
| `{00020425-0000-0000-C000-000000000046}` | PSTypeComp | default = `C:\Windows\System32\oleaut32.dll`; ThreadingModel = `Both` | default = `C:\Windows\SysWOW64\oleaut32.dll`; ThreadingModel = `Both` |
| `{DF0B3D60-548F-101B-8E65-08002B2BD119}` | PSSupportErrorInfo | default = `C:\Windows\System32\oleaut32.dll` | default = `C:\Windows\SysWOW64\oleaut32.dll` |
| `{B196B286-BAB4-101A-B69C-00AA00341D07}` | PSFactoryBuffer | default = `C:\Windows\System32\oleaut32.dll`; ThreadingModel = `Both` | default = `C:\Windows\SysWOW64\oleaut32.dll`; ThreadingModel = `Both` |
| `{4590F811-1D3A-11D0-891F-00AA004B2E24}` | WBEM Locator | ThreadingModel = `Both` | same |

### 5.3 `Classes\TypeLib\{00020430-0000-0000-C000-000000000046}` (stdole), one key for both views

| Key | Default value / values |
|---|---|
| `...\2.0` | `OLE Automation`; `PrimaryInteropAssemblyName` = a .NET assembly name |
| `...\2.0\0\win32` | `C:\Windows\SysWOW64\stdole2.tlb` |
| `...\2.0\0\win64` | `C:\Windows\System32\stdole2.tlb` |
| `...\2.0\FLAGS` | `0` |
| `...\2.0\HELPDIR` | `C:\Windows\System32` |
| `...\1.0` | `OLE Automation` (children `0`, `FLAGS`, `HELPDIR`) |
| `...\1.0\0\win32` | `C:\Windows\System32\stdole32.tlb` (no `win64`) |

## 6. Seeding Poly's hive: where each key goes

Poly's `SOFTWARE_KEYS` today holds `Classes\Interface`, `Classes\CLSID` and `Classes\TypeLib` entries once, which is the 64-bit view only (`Wow6432Node` is an empty key). To match the table above:

1. Make the empty keys `Wow6432Node\{Classes, Clients, Microsoft, Policies, RegisteredApplications}` and `Classes\Wow6432Node\{CLSID, Interface, TypeLib, AppID, DirectShow, "Media Type", MediaFoundation}`, and
   `Wow6432Node\Classes` as the same key as `Classes\Wow6432Node` (a second name for it: a key made at either path is found at the other).
2. Write every `Classes\Interface\{iid}` entry of 5.1 **twice**: at `Classes\Interface\{iid}` and at `Classes\Wow6432Node\Interface\{iid}` (same values).
3. Write every `Classes\CLSID\{clsid}` entry of 5.2 twice: at `Classes\CLSID\{clsid}` with the 64-bit server paths and at `Classes\Wow6432Node\CLSID\{clsid}` with the 32-bit server paths
   (`SysWOW64` where an absolute `System32` path stands).
4. Write the stdole type library once, at `Classes\TypeLib\...` (shared), with both `win32` and `win64` paths.
5. Write `Microsoft\Windows\CurrentVersion` twice: at `Microsoft\Windows\CurrentVersion` (64-bit values) and at `Wow6432Node\Microsoft\Windows\CurrentVersion` (32-bit values, the (x86) folders);
   and `Microsoft\Windows NT\CurrentVersion` twice with the same values. Also `Wow6432Node\Microsoft\Windows\CurrentVersion\{Uninstall, App Paths, Run, RunOnce}` as empty keys.
6. Poly's existing short names (`oleaut32.dll` for the proxy/stub servers) work for loading; the oracle stores the absolute path for the `oleaut32` classes and the bare `combase.dll` for the classes of `combase`.

The aliases of section 2 are what the B7 scout needed in `personalities/nt/src/registry/wow64.rs` (a 32-bit view's `Wow6432Node\Classes\X` is that view's `Classes\X`).

## 7. How this was observed

Test programs `wd_*` (see the sheets) on the oracle (`homeserver2`, Windows 11 26H2, build 26300.9974): 64-bit builds asked for `KEY_WOW64_64KEY` and `KEY_WOW64_32KEY`; 32-bit builds asked for their own default view.
The 32-bit default view and the 64-bit build's `KEY_WOW64_32KEY` view gave identical output, line for line.


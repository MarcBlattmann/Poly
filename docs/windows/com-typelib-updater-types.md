# The type libraries of the updater, as `ITypeLib` / `ITypeInfo` report them

Observation for [com-typelib-plan.md](com-typelib-plan.md): `updater.exe` 159.0.8140.0 (the survey's copy, put in a scratch
directory of the oracle, homeserver2; the program was not run: its resources were opened with `LoadTypeLibEx("updater.exe\N")`
and read through the public interfaces of `ITypeLib` and `ITypeInfo`). Resources 1 to 6 hold libraries, 7 to 16 do not exist
(`0x80029c4a`). Resources 4, 5 and 6 are listed in full. Notation: `in`/`out`/`retval` are the parameter flags;
`PTR>X` is a pointer to X; `vtNN` is the VARTYPE number (31 = `LPWSTR`, 20 = `I8`, 23 = `UINT`, 19 = `ULONG`, 24 = `VOID`);
`D:` marks a function of a dual interface (kind 4) that is dispatched; `invN` is the invoke kind (1 = function,
2 = property get); `id` is the member id. Kinds: 3 interface, 4 dispatch (the dual form, with the `IUnknown` and `IDispatch` methods
listed first), 5 coclass, 6 alias.

```
resource 1: library UpdaterLib version 1.0 with 11 type infos
    UpdaterUserClass(5), UpdaterSystemClass(5), IUpdateStateUser(3), ICompleteStatusUser(3), IUpdaterObserverUser(3), IUpdaterCallbackUser(3), IUpdaterAppState(4), IUpdaterAppStateUser(4), IUpdaterAppStatesCallbackUser(3), IUpdaterUser(3), IUpdater2User(3)
resource 2: library UpdaterInternalLib version 1.0 with 4 type infos
    UpdaterInternalUserClass(5), UpdaterInternalSystemClass(5), IUpdaterInternalCallbackUser(3), IUpdaterInternalUser(3)
resource 3: library UpdaterLegacyLib version 1.0 with 29 type infos
    GoogleUpdate3WebUserClass(5), GoogleUpdate3WebSystemClass(5), GoogleUpdate3WebServiceClass(5), PolicyStatusUserClass(5), PolicyStatusSystemClass(5), ProcessLauncherClass(5), IAppVersionWeb(4), IAppVersionWebUser(4), ICurrentState(4), ICurrentStateUser(4), IGoogleUpdate3Web(4), IGoogleUpdate3WebUser(4), IAppBundleWeb(4), ULONG_PTR(6), IAppBundleWebUser(4), IAppWeb(4), IAppWebUser(4), IAppCommandWeb(4), IAppCommandWebUser(4), IPolicyStatus(4), IPolicyStatusUser(4), IPolicyStatusValue(4), IPolicySt
resource 4: library UpdaterLib version 1.0 with 11 type infos
    UpdaterUserClass(5), UpdaterSystemClass(5), IUpdateStateSystem(3), ICompleteStatusSystem(3), IUpdaterObserverSystem(3), IUpdaterCallbackSystem(3), IUpdaterAppState(4), IUpdaterAppStateSystem(4), IUpdaterAppStatesCallbackSystem(3), IUpdaterSystem(3), IUpdater2System(3)
resource 5: library UpdaterInternalLib version 1.0 with 4 type infos
    UpdaterInternalUserClass(5), UpdaterInternalSystemClass(5), IUpdaterInternalCallbackSystem(3), IUpdaterInternalSystem(3)
resource 6: library UpdaterLegacyLib version 1.0 with 33 type infos
    GoogleUpdate3WebUserClass(5), GoogleUpdate3WebSystemClass(5), GoogleUpdate3WebServiceClass(5), PolicyStatusUserClass(5), PolicyStatusSystemClass(5), ProcessLauncherClass(5), IAppVersionWeb(4), IAppVersionWebSystem(4), ICurrentState(4), ICurrentStateSystem(4), IGoogleUpdate3Web(4), IGoogleUpdate3WebSystem(4), IAppBundleWeb(4), ULONG_PTR(6), IAppBundleWebSystem(4), IAppWeb(4), IAppWebSystem(4), IAppCommandWeb(4), IAppCommandWebSystem(4), IPolicyStatus(4), IPolicyStatusSystem(4), IPolicyStatusValue
resource 7: 0x80029c4a
resource 8: 0x80029c4a
resource 9: 0x80029c4a
resource 10: 0x80029c4a
resource 11: 0x80029c4a
resource 12: 0x80029c4a
resource 13: 0x80029c4a
resource 14: 0x80029c4a
resource 15: 0x80029c4a
resource 16: 0x80029c4a
LoadTypeLibEx(updater.exe\4) = 0x00000000
  lib attr (the library id is not printed), lcid 0, syskind 3, version 1.0, flags 0x8
  lib name UpdaterLib doc Chromium Updater type library
  type infos: 11
  [0] UpdaterUserClass: kind 5, flags 0x2, funcs 0, impl types 1, vft size 0, version 0.0
  [1] UpdaterSystemClass: kind 5, flags 0x2, funcs 0, impl types 1, vft size 0, version 0.0
  [2] IUpdateStateSystem: kind 3, flags 0x100, funcs 11, impl types 1, vft size 112, version 0.0
  [3] ICompleteStatusSystem: kind 3, flags 0x100, funcs 2, impl types 1, vft size 40, version 0.0
  [4] IUpdaterObserverSystem: kind 3, flags 0x100, funcs 2, impl types 1, vft size 40, version 0.0
  [5] IUpdaterCallbackSystem: kind 3, flags 0x100, funcs 1, impl types 1, vft size 32, version 0.0
  [6] IUpdaterAppState: kind 4, flags 0x1040, funcs 13, impl types 1, vft size 56, version 0.0
  [7] IUpdaterAppStateSystem: kind 4, flags 0x1040, funcs 13, impl types 1, vft size 56, version 0.0
  [8] IUpdaterAppStatesCallbackSystem: kind 3, flags 0x100, funcs 1, impl types 1, vft size 32, version 0.0
  [9] IUpdaterSystem: kind 3, flags 0x100, funcs 11, impl types 1, vft size 112, version 0.0
  [10] IUpdater2System: kind 3, flags 0x100, funcs 5, impl types 1, vft size 152, version 0.0
[0] UpdaterUserClass kind 5 flags 0x2 funcs 0 impl 1 id {547E9AEF-8043-5D26-879F-01E7664192DC}
[1] UpdaterSystemClass kind 5 flags 0x2 funcs 0 impl 1 id {8018F647-BF07-55BB-82BE-A2D7049F7CE4}
[2] IUpdateStateSystem kind 3 flags 0x100 funcs 11 impl 1 id {1588C1A8-27D9-563E-9641-8D20767FB258}
    state(out retval PTR>I4) inv2 id0x60010000
    appId(out retval PTR>BSTR) inv2 id0x60010001
    nextVersion(out retval PTR>BSTR) inv2 id0x60010002
    downloadedBytes(out retval PTR>vt20) inv2 id0x60010003
    totalBytes(out retval PTR>vt20) inv2 id0x60010004
    installProgress(out retval PTR>I4) inv2 id0x60010005
    errorCategory(out retval PTR>I4) inv2 id0x60010006
    errorCode(out retval PTR>I4) inv2 id0x60010007
    extraCode1(out retval PTR>I4) inv2 id0x60010008
    installerText(out retval PTR>BSTR) inv2 id0x60010009
    installerCommandLine(out retval PTR>BSTR) inv2 id0x6001000a
[3] ICompleteStatusSystem kind 3 flags 0x100 funcs 2 impl 1 id {85AE4AE3-8530-516B-8BE4-A456BF2637D3}
    statusCode(out retval PTR>I4) inv2 id0x60010000
    statusMessage(out retval PTR>BSTR) inv2 id0x60010001
[4] IUpdaterObserverSystem kind 3 flags 0x100 funcs 2 impl 1 id {B16B5A0E-3B72-5223-8DF0-9117CD64DE77}
    OnStateChange(in PTR>USERDEFINED(IUpdateStateSystem)) inv1 id0x60010000
    OnComplete(in PTR>USERDEFINED(ICompleteStatusSystem)) inv1 id0x60010001
[5] IUpdaterCallbackSystem kind 3 flags 0x100 funcs 1 impl 1 id {0486745C-8D9B-5377-A54C-A61FFAA0BBE4}
    Run(in I4) inv1 id0x60010000
[6] IUpdaterAppState kind 4 flags 0x1040 funcs 13 impl 1 id {F3B7B6A3-0E48-5730-9012-AC985F2D6B37}
    D:QueryInterface(in PTR>USERDEFINED(GUID), out PTR>PTR>vt24) inv1 id0x60000000
    D:AddRef() inv1 id0x60000001
    D:Release() inv1 id0x60000002
    D:GetTypeInfoCount(out PTR>vt23) inv1 id0x60010000
    D:GetTypeInfo(in vt23, in vt19, out PTR>PTR>vt24) inv1 id0x60010001
    D:GetIDsOfNames(in PTR>USERDEFINED(GUID), in PTR>PTR>vt16, in vt23, in vt19, out PTR>I4) inv1 id0x60010002
    D:Invoke(in I4, in PTR>USERDEFINED(GUID), in vt19, in vt18, in PTR>USERDEFINED(DISPPARAMS), out PTR>VARIANT, out PTR>USERDEFINED(EXCEPINFO), out PTR>vt23) inv1 id0x60010003
    D:appId() inv2 id0x60020000
    D:version() inv2 id0x60020001
    D:ap() inv2 id0x60020002
    D:brandCode() inv2 id0x60020003
    D:brandPath() inv2 id0x60020004
    D:ecp() inv2 id0x60020005
[7] IUpdaterAppStateSystem kind 4 flags 0x1040 funcs 13 impl 1 id {B7FD5390-D593-5A8B-9AE2-23CE39822FD4}
    D:QueryInterface(in PTR>USERDEFINED(GUID), out PTR>PTR>vt24) inv1 id0x60000000
    D:AddRef() inv1 id0x60000001
    D:Release() inv1 id0x60000002
    D:GetTypeInfoCount(out PTR>vt23) inv1 id0x60010000
    D:GetTypeInfo(in vt23, in vt19, out PTR>PTR>vt24) inv1 id0x60010001
    D:GetIDsOfNames(in PTR>USERDEFINED(GUID), in PTR>PTR>vt16, in vt23, in vt19, out PTR>I4) inv1 id0x60010002
    D:Invoke(in I4, in PTR>USERDEFINED(GUID), in vt19, in vt18, in PTR>USERDEFINED(DISPPARAMS), out PTR>VARIANT, out PTR>USERDEFINED(EXCEPINFO), out PTR>vt23) inv1 id0x60010003
    D:appId() inv2 id0x60020000
    D:version() inv2 id0x60020001
    D:ap() inv2 id0x60020002
    D:brandCode() inv2 id0x60020003
    D:brandPath() inv2 id0x60020004
    D:ecp() inv2 id0x60020005
[8] IUpdaterAppStatesCallbackSystem kind 3 flags 0x100 funcs 1 impl 1 id {699F07AD-304C-5F71-A2DA-ABD765965B54}
    Run(in VARIANT) inv1 id0x60010000
[9] IUpdaterSystem kind 3 flags 0x100 funcs 11 impl 1 id {ACAB122B-29C0-56A9-8145-AFA2F82A547C}
    GetVersion(out retval PTR>BSTR) inv1 id0x60010000
    FetchPolicies(in PTR>USERDEFINED(IUpdaterCallbackSystem)) inv1 id0x60010001
    RegisterApp(in vt31, in vt31, in vt31, in vt31, in vt31, in vt31, in PTR>USERDEFINED(IUpdaterCallbackSystem)) inv1 id0x60010002
    RunPeriodicTasks(in PTR>USERDEFINED(IUpdaterCallbackSystem)) inv1 id0x60010003
    CheckForUpdate(in vt31, in I4, in I4, in PTR>USERDEFINED(IUpdaterObserverSystem)) inv1 id0x60010004
    Update(in vt31, in vt31, in I4, in I4, in PTR>USERDEFINED(IUpdaterObserverSystem)) inv1 id0x60010005
    UpdateAll(in PTR>USERDEFINED(IUpdaterObserverSystem)) inv1 id0x60010006
    Install(in vt31, in vt31, in vt31, in vt31, in vt31, in vt31, in vt31, in vt31, in I4, in PTR>USERDEFINED(IUpdaterObserverSystem)) inv1 id0x60010007
    CancelInstalls(in vt31) inv1 id0x60010008
    RunInstaller(in vt31, in vt31, in vt31, in vt31, in vt31, in PTR>USERDEFINED(IUpdaterObserverSystem)) inv1 id0x60010009
    GetAppStates(in PTR>USERDEFINED(IUpdaterAppStatesCallbackSystem)) inv1 id0x6001000a
[10] IUpdater2System kind 3 flags 0x100 funcs 5 impl 1 id {D39AC5FB-3662-521F-B4DA-149AA6CB515E}
    RegisterApp2(in vt31, in vt31, in vt31, in vt31, in vt31, in vt31, in vt31, in PTR>USERDEFINED(IUpdaterCallbackSystem)) inv1 id0x60020000
    CheckForUpdate2(in vt31, in I4, in I4, in vt31, in PTR>USERDEFINED(IUpdaterObserverSystem)) inv1 id0x60020001
    Update2(in vt31, in vt31, in I4, in I4, in vt31, in PTR>USERDEFINED(IUpdaterObserverSystem)) inv1 id0x60020002
    Install2(in vt31, in vt31, in vt31, in vt31, in vt31, in vt31, in vt31, in vt31, in vt31, in I4, in vt31, in PTR>USERDEFINED(IUpdaterObserverSystem)) inv1 id0x60020003
    RunInstaller2(in vt31, in vt31, in vt31, in vt31, in vt31, in vt31, in PTR>USERDEFINED(IUpdaterObserverSystem)) inv1 id0x60020004
LoadTypeLibEx(updater.exe\4) = 0x00000000
  lib attr (the library id is not printed), lcid 0, syskind 3, version 1.0, flags 0x8
  lib name UpdaterInternalLib doc Chromium Updater internal type library.
  type infos: 4
  [0] UpdaterInternalUserClass: kind 5, flags 0x2, funcs 0, impl types 1, vft size 0, version 0.0
  [1] UpdaterInternalSystemClass: kind 5, flags 0x2, funcs 0, impl types 1, vft size 0, version 0.0
  [2] IUpdaterInternalCallbackSystem: kind 3, flags 0x100, funcs 1, impl types 1, vft size 32, version 0.0
  [3] IUpdaterInternalSystem: kind 3, flags 0x100, funcs 2, impl types 1, vft size 40, version 0.0
[0] UpdaterInternalUserClass kind 5 flags 0x2 funcs 0 impl 1 id {D61B1035-2A21-5FF3-AFC1-D3974268E5C6}
[1] UpdaterInternalSystemClass kind 5 flags 0x2 funcs 0 impl 1 id {4B0E051D-1299-582C-BD97-9FDA419030B1}
[2] IUpdaterInternalCallbackSystem kind 3 flags 0x100 funcs 1 impl 1 id {6F09817C-2C44-54EC-96E9-340B4676E999}
    Run(in I4) inv1 id0x60010000
[3] IUpdaterInternalSystem kind 3 flags 0x100 funcs 2 impl 1 id {20BB83C0-F321-5C93-B4D6-B560CE1BC4F0}
    Run(in PTR>USERDEFINED(IUpdaterInternalCallbackSystem)) inv1 id0x60010000
    Hello(in PTR>USERDEFINED(IUpdaterInternalCallbackSystem)) inv1 id0x60010001
LoadTypeLibEx(updater.exe\4) = 0x00000000
  lib attr (the library id is not printed), lcid 0, syskind 3, version 1.0, flags 0x8
  lib name UpdaterLegacyLib doc Chromium Updater legacy type library.
  type infos: 33
  [0] GoogleUpdate3WebUserClass: kind 5, flags 0x2, funcs 0, impl types 1, vft size 0, version 0.0
  [1] GoogleUpdate3WebSystemClass: kind 5, flags 0x2, funcs 0, impl types 1, vft size 0, version 0.0
  [2] GoogleUpdate3WebServiceClass: kind 5, flags 0x2, funcs 0, impl types 1, vft size 0, version 0.0
  [3] PolicyStatusUserClass: kind 5, flags 0x2, funcs 0, impl types 1, vft size 0, version 0.0
  [4] PolicyStatusSystemClass: kind 5, flags 0x2, funcs 0, impl types 1, vft size 0, version 0.0
  [5] ProcessLauncherClass: kind 5, flags 0x2, funcs 0, impl types 1, vft size 0, version 0.0
  [6] IAppVersionWeb: kind 4, flags 0x1040, funcs 10, impl types 1, vft size 56, version 0.0
  [7] IAppVersionWebSystem: kind 4, flags 0x1040, funcs 10, impl types 1, vft size 56, version 0.0
  [8] ICurrentState: kind 4, flags 0x1040, funcs 24, impl types 1, vft size 56, version 0.0
  [9] ICurrentStateSystem: kind 4, flags 0x1040, funcs 24, impl types 1, vft size 56, version 0.0
  [10] IGoogleUpdate3Web: kind 4, flags 0x1040, funcs 8, impl types 1, vft size 56, version 0.0
  [11] IGoogleUpdate3WebSystem: kind 4, flags 0x1040, funcs 8, impl types 1, vft size 56, version 0.0
  [12] IAppBundleWeb: kind 4, flags 0x1040, funcs 24, impl types 1, vft size 56, version 0.0
  [13] ULONG_PTR: kind 6, flags 0x0, funcs 0, impl types 0, vft size 0, version 0.0
  [14] IAppBundleWebSystem: kind 4, flags 0x1040, funcs 24, impl types 1, vft size 56, version 0.0
  [15] IAppWeb: kind 4, flags 0x1040, funcs 17, impl types 1, vft size 56, version 0.0
  [16] IAppWebSystem: kind 4, flags 0x1040, funcs 17, impl types 1, vft size 56, version 0.0
  [17] IAppCommandWeb: kind 4, flags 0x1040, funcs 11, impl types 1, vft size 56, version 0.0
  [18] IAppCommandWebSystem: kind 4, flags 0x1040, funcs 11, impl types 1, vft size 56, version 0.0
  [19] IPolicyStatus: kind 4, flags 0x1040, funcs 16, impl types 1, vft size 56, version 0.0
  [20] IPolicyStatusSystem: kind 4, flags 0x1040, funcs 16, impl types 1, vft size 56, version 0.0
  [21] IPolicyStatusValue: kind 4, flags 0x1040, funcs 12, impl types 1, vft size 56, version 0.0
  [22] IPolicyStatusValueSystem: kind 4, flags 0x1040, funcs 12, impl types 1, vft size 56, version 0.0
  [23] IPolicyStatus2: kind 4, flags 0x1040, funcs 23, impl types 1, vft size 56, version 0.0
  [24] IPolicyStatus2System: kind 4, flags 0x1040, funcs 23, impl types 1, vft size 56, version 0.0
  [25] IPolicyStatus3: kind 4, flags 0x1040, funcs 24, impl types 1, vft size 56, version 0.0
  [26] IPolicyStatus3System: kind 4, flags 0x1040, funcs 24, impl types 1, vft size 56, version 0.0
  [27] IPolicyStatus4: kind 4, flags 0x1040, funcs 25, impl types 1, vft size 56, version 0.0
  [28] IPolicyStatus4System: kind 4, flags 0x1040, funcs 25, impl types 1, vft size 56, version 0.0
  [29] IProcessLauncher: kind 3, flags 0x100, funcs 3, impl types 1, vft size 48, version 0.0
  [30] IProcessLauncherSystem: kind 3, flags 0x100, funcs 3, impl types 1, vft size 48, version 0.0
  [31] IProcessLauncher2: kind 3, flags 0x100, funcs 1, impl types 1, vft size 56, version 0.0
  [32] IProcessLauncher2System: kind 3, flags 0x100, funcs 1, impl types 1, vft size 56, version 0.0
[0] GoogleUpdate3WebUserClass kind 5 flags 0x2 funcs 0 impl 1 id {22181302-A8A6-4F84-A541-E5CBFC70CC43}
[1] GoogleUpdate3WebSystemClass kind 5 flags 0x2 funcs 0 impl 1 id {8A1D4361-2C08-4700-A351-3EAA9CBFF5E4}
[2] GoogleUpdate3WebServiceClass kind 5 flags 0x2 funcs 0 impl 1 id {534F5323-3569-4F42-919D-1E1CF93E5BF6}
[3] PolicyStatusUserClass kind 5 flags 0x2 funcs 0 impl 1 id {6DDCE70D-A4AE-4E97-908C-BE7B2DB750AD}
[4] PolicyStatusSystemClass kind 5 flags 0x2 funcs 0 impl 1 id {521FDB42-7130-4806-822A-FC5163FAD983}
[5] ProcessLauncherClass kind 5 flags 0x2 funcs 0 impl 1 id {ABC01078-F197-4B0B-ADBC-CFE684B39C82}
[6] IAppVersionWeb kind 4 flags 0x1040 funcs 10 impl 1 id {0CD01D1E-4A1C-489D-93B9-9B6672877C57}
    D:QueryInterface(in PTR>USERDEFINED(GUID), out PTR>PTR>vt24) inv1 id0x60000000
    D:AddRef() inv1 id0x60000001
    D:Release() inv1 id0x60000002
    D:GetTypeInfoCount(out PTR>vt23) inv1 id0x60010000
    D:GetTypeInfo(in vt23, in vt19, out PTR>PTR>vt24) inv1 id0x60010001
    D:GetIDsOfNames(in PTR>USERDEFINED(GUID), in PTR>PTR>vt16, in vt23, in vt19, out PTR>I4) inv1 id0x60010002
    D:Invoke(in I4, in PTR>USERDEFINED(GUID), in vt19, in vt18, in PTR>USERDEFINED(DISPPARAMS), out PTR>VARIANT, out PTR>USERDEFINED(EXCEPINFO), out PTR>vt23) inv1 id0x60010003
    D:version() inv2 id0x60020000
    D:packageCount() inv2 id0x60020001
    D:packageWeb(in I4) inv2 id0x60020002
[7] IAppVersionWebSystem kind 4 flags 0x1040 funcs 10 impl 1 id {F258BE54-7C5F-44A0-AAE0-730620A31D23}
    D:QueryInterface(in PTR>USERDEFINED(GUID), out PTR>PTR>vt24) inv1 id0x60000000
    D:AddRef() inv1 id0x60000001
    D:Release() inv1 id0x60000002
    D:GetTypeInfoCount(out PTR>vt23) inv1 id0x60010000
    D:GetTypeInfo(in vt23, in vt19, out PTR>PTR>vt24) inv1 id0x60010001
    D:GetIDsOfNames(in PTR>USERDEFINED(GUID), in PTR>PTR>vt16, in vt23, in vt19, out PTR>I4) inv1 id0x60010002
    D:Invoke(in I4, in PTR>USERDEFINED(GUID), in vt19, in vt18, in PTR>USERDEFINED(DISPPARAMS), out PTR>VARIANT, out PTR>USERDEFINED(EXCEPINFO), out PTR>vt23) inv1 id0x60010003
    D:version() inv2 id0x60020000
    D:packageCount() inv2 id0x60020001
    D:packageWeb(in I4) inv2 id0x60020002
[8] ICurrentState kind 4 flags 0x1040 funcs 24 impl 1 id {247954F9-9EDC-4E68-8CC3-150C2B89EADF}
    D:QueryInterface(in PTR>USERDEFINED(GUID), out PTR>PTR>vt24) inv1 id0x60000000
    D:AddRef() inv1 id0x60000001
    D:Release() inv1 id0x60000002
    D:GetTypeInfoCount(out PTR>vt23) inv1 id0x60010000
    D:GetTypeInfo(in vt23, in vt19, out PTR>PTR>vt24) inv1 id0x60010001
    D:GetIDsOfNames(in PTR>USERDEFINED(GUID), in PTR>PTR>vt16, in vt23, in vt19, out PTR>I4) inv1 id0x60010002
    D:Invoke(in I4, in PTR>USERDEFINED(GUID), in vt19, in vt18, in PTR>USERDEFINED(DISPPARAMS), out PTR>VARIANT, out PTR>USERDEFINED(EXCEPINFO), out PTR>vt23) inv1 id0x60010003
    D:stateValue() inv2 id0x60020000
    D:availableVersion() inv2 id0x60020001
    D:bytesDownloaded() inv2 id0x60020002
    D:totalBytesToDownload() inv2 id0x60020003
    D:downloadTimeRemainingMs() inv2 id0x60020004
    D:nextRetryTime() inv2 id0x60020005
    D:installProgress() inv2 id0x60020006
    D:installTimeRemainingMs() inv2 id0x60020007
    D:isCanceled() inv2 id0x60020008
    D:errorCode() inv2 id0x60020009
    D:extraCode1() inv2 id0x6002000a
    D:completionMessage() inv2 id0x6002000b
    D:installerResultCode() inv2 id0x6002000c
    D:installerResultExtraCode1() inv2 id0x6002000d
    D:postInstallLaunchCommandLine() inv2 id0x6002000e
    D:postInstallUrl() inv2 id0x6002000f
    D:postInstallAction() inv2 id0x60020010
[9] ICurrentStateSystem kind 4 flags 0x1040 funcs 24 impl 1 id {E9CD91E3-A00C-4B9E-BD63-7F34EB815D98}
    D:QueryInterface(in PTR>USERDEFINED(GUID), out PTR>PTR>vt24) inv1 id0x60000000
    D:AddRef() inv1 id0x60000001
    D:Release() inv1 id0x60000002
    D:GetTypeInfoCount(out PTR>vt23) inv1 id0x60010000
    D:GetTypeInfo(in vt23, in vt19, out PTR>PTR>vt24) inv1 id0x60010001
    D:GetIDsOfNames(in PTR>USERDEFINED(GUID), in PTR>PTR>vt16, in vt23, in vt19, out PTR>I4) inv1 id0x60010002
    D:Invoke(in I4, in PTR>USERDEFINED(GUID), in vt19, in vt18, in PTR>USERDEFINED(DISPPARAMS), out PTR>VARIANT, out PTR>USERDEFINED(EXCEPINFO), out PTR>vt23) inv1 id0x60010003
    D:stateValue() inv2 id0x60020000
    D:availableVersion() inv2 id0x60020001
    D:bytesDownloaded() inv2 id0x60020002
    D:totalBytesToDownload() inv2 id0x60020003
    D:downloadTimeRemainingMs() inv2 id0x60020004
    D:nextRetryTime() inv2 id0x60020005
    D:installProgress() inv2 id0x60020006
    D:installTimeRemainingMs() inv2 id0x60020007
    D:isCanceled() inv2 id0x60020008
    D:errorCode() inv2 id0x60020009
    D:extraCode1() inv2 id0x6002000a
    D:completionMessage() inv2 id0x6002000b
    D:installerResultCode() inv2 id0x6002000c
    D:installerResultExtraCode1() inv2 id0x6002000d
    D:postInstallLaunchCommandLine() inv2 id0x6002000e
    D:postInstallUrl() inv2 id0x6002000f
    D:postInstallAction() inv2 id0x60020010
[10] IGoogleUpdate3Web kind 4 flags 0x1040 funcs 8 impl 1 id {494B20CF-282E-4BDD-9F5D-B70CB09D351E}
    D:QueryInterface(in PTR>USERDEFINED(GUID), out PTR>PTR>vt24) inv1 id0x60000000
    D:AddRef() inv1 id0x60000001
    D:Release() inv1 id0x60000002
    D:GetTypeInfoCount(out PTR>vt23) inv1 id0x60010000
    D:GetTypeInfo(in vt23, in vt19, out PTR>PTR>vt24) inv1 id0x60010001
    D:GetIDsOfNames(in PTR>USERDEFINED(GUID), in PTR>PTR>vt16, in vt23, in vt19, out PTR>I4) inv1 id0x60010002
    D:Invoke(in I4, in PTR>USERDEFINED(GUID), in vt19, in vt18, in PTR>USERDEFINED(DISPPARAMS), out PTR>VARIANT, out PTR>USERDEFINED(EXCEPINFO), out PTR>vt23) inv1 id0x60010003
    D:createAppBundleWeb() inv1 id0x60020000
[11] IGoogleUpdate3WebSystem kind 4 flags 0x1040 funcs 8 impl 1 id {5F793925-C903-4E92-9AE3-77CA5EAB1716}
    D:QueryInterface(in PTR>USERDEFINED(GUID), out PTR>PTR>vt24) inv1 id0x60000000
    D:AddRef() inv1 id0x60000001
    D:Release() inv1 id0x60000002
    D:GetTypeInfoCount(out PTR>vt23) inv1 id0x60010000
    D:GetTypeInfo(in vt23, in vt19, out PTR>PTR>vt24) inv1 id0x60010001
    D:GetIDsOfNames(in PTR>USERDEFINED(GUID), in PTR>PTR>vt16, in vt23, in vt19, out PTR>I4) inv1 id0x60010002
    D:Invoke(in I4, in PTR>USERDEFINED(GUID), in vt19, in vt18, in PTR>USERDEFINED(DISPPARAMS), out PTR>VARIANT, out PTR>USERDEFINED(EXCEPINFO), out PTR>vt23) inv1 id0x60010003
    D:createAppBundleWeb() inv1 id0x60020000
[12] IAppBundleWeb kind 4 flags 0x1040 funcs 24 impl 1 id {DD42475D-6D46-496A-924E-BD5630B4CBBA}
    D:QueryInterface(in PTR>USERDEFINED(GUID), out PTR>PTR>vt24) inv1 id0x60000000
    D:AddRef() inv1 id0x60000001
    D:Release() inv1 id0x60000002
    D:GetTypeInfoCount(out PTR>vt23) inv1 id0x60010000
    D:GetTypeInfo(in vt23, in vt19, out PTR>PTR>vt24) inv1 id0x60010001
    D:GetIDsOfNames(in PTR>USERDEFINED(GUID), in PTR>PTR>vt16, in vt23, in vt19, out PTR>I4) inv1 id0x60010002
    D:Invoke(in I4, in PTR>USERDEFINED(GUID), in vt19, in vt18, in PTR>USERDEFINED(DISPPARAMS), out PTR>VARIANT, out PTR>USERDEFINED(EXCEPINFO), out PTR>vt23) inv1 id0x60010003
    D:createApp(in BSTR, in BSTR, in BSTR, in BSTR) inv1 id0x2
    D:createInstalledApp(in BSTR) inv1 id0x3
    D:createAllInstalledApps() inv1 id0x4
    D:displayLanguage() inv2 id0x60020003
    D:displayLanguage(in BSTR) inv4 id0x60020003
    D:parentHWND(in USERDEFINED(ULONG_PTR)) inv4 id0x60020005
    D:length() inv2 id0x60020006
    D:appWeb(in vt22) inv2 id0x0
    D:initialize() inv1 id0x60020008
    D:checkForUpdate() inv1 id0x60020009
    D:download() inv1 id0x6002000a
    D:install() inv1 id0x6002000b
    D:pause() inv1 id0x6002000c
    D:resume() inv1 id0x6002000d
    D:cancel() inv1 id0x6002000e
    D:downloadPackage(in BSTR, in BSTR) inv1 id0x6002000f
    D:currentState() inv2 id0x60020010
[13] ULONG_PTR kind 6 flags 0x0 funcs 0 impl 0 id {00000000-0000-0000-0000-000000000000}
[14] IAppBundleWebSystem kind 4 flags 0x1040 funcs 24 impl 1 id {C4622B28-A747-44C7-96AF-319BE5C3B261}
    D:QueryInterface(in PTR>USERDEFINED(GUID), out PTR>PTR>vt24) inv1 id0x60000000
    D:AddRef() inv1 id0x60000001
    D:Release() inv1 id0x60000002
    D:GetTypeInfoCount(out PTR>vt23) inv1 id0x60010000
    D:GetTypeInfo(in vt23, in vt19, out PTR>PTR>vt24) inv1 id0x60010001
    D:GetIDsOfNames(in PTR>USERDEFINED(GUID), in PTR>PTR>vt16, in vt23, in vt19, out PTR>I4) inv1 id0x60010002
    D:Invoke(in I4, in PTR>USERDEFINED(GUID), in vt19, in vt18, in PTR>USERDEFINED(DISPPARAMS), out PTR>VARIANT, out PTR>USERDEFINED(EXCEPINFO), out PTR>vt23) inv1 id0x60010003
    D:createApp(in BSTR, in BSTR, in BSTR, in BSTR) inv1 id0x2
    D:createInstalledApp(in BSTR) inv1 id0x3
    D:createAllInstalledApps() inv1 id0x4
    D:displayLanguage() inv2 id0x60020003
    D:displayLanguage(in BSTR) inv4 id0x60020003
    D:parentHWND(in USERDEFINED(ULONG_PTR)) inv4 id0x60020005
    D:length() inv2 id0x60020006
    D:appWeb(in vt22) inv2 id0x0
    D:initialize() inv1 id0x60020008
    D:checkForUpdate() inv1 id0x60020009
    D:download() inv1 id0x6002000a
    D:install() inv1 id0x6002000b
    D:pause() inv1 id0x6002000c
    D:resume() inv1 id0x6002000d
    D:cancel() inv1 id0x6002000e
    D:downloadPackage(in BSTR, in BSTR) inv1 id0x6002000f
    D:currentState() inv2 id0x60020010
[15] IAppWeb kind 4 flags 0x1040 funcs 17 impl 1 id {18D0F672-18B4-48E6-AD36-6E6BF01DBBC4}
    D:QueryInterface(in PTR>USERDEFINED(GUID), out PTR>PTR>vt24) inv1 id0x60000000
    D:AddRef() inv1 id0x60000001
    D:Release() inv1 id0x60000002
    D:GetTypeInfoCount(out PTR>vt23) inv1 id0x60010000
    D:GetTypeInfo(in vt23, in vt19, out PTR>PTR>vt24) inv1 id0x60010001
    D:GetIDsOfNames(in PTR>USERDEFINED(GUID), in PTR>PTR>vt16, in vt23, in vt19, out PTR>I4) inv1 id0x60010002
    D:Invoke(in I4, in PTR>USERDEFINED(GUID), in vt19, in vt18, in PTR>USERDEFINED(DISPPARAMS), out PTR>VARIANT, out PTR>USERDEFINED(EXCEPINFO), out PTR>vt23) inv1 id0x60010003
    D:appId() inv2 id0x60020000
    D:currentVersionWeb() inv2 id0x60020001
    D:nextVersionWeb() inv2 id0x60020002
    D:command(in BSTR) inv2 id0x60020003
    D:cancel() inv1 id0x60020004
    D:currentState() inv2 id0x60020005
    D:launch() inv1 id0x60020006
    D:uninstall() inv1 id0x60020007
    D:serverInstallDataIndex() inv2 id0x60020008
    D:serverInstallDataIndex(in BSTR) inv4 id0x60020008
[16] IAppWebSystem kind 4 flags 0x1040 funcs 17 impl 1 id {D576ED7F-31DA-4EE1-98CE-1F882FB3047A}
    D:QueryInterface(in PTR>USERDEFINED(GUID), out PTR>PTR>vt24) inv1 id0x60000000
    D:AddRef() inv1 id0x60000001
    D:Release() inv1 id0x60000002
    D:GetTypeInfoCount(out PTR>vt23) inv1 id0x60010000
    D:GetTypeInfo(in vt23, in vt19, out PTR>PTR>vt24) inv1 id0x60010001
    D:GetIDsOfNames(in PTR>USERDEFINED(GUID), in PTR>PTR>vt16, in vt23, in vt19, out PTR>I4) inv1 id0x60010002
    D:Invoke(in I4, in PTR>USERDEFINED(GUID), in vt19, in vt18, in PTR>USERDEFINED(DISPPARAMS), out PTR>VARIANT, out PTR>USERDEFINED(EXCEPINFO), out PTR>vt23) inv1 id0x60010003
    D:appId() inv2 id0x60020000
    D:currentVersionWeb() inv2 id0x60020001
    D:nextVersionWeb() inv2 id0x60020002
    D:command(in BSTR) inv2 id0x60020003
    D:cancel() inv1 id0x60020004
    D:currentState() inv2 id0x60020005
    D:launch() inv1 id0x60020006
    D:uninstall() inv1 id0x60020007
    D:serverInstallDataIndex() inv2 id0x60020008
    D:serverInstallDataIndex(in BSTR) inv4 id0x60020008
[17] IAppCommandWeb kind 4 flags 0x1040 funcs 11 impl 1 id {8476CE12-AE1F-4198-805C-BA0F9B783F57}
    D:QueryInterface(in PTR>USERDEFINED(GUID), out PTR>PTR>vt24) inv1 id0x60000000
    D:AddRef() inv1 id0x60000001
    D:Release() inv1 id0x60000002
    D:GetTypeInfoCount(out PTR>vt23) inv1 id0x60010000
    D:GetTypeInfo(in vt23, in vt19, out PTR>PTR>vt24) inv1 id0x60010001
    D:GetIDsOfNames(in PTR>USERDEFINED(GUID), in PTR>PTR>vt16, in vt23, in vt19, out PTR>I4) inv1 id0x60010002
    D:Invoke(in I4, in PTR>USERDEFINED(GUID), in vt19, in vt18, in PTR>USERDEFINED(DISPPARAMS), out PTR>VARIANT, out PTR>USERDEFINED(EXCEPINFO), out PTR>vt23) inv1 id0x60010003
    D:status() inv2 id0x60020000
    D:exitCode() inv2 id0x60020001
    D:output() inv2 id0x60020002
    D:execute(in VARIANT, in VARIANT, in VARIANT, in VARIANT, in VARIANT, in VARIANT, in VARIANT, in VARIANT, in VARIANT) inv1 id0x60020003
[18] IAppCommandWebSystem kind 4 flags 0x1040 funcs 11 impl 1 id {1F1289FD-DD10-4579-81F6-1C59AAF2E1A9}
    D:QueryInterface(in PTR>USERDEFINED(GUID), out PTR>PTR>vt24) inv1 id0x60000000
    D:AddRef() inv1 id0x60000001
    D:Release() inv1 id0x60000002
    D:GetTypeInfoCount(out PTR>vt23) inv1 id0x60010000
    D:GetTypeInfo(in vt23, in vt19, out PTR>PTR>vt24) inv1 id0x60010001
    D:GetIDsOfNames(in PTR>USERDEFINED(GUID), in PTR>PTR>vt16, in vt23, in vt19, out PTR>I4) inv1 id0x60010002
    D:Invoke(in I4, in PTR>USERDEFINED(GUID), in vt19, in vt18, in PTR>USERDEFINED(DISPPARAMS), out PTR>VARIANT, out PTR>USERDEFINED(EXCEPINFO), out PTR>vt23) inv1 id0x60010003
    D:status() inv2 id0x60020000
    D:exitCode() inv2 id0x60020001
    D:output() inv2 id0x60020002
    D:execute(in VARIANT, in VARIANT, in VARIANT, in VARIANT, in VARIANT, in VARIANT, in VARIANT, in VARIANT, in VARIANT) inv1 id0x60020003
[19] IPolicyStatus kind 4 flags 0x1040 funcs 16 impl 1 id {F63F6F8B-ACD5-413C-A44B-0409136D26CB}
    D:QueryInterface(in PTR>USERDEFINED(GUID), out PTR>PTR>vt24) inv1 id0x60000000
    D:AddRef() inv1 id0x60000001
    D:Release() inv1 id0x60000002
    D:GetTypeInfoCount(out PTR>vt23) inv1 id0x60010000
    D:GetTypeInfo(in vt23, in vt19, out PTR>PTR>vt24) inv1 id0x60010001
    D:GetIDsOfNames(in PTR>USERDEFINED(GUID), in PTR>PTR>vt16, in vt23, in vt19, out PTR>I4) inv1 id0x60010002
    D:Invoke(in I4, in PTR>USERDEFINED(GUID), in vt19, in vt18, in PTR>USERDEFINED(DISPPARAMS), out PTR>VARIANT, out PTR>USERDEFINED(EXCEPINFO), out PTR>vt23) inv1 id0x60010003
    D:lastCheckPeriodMinutes() inv2 id0x60020000
    D:updatesSuppressedTimes(out PTR>vt19, out PTR>vt19, out PTR>vt19, out PTR>vt11) inv2 id0x60020001
    D:downloadPreferenceGroupPolicy() inv2 id0x60020002
    D:packageCacheSizeLimitMBytes() inv2 id0x60020003
    D:packageCacheExpirationTimeDays() inv2 id0x60020004
    D:effectivePolicyForAppInstalls(in BSTR) inv2 id0x60020005
    D:effectivePolicyForAppUpdates(in BSTR) inv2 id0x60020006
    D:targetVersionPrefix(in BSTR) inv2 id0x60020007
    D:isRollbackToTargetVersionAllowed(in BSTR) inv2 id0x60020008
[20] IPolicyStatusSystem kind 4 flags 0x1040 funcs 16 impl 1 id {B685B009-DBC4-4F24-9542-A162C3793E77}
    D:QueryInterface(in PTR>USERDEFINED(GUID), out PTR>PTR>vt24) inv1 id0x60000000
    D:AddRef() inv1 id0x60000001
    D:Release() inv1 id0x60000002
    D:GetTypeInfoCount(out PTR>vt23) inv1 id0x60010000
    D:GetTypeInfo(in vt23, in vt19, out PTR>PTR>vt24) inv1 id0x60010001
    D:GetIDsOfNames(in PTR>USERDEFINED(GUID), in PTR>PTR>vt16, in vt23, in vt19, out PTR>I4) inv1 id0x60010002
    D:Invoke(in I4, in PTR>USERDEFINED(GUID), in vt19, in vt18, in PTR>USERDEFINED(DISPPARAMS), out PTR>VARIANT, out PTR>USERDEFINED(EXCEPINFO), out PTR>vt23) inv1 id0x60010003
    D:lastCheckPeriodMinutes() inv2 id0x60020000
    D:updatesSuppressedTimes(out PTR>vt19, out PTR>vt19, out PTR>vt19, out PTR>vt11) inv2 id0x60020001
    D:downloadPreferenceGroupPolicy() inv2 id0x60020002
    D:packageCacheSizeLimitMBytes() inv2 id0x60020003
    D:packageCacheExpirationTimeDays() inv2 id0x60020004
    D:effectivePolicyForAppInstalls(in BSTR) inv2 id0x60020005
    D:effectivePolicyForAppUpdates(in BSTR) inv2 id0x60020006
    D:targetVersionPrefix(in BSTR) inv2 id0x60020007
    D:isRollbackToTargetVersionAllowed(in BSTR) inv2 id0x60020008
[21] IPolicyStatusValue kind 4 flags 0x1040 funcs 12 impl 1 id {27634814-8E41-4C35-8577-980134A96544}
    D:QueryInterface(in PTR>USERDEFINED(GUID), out PTR>PTR>vt24) inv1 id0x60000000
    D:AddRef() inv1 id0x60000001
    D:Release() inv1 id0x60000002
    D:GetTypeInfoCount(out PTR>vt23) inv1 id0x60010000
    D:GetTypeInfo(in vt23, in vt19, out PTR>PTR>vt24) inv1 id0x60010001
    D:GetIDsOfNames(in PTR>USERDEFINED(GUID), in PTR>PTR>vt16, in vt23, in vt19, out PTR>I4) inv1 id0x60010002
    D:Invoke(in I4, in PTR>USERDEFINED(GUID), in vt19, in vt18, in PTR>USERDEFINED(DISPPARAMS), out PTR>VARIANT, out PTR>USERDEFINED(EXCEPINFO), out PTR>vt23) inv1 id0x60010003
    D:source() inv2 id0x60020000
    D:value() inv2 id0x60020001
    D:hasConflict() inv2 id0x60020002
    D:conflictSource() inv2 id0x60020003
    D:conflictValue() inv2 id0x60020004
[22] IPolicyStatusValueSystem kind 4 flags 0x1040 funcs 12 impl 1 id {4DC034A8-4BFC-4D43-9250-914163356BB0}
    D:QueryInterface(in PTR>USERDEFINED(GUID), out PTR>PTR>vt24) inv1 id0x60000000
    D:AddRef() inv1 id0x60000001
    D:Release() inv1 id0x60000002
    D:GetTypeInfoCount(out PTR>vt23) inv1 id0x60010000
    D:GetTypeInfo(in vt23, in vt19, out PTR>PTR>vt24) inv1 id0x60010001
    D:GetIDsOfNames(in PTR>USERDEFINED(GUID), in PTR>PTR>vt16, in vt23, in vt19, out PTR>I4) inv1 id0x60010002
    D:Invoke(in I4, in PTR>USERDEFINED(GUID), in vt19, in vt18, in PTR>USERDEFINED(DISPPARAMS), out PTR>VARIANT, out PTR>USERDEFINED(EXCEPINFO), out PTR>vt23) inv1 id0x60010003
    D:source() inv2 id0x60020000
    D:value() inv2 id0x60020001
    D:hasConflict() inv2 id0x60020002
    D:conflictSource() inv2 id0x60020003
    D:conflictValue() inv2 id0x60020004
[23] IPolicyStatus2 kind 4 flags 0x1040 funcs 23 impl 1 id {34527502-D3DB-4205-A69B-789B27EE0414}
    D:QueryInterface(in PTR>USERDEFINED(GUID), out PTR>PTR>vt24) inv1 id0x60000000
    D:AddRef() inv1 id0x60000001
    D:Release() inv1 id0x60000002
    D:GetTypeInfoCount(out PTR>vt23) inv1 id0x60010000
    D:GetTypeInfo(in vt23, in vt19, out PTR>PTR>vt24) inv1 id0x60010001
    D:GetIDsOfNames(in PTR>USERDEFINED(GUID), in PTR>PTR>vt16, in vt23, in vt19, out PTR>I4) inv1 id0x60010002
    D:Invoke(in I4, in PTR>USERDEFINED(GUID), in vt19, in vt18, in PTR>USERDEFINED(DISPPARAMS), out PTR>VARIANT, out PTR>USERDEFINED(EXCEPINFO), out PTR>vt23) inv1 id0x60010003
    D:updaterVersion() inv2 id0x60020000
    D:lastCheckedTime() inv2 id0x60020001
    D:refreshPolicies() inv1 id0x60020002
    D:lastCheckPeriodMinutes() inv2 id0x60020003
    D:updatesSuppressedTimes(out PTR>PTR>USERDEFINED(IPolicyStatusValue), PTR>vt11) inv2 id0x60020004
    D:downloadPreferenceGroupPolicy() inv2 id0x60020005
    D:packageCacheSizeLimitMBytes() inv2 id0x60020006
    D:packageCacheExpirationTimeDays() inv2 id0x60020007
    D:proxyMode() inv2 id0x60020008
    D:proxyPacUrl() inv2 id0x60020009
    D:proxyServer() inv2 id0x6002000a
    D:effectivePolicyForAppInstalls(in BSTR) inv2 id0x6002000b
    D:effectivePolicyForAppUpdates(in BSTR) inv2 id0x6002000c
    D:targetVersionPrefix(in BSTR) inv2 id0x6002000d
    D:isRollbackToTargetVersionAllowed(in BSTR) inv2 id0x6002000e
    D:targetChannel(in BSTR) inv2 id0x6002000f
[24] IPolicyStatus2System kind 4 flags 0x1040 funcs 23 impl 1 id {513BC7DA-6B8D-45F7-90A0-2E9F66CEF962}
    D:QueryInterface(in PTR>USERDEFINED(GUID), out PTR>PTR>vt24) inv1 id0x60000000
    D:AddRef() inv1 id0x60000001
    D:Release() inv1 id0x60000002
    D:GetTypeInfoCount(out PTR>vt23) inv1 id0x60010000
    D:GetTypeInfo(in vt23, in vt19, out PTR>PTR>vt24) inv1 id0x60010001
    D:GetIDsOfNames(in PTR>USERDEFINED(GUID), in PTR>PTR>vt16, in vt23, in vt19, out PTR>I4) inv1 id0x60010002
    D:Invoke(in I4, in PTR>USERDEFINED(GUID), in vt19, in vt18, in PTR>USERDEFINED(DISPPARAMS), out PTR>VARIANT, out PTR>USERDEFINED(EXCEPINFO), out PTR>vt23) inv1 id0x60010003
    D:updaterVersion() inv2 id0x60020000
    D:lastCheckedTime() inv2 id0x60020001
    D:refreshPolicies() inv1 id0x60020002
    D:lastCheckPeriodMinutes() inv2 id0x60020003
    D:updatesSuppressedTimes(out PTR>PTR>USERDEFINED(IPolicyStatusValueSystem), PTR>vt11) inv2 id0x60020004
    D:downloadPreferenceGroupPolicy() inv2 id0x60020005
    D:packageCacheSizeLimitMBytes() inv2 id0x60020006
    D:packageCacheExpirationTimeDays() inv2 id0x60020007
    D:proxyMode() inv2 id0x60020008
    D:proxyPacUrl() inv2 id0x60020009
    D:proxyServer() inv2 id0x6002000a
    D:effectivePolicyForAppInstalls(in BSTR) inv2 id0x6002000b
    D:effectivePolicyForAppUpdates(in BSTR) inv2 id0x6002000c
    D:targetVersionPrefix(in BSTR) inv2 id0x6002000d
    D:isRollbackToTargetVersionAllowed(in BSTR) inv2 id0x6002000e
    D:targetChannel(in BSTR) inv2 id0x6002000f
[25] IPolicyStatus3 kind 4 flags 0x1040 funcs 24 impl 1 id {05A30352-EB25-45B6-8449-BCA7B0542CE5}
    D:QueryInterface(in PTR>USERDEFINED(GUID), out PTR>PTR>vt24) inv1 id0x60000000
    D:AddRef() inv1 id0x60000001
    D:Release() inv1 id0x60000002
    D:GetTypeInfoCount(out PTR>vt23) inv1 id0x60010000
    D:GetTypeInfo(in vt23, in vt19, out PTR>PTR>vt24) inv1 id0x60010001
    D:GetIDsOfNames(in PTR>USERDEFINED(GUID), in PTR>PTR>vt16, in vt23, in vt19, out PTR>I4) inv1 id0x60010002
    D:Invoke(in I4, in PTR>USERDEFINED(GUID), in vt19, in vt18, in PTR>USERDEFINED(DISPPARAMS), out PTR>VARIANT, out PTR>USERDEFINED(EXCEPINFO), out PTR>vt23) inv1 id0x60010003
    D:updaterVersion() inv2 id0x60020000
    D:lastCheckedTime() inv2 id0x60020001
    D:refreshPolicies() inv1 id0x60020002
    D:lastCheckPeriodMinutes() inv2 id0x60020003
    D:updatesSuppressedTimes(out PTR>PTR>USERDEFINED(IPolicyStatusValue), PTR>vt11) inv2 id0x60020004
    D:downloadPreferenceGroupPolicy() inv2 id0x60020005
    D:packageCacheSizeLimitMBytes() inv2 id0x60020006
    D:packageCacheExpirationTimeDays() inv2 id0x60020007
    D:proxyMode() inv2 id0x60020008
    D:proxyPacUrl() inv2 id0x60020009
    D:proxyServer() inv2 id0x6002000a
    D:effectivePolicyForAppInstalls(in BSTR) inv2 id0x6002000b
    D:effectivePolicyForAppUpdates(in BSTR) inv2 id0x6002000c
    D:targetVersionPrefix(in BSTR) inv2 id0x6002000d
    D:isRollbackToTargetVersionAllowed(in BSTR) inv2 id0x6002000e
    D:targetChannel(in BSTR) inv2 id0x6002000f
    D:forceInstallApps(in vt11) inv2 id0x60030000
[26] IPolicyStatus3System kind 4 flags 0x1040 funcs 24 impl 1 id {F4334319-8210-469B-8262-DD03623FEB5B}
    D:QueryInterface(in PTR>USERDEFINED(GUID), out PTR>PTR>vt24) inv1 id0x60000000
    D:AddRef() inv1 id0x60000001
    D:Release() inv1 id0x60000002
    D:GetTypeInfoCount(out PTR>vt23) inv1 id0x60010000
    D:GetTypeInfo(in vt23, in vt19, out PTR>PTR>vt24) inv1 id0x60010001
    D:GetIDsOfNames(in PTR>USERDEFINED(GUID), in PTR>PTR>vt16, in vt23, in vt19, out PTR>I4) inv1 id0x60010002
    D:Invoke(in I4, in PTR>USERDEFINED(GUID), in vt19, in vt18, in PTR>USERDEFINED(DISPPARAMS), out PTR>VARIANT, out PTR>USERDEFINED(EXCEPINFO), out PTR>vt23) inv1 id0x60010003
    D:updaterVersion() inv2 id0x60020000
    D:lastCheckedTime() inv2 id0x60020001
    D:refreshPolicies() inv1 id0x60020002
    D:lastCheckPeriodMinutes() inv2 id0x60020003
    D:updatesSuppressedTimes(out PTR>PTR>USERDEFINED(IPolicyStatusValueSystem), PTR>vt11) inv2 id0x60020004
    D:downloadPreferenceGroupPolicy() inv2 id0x60020005
    D:packageCacheSizeLimitMBytes() inv2 id0x60020006
    D:packageCacheExpirationTimeDays() inv2 id0x60020007
    D:proxyMode() inv2 id0x60020008
    D:proxyPacUrl() inv2 id0x60020009
    D:proxyServer() inv2 id0x6002000a
    D:effectivePolicyForAppInstalls(in BSTR) inv2 id0x6002000b
    D:effectivePolicyForAppUpdates(in BSTR) inv2 id0x6002000c
    D:targetVersionPrefix(in BSTR) inv2 id0x6002000d
    D:isRollbackToTargetVersionAllowed(in BSTR) inv2 id0x6002000e
    D:targetChannel(in BSTR) inv2 id0x6002000f
    D:forceInstallApps(in vt11) inv2 id0x60030000
[27] IPolicyStatus4 kind 4 flags 0x1040 funcs 25 impl 1 id {FD0FDA43-AF97-4F1C-BD68-3355FB4F1C92}
    D:QueryInterface(in PTR>USERDEFINED(GUID), out PTR>PTR>vt24) inv1 id0x60000000
    D:AddRef() inv1 id0x60000001
    D:Release() inv1 id0x60000002
    D:GetTypeInfoCount(out PTR>vt23) inv1 id0x60010000
    D:GetTypeInfo(in vt23, in vt19, out PTR>PTR>vt24) inv1 id0x60010001
    D:GetIDsOfNames(in PTR>USERDEFINED(GUID), in PTR>PTR>vt16, in vt23, in vt19, out PTR>I4) inv1 id0x60010002
    D:Invoke(in I4, in PTR>USERDEFINED(GUID), in vt19, in vt18, in PTR>USERDEFINED(DISPPARAMS), out PTR>VARIANT, out PTR>USERDEFINED(EXCEPINFO), out PTR>vt23) inv1 id0x60010003
    D:updaterVersion() inv2 id0x60020000
    D:lastCheckedTime() inv2 id0x60020001
    D:refreshPolicies() inv1 id0x60020002
    D:lastCheckPeriodMinutes() inv2 id0x60020003
    D:updatesSuppressedTimes(out PTR>PTR>USERDEFINED(IPolicyStatusValue), PTR>vt11) inv2 id0x60020004
    D:downloadPreferenceGroupPolicy() inv2 id0x60020005
    D:packageCacheSizeLimitMBytes() inv2 id0x60020006
    D:packageCacheExpirationTimeDays() inv2 id0x60020007
    D:proxyMode() inv2 id0x60020008
    D:proxyPacUrl() inv2 id0x60020009
    D:proxyServer() inv2 id0x6002000a
    D:effectivePolicyForAppInstalls(in BSTR) inv2 id0x6002000b
    D:effectivePolicyForAppUpdates(in BSTR) inv2 id0x6002000c
    D:targetVersionPrefix(in BSTR) inv2 id0x6002000d
    D:isRollbackToTargetVersionAllowed(in BSTR) inv2 id0x6002000e
    D:targetChannel(in BSTR) inv2 id0x6002000f
    D:forceInstallApps(in vt11) inv2 id0x60030000
    D:cloudPolicyOverridesPlatformPolicy() inv2 id0x60040000
[28] IPolicyStatus4System kind 4 flags 0x1040 funcs 25 impl 1 id {F966A529-43C6-4710-8FF4-0B456324C8F4}
    D:QueryInterface(in PTR>USERDEFINED(GUID), out PTR>PTR>vt24) inv1 id0x60000000
    D:AddRef() inv1 id0x60000001
    D:Release() inv1 id0x60000002
    D:GetTypeInfoCount(out PTR>vt23) inv1 id0x60010000
    D:GetTypeInfo(in vt23, in vt19, out PTR>PTR>vt24) inv1 id0x60010001
    D:GetIDsOfNames(in PTR>USERDEFINED(GUID), in PTR>PTR>vt16, in vt23, in vt19, out PTR>I4) inv1 id0x60010002
    D:Invoke(in I4, in PTR>USERDEFINED(GUID), in vt19, in vt18, in PTR>USERDEFINED(DISPPARAMS), out PTR>VARIANT, out PTR>USERDEFINED(EXCEPINFO), out PTR>vt23) inv1 id0x60010003
    D:updaterVersion() inv2 id0x60020000
    D:lastCheckedTime() inv2 id0x60020001
    D:refreshPolicies() inv1 id0x60020002
    D:lastCheckPeriodMinutes() inv2 id0x60020003
    D:updatesSuppressedTimes(out PTR>PTR>USERDEFINED(IPolicyStatusValueSystem), PTR>vt11) inv2 id0x60020004
    D:downloadPreferenceGroupPolicy() inv2 id0x60020005
    D:packageCacheSizeLimitMBytes() inv2 id0x60020006
    D:packageCacheExpirationTimeDays() inv2 id0x60020007
    D:proxyMode() inv2 id0x60020008
    D:proxyPacUrl() inv2 id0x60020009
    D:proxyServer() inv2 id0x6002000a
    D:effectivePolicyForAppInstalls(in BSTR) inv2 id0x6002000b
    D:effectivePolicyForAppUpdates(in BSTR) inv2 id0x6002000c
    D:targetVersionPrefix(in BSTR) inv2 id0x6002000d
    D:isRollbackToTargetVersionAllowed(in BSTR) inv2 id0x6002000e
    D:targetChannel(in BSTR) inv2 id0x6002000f
    D:forceInstallApps(in vt11) inv2 id0x60030000
    D:cloudPolicyOverridesPlatformPolicy() inv2 id0x60040000
[29] IProcessLauncher kind 3 flags 0x100 funcs 3 impl 1 id {128C2DA6-2BC0-44C0-B3F6-4EC22E647964}
    LaunchCmdLine(in vt31) inv1 id0x60010000
    LaunchBrowser(in vt19, in vt31) inv1 id0x60010001
    LaunchCmdElevated(in vt31, in vt31, in vt19, out PTR>USERDEFINED(ULONG_PTR)) inv1 id0x60010002
[30] IProcessLauncherSystem kind 3 flags 0x100 funcs 3 impl 1 id {F4FE76BC-62B9-49FC-972F-C81FC3A926DB}
    LaunchCmdLine(in vt31) inv1 id0x60010000
    LaunchBrowser(in vt19, in vt31) inv1 id0x60010001
    LaunchCmdElevated(in vt31, in vt31, in vt19, out PTR>USERDEFINED(ULONG_PTR)) inv1 id0x60010002
[31] IProcessLauncher2 kind 3 flags 0x100 funcs 1 impl 1 id {D106AB5F-A70E-400E-A21B-96208C1D8DBB}
    LaunchCmdLineEx(in vt31, out PTR>vt19, out PTR>USERDEFINED(ULONG_PTR), out PTR>USERDEFINED(ULONG_PTR)) inv1 id0x60020000
[32] IProcessLauncher2System kind 3 flags 0x100 funcs 1 impl 1 id {6430040A-5EBD-4E63-A56F-C71D5990F827}
    LaunchCmdLineEx(in vt31, out PTR>vt19, out PTR>USERDEFINED(ULONG_PTR), out PTR>USERDEFINED(ULONG_PTR)) inv1 id0x60020000
summary 0 passed, 0 failed
```

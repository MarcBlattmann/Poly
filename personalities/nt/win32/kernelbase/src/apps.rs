//! The functions the Windows applications of the survey (`windows-app-survey`) import from `kernelbase.dll` that Poly does
//! not implement yet (ADR 0155, step 10): each is an export that logs `stub: kernelbase!name` and fails as ADR 0155 fixes for a
//! function the system does not provide, so that the programs bind it (and `GetProcAddress` finds it). An
//! implementation replaces a name here by defining it elsewhere in the crate and deleting it from the list.
//! Generated from the programs' import tables (names only) and our export table.

poly_win32_rt::fail_exports!("kernelbase", invalid_handle, [CreateFile2,]);

poly_win32_rt::fail_exports!(
    "kernelbase",
    value(0),
    [
        CloseThreadpoolTimer,
        CloseThreadpoolWait,
        CloseThreadpoolWork,
        DebugBreak,
        DelayLoadFailureHook,
        FreeLibraryWhenCallbackReturns,
        GetFeatureEnabledState,
        GetFeatureVariant,
        RaiseFailFastException,
        RecordFeatureError,
        RecordFeatureUsage,
        SetThreadpoolTimer,
        SetThreadpoolWait,
        SubscribeFeatureStateChangeNotification,
        UnsubscribeFeatureStateChangeNotification,
        WaitForThreadpoolTimerCallbacks,
        WaitForThreadpoolWaitCallbacks,
    ]
);

poly_win32_rt::fail_exports!("kernelbase", value(120), [GetPackagePathByFullName, PackageFamilyNameFromFullName,]);

poly_win32_rt::fail_exports!(
    "kernelbase",
    value(15700),
    [
        AppPolicyGetProcessTerminationMethod,
        AppPolicyGetThreadInitializationType,
        GetCurrentPackageFullName,
        GetPackageFullName,
    ]
);

poly_win32_rt::fail_exports!(
    "kernelbase",
    win32,
    [
        ApiSetQueryApiSetPresence,
        CharLowerW,
        CharNextW,
        CreateActCtxW,
        CreateEventExW,
        CreateMutexExW,
        CreateSemaphoreExW,
        CreateThreadpoolTimer,
        CreateThreadpoolWait,
        CreateWaitableTimerExW,
        DeriveCapabilitySidsFromName,
        EnumSystemLocalesEx,
        EnumSystemLocalesW,
        FindAtomW,
        FlushInstructionCache,
        FreeLibraryAndExitThread,
        GetDateFormatEx,
        GetDateFormatW,
        GetDiskFreeSpaceA,
        GetEffectivePackageStatusForUserSid,
        GetNumaProcessorNode,
        GetSystemMetadataPathForPackage,
        GetTempPathA,
        GetTimeFormatEx,
        GetTimeFormatW,
        GetVolumeInformationW,
        GlobalAddAtomW,
        GlobalDeleteAtom,
        HeapCompact,
        HeapValidate,
        IsApiSetImplemented,
        K32EnumProcessModules,
        K32GetModuleFileNameExW,
        LCIDToLocaleName,
        LocaleNameToLCID,
        LockFile,
        OpenSemaphoreW,
        PathAllocCombine,
        PathCchRemoveBackslash,
        PathCchRemoveExtension,
        PathCchRemoveFileSpec,
        PathFileExistsW,
        RegCreateKeyW,
        RegEnumKeyW,
        RegOpenCurrentUser,
        RegOpenKeyW,
        ResolveDelayLoadedAPI,
        SearchPathW,
        SetDllDirectoryA,
        SetProcessPreferredUILanguages,
        SetWaitableTimerEx,
        TrySubmitThreadpoolCallback,
        UnlockFile,
        VerifyPackageFamilyName,
        VerifyPackageFullName,
        VirtualAlloc2FromApp,
    ]
);

// The real versions are written by the implementer track `impl-win-apps` (branch `impl-win-apps`, spec-win-apps-1): when it
// merges, delete these names (and the group when it is empty).
poly_win32_rt::fail_exports!("kernelbase", win32, [LoadStringA, LoadStringW,]);

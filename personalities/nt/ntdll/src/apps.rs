//! The functions the Windows applications of the survey import from `ntdll.dll` that Poly does not implement
//! yet (ADR 0155, step 10): each is an export that logs `stub: ntdll!name` and fails as ADR 0155 fixes for a
//! function the system does not provide (`STATUS_NOT_IMPLEMENTED` for the functions that return an
//! `NTSTATUS`; 0 / NULL / nothing for the ones that return a pointer, a count or nothing). Generated from the
//! programs' import tables (names only) and our export table. `VerSetConditionMask` is real
//! (`poly_rtl::version`), and `memcpy`, `memmove`, `memset` and `memcmp` are `compiler_builtins`'
//! (`build.rs`).

use crate::debug_print;

/// `STATUS_NOT_IMPLEMENTED` (`ntstatus.h`).
const STATUS_NOT_IMPLEMENTED: u64 = 0xC000_0002;
/// `EINVAL` (`errno.h`).
const EINVAL: u64 = 22;

macro_rules! fail_exports {
    ($value:expr, [$($name:ident),* $(,)?]) => {$(
        #[doc = concat!("`", stringify!($name), "`: not implemented yet; fails (ADR 0155, step 10).")]
        #[unsafe(no_mangle)]
        pub extern "system" fn $name() -> u64 {
            debug_print(concat!("stub: ntdll!", stringify!($name)));
            $value
        }
    )*};
}

fail_exports!(
    STATUS_NOT_IMPLEMENTED,
    [
        AlpcInitializeMessageAttribute,
        DbgPrintEx,
        DbgPrompt,
        NtAllocateLocallyUniqueId,
        NtAllocateReserveObject,
        NtAlpcAcceptConnectPort,
        NtAlpcConnectPort,
        NtAlpcCreatePort,
        NtAlpcDisconnectPort,
        NtAlpcImpersonateClientOfPort,
        NtAlpcQueryInformation,
        NtAlpcSendWaitReceivePort,
        NtAssociateWaitCompletionPacket,
        NtCancelWaitCompletionPacket,
        NtCommitTransaction,
        NtCreateToken,
        NtCreateTransaction,
        NtCreateWaitCompletionPacket,
        NtLoadDriver,
        NtLockVirtualMemory,
        NtPowerInformation,
        NtQueryEaFile,
        NtQueryQuotaInformationFile,
        NtQueryTimerResolution,
        NtQueryWnfStateData,
        NtReadVirtualMemory,
        NtRollbackTransaction,
        NtSetEaFile,
        NtSetIoCompletionEx,
        NtSetQuotaInformationFile,
        NtSetVolumeInformationFile,
        NtUnloadDriver,
        NtUnlockVirtualMemory,
        RtlActivateActivationContextUnsafeFast,
        RtlCheckRegistryKey,
        RtlCreateQueryDebugBuffer,
        RtlDeactivateActivationContextUnsafeFast,
        RtlDosPathNameToNtPathName_U_WithStatus,
        RtlDuplicateUnicodeString,
        RtlIpv4AddressToStringExA,
        RtlIpv6AddressToStringExA,
        RtlQueryInformationActiveActivationContext,
        RtlQueryProcessDebugInformation,
        RtlQueryRegistryValues,
        RtlRunOnceExecuteOnce,
        RtlSetCurrentDirectory_U,
        RtlSleepConditionVariableSRW,
        RtlSubscribeWnfStateChangeNotification,
        RtlUnsubscribeWnfStateChangeNotification,
        RtlWaitOnAddress,
    ]
);

fail_exports!(
    0,
    [
        AlpcGetMessageAttribute,
        DbgBreakPoint,
        RtlAcquireSRWLockExclusive,
        RtlAcquireSRWLockShared,
        RtlClearBits,
        RtlCreateUnicodeStringFromAsciiz,
        RtlDeleteElementGenericTable,
        RtlDestroyQueryDebugBuffer,
        RtlEnumerateGenericTableWithoutSplaying,
        RtlGetCurrentTransaction,
        RtlInitializeBitMap,
        RtlInitializeGenericTable,
        RtlInsertElementGenericTable,
        RtlInterlockedFlushSList,
        RtlIsMultiSessionSku,
        RtlLookupElementGenericTable,
        RtlQueryPerformanceCounter,
        RtlReleaseActivationContext,
        RtlReleaseSRWLockExclusive,
        RtlReleaseSRWLockShared,
        RtlRunOnceInitialize,
        RtlSetBits,
        RtlSetCurrentTransaction,
        RtlUnicodeStringToAnsiSize,
        RtlUnwind,
        RtlWakeAddressSingle,
        RtlWakeAllConditionVariable,
        _wcsicmp,
        iswspace,
    ]
);

fail_exports!(0xFFFF_FFFF, [RtlFindClearBitsAndSet,]);

fail_exports!(EINVAL, [strcpy_s,]);

/// `VerSetConditionMask(ConditionMask, TypeMask, Condition)` (Learn: "VerSetConditionMask function").
#[unsafe(no_mangle)]
pub extern "system" fn VerSetConditionMask(mask: u64, type_mask: u32, condition: u8) -> u64 {
    poly_rtl::version::set_condition(mask, type_mask, condition)
}

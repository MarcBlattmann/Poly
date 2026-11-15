//! More names the Windows applications of the survey import from `hid.dll` (ADR 0155, step 10), fail-only like the
//! ones of `lib.rs` (same failure values); generated from the import tables of the programs and the coverage map.

poly_win32_rt::fail_exports!(
    "hid",
    value(0xC0110020),
    [
        HidP_GetData,
        HidP_GetExtendedAttributes,
        HidP_GetSpecificButtonCaps,
        HidP_GetSpecificValueCaps,
        HidP_GetUsageValueArray,
        HidP_GetUsages,
        HidP_InitializeReportForID,
        HidP_MaxDataListLength,
        HidP_MaxUsageListLength,
        HidP_SetData,
        HidP_SetScaledUsageValue,
        HidP_SetUsageValue,
        HidP_SetUsageValueArray,
        HidP_SetUsages,
        HidP_TranslateUsagesToI8042ScanCodes,
        HidP_UnsetUsages,
        HidP_UsageListDifference,
    ]
);

poly_win32_rt::fail_exports!(
    "hid",
    win32,
    [
        HidD_FlushQueue,
        HidD_GetConfiguration,
        HidD_GetFeature,
        HidD_GetHidGuid,
        HidD_GetIndexedString,
        HidD_GetInputReport,
        HidD_GetManufacturerString,
        HidD_GetMsGenreDescriptor,
        HidD_GetNumInputBuffers,
        HidD_GetPhysicalDescriptor,
        HidD_Hello,
        HidD_SetConfiguration,
        HidD_SetFeature,
        HidD_SetNumInputBuffers,
        HidD_SetOutputReport,
    ]
);

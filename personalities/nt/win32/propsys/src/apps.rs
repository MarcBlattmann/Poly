//! More names the Windows applications of the survey import from `propsys.dll` (ADR 0155, step 10), fail-only like the
//! ones of `lib.rs` (same failure values); generated from the import tables of the programs and the coverage map.

poly_win32_rt::fail_exports!(
    "propsys",
    hresult,
    [
        InitPropVariantFromBuffer,
        InitPropVariantFromFileTime,
        PSCoerceToCanonicalValue,
        PSEnumeratePropertyDescriptions,
        PSFormatForDisplayAlloc,
        PSGetPropertyDescription,
        PropVariantToBoolean,
        PropVariantToDouble,
        PropVariantToInt16,
        PropVariantToString,
        PropVariantToStringAlloc,
        PropVariantToStringWithDefault,
        PropVariantToUInt32,
        VariantToString,
    ]
);

poly_win32_rt::fail_exports!("propsys", value(0), [PropVariantCompareEx,]);

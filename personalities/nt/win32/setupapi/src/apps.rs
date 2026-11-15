//! More names the Windows applications of the survey import from `setupapi.dll` (ADR 0155, step 10), fail-only like the
//! ones of `lib.rs` (same failure values); generated from the import tables of the programs and the coverage map.

poly_win32_rt::fail_exports!("setupapi", invalid_handle, [SetupDiGetClassDevsA,]);

poly_win32_rt::fail_exports!("setupapi", win32, [SetupDiGetDeviceInterfaceDetailA, SetupDiGetDeviceRegistryPropertyW,]);

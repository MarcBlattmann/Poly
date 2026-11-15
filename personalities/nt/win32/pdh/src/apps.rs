//! More names the Windows applications of the survey import from `pdh.dll` (ADR 0155, step 10), fail-only like the
//! ones of `lib.rs` (same failure values); generated from the import tables of the programs and the coverage map.

poly_win32_rt::fail_exports!("pdh", value(0xC0000BD3), [PdhOpenQueryA,]);

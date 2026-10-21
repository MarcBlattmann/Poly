//! NtQuerySystemTime: succeeds, the time is a plausible count of 100 ns
//! intervals since 1601-01-01 UTC (after 2020, before 2100), and a second
//! query is not earlier than the first (Learn: "NtQuerySystemTime",
//! "ZwQuerySystemTime": "the current system time in GMT format").
#![no_std]
#![no_main]

use poly_wtest::*;

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtQuerySystemTime(system_time: *mut i64) -> NTSTATUS;
}

/// 2020-01-01 and 2100-01-01 00:00 UTC in 100 ns intervals since 1601-01-01.
const Y2020: i64 = 132_223_104_000_000_000;
const Y2100: i64 = 157_469_184_000_000_000;

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let mut first = 0i64;
    // SAFETY: `first` is a writable LARGE_INTEGER.
    let status = unsafe { NtQuerySystemTime(&mut first) };
    t.expect_status("query_succeeds", status, STATUS_SUCCESS);
    t.check("after_2020", first > Y2020, 0);
    t.check("before_2100", first < Y2100, 0);
    let mut second = 0i64;
    // SAFETY: as above.
    let status = unsafe { NtQuerySystemTime(&mut second) };
    t.expect_status("second_query_succeeds", status, STATUS_SUCCESS);
    t.check("not_earlier", second >= first, 0);
}

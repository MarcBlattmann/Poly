//! LookupPrivilegeValueW: the LUID (low part; the high part is 0) of each standard privilege
//! name, the unknown-name error, and case handling.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::fmt::Write as _;
use poly_wtest::*;

type BOOL = i32;
/// A fixed-size text buffer for formatting lines.
struct Buf<const N: usize> {
    b: [u8; N],
    n: usize,
}

impl<const N: usize> Buf<N> {
    const fn new() -> Self {
        Buf { b: [0; N], n: 0 }
    }
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.b[..self.n]).unwrap_or("?")
    }
}

impl<const N: usize> core::fmt::Write for Buf<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &c in s.as_bytes() {
            if self.n < N {
                self.b[self.n] = c;
                self.n += 1;
            }
        }
        Ok(())
    }
}

/// `t.line` with formatting.
macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<400>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

/// winnt.h: `LUID`.
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct LUID {
    low: u32,
    high: i32,
}

/// The last-error code after a failed call, as a plain number.
fn gle() -> u32 {
    // SAFETY: reads the calling thread's last error.
    unsafe { GetLastError() }
}

fn clear_error() {
    // SAFETY: sets the calling thread's last error.
    unsafe { SetLastError(0xDEAD) };
}

/// A NUL-terminated UTF-16 copy of an ASCII string (up to 255 units).
fn wide(s: &str) -> [u16; 256] {
    let mut w = [0u16; 256];
    for (i, c) in s.bytes().enumerate().take(255) {
        w[i] = u16::from(c);
    }
    w
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn LookupPrivilegeValueW(system: *const u16, name: *const u16, luid: *mut LUID) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let names = [
        "SeCreateTokenPrivilege",
        "SeAssignPrimaryTokenPrivilege",
        "SeLockMemoryPrivilege",
        "SeIncreaseQuotaPrivilege",
        "SeMachineAccountPrivilege",
        "SeTcbPrivilege",
        "SeSecurityPrivilege",
        "SeTakeOwnershipPrivilege",
        "SeLoadDriverPrivilege",
        "SeSystemProfilePrivilege",
        "SeSystemtimePrivilege",
        "SeProfileSingleProcessPrivilege",
        "SeIncreaseBasePriorityPrivilege",
        "SeCreatePagefilePrivilege",
        "SeCreatePermanentPrivilege",
        "SeBackupPrivilege",
        "SeRestorePrivilege",
        "SeShutdownPrivilege",
        "SeDebugPrivilege",
        "SeAuditPrivilege",
        "SeSystemEnvironmentPrivilege",
        "SeChangeNotifyPrivilege",
        "SeRemoteShutdownPrivilege",
        "SeUndockPrivilege",
        "SeSyncAgentPrivilege",
        "SeEnableDelegationPrivilege",
        "SeManageVolumePrivilege",
        "SeImpersonatePrivilege",
        "SeCreateGlobalPrivilege",
        "SeTrustedCredManAccessPrivilege",
        "SeRelabelPrivilege",
        "SeIncreaseWorkingSetPrivilege",
        "SeTimeZonePrivilege",
        "SeCreateSymbolicLinkPrivilege",
        "SeDelegateSessionUserImpersonatePrivilege",
        "sechangenotifyprivilege",
        "SECHANGENOTIFYPRIVILEGE",
        "SeChangeNotify",
        "SeFooPrivilege",
        "",
        "SeChangeNotifyPrivilege ",
        "SeTcbPrivilegeX",
    ];
    for n in names {
        let w = wide(n);
        let mut luid = LUID { low: 0xAA, high: 0x55 };
        clear_error();
        // SAFETY: NUL-terminated name, valid out pointer.
        let ok = unsafe { LookupPrivilegeValueW(core::ptr::null(), w.as_ptr(), &mut luid) };
        if ok != 0 {
            say!(t, "[{n}]: low={} high={}", luid.low, luid.high);
        } else {
            say!(t, "[{n}]: FAIL err={} luid={}:{}", gle(), luid.high, luid.low);
        }
    }
}

//! `explorer.exe` (ADR 0222): the stand-in of the shell process of an administrator's session.
//!
//! Poly's desktop is its own (ADR 0124), so there is no Windows shell. But Windows programs that
//! run elevated find the session's shell (`explorer.exe`) and start a process de-elevated with a
//! copy of its token: the shell holds the logon's filtered token (Learn: "CreateProcessWithTokenW";
//! the COM security sheets' de-elevated clients, Google's installer starting Chrome). The kernel
//! starts this program for the elevated logon and gives it the filtered token; it does nothing
//! and waits until the system ends. It imports only `ntdll.dll`.

#![no_std]
#![no_main]

use poly_win32_rt as _;

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    fn NtWaitForSingleObject(handle: isize, alertable: u8, timeout: *const i64) -> i32;
}

/// `NtCurrentProcess()`: a process is signalled when it ends, so waiting on itself waits forever.
const CURRENT_PROCESS: isize = -1;

#[unsafe(no_mangle)]
pub extern "system" fn explorer_main() -> ! {
    loop {
        // SAFETY: the current-process pseudo-handle and no timeout.
        unsafe { NtWaitForSingleObject(CURRENT_PROCESS, 0, core::ptr::null()) };
    }
}

//! `services.exe` (ADR 0221): the host of Poly's service control manager.
//!
//! The manager itself lives in the NT personality (`poly_scm`, `personalities/nt/src/scm.rs`);
//! what it cannot do from the call of the program that asked is to start a process of its own
//! (at boot, after a failure's delay) and to end one that did not connect in time. The kernel starts
//! this program from `C:\Windows\System32` when the manager needs it; it becomes LocalSystem in
//! session 0 on its first call and then loops: `PolyServiceCall(HOST)` starts the processes that
//! are due, as this process's children, and says how long it may wait; it waits on the event the
//! manager sets when there is more to do. It imports only `ntdll.dll`.

#![no_std]
#![no_main]

use poly_win32_rt as _;

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    fn PolyServiceCall(op: u32, input: *const u8, in_len: usize, out: *mut u8, out_len: usize, ret: *mut u32) -> i32;
    fn NtWaitForSingleObject(handle: isize, alertable: u8, timeout: *const i64) -> i32;
    fn NtTerminateProcess(handle: isize, status: i32) -> i32;
}

/// `poly_scm::wire::op::HOST`.
const OP_HOST: u32 = 48;

fn u64_at(b: &[u8], at: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(a)
}

#[unsafe(no_mangle)]
pub extern "system" fn services_main() -> ! {
    loop {
        // The reply: error code (4), the event handle (8), the milliseconds to wait (8).
        let mut out = [0u8; 20];
        let mut ret = 0u32;
        // SAFETY: an empty request and a 20-byte reply buffer that live for the call.
        let st = unsafe { PolyServiceCall(OP_HOST, core::ptr::null(), 0, out.as_mut_ptr(), out.len(), &mut ret) };
        if st < 0 || ret < 20 || out[..4] != [0; 4] {
            // Not (or no longer) the host: another one runs, or this one was not started as it.
            // SAFETY: ends this process.
            unsafe { NtTerminateProcess(-1, 1) };
            loop {}
        }
        let event = u64_at(&out, 4) as isize;
        let wait = u64_at(&out, 12);
        let timeout: i64 = -((wait.min(i64::MAX as u64 / 10_000) as i64) * 10_000);
        let limit = if wait == u64::MAX { core::ptr::null() } else { &timeout as *const i64 };
        // SAFETY: the manager's event in this process's table; the timeout lives for the call.
        unsafe { NtWaitForSingleObject(event, 0, limit) };
    }
}

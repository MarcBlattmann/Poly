//! `SEE_MASK_WAITFORINPUTIDLE` (ADR 0245; `specs/windows/shell32.dll/ShellExecuteExW.WaitForInputIdle.yaml`):
//! the wait is `poly_win32k::idle::shell_wait`'s; this is the system it asks.

use poly_win32k::idle::{self, IdleEnv};
use poly_win32k::proto::{Reply, Request};

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    fn NtQueryInformationProcess(process: usize, class: u32, info: *mut u8, length: u32, returned: *mut u32) -> i32;
}

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetTickCount64() -> u64;
    fn SleepEx(ms: u32, alertable: i32) -> u32;
}

#[cfg_attr(target_arch = "x86", link(name = "win32u", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "win32u", kind = "raw-dylib"))]
unsafe extern "system" {
    fn PolyUserCall(req: *const u8, len: usize, out: *mut u8, cap: usize, out_len: *mut usize) -> u32;
}

/// `SEE_MASK_WAITFORINPUTIDLE` (`shellapi.h`).
pub const SEE_MASK_WAITFORINPUTIDLE: u32 = 0x0200_0000;

/// `ProcessBasicInformation` (`winternl.h`): `PROCESS_BASIC_INFORMATION`'s exit status first,
/// `UniqueProcessId` after `PebBaseAddress`, `AffinityMask` and `BasePriority`.
const BASIC_SIZE: usize = 6 * core::mem::size_of::<usize>();
const PID_AT: usize = 4 * core::mem::size_of::<usize>();
/// `STATUS_PENDING` (`ntstatus.h`): the exit status of a process that runs.
const STATUS_PENDING: u32 = 0x103;
/// Poly's own class: the subsystem of the process's executable, a `u32`.
const PROCESS_POLY_SUBSYSTEM: u32 = 0x1003;

struct System;

impl IdleEnv for System {
    fn process(&self, h: u64) -> Option<(u32, bool)> {
        let mut b = [0u8; BASIC_SIZE];
        // SAFETY: a buffer of the structure's size for a process handle of ours.
        let st = unsafe {
            NtQueryInformationProcess(h as usize, 0, b.as_mut_ptr(), BASIC_SIZE as u32, core::ptr::null_mut())
        };
        if st != 0 {
            return None;
        }
        let exit = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        let pid = u32::from_le_bytes([b[PID_AT], b[PID_AT + 1], b[PID_AT + 2], b[PID_AT + 3]]);
        Some((pid, exit == STATUS_PENDING))
    }

    fn subsystem(&self, h: u64) -> Option<u16> {
        let mut b = [0u8; 4];
        // SAFETY: a 4-byte buffer for the 4-byte answer.
        let st = unsafe {
            NtQueryInformationProcess(h as usize, PROCESS_POLY_SUBSYSTEM, b.as_mut_ptr(), 4, core::ptr::null_mut())
        };
        (st == 0).then(|| u32::from_le_bytes(b) as u16)
    }

    fn idle_state(&self, pid: u32) -> u8 {
        let req = poly_win32k::wire::encode(&Request::IdleState { pid });
        let mut out = [0u8; 64];
        let mut len = 0usize;
        // SAFETY: the buffers are ours and sized as passed.
        if unsafe { PolyUserCall(req.as_ptr(), req.len(), out.as_mut_ptr(), out.len(), &mut len) } != 0 {
            return 0;
        }
        match poly_win32k::wire::decode(&out[..len.min(out.len())]) {
            Some(Reply::Value { value }) => value as u8,
            _ => 0,
        }
    }

    fn now_ms(&self) -> u64 {
        // SAFETY: a plain call.
        unsafe { GetTickCount64() }
    }

    fn sleep(&self, ms: u32) {
        // SAFETY: a plain call.
        unsafe { SleepEx(ms, 0) };
    }
}

/// Wait for process `h`, which the call just started, as the mask asks.
pub fn wait(h: usize) {
    idle::shell_wait(&System, h as u64);
}

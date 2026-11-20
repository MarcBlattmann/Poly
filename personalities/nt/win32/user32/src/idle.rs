//! `WaitForInputIdle` (ADR 0245; `specs/windows/user32.dll/WaitForInputIdle.yaml`): the rules are
//! `poly_win32k::idle`'s; this is the system side, and the once-per-process report of this
//! process's own idle point to win32k.

use core::sync::atomic::{AtomicU8, Ordering};
use poly_win32_gui::set_last_error;
use poly_win32k::idle::{self, IdleEnv};
use poly_win32k::proto::{Reply, Request};
use poly_win32k::user::Env;

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

/// `ProcessBasicInformation` (`winternl.h`) and the 64- and 32-bit layouts of
/// `PROCESS_BASIC_INFORMATION`: the exit status first, `UniqueProcessId` after `PebBaseAddress`,
/// `AffinityMask` and `BasePriority`.
const PROCESS_BASIC_INFORMATION: u32 = 0;
const BASIC_SIZE: usize = 6 * core::mem::size_of::<usize>();
const PID_AT: usize = 4 * core::mem::size_of::<usize>();
/// `STATUS_PENDING` (`ntstatus.h`): the exit status of a process that runs.
const STATUS_PENDING: u32 = 0x103;
/// Poly's own class: the subsystem of the process's executable, a `u32` (the NT personality's
/// `PROCESS_POLY_SUBSYSTEM`).
const PROCESS_POLY_SUBSYSTEM: u32 = 0x1003;

/// The bits this process has told win32k already ([`idle::BLOCKED`], [`idle::LOOKED`]).
static TOLD: AtomicU8 = AtomicU8::new(0);

/// A thread of this process found its queue empty: tell win32k, once of each kind.
pub fn note(sys: &impl Env, blocking: bool) {
    let bits = if blocking { idle::BLOCKED | idle::LOOKED } else { idle::LOOKED };
    if TOLD.fetch_or(bits, Ordering::Relaxed) & bits != bits {
        sys.call(Request::InputIdle { blocking });
    }
}

/// The system for `poly_win32k::idle`'s waits.
pub struct System<'a, E: Env>(pub &'a E);

impl<E: Env> IdleEnv for System<'_, E> {
    fn process(&self, h: u64) -> Option<(u32, bool)> {
        let mut b = [0u8; BASIC_SIZE];
        // SAFETY: a buffer of the structure's size; any handle value is checked by the call.
        let st = unsafe {
            NtQueryInformationProcess(
                h as usize,
                PROCESS_BASIC_INFORMATION,
                b.as_mut_ptr(),
                BASIC_SIZE as u32,
                core::ptr::null_mut(),
            )
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
        match self.0.call(Request::IdleState { pid }) {
            Reply::Value { value } => value as u8,
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

/// `WaitForInputIdle(hProcess, dwMilliseconds)`: 0 once the process has waited for input (or
/// ended during the wait), `WAIT_TIMEOUT`, or `WAIT_FAILED` (`ERROR_INVALID_HANDLE`,
/// `ERROR_NOT_GUI_PROCESS`, or an ended process with the last error left alone).
#[unsafe(no_mangle)]
pub extern "system" fn WaitForInputIdle(process: usize, ms: u32) -> u32 {
    let (r, error) = idle::wait_for_input_idle(&System(&crate::SYS), process as u64, ms);
    if let Some(e) = error {
        set_last_error(e);
    }
    r
}

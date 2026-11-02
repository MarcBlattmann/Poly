//! win32k: table 1 of the NT system calls (ADR 0128).
//!
//! Windows GUI apps reach win32k through Poly's own `PolyUser*` /
//! `PolyGdi*` services (`poly_win32k::services`), whose stubs are in our
//! `win32u.dll` and which our `user32.dll` and `gdi32.dll` call. This
//! module dispatches them: it checks the calling process's win32k lockdown
//! (`DisallowWin32kSystemCalls`, [`NtProcess::win32k_disabled`]), then runs
//! the service. The logic (windows, queues, timers, hooks, input, the
//! desktop link to the compositor) is `poly-win32k`'s; the services decode
//! arguments, copy user memory and block through [`NtKernel`].
//!
//! **No service is implemented yet**: each waits for the spec sheets of its
//! slice and answers `STATUS_NOT_IMPLEMENTED`, reported to the kernel as
//! [`Outcome::Unimplemented`] so it is logged with its name
//! (`syscall: pid N nt/win32k #i PolyUser... not implemented`). The match in
//! [`run`] names every service, so a new one cannot be forgotten.
//!
//! [`NtProcess::win32k_disabled`]: crate::system::NtProcess::win32k_disabled

use crate::kernel::NtKernel;
use crate::{STATUS_NOT_IMPLEMENTED, Table, decode};
use poly_abi::{Outcome, SyscallRegs};
use poly_ntobj::NtStatus;
use poly_win32k::desktop::Desktop;
use poly_win32k::services::Win32kService;

/// What a process under the win32k lockdown gets for every table-1 call.
/// Provisional: Learn says only that such a process "is not permitted to
/// perform GUI system calls"; what Windows returns is to be confirmed by
/// the spec agent against real Windows (ADR 0128).
pub const LOCKDOWN_STATUS: NtStatus = poly_ntobj::status::STATUS_ACCESS_DENIED;

/// win32k's machine-wide state, in [`crate::NtSystem`].
#[derive(Debug, Default)]
pub struct Win32k {
    /// The desktop link's win32k end: the surfaces of top-level windows
    /// as the compositor is told about them.
    pub desktop: Desktop,
}

/// Why a service did not run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Known, but waiting for its slice's spec sheets.
    NotImplemented(Win32kService),
}

/// The name of win32k service `index` (the low 12 bits of its number), for
/// the kernel's log of unimplemented calls.
pub fn service_name(index: u64) -> Option<&'static str> {
    let number = (poly_win32k::services::TABLE_WIN32K << 12) | (index as u32 & 0xfff);
    Win32kService::from_number(number).map(Win32kService::name)
}

/// Run the table-1 call in `regs` against `k`, its `NTSTATUS` into `rax`.
pub fn dispatch<K: NtKernel>(k: &mut K, regs: &mut SyscallRegs) -> Outcome {
    let number = regs.rax as u32;
    let (table, index) = decode(number);
    debug_assert_eq!(table, Table::Win32k);
    if k.with_state(|_, p, _| p.win32k_disabled) {
        regs.rax = u64::from(LOCKDOWN_STATUS.0);
        return Outcome::Handled;
    }
    let unimplemented = |regs: &mut SyscallRegs| {
        regs.rax = u64::from(STATUS_NOT_IMPLEMENTED);
        Outcome::Unimplemented { table: Table::Win32k.name(), number: u64::from(index) }
    };
    let Some(service) = Win32kService::from_number(number) else {
        return unimplemented(regs);
    };
    match run(k, regs, service) {
        Ok(status) => {
            regs.rax = u64::from(status.0);
            Outcome::Handled
        }
        Err(Refused::NotImplemented(_)) => unimplemented(regs),
    }
}

/// A service whose slice has not arrived.
fn not_implemented(service: Win32kService) -> Result<NtStatus, Refused> {
    Err(Refused::NotImplemented(service))
}

fn run<K: NtKernel>(_k: &mut K, _regs: &SyscallRegs, service: Win32kService) -> Result<NtStatus, Refused> {
    use Win32kService::*;
    match service {
        // Classes and windows.
        RegisterClass | UnregisterClass | GetClassInfo | CreateWindow | DestroyWindow | ShowWindow | SetWindowPos
        | QueryWindow | GetWindowLong | SetWindowLong | SetWindowText => not_implemented(service),
        // Messages and queues.
        PostMessage
        | PostThreadMessage
        | SendMessage
        | GetMessage
        | PeekMessage
        | ReplyMessage
        | PostQuitMessage
        | WaitMessage
        | MsgWaitForMultipleObjects
        | GetQueueStatus
        | CalloutReturn => not_implemented(service),
        // Input.
        SetFocus | SetActiveWindow | SetForegroundWindow | SetCapture | ReleaseCapture | GetKeyState
        | GetAsyncKeyState | GetKeyboardState | GetCursorPos | SetCursor => not_implemented(service),
        // Timers and hooks.
        SetTimer | KillTimer | SetWindowsHook | UnhookWindowsHook => not_implemented(service),
        // Painting and surfaces.
        BeginPaint | EndPaint | InvalidateRect | ValidateRect | GetUpdateRect | GetDC | ReleaseDC | CommitSurface => {
            not_implemented(service)
        }
        // GDI.
        QueryDisplay => not_implemented(service),
    }
}

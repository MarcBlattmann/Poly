//! `SetWinEventHook`, `UnhookWinEvent` and `NotifyWinEvent` over `poly_win32k::winevent`: the
//! hooks of this process (out-of-context), events raised by the window functions and by
//! `NotifyWinEvent`, callbacks run by the installing thread when it looks into its queue.

use super::{H, SYS, info};
use poly_win32_gui::{SpinLock, set_last_error};
use poly_win32k::user::Env;
use poly_win32k::winevent::Table;

type EventProc = unsafe extern "system" fn(usize, u32, usize, i32, i32, u32, u32);

static TABLE: SpinLock<Table> = SpinLock::new(Table::new());

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetCurrentProcessId() -> u32;
    fn OpenThread(access: u32, inherit: i32, tid: u32) -> usize;
    fn CloseHandle(h: usize) -> i32;
}

/// Raise `event` for window `hwnd` (0: none of ours): queued for every hook that wants it.
pub fn raise(event: u32, hwnd: u32, id_object: i32, id_child: i32) {
    if !TABLE.with(|t| t.any()) {
        return;
    }
    let (thread, pid) = match info(hwnd as H) {
        Some(i) if hwnd != 0 => (i.tid, i.pid),
        // SAFETY: no preconditions.
        _ => (SYS.tid(), unsafe { GetCurrentProcessId() }),
    };
    let time = SYS.tick();
    TABLE.with(|t| t.raise(event, u64::from(hwnd), id_object, id_child, thread, pid, time));
}

/// Run the callbacks queued for the calling thread's hooks, one after the other; a callback may
/// remove hooks (what is queued for them goes).
pub fn pump() {
    let tid = SYS.tid();
    loop {
        let Some((hook, q)) = TABLE.with(|t| if t.pending_for(tid) { t.next_for(tid) } else { None }) else { return };
        // SAFETY: a procedure the app registered.
        let f: EventProc = unsafe { core::mem::transmute(hook.proc_ as usize) };
        // SAFETY: the documented call.
        unsafe { f(q.hook as usize, q.event, q.hwnd as usize, q.id_object, q.id_child, q.thread, q.time) };
    }
}

#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn SetWinEventHook(
    min: u32,
    max: u32,
    module: usize,
    proc_: usize,
    pid: u32,
    tid: u32,
    flags: u32,
) -> usize {
    // A thread that does not exist makes the call fail (SetWinEventHook.yaml `params`).
    let thread_exists = tid == 0 || tid == SYS.tid() || {
        // SAFETY: plain kernel32 calls; THREAD_QUERY_LIMITED_INFORMATION is 0x800.
        let h = unsafe { OpenThread(0x800, 0, tid) };
        if h != 0 {
            // SAFETY: the handle just opened.
            unsafe { CloseHandle(h) };
        }
        h != 0
    };
    // SAFETY: no preconditions.
    let own_pid = unsafe { GetCurrentProcessId() };
    let r = TABLE
        .with(|t| t.install(min, max, module as u64, proc_ as u64, pid, tid, flags, SYS.tid(), own_pid, thread_exists));
    match r {
        Ok(h) => h as usize,
        Err(e) => {
            set_last_error(e);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn UnhookWinEvent(hook: usize) -> i32 {
    match TABLE.with(|t| t.remove(hook as u64, SYS.tid())) {
        Ok(()) => 1,
        Err(e) => {
            set_last_error(e);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn NotifyWinEvent(event: u32, hwnd: H, id_object: i32, id_child: i32) {
    // A window that is none (NULL, made up, destroyed) is no object: the call reports it, nothing is raised.
    let Some(i) = info(hwnd) else {
        set_last_error(1400);
        return;
    };
    // The desktop window gives no callback.
    if i.desktop {
        return;
    }
    raise(event, hwnd as u32, id_object, id_child);
}

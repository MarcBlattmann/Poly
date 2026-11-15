//! `SetWindowsHookExW`, `UnhookWindowsHookEx` and `CallNextHookEx` for the hooks of this process
//! (`poly_win32k::hooks`): the procedures run on the thread whose message they watch, the
//! chain of one call is a frame on that thread's stack, and `CallNextHookEx` continues it.

use super::{H, SYS};
use alloc::vec::Vec;
use poly_win32_gui::{SpinLock, set_last_error};
use poly_win32k::hooks::{self as h, Table};
use poly_win32k::user::{CMsg, Env};

type HookProc = unsafe extern "system" fn(i32, usize, isize) -> isize;

static TABLE: SpinLock<Table> = SpinLock::new(Table::new());

/// The chain of one hook call that is running on a thread: the procedures still to run.
struct Frame {
    tid: u32,
    rest: Vec<u64>,
}

static FRAMES: SpinLock<Vec<Frame>> = SpinLock::new(Vec::new());

/// `CWPSTRUCT` (winuser.h), what a `WH_CALLWNDPROC` hook's `lParam` points at.
#[repr(C)]
struct CwpStruct {
    l: isize,
    w: usize,
    message: u32,
    hwnd: usize,
}

/// Run the hooks of `id` for the calling thread: the first procedure of the chain is called, and
/// it reaches the others through `CallNextHookEx`. `None` when no hook applies.
pub fn run(id: i32, code: i32, w: usize, l: isize) -> Option<isize> {
    let tid = SYS.tid();
    let chain = TABLE.with(|t| if t.any(id) { t.chain(id, tid) } else { Vec::new() });
    let (first, rest) = chain.split_first()?;
    FRAMES.with(|f| f.push(Frame { tid, rest: rest.to_vec() }));
    // SAFETY: a hook procedure the app registered.
    let f: HookProc = unsafe { core::mem::transmute(*first as usize) };
    // SAFETY: the documented call.
    let r = unsafe { f(code, w, l) };
    FRAMES.with(|f| {
        if let Some(i) = f.iter().rposition(|fr| fr.tid == tid) {
            f.remove(i);
        }
    });
    Some(r)
}

/// `WH_CALLWNDPROC`: a message is sent to a window procedure.
pub fn sent(hwnd: u32, msg: u32, w: u64, l: u64, same_thread: bool) {
    if !TABLE.with(|t| t.any(h::WH_CALLWNDPROC)) {
        return;
    }
    let cwp = CwpStruct { l: l as isize, w: w as usize, message: msg, hwnd: hwnd as usize };
    run(h::WH_CALLWNDPROC, h::HC_ACTION, usize::from(same_thread), &cwp as *const CwpStruct as isize);
}

/// `WH_GETMESSAGE`: a message was retrieved; the hook may change it.
pub fn retrieved(msg: &mut CMsg, remove: bool) {
    if !TABLE.with(|t| t.any(h::WH_GETMESSAGE)) {
        return;
    }
    let (code, w) = if remove { (h::HC_ACTION, 1) } else { (h::HC_NOREMOVE, 0) };
    run(h::WH_GETMESSAGE, code, w, msg as *mut CMsg as isize);
}

#[unsafe(no_mangle)]
pub extern "system" fn SetWindowsHookExW(id: i32, proc_: usize, module: H, tid: u32) -> H {
    match TABLE.with(|t| t.install(id, proc_ as u64, module as u64, tid)) {
        Ok(handle) => handle as H,
        Err(e) => {
            set_last_error(e);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn UnhookWindowsHookEx(hook: H) -> i32 {
    if TABLE.with(|t| t.remove(hook as u64)) {
        1
    } else {
        set_last_error(h::ERROR_INVALID_HOOK_HANDLE);
        0
    }
}

/// `CallNextHookEx`: the next procedure of the chain that is running on this thread; at the end
/// of the chain, or outside any hook, 0.
#[unsafe(no_mangle)]
pub extern "system" fn CallNextHookEx(_hook: H, code: i32, w: usize, l: isize) -> isize {
    let tid = SYS.tid();
    let next = FRAMES.with(|f| {
        let fr = f.iter_mut().rev().find(|fr| fr.tid == tid)?;
        (!fr.rest.is_empty()).then(|| fr.rest.remove(0))
    });
    let Some(next) = next else { return 0 };
    // SAFETY: a hook procedure the app registered.
    let f: HookProc = unsafe { core::mem::transmute(next as usize) };
    // SAFETY: the documented call.
    unsafe { f(code, w, l) }
}

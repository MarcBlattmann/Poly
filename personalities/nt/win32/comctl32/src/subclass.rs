//! Window subclassing (`commctrl.h`; Learn, "Subclassing Controls"): `SetWindowSubclass` (ordinal
//! 410), `GetWindowSubclass` (411), `RemoveWindowSubclass` (412) and `DefSubclassProc` (413), from
//! `specs/windows/comctl32.dll/{SetWindowSubclass,GetWindowSubclass,RemoveWindowSubclass,DefSubclassProc}.yaml`.
//!
//! Each subclassed window has a chain in this process: the window procedure it had before, and
//! its subclasses in the order they were set (a pair set again keeps its place). The first
//! subclass makes [`dispatch`] the window's procedure; it runs the chain newest first. Each run
//! of the chain is a frame on its thread's stack of frames, which remembers which subclass is
//! running, so that `DefSubclassProc` calls the next older one: one that was removed meanwhile is
//! skipped, one set meanwhile is newer than the run and not called by it.

use alloc::vec::Vec;
use poly_win32_gui::SpinLock;

#[cfg_attr(target_arch = "x86", link(name = "user32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "user32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetWindowLongPtrW(hwnd: usize, index: i32) -> isize;
    fn SetWindowLongPtrW(hwnd: usize, index: i32, value: isize) -> isize;
    fn CallWindowProcW(proc_: usize, hwnd: usize, msg: u32, w: usize, l: isize) -> isize;
    fn GetWindowThreadProcessId(hwnd: usize, pid: *mut u32) -> u32;
}

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetCurrentThreadId() -> u32;
    fn GetCurrentProcessId() -> u32;
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

/// `SUBCLASSPROC`: `LRESULT CALLBACK (HWND, UINT, WPARAM, LPARAM, UINT_PTR uIdSubclass, DWORD_PTR dwRefData)`.
type SubclassProc = unsafe extern "system" fn(usize, u32, usize, isize, usize, usize) -> isize;

/// `GWLP_WNDPROC` (`winuser.h`).
const GWLP_WNDPROC: i32 = -4;
/// `WM_NCDESTROY` (`winuser.h`).
const WM_NCDESTROY: u32 = 0x0082;

/// One subclass: the pair that names it, its reference data, and where it is in the chain.
#[derive(Clone, Copy)]
struct Entry {
    proc_: usize,
    id: usize,
    data: usize,
    /// Larger is newer.
    order: u64,
}

/// A window's chain.
struct Chain {
    hwnd: usize,
    /// The procedure the window had before its first subclass.
    original: usize,
    entries: Vec<Entry>,
}

/// A run of a window's chain on a thread: the subclass running now (`order`; `u64::MAX` before
/// the first, 0 while the window procedure runs).
#[derive(Clone, Copy)]
struct Frame {
    /// Names the run (frames of other threads come and go around it).
    serial: u64,
    thread: u32,
    hwnd: usize,
    order: u64,
}

struct State {
    chains: Vec<Chain>,
    frames: Vec<Frame>,
    next_order: u64,
    next_serial: u64,
}

impl State {
    fn frame(&mut self, serial: u64) -> Option<&mut Frame> {
        self.frames.iter_mut().find(|f| f.serial == serial)
    }
}

static STATE: SpinLock<State> =
    SpinLock::new(State { chains: Vec::new(), frames: Vec::new(), next_order: 1, next_serial: 1 });

/// Run `f` with the last error put back afterwards: none of these functions sets it (the sheets'
/// `errors`), whatever the user32 calls they make do.
fn keep_last_error<R>(f: impl FnOnce() -> R) -> R {
    // SAFETY: no arguments.
    let saved = unsafe { GetLastError() };
    let r = f();
    // SAFETY: as above.
    unsafe { SetLastError(saved) };
    r
}

/// The thread of `hwnd` when it is a window of this process (0: none).
fn own_window_thread(hwnd: usize) -> u32 {
    if hwnd == 0 {
        return 0;
    }
    let mut pid = 0u32;
    // SAFETY: a plain user32 call with an out pointer.
    let tid = unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    // SAFETY: no arguments.
    if tid == 0 || pid != unsafe { GetCurrentProcessId() } { 0 } else { tid }
}

/// `SetWindowSubclass(hWnd, pfnSubclass, uIdSubclass, dwRefData)` (ordinal 410).
#[unsafe(no_mangle)]
pub extern "system" fn SetWindowSubclass(hwnd: usize, proc_: usize, id: usize, data: usize) -> i32 {
    keep_last_error(|| {
        if proc_ == 0 {
            return 0;
        }
        let thread = own_window_thread(hwnd);
        // SAFETY: no arguments.
        if thread == 0 || thread != unsafe { GetCurrentThreadId() } {
            return 0;
        }
        let first = STATE.with(|s| {
            let order = s.next_order;
            s.next_order += 1;
            match s.chains.iter_mut().find(|c| c.hwnd == hwnd) {
                Some(c) => {
                    // A chain left without subclasses while a run was in flight gave the window
                    // its own procedure back: the dispatcher goes in again.
                    let again = c.entries.is_empty();
                    match c.entries.iter_mut().find(|e| e.proc_ == proc_ && e.id == id) {
                        Some(e) => e.data = data,
                        None => c.entries.push(Entry { proc_, id, data, order }),
                    }
                    again
                }
                None => {
                    s.chains.push(Chain { hwnd, original: 0, entries: alloc::vec![Entry { proc_, id, data, order }] });
                    true
                }
            }
        });
        if first {
            // SAFETY: the window is this thread's; the dispatcher is a window procedure.
            let original = unsafe { SetWindowLongPtrW(hwnd, GWLP_WNDPROC, dispatch as *const () as isize) } as usize;
            STATE.with(|s| {
                if let Some(c) = s.chains.iter_mut().find(|c| c.hwnd == hwnd) {
                    c.original = original;
                }
            });
        }
        1
    })
}

/// `GetWindowSubclass(hWnd, pfnSubclass, uIdSubclass, pdwRefData)` (ordinal 411): from any
/// thread; `*pdwRefData` is 0 when the pair is not set.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetWindowSubclass(hwnd: usize, proc_: usize, id: usize, data: *mut usize) -> i32 {
    keep_last_error(|| forget_if_gone(hwnd));
    let found = STATE.with(|s| {
        s.chains
            .iter()
            .find(|c| c.hwnd == hwnd)
            .and_then(|c| c.entries.iter().find(|e| e.proc_ == proc_ && e.id == id).map(|e| e.data))
    });
    if !data.is_null() {
        // SAFETY: the caller's DWORD_PTR.
        unsafe { data.write_unaligned(found.unwrap_or(0)) };
    }
    i32::from(found.is_some())
}

/// `RemoveWindowSubclass(hWnd, pfnSubclass, uIdSubclass)` (ordinal 412): from any thread, also
/// from inside the chain (the removed subclass is not called any more, even by the run in
/// flight). The last one gone, the window gets its own procedure back if the dispatcher is still
/// its procedure.
#[unsafe(no_mangle)]
pub extern "system" fn RemoveWindowSubclass(hwnd: usize, proc_: usize, id: usize) -> i32 {
    keep_last_error(|| {
        forget_if_gone(hwnd);
        let removed = STATE.with(|s| {
            let i = s.chains.iter().position(|c| c.hwnd == hwnd)?;
            let c = &mut s.chains[i];
            let at = c.entries.iter().position(|e| e.proc_ == proc_ && e.id == id)?;
            c.entries.remove(at);
            // A chain without subclasses stays while one of its runs is in flight (the run still
            // has to reach the window procedure); `dispatch` drops it afterwards.
            let empty = c.entries.is_empty();
            let original = c.original;
            if empty && !s.frames.iter().any(|f| f.hwnd == hwnd) {
                s.chains.remove(i);
            }
            Some((empty, original))
        });
        match removed {
            None => 0,
            Some((empty, original)) => {
                if empty {
                    restore(hwnd, original);
                }
                1
            }
        }
    })
}

/// Drop the chain of a window that no longer exists (destroyed without its `WM_NCDESTROY`
/// going through the dispatcher: the program had replaced the window procedure).
fn forget_if_gone(hwnd: usize) {
    if own_window_thread(hwnd) == 0 {
        STATE.with(|s| s.chains.retain(|c| c.hwnd != hwnd));
    }
}

/// Give `hwnd` its own procedure back when the dispatcher is still its procedure.
fn restore(hwnd: usize, original: usize) {
    // SAFETY: plain user32 calls on a window of this process.
    unsafe {
        if GetWindowLongPtrW(hwnd, GWLP_WNDPROC) as usize == dispatch as *const () as usize {
            SetWindowLongPtrW(hwnd, GWLP_WNDPROC, original as isize);
        }
    }
}

/// Call what comes after the subclass `order` in `hwnd`'s chain for the run `frame` (its
/// serial): the next older subclass, or the window procedure after the oldest.
fn call_next(frame: u64, hwnd: usize, order: u64, msg: u32, w: usize, l: isize) -> isize {
    enum Next {
        Sub(Entry),
        Window(usize),
        Gone,
    }
    let next = STATE.with(|s| {
        let Some(c) = s.chains.iter().find(|c| c.hwnd == hwnd) else { return Next::Gone };
        match c.entries.iter().filter(|e| e.order < order).max_by_key(|e| e.order) {
            Some(e) => Next::Sub(*e),
            None => Next::Window(c.original),
        }
    });
    let set_frame = |o: u64| {
        STATE.with(|s| {
            if let Some(f) = s.frame(frame) {
                f.order = o;
            }
        })
    };
    match next {
        Next::Sub(e) => {
            set_frame(e.order);
            // SAFETY: a SUBCLASSPROC the program registered with SetWindowSubclass.
            let r = unsafe { core::mem::transmute::<usize, SubclassProc>(e.proc_)(hwnd, msg, w, l, e.id, e.data) };
            set_frame(order);
            r
        }
        Next::Window(original) => {
            set_frame(0);
            // SAFETY: the window's own procedure, called as user32 calls window procedures.
            let r = unsafe { CallWindowProcW(original, hwnd, msg, w, l) };
            set_frame(order);
            r
        }
        Next::Gone => 0,
    }
}

/// The window procedure of a subclassed window: one run of its chain, newest first. After
/// `WM_NCDESTROY` has gone through it the chain is dropped.
unsafe extern "system" fn dispatch(hwnd: usize, msg: u32, w: usize, l: isize) -> isize {
    // SAFETY: no arguments.
    let thread = unsafe { GetCurrentThreadId() };
    let frame = STATE.with(|s| {
        let serial = s.next_serial;
        s.next_serial += 1;
        s.frames.push(Frame { serial, thread, hwnd, order: u64::MAX });
        serial
    });
    let r = call_next(frame, hwnd, u64::MAX, msg, w, l);
    let drop_chain = STATE.with(|s| {
        s.frames.retain(|f| f.serial != frame);
        let idle = !s.frames.iter().any(|f| f.hwnd == hwnd);
        let i = s.chains.iter().position(|c| c.hwnd == hwnd)?;
        if idle && (msg == WM_NCDESTROY || s.chains[i].entries.is_empty()) {
            let c = s.chains.remove(i);
            return Some((msg == WM_NCDESTROY, c.original));
        }
        None
    });
    if let Some((destroyed, original)) = drop_chain
        && !destroyed
    {
        keep_last_error(|| restore(hwnd, original));
    }
    r
}

/// `DefSubclassProc(hWnd, uMsg, wParam, lParam)` (ordinal 413): inside a subclass procedure, the
/// next older subclass or the window procedure, with the arguments given; 0 outside.
#[unsafe(no_mangle)]
pub extern "system" fn DefSubclassProc(hwnd: usize, msg: u32, w: usize, l: isize) -> isize {
    // SAFETY: no arguments.
    let thread = unsafe { GetCurrentThreadId() };
    let found = STATE.with(|s| {
        let f = s.frames.iter().rev().find(|f| f.thread == thread && f.hwnd == hwnd)?;
        // Before the first subclass or in the window procedure: not inside a subclass procedure.
        (f.order != u64::MAX && f.order != 0).then_some((f.serial, f.order))
    });
    match found {
        Some((frame, order)) => call_next(frame, hwnd, order, msg, w, l),
        None => 0,
    }
}

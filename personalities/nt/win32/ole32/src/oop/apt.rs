//! Running a call in an apartment (ADR 0220): an STA's calls are posted to a message-only window
//! its thread owns and run when it pumps messages; the MTA's run on a thread of the MTA (a new one
//! when the caller is not in it). The caller waits; a caller in an STA pumps its own COM window
//! meanwhile, so that calls into its apartment still run.

use super::*;
use crate::current_owner;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};
use poly_com::git::Owner;

/// The message a posted call arrives as (`WM_APP` + 0x3C0, private to the window class).
const WM_COM_CALL: u32 = 0x8000 + 0x3C0;
/// `HWND_MESSAGE`: the parent of message-only windows (`winuser.h`).
const HWND_MESSAGE: isize = -3;
/// `PM_REMOVE`.
const PM_REMOVE: u32 = 1;

/// The COM window of each STA thread that has one: (thread id, window).
static WINDOWS: Locked<Vec<(u32, usize)>> = Locked::new(Vec::new());

/// The functions of `user32.dll` used here, looked up on first use.
static USER32: [AtomicUsize; 7] = [const { AtomicUsize::new(0) }; 7];
const NAMES: [&[u8]; 7] = [
    b"RegisterClassW\0",
    b"CreateWindowExW\0",
    b"DefWindowProcW\0",
    b"PostMessageW\0",
    b"PeekMessageW\0",
    b"DispatchMessageW\0",
    b"DestroyWindow\0",
];
const REGISTER_CLASS: usize = 0;
const CREATE_WINDOW: usize = 1;
const DEF_WINDOW_PROC: usize = 2;
const POST_MESSAGE: usize = 3;
const PEEK_MESSAGE: usize = 4;
const DISPATCH_MESSAGE: usize = 5;
const DESTROY_WINDOW: usize = 6;

fn user32(which: usize) -> usize {
    let p = USER32[which].load(Ordering::Acquire);
    if p != 0 {
        return p;
    }
    let lib = wide("user32.dll");
    // SAFETY: kernel32 calls with NUL-terminated names.
    let p = unsafe { crate::GetProcAddress(crate::LoadLibraryW(lib.as_ptr()), NAMES[which].as_ptr()) };
    USER32[which].store(p, Ordering::Release);
    p
}

/// A call waiting to run in another apartment.
struct Job<'a> {
    call: &'a mut dyn FnMut(),
    done: Handle,
}

/// `WNDCLASSW` (`winuser.h`).
#[repr(C)]
struct WndClass {
    style: u32,
    proc_: unsafe extern "system" fn(usize, u32, usize, isize) -> isize,
    cls_extra: i32,
    wnd_extra: i32,
    instance: usize,
    icon: usize,
    cursor: usize,
    background: usize,
    menu: *const u16,
    class: *const u16,
}

/// `MSG` (`winuser.h`).
#[repr(C)]
struct Msg {
    hwnd: usize,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    pt: [i32; 2],
    private: u32,
}

const CLASS_NAME: &str = "PolyComApartment";
static CLASS_ATOM: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn com_window_proc(hwnd: usize, msg: u32, wp: usize, lp: isize) -> isize {
    if msg == WM_COM_CALL && lp != 0 {
        // SAFETY: a `Job` the poster keeps alive until `done` is set.
        unsafe {
            let job = &mut *(lp as *mut Job<'static>);
            (job.call)();
            SetEvent(job.done);
        }
        return 0;
    }
    let p = user32(DEF_WINDOW_PROC);
    if p == 0 {
        return 0;
    }
    // SAFETY: user32's DefWindowProcW.
    unsafe {
        core::mem::transmute::<usize, unsafe extern "system" fn(usize, u32, usize, isize) -> isize>(p)(
            hwnd, msg, wp, lp,
        )
    }
}

/// The current thread's COM window, if it has one.
fn my_window() -> Option<usize> {
    let me = crate::thread_id();
    WINDOWS.with(|w| w.iter().find(|e| e.0 == me).map(|e| e.1))
}

/// Make the current STA thread's COM window (once): calls from other apartments and processes
/// into this apartment are posted to it.
pub(crate) fn ensure_window() {
    if my_window().is_some() {
        return;
    }
    // SAFETY: user32's functions on our own class and window.
    unsafe {
        let module = GetModuleHandleW(core::ptr::null());
        let class = wide(CLASS_NAME);
        if CLASS_ATOM.load(Ordering::Acquire) == 0 {
            let reg = user32(REGISTER_CLASS);
            if reg == 0 {
                return;
            }
            let wc = WndClass {
                style: 0,
                proc_: com_window_proc,
                cls_extra: 0,
                wnd_extra: 0,
                instance: module,
                icon: 0,
                cursor: 0,
                background: 0,
                menu: core::ptr::null(),
                class: class.as_ptr(),
            };
            let atom = core::mem::transmute::<usize, unsafe extern "system" fn(*const WndClass) -> u16>(reg)(&wc);
            // A second thread racing here gets 0 (the class exists): that is fine.
            CLASS_ATOM.store(usize::from(atom).max(1), Ordering::Release);
        }
        let create = user32(CREATE_WINDOW);
        if create == 0 {
            return;
        }
        type Create = unsafe extern "system" fn(
            u32,
            *const u16,
            *const u16,
            u32,
            i32,
            i32,
            i32,
            i32,
            isize,
            usize,
            usize,
            *mut c_void,
        ) -> usize;
        let hwnd = core::mem::transmute::<usize, Create>(create)(
            0,
            class.as_ptr(),
            core::ptr::null(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            0,
            module,
            core::ptr::null_mut(),
        );
        if hwnd != 0 {
            let me = crate::thread_id();
            WINDOWS.with(|w| w.push((me, hwnd)));
        }
    }
}

/// The current STA thread's apartment ended: its COM window goes.
pub(crate) fn drop_window() {
    let me = crate::thread_id();
    let w = WINDOWS.with(|w| {
        let i = w.iter().position(|e| e.0 == me)?;
        Some(w.remove(i).1)
    });
    if let Some(hwnd) = w {
        let d = user32(DESTROY_WINDOW);
        if d != 0 {
            // SAFETY: user32's DestroyWindow on the window this thread made.
            unsafe { core::mem::transmute::<usize, unsafe extern "system" fn(usize) -> i32>(d)(hwnd) };
        }
    }
}

/// Run the posted calls waiting for the current thread's COM window.
fn pump_mine(hwnd: usize) {
    let peek = user32(PEEK_MESSAGE);
    let dispatch = user32(DISPATCH_MESSAGE);
    if peek == 0 || dispatch == 0 {
        return;
    }
    let mut m = Msg { hwnd: 0, message: 0, wparam: 0, lparam: 0, time: 0, pt: [0; 2], private: 0 };
    // SAFETY: user32's PeekMessageW/DispatchMessageW with a live MSG; only this window's messages.
    unsafe {
        let peek =
            core::mem::transmute::<usize, unsafe extern "system" fn(*mut Msg, usize, u32, u32, u32) -> i32>(peek);
        let dispatch = core::mem::transmute::<usize, unsafe extern "system" fn(*const Msg) -> isize>(dispatch);
        while peek(&mut m, hwnd, 0, 0, PM_REMOVE) != 0 {
            dispatch(&m);
        }
    }
}

/// Wait for `event`; an STA thread with a COM window runs the calls posted to it meanwhile.
pub(crate) fn wait(event: Handle) {
    match my_window() {
        None => {
            // SAFETY: an event handle of the caller.
            unsafe { crate::WaitForSingleObject(event as usize, 0xFFFF_FFFF) };
        }
        Some(hwnd) => loop {
            // SAFETY: as above.
            if unsafe { crate::WaitForSingleObject(event as usize, 5) } == 0 {
                break;
            }
            pump_mine(hwnd);
        },
    }
}

unsafe extern "system" fn mta_worker(arg: *mut c_void) -> u32 {
    // SAFETY: a `Job` the caller keeps alive until `done` is set.
    unsafe {
        let job = &mut *(arg as *mut Job<'static>);
        (job.call)();
        SetEvent(job.done);
    }
    0
}

/// Run `f` in apartment `owner` and wait for it.
pub(crate) fn run_in<R>(owner: Owner, f: impl FnOnce() -> R) -> R {
    let mut f = Some(f);
    let mut out: Option<R> = None;
    let mut call = || {
        if let Some(f) = f.take() {
            out = Some(f());
        }
    };
    run_dyn(owner, &mut call);
    // `run_dyn` runs the call in every case (directly when it cannot be moved).
    out.unwrap_or_else(|| unreachable!())
}

fn run_dyn(owner: Owner, call: &mut dyn FnMut()) {
    if crate::com_usable() && current_owner() == owner {
        call();
        return;
    }
    // SAFETY: an unnamed auto-reset event, closed below.
    let done = unsafe { CreateEventW(core::ptr::null(), 0, 0, core::ptr::null()) };
    if done == 0 {
        call();
        return;
    }
    // SAFETY: the job lives on this stack frame until `done` is set: the waits below return only then.
    let mut job =
        Job { call: unsafe { core::mem::transmute::<&mut dyn FnMut(), &'static mut dyn FnMut()>(call) }, done };
    let jp = (&raw mut job) as isize;
    let started = match owner {
        Owner::Sta(tid) => {
            let hwnd = WINDOWS.with(|w| w.iter().find(|e| e.0 == tid).map(|e| e.1));
            let post = user32(POST_MESSAGE);
            match hwnd {
                // SAFETY: user32's PostMessageW; the window proc runs the job.
                Some(h) if post != 0 => unsafe {
                    core::mem::transmute::<usize, unsafe extern "system" fn(usize, u32, usize, isize) -> i32>(post)(
                        h,
                        WM_COM_CALL,
                        0,
                        jp,
                    ) != 0
                },
                _ => false,
            }
        }
        Owner::Mta => {
            let mut tid = 0u32;
            // SAFETY: the worker runs the job and sets `done`.
            let t = unsafe { CreateThread(core::ptr::null(), 0, mta_worker, jp as *mut c_void, 0, &mut tid) };
            close(t);
            t != 0
        }
    };
    if started {
        wait(done);
    } else {
        // Nowhere to send it (the apartment has no window, or no thread could start): run it here.
        (job.call)();
    }
    close(done);
}

/// Run `f` (a call that blocks, such as a pipe transaction to another process) so that an STA
/// caller keeps running the calls posted to its apartment meanwhile (callbacks into it: sheet
/// `CoMarshalInterface`, rule `callback_parameter`, "the client needs no explicit message loop").
/// On a thread without a COM window `f` runs directly.
pub(crate) fn blocking<R: Send>(f: impl FnOnce() -> R + Send) -> R {
    if my_window().is_none() {
        return f();
    }
    let mut f = Some(f);
    let mut out: Option<R> = None;
    let mut call = || {
        if let Some(f) = f.take() {
            out = Some(f());
        }
    };
    // SAFETY: an unnamed auto-reset event, closed below.
    let done = unsafe { CreateEventW(core::ptr::null(), 0, 0, core::ptr::null()) };
    if done == 0 {
        call();
        return out.unwrap_or_else(|| unreachable!());
    }
    // SAFETY: the job lives on this stack frame until `done` is set: `wait` returns only then.
    let mut job =
        Job { call: unsafe { core::mem::transmute::<&mut dyn FnMut(), &'static mut dyn FnMut()>(&mut call) }, done };
    let jp = (&raw mut job) as *mut c_void;
    let mut tid = 0u32;
    // SAFETY: the worker runs the job and sets `done`.
    let t = unsafe { CreateThread(core::ptr::null(), 0, mta_worker, jp, 0, &mut tid) };
    if t == 0 {
        (job.call)();
    } else {
        close(t);
        wait(done);
    }
    close(done);
    drop(job);
    out.unwrap_or_else(|| unreachable!())
}

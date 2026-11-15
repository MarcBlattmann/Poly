//! `WSAAsyncSelect(s, hWnd, wMsg, lEvent)` (Learn, "WSAAsyncSelect function"): the window-message form of
//! `WSAEventSelect`, which the PuTTY-derived networking of WinSCP and the Delphi `Winsock` unit use. Each
//! selected socket gets an event object of this DLL's own and the `WSAEventSelect` machinery of `events`
//! (which decides what is recorded and when it is re-enabled); one helper thread waits on those events and, for
//! each network event it takes with `WSAEnumNetworkEvents`, posts `wMsg` to the window with the socket as
//! `wParam` and `WSAMAKESELECTREPLY(event, error)` as `lParam` (`winsock2.h`: the event in the low word, the
//! error in the high word).

use crate::sock::Socket;
use crate::{Handle, check_started, fail};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use poly_winsock::consts::*;

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn CreateThread(
        security: *const u8,
        stack: usize,
        start: unsafe extern "system" fn(*mut u8) -> u32,
        parameter: *mut u8,
        flags: u32,
        id: *mut u32,
    ) -> Handle;
}

#[cfg_attr(target_arch = "x86", link(name = "user32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "user32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn PostMessageW(hwnd: usize, message: u32, wparam: usize, lparam: isize) -> i32;
    fn IsWindow(hwnd: usize) -> i32;
}

/// One socket selected by message.
#[derive(Clone, Copy)]
struct Selected {
    socket: Socket,
    event: Handle,
    hwnd: usize,
    message: u32,
}

struct Table {
    locked: AtomicBool,
    list: core::cell::UnsafeCell<Vec<Selected>>,
}

// SAFETY: access to `list` is serialised by `locked`.
unsafe impl Sync for Table {}

static TABLE: Table = Table { locked: AtomicBool::new(false), list: core::cell::UnsafeCell::new(Vec::new()) };
static HELPER_STARTED: AtomicBool = AtomicBool::new(false);
/// An event of ours the helper also waits on, set when the table changes (so it reads it again).
static WAKE: AtomicIsize = AtomicIsize::new(0);

fn with_table<R>(f: impl FnOnce(&mut Vec<Selected>) -> R) -> R {
    while TABLE.locked.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
    // SAFETY: the lock is held.
    let r = f(unsafe { &mut *TABLE.list.get() });
    TABLE.locked.store(false, Ordering::Release);
    r
}

fn wake() {
    let w = WAKE.load(Ordering::Acquire);
    if w != 0 {
        // SAFETY: our own event.
        unsafe { crate::SetEvent(w) };
    }
}

/// `WSANETWORKEVENTS`: `lNetworkEvents` and `iErrorCode[FD_MAX_EVENTS = 10]`.
const NETWORK_EVENTS_SIZE: usize = 4 + 4 * 10;

/// The helper: wait for the wake event and the events of the selected sockets (at most 63 at a time, and
/// every 50 ms whatever happened); take what each recorded and post it.
unsafe extern "system" fn helper(_: *mut u8) -> u32 {
    loop {
        let list: Vec<Selected> = with_table(|t| t.clone());
        let mut handles: Vec<Handle> = alloc::vec![WAKE.load(Ordering::Acquire)];
        handles.extend(list.iter().take(63).map(|s| s.event));
        // SAFETY: handles of ours.
        unsafe { crate::WaitForMultipleObjectsEx(handles.len() as u32, handles.as_ptr(), 0, 50, 0) };
        for s in &list {
            let mut out = [0u8; NETWORK_EVENTS_SIZE];
            // SAFETY: a buffer of WSANETWORKEVENTS.
            let status = unsafe { crate::events::WSAEnumNetworkEvents(s.socket, s.event, out.as_mut_ptr()) };
            if status != 0 {
                // The socket is gone (closed by the program): drop the selection.
                with_table(|t| t.retain(|x| x.socket != s.socket));
                // SAFETY: our event.
                unsafe { crate::CloseHandle(s.event) };
                continue;
            }
            let events = u32::from_le_bytes([out[0], out[1], out[2], out[3]]);
            for bit in 0..10 {
                if events & (1 << bit) == 0 {
                    continue;
                }
                let at = 4 + 4 * bit;
                let error = u32::from_le_bytes([out[at], out[at + 1], out[at + 2], out[at + 3]]);
                let lparam = ((error & 0xFFFF) << 16 | (1 << bit)) as i32 as isize;
                // SAFETY: a window handle the program gave.
                unsafe {
                    if IsWindow(s.hwnd) != 0 {
                        PostMessageW(s.hwnd, s.message, s.socket, lparam);
                    }
                }
            }
        }
    }
}

fn start_helper() -> Result<(), u32> {
    if HELPER_STARTED.swap(true, Ordering::AcqRel) {
        return Ok(());
    }
    // SAFETY: an auto-reset event of ours for the wake-up.
    let w = unsafe { crate::CreateEventW(core::ptr::null(), 0, 0, core::ptr::null()) };
    if w == 0 {
        HELPER_STARTED.store(false, Ordering::Release);
        return Err(WSAENOBUFS);
    }
    WAKE.store(w, Ordering::Release);
    // SAFETY: `helper` is a thread routine; no parameter.
    let h = unsafe { CreateThread(core::ptr::null(), 0, helper, core::ptr::null_mut(), 0, core::ptr::null_mut()) };
    if h == 0 {
        HELPER_STARTED.store(false, Ordering::Release);
        return Err(WSAENOBUFS);
    }
    // SAFETY: we keep no use of the thread's handle.
    unsafe { crate::CloseHandle(h) };
    Ok(())
}

/// `WSAAsyncSelect`: select `events` (`FD_*`) of `s` to be posted to `hwnd` as `message`; `events` 0 cancels
/// the selection. The socket becomes non-blocking (as with `WSAEventSelect`).
#[unsafe(no_mangle)]
pub extern "system" fn WSAAsyncSelect(s: Socket, hwnd: usize, message: u32, events: i32) -> i32 {
    if let Err(e) = check_started() {
        return fail(e);
    }
    // SAFETY: no preconditions.
    if hwnd == 0 || unsafe { IsWindow(hwnd) } == 0 {
        return fail(WSAEINVAL);
    }
    if events == 0 {
        let old = with_table(|t| t.iter().position(|x| x.socket == s).map(|i| t.remove(i)));
        let Some(old) = old else { return 0 };
        crate::events::WSAEventSelect(s, 0, 0);
        // SAFETY: our event.
        unsafe { crate::CloseHandle(old.event) };
        wake();
        return 0;
    }
    if let Err(e) = start_helper() {
        return fail(e);
    }
    // One event per socket, kept while it is selected.
    let existing = with_table(|t| t.iter().find(|x| x.socket == s).map(|x| x.event));
    let event = match existing {
        Some(e) => e,
        // SAFETY: a manual-reset event of ours.
        None => unsafe { crate::CreateEventW(core::ptr::null(), 1, 0, core::ptr::null()) },
    };
    if event == 0 {
        return fail(WSAENOBUFS);
    }
    let status = crate::events::WSAEventSelect(s, event, events);
    if status != 0 {
        if existing.is_none() {
            // SAFETY: our event.
            unsafe { crate::CloseHandle(event) };
        }
        return status;
    }
    with_table(|t| match t.iter_mut().find(|x| x.socket == s) {
        Some(x) => {
            x.hwnd = hwnd;
            x.message = message;
        }
        None => t.push(Selected { socket: s, event, hwnd, message }),
    });
    wake();
    0
}

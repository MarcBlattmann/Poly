//! `WSAEventSelect`, `WSAEnumNetworkEvents`, `WSADuplicateSocketW` and the
//! `WSALookupService*` calls (ADR 0155). Chrome's network service waits
//! for its sockets with `WSAEventSelect` and a `WaitForMultipleObjects` on
//! the event objects, so the events must be set by someone: a helper thread
//! of this process (started by the first `WSAEventSelect`) polls the
//! selected sockets through `\Device\Afd`'s `POLL` request and feeds
//! `poly_winsock::events`' state machine, which decides what is recorded
//! (host-tested). The socket calls that re-enable an event (`recv`,
//! `send` that blocked, `accept`, `connect`) tell this module through the
//! `after_*` hooks.

use crate::afd;
use crate::sock::{self, Socket};
use crate::{Handle, check_started, fail};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use poly_winsock::afd as proto;
use poly_winsock::consts::*;
use poly_winsock::error::STATUS_SUCCESS;
use poly_winsock::events::{self as ev, Inputs, Registration};
use poly_winsock::{dup, poll};

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
    fn Sleep(ms: u32);
    fn GetCurrentProcess() -> Handle;
    fn GetCurrentProcessId() -> u32;
    fn GetModuleHandleW(name: *const u16) -> Handle;
    fn LoadLibraryW(name: *const u16) -> Handle;
    fn GetProcAddress(module: Handle, name: *const u8) -> usize;
    fn DuplicateHandle(
        source_process: Handle,
        source: Handle,
        target_process: Handle,
        target: *mut Handle,
        access: u32,
        inherit: i32,
        options: u32,
    ) -> i32;
}

/// `PROCESS_DUP_HANDLE`, `DUPLICATE_SAME_ACCESS` (`winnt.h`, `handleapi.h`).
const PROCESS_DUP_HANDLE: u32 = 0x40;
const DUPLICATE_SAME_ACCESS: u32 = 2;

/// How long the helper waits in one poll: selected sockets are looked at at least this often (ms).
const POLL_MS: i64 = 20;

// ----- the table of selected sockets ------------------------------------------------------

struct Table {
    locked: AtomicBool,
    list: core::cell::UnsafeCell<Vec<(Socket, Registration)>>,
}

// SAFETY: access to `list` is serialised by `locked`.
unsafe impl Sync for Table {}

static TABLE: Table = Table { locked: AtomicBool::new(false), list: core::cell::UnsafeCell::new(Vec::new()) };
/// How many sockets are selected (so the hooks cost nothing when none is).
static COUNT: AtomicUsize = AtomicUsize::new(0);
static HELPER_STARTED: AtomicBool = AtomicBool::new(false);

fn with_table<R>(f: impl FnOnce(&mut Vec<(Socket, Registration)>) -> R) -> R {
    while TABLE.locked.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
    // SAFETY: the lock is held.
    let list = unsafe { &mut *TABLE.list.get() };
    let r = f(list);
    COUNT.store(list.len(), Ordering::Release);
    TABLE.locked.store(false, Ordering::Release);
    flush_posts();
    r
}

fn find(t: &mut [(Socket, Registration)], s: Socket) -> Option<&mut Registration> {
    t.iter_mut().find(|(k, _)| *k == s).map(|(_, r)| r)
}

fn hook(s: Socket, f: impl FnOnce(&mut Registration)) {
    if COUNT.load(Ordering::Acquire) == 0 {
        return;
    }
    with_table(|t| {
        if let Some(r) = find(t, s) {
            f(r);
        }
    });
}

/// `recv` ran on `s`: `FD_READ` may be recorded again, and when no data is left the record that
/// was not taken yet loses its `FD_READ` (the event object stays signalled).
pub fn after_recv(s: Socket) {
    if COUNT.load(Ordering::Acquire) == 0 {
        return;
    }
    let drained = bytes_readable(s) == 0;
    hook(s, |r| {
        r.after_recv();
        if drained {
            r.data_drained();
        }
    });
}

/// `send` failed with `WSAEWOULDBLOCK` on `s`.
pub fn after_send_blocked(s: Socket) {
    hook(s, Registration::after_send_blocked);
}

/// `accept` ran on `s`.
pub fn after_accept(s: Socket) {
    hook(s, Registration::after_accept);
}

/// `accept` on `s` made socket `new`: a socket selected with `WSAAsyncSelect` hands its window,
/// message and events on, and the new socket is non-blocking (`WSAAsyncSelect.yaml` `accept`).
pub fn accepted_into(s: Socket, new: Socket) {
    if COUNT.load(Ordering::Acquire) == 0 {
        return;
    }
    let found = with_table(|t| find(t, s).filter(|r| r.window != 0).map(|r| (r.mask, r.window, r.message)));
    let Some((mask, window, message)) = found else { return };
    let mut nb = [0u8; 0];
    afd::ioctl(new as Handle, proto::SET_NONBLOCKING, &1u32.to_le_bytes(), &mut nb);
    with_table(|t| {
        let mut r = Registration::new(mask, 0, false, true);
        r.post_to(window, message);
        t.push((new, r));
    });
    let _ = start_helper();
}

/// A non-blocking `connect` on `s` is under way.
pub fn connecting(s: Socket) {
    hook(s, Registration::connecting);
}

/// `connect` on `s` finished at once.
pub fn connected(s: Socket) {
    hook(s, |r| {
        r.connected();
        signal(s, r);
    });
}

/// Has `s` a selection (`ioctlsocket(FIONBIO, 0)` is refused while it has)?
pub fn is_selected(s: Socket) -> bool {
    if COUNT.load(Ordering::Acquire) == 0 {
        return false;
    }
    with_table(|t| find(t, s).is_some())
}

/// `shutdown(s, how)` ran: `receive` when the receiving side was shut down.
pub fn local_shutdown(s: Socket, receive: bool) {
    hook(s, |r| r.local_shutdown(receive));
}

/// `closesocket(s)`: the association ends.
pub fn forget(s: Socket) {
    if COUNT.load(Ordering::Acquire) == 0 {
        return;
    }
    with_table(|t| t.retain(|(k, _)| *k != s));
}

/// Set the event of a registration that has something pending (with the table locked), or queue
/// its window messages (`WSAAsyncSelect`): they are posted when the table is unlocked again.
fn signal(s: Socket, r: &mut Registration) {
    if r.pending == 0 {
        return;
    }
    if r.window != 0 {
        let (window, message) = (r.window, r.message);
        let batch = r.take_messages();
        OUTBOX.with(|o| {
            for (event, error) in batch {
                o.push((window, message, s, event | (error << 16)));
            }
        });
    } else if r.event != 0 {
        // SAFETY: the program's event handle; a bad one fails harmlessly.
        unsafe { crate::SetEvent(r.event) };
    }
}

/// The window messages `signal` queued: window, message, socket (`wParam`) and `lParam`.
static OUTBOX: SpinList = SpinList::new();

/// A list behind a spin lock.
struct SpinList {
    locked: AtomicBool,
    list: core::cell::UnsafeCell<Vec<(usize, u32, Socket, u32)>>,
}

// SAFETY: access to `list` is serialised by `locked`.
unsafe impl Sync for SpinList {}

impl SpinList {
    const fn new() -> SpinList {
        SpinList { locked: AtomicBool::new(false), list: core::cell::UnsafeCell::new(Vec::new()) }
    }

    fn with<R>(&self, f: impl FnOnce(&mut Vec<(usize, u32, Socket, u32)>) -> R) -> R {
        while self.locked.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            core::hint::spin_loop();
        }
        // SAFETY: the lock is held.
        let r = f(unsafe { &mut *self.list.get() });
        self.locked.store(false, Ordering::Release);
        r
    }
}

/// Post the window messages that are queued (with no table lock held: `PostMessageW` is user32's).
fn flush_posts() {
    let batch: Vec<_> = OUTBOX.with(core::mem::take);
    if batch.is_empty() {
        return;
    }
    let Some(post) = user32_fn::<PostMessage>(b"PostMessageW\0") else { return };
    for (window, message, s, lparam) in batch {
        // SAFETY: user32's documented call; a window that is gone makes it fail, which is the
        // lost notification the sheet describes.
        unsafe { post(window, message, s, lparam as isize) };
    }
}

type PostMessage = unsafe extern "system" fn(usize, u32, usize, isize) -> i32;
type IsWindow = unsafe extern "system" fn(usize) -> i32;

/// A function of `user32.dll`, looked up when first needed: a static import would make every
/// Winsock program load the GUI, and only `WSAAsyncSelect` needs it.
fn user32_fn<F: Copy>(name: &[u8]) -> Option<F> {
    const { assert!(core::mem::size_of::<F>() == core::mem::size_of::<usize>()) };
    let user32: [u16; 11] = [b'u', b's', b'e', b'r', b'3', b'2', b'.', b'd', b'l', b'l', 0].map(u16::from);
    // SAFETY: a NUL-terminated name; the module stays loaded.
    let m = unsafe { LoadLibraryW(user32.as_ptr()) };
    if m == 0 {
        return None;
    }
    // SAFETY: a NUL-terminated name.
    let p = unsafe { GetProcAddress(m, name.as_ptr()) };
    // SAFETY: the export has the signature `F` says (the caller names both).
    (p != 0).then(|| unsafe { core::mem::transmute_copy::<usize, F>(&p) })
}

// ----- the helper thread ------------------------------------------------------------------

fn bytes_readable(s: Socket) -> u32 {
    let mut b = [0u8; 4];
    match afd::ioctl(s as Handle, proto::READABLE, &[], &mut b) {
        (STATUS_SUCCESS, _) => u32::from_le_bytes(b),
        _ => 0,
    }
}

fn so_error(s: Socket) -> u32 {
    let mut out = [0u8; 16];
    match afd::ioctl(s as Handle, proto::GET_OPTION, &proto::option_header(SOL_SOCKET, SO_ERROR), &mut out) {
        (STATUS_SUCCESS, n) if n >= 4 => u32::from_le_bytes([out[0], out[1], out[2], out[3]]),
        _ => 0,
    }
}

/// Numbers the looks at the sockets (taken before the device is asked, see `Registration::last_look`).
static LOOKS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(1);

fn next_look() -> u64 {
    LOOKS.fetch_add(1, Ordering::AcqRel)
}

/// Fold what the device says about `s` (`revents`, asked for at `look`) into its registration; true
/// when something is recorded that was not before.
fn observe(s: Socket, r: u32, look: u64) -> bool {
    let (readable, writable, error, hangup) = poll::flags(r);
    if !(readable || writable || error || hangup) {
        return false;
    }
    let inputs = Inputs {
        readable,
        writable,
        error,
        hangup,
        bytes: if readable { bytes_readable(s) } else { 0 },
        so_error: if error { so_error(s) } else { 0 },
    };
    let mut moved = false;
    with_table(|t| {
        if let Some(reg) = find(t, s)
            && look > reg.last_look
        {
            reg.last_look = look;
            let before = reg.pending;
            if reg.step(&inputs) {
                moved = reg.pending != before;
                signal(s, reg);
            }
        }
    });
    moved
}

/// Look at `s` right now, as the helper would a moment later: `WSAEnumNetworkEvents` answers with
/// what has happened, not with what the helper had time to see.
fn refresh(s: Socket) {
    if COUNT.load(Ordering::Acquire) == 0 {
        return;
    }
    let interest = with_table(|t| {
        find(t, s).map(|r| {
            let (read, write) = r.wants();
            poll::events_interest(read, write, r.watches_end())
        })
    });
    let Some(interest) = interest else { return };
    let look = next_look();
    if let Ok(revents) = sock::poll_handles(&[(s as u64, interest)], 0)
        && let Some(&r) = revents.first()
        && r & poll::nval() == 0
    {
        observe(s, r, look);
    }
}

unsafe extern "system" fn helper(_parameter: *mut u8) -> u32 {
    loop {
        let snap: Vec<(Socket, u32)> = with_table(|t| {
            t.iter()
                .filter_map(|(s, r)| {
                    let (read, write) = r.wants();
                    let end = r.watches_end();
                    // A socket with nothing left to watch is not polled (it would return at once).
                    (read || write || end).then(|| (*s, poll::events_interest(read, write, end)))
                })
                .collect()
        });
        if snap.is_empty() {
            // SAFETY: plain kernel32 call.
            unsafe { Sleep(50) };
            continue;
        }
        let entries: Vec<(u64, u32)> = snap.iter().map(|&(s, i)| (s as u64, i)).collect();
        let look = next_look();
        let Ok(revents) = sock::poll_handles(&entries, POLL_MS) else {
            // SAFETY: plain kernel32 call.
            unsafe { Sleep(20) };
            continue;
        };
        let mut moved = false;
        for (&(s, _), &r) in snap.iter().zip(&revents) {
            if r & poll::nval() != 0 {
                // Closed behind our back: forget it.
                with_table(|t| t.retain(|(k, _)| *k != s));
                continue;
            }
            moved |= observe(s, r, look);
        }
        if !moved {
            // Ready sockets nobody can record anything for (the program has not acted yet) would
            // return at once: do not spin.
            // SAFETY: plain kernel32 call.
            unsafe { Sleep(2) };
        }
    }
}

fn start_helper() -> Result<(), u32> {
    if HELPER_STARTED.swap(true, Ordering::AcqRel) {
        return Ok(());
    }
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

// ----- WSAEventSelect / WSAEnumNetworkEvents ------------------------------------------------

/// `WSAEventSelect(s, hEventObject, lNetworkEvents)`: associates the event object with the `FD_*`
/// events (and makes the socket non-blocking); a zero mask ends the association.
#[unsafe(no_mangle)]
pub extern "system" fn WSAEventSelect(s: Socket, event: Handle, mask: i32) -> i32 {
    if let Err(e) = check_started() {
        return fail(e);
    }
    let mask = mask as u32;
    let Some(info) = afd::info(s as Handle) else { return fail(WSAENOTSOCK) };
    if mask == 0 {
        with_table(|t| t.retain(|(k, _)| *k != s));
        return 0;
    }
    if let Err(e) = ev::check_select(event, mask) {
        return fail(e);
    }
    // Learn: the event has to be an event object (Windows says "not a socket" for anything else).
    if event != 0 && !afd::is_event(event) {
        return fail(WSAENOTSOCK);
    }
    // Learn: the socket becomes non-blocking.
    let mut nb = [0u8; 0];
    let (status, _) = afd::ioctl(s as Handle, proto::SET_NONBLOCKING, &1u32.to_le_bytes(), &mut nb);
    if status >= 0xC000_0000 {
        return fail(WSAENOTSOCK);
    }
    let listening = info.listening != 0;
    // A stream with a peer is connected.
    let mut peer = [0u8; poly_winsock::addr::MAX_SIZE];
    let connected = info.ty == SOCK_STREAM
        && !listening
        && afd::ioctl(s as Handle, proto::NAME, &1u32.to_le_bytes(), &mut peer).0 < 0xC000_0000;
    with_table(|t| match find(t, s) {
        Some(r) => r.reselect(mask, event),
        None => t.push((s, Registration::new(mask, event, listening, connected))),
    });
    match start_helper() {
        Ok(()) => 0,
        Err(e) => fail(e),
    }
}

/// `WSAAsyncSelect(s, hWnd, wMsg, lEvent)`: the `FD_*` events are posted to `hWnd` as `wMsg`
/// (`wParam` the socket, the low word of `lParam` the event, the high word its error code) and the
/// socket becomes non-blocking; a zero `lEvent` ends the association.
#[unsafe(no_mangle)]
pub extern "system" fn WSAAsyncSelect(s: Socket, window: usize, message: u32, mask: i32) -> i32 {
    if let Err(e) = check_started() {
        return fail(e);
    }
    let mask = mask as u32;
    let Some(info) = afd::info(s as Handle) else { return fail(WSAENOTSOCK) };
    let valid = window != 0
        && user32_fn::<IsWindow>(b"IsWindow\0").is_some_and(|f| {
            // SAFETY: user32's documented call.
            unsafe { f(window) != 0 }
        });
    if !valid {
        return fail(WSAEINVAL);
    }
    if let Err(e) = ev::check_async(mask) {
        return fail(e);
    }
    if mask == 0 {
        with_table(|t| t.retain(|(k, _)| *k != s));
        return 0;
    }
    let mut nb = [0u8; 0];
    let (status, _) = afd::ioctl(s as Handle, proto::SET_NONBLOCKING, &1u32.to_le_bytes(), &mut nb);
    if status >= 0xC000_0000 {
        return fail(WSAENOTSOCK);
    }
    let listening = info.listening != 0;
    let mut peer = [0u8; poly_winsock::addr::MAX_SIZE];
    let connected = info.ty == SOCK_STREAM
        && !listening
        && afd::ioctl(s as Handle, proto::NAME, &1u32.to_le_bytes(), &mut peer).0 < 0xC000_0000;
    with_table(|t| match find(t, s) {
        Some(r) => {
            r.reselect(mask, 0);
            r.post_to(window, message);
        }
        None => {
            let mut r = Registration::new(mask, 0, listening, connected);
            r.post_to(window, message);
            t.push((s, r));
        }
    });
    match start_helper() {
        Ok(()) => 0,
        Err(e) => fail(e),
    }
}

/// `WSAEnumNetworkEvents(s, hEventObject, lpNetworkEvents)`: the events recorded since the last
/// call and their error codes; the record and the event object are reset.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WSAEnumNetworkEvents(s: Socket, event: Handle, out: *mut u8) -> i32 {
    if let Err(e) = check_started() {
        return fail(e);
    }
    if afd::info(s as Handle).is_none() {
        return fail(WSAENOTSOCK);
    }
    // A bogus event handle is "not a socket", as on Windows; a missing record is an invalid call.
    if event != 0 && !afd::is_event(event) {
        return fail(WSAENOTSOCK);
    }
    if out.is_null() {
        return fail(WSAEINVAL);
    }
    refresh(s);
    // Reset the event and take the record in one step, so an event recorded meanwhile is not lost.
    let (events, errors) = with_table(|t| {
        if event != 0 {
            // SAFETY: the program's event handle.
            unsafe { crate::ResetEvent(event) };
        }
        find(t, s).map_or((0, [0; ev::FD_MAX_EVENTS]), |r| r.take())
    });
    // SAFETY: the caller's WSANETWORKEVENTS (44 bytes): only the slots of events that happened are
    // written, the others keep what the program put there.
    unsafe {
        let mut b = [0u8; ev::NETWORK_EVENTS_SIZE];
        core::ptr::copy_nonoverlapping(out, b.as_mut_ptr(), b.len());
        ev::fill_network_events(&mut b, events, &errors);
        core::ptr::copy_nonoverlapping(b.as_ptr(), out, b.len());
    }
    0
}

// ----- WSADuplicateSocketW ----------------------------------------------------------------

/// `OpenProcess`, looked up when needed: a static import would keep `ws2_32` from loading on a system
/// whose `kernel32` lacks it (it is another batch's work), and only duplicating a socket into
/// another process needs it. 0 when it is not there.
unsafe fn open_process(access: u32, pid: u32) -> Handle {
    let name: [u16; 13] = [75, 69, 82, 78, 69, 76, 51, 50, 46, 68, 76, 76, 0];
    // SAFETY: kernel32 calls with valid NUL-terminated names.
    unsafe {
        let m = GetModuleHandleW(name.as_ptr());
        if m == 0 {
            return 0;
        }
        let f = GetProcAddress(m, c"OpenProcess".as_ptr().cast());
        if f == 0 {
            return 0;
        }
        let f: unsafe extern "system" fn(u32, i32, u32) -> Handle = core::mem::transmute(f);
        f(access, 0, pid)
    }
}

/// A new socket like `s` (an unbound, unconnected one): the family, type, protocol and the options a
/// program sets before it hands the socket on (`IPV6_V6ONLY` 0 for Chrome's).
fn clone_unbound(s: Socket, info: &proto::Info) -> Option<Handle> {
    let h = afd::open_endpoint(true).ok()?;
    let req = proto::SocketReq { family: info.family, ty: info.ty, protocol: info.protocol };
    if afd::ioctl(h, proto::SOCKET, &req.encode(), &mut []).0 >= 0xC000_0000 {
        afd::close(h);
        return None;
    }
    let options = [
        (SOL_SOCKET, SO_REUSEADDR),
        (SOL_SOCKET, SO_KEEPALIVE),
        (SOL_SOCKET, SO_BROADCAST),
        (SOL_SOCKET, SO_SNDBUF),
        (SOL_SOCKET, SO_RCVBUF),
        (SOL_SOCKET, 0x3005), // SO_RANDOMIZE_PORT
        (IPPROTO_TCP, TCP_NODELAY),
        (IPPROTO_IPV6, IPV6_V6ONLY),
    ];
    for (level, name) in options {
        let mut out = [0u8; 16];
        let (status, n) = afd::ioctl(s as Handle, proto::GET_OPTION, &proto::option_header(level, name), &mut out);
        if status < 0xC000_0000 && n > 0 && n <= out.len() {
            let mut set = proto::option_header(level, name).to_vec();
            set.extend_from_slice(&out[..n]);
            let _ = afd::ioctl(h, proto::SET_OPTION, &set, &mut []);
        }
    }
    Some(h)
}

/// `WSADuplicateSocketW(s, dwProcessId, lpProtocolInfo)`: the endpoint's handle is duplicated into
/// the target process and the record names it (`WSASocketW` with `FROM_PROTOCOL_INFO` in the
/// target takes it up). A socket that is not bound, connected or listening yet is not shared: the
/// duplicate is a new socket with the same options (Windows' own: a bind made through one does not
/// show on the other); the others are the same socket.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WSADuplicateSocketW(s: Socket, pid: u32, info: *mut u8) -> i32 {
    if let Err(e) = check_started() {
        return fail(e);
    }
    if info.is_null() {
        return fail(WSAEFAULT);
    }
    let Some(i) = afd::info(s as Handle) else { return fail(WSAENOTSOCK) };
    let mut name = [0u8; poly_winsock::addr::MAX_SIZE];
    let unbound = i.listening == 0
        && afd::ioctl(s as Handle, proto::NAME, &0u32.to_le_bytes(), &mut name).0 >= 0xC000_0000
        && afd::ioctl(s as Handle, proto::NAME, &1u32.to_le_bytes(), &mut name).0 >= 0xC000_0000;
    let (source, cloned) = if unbound {
        match clone_unbound(s, &i) {
            Some(h) => (h, true),
            None => return fail(WSAEINVAL),
        }
    } else {
        (s as Handle, false)
    };
    // SAFETY: plain kernel32 calls on our own and the target's handles.
    let dup_handle = unsafe {
        let me = GetCurrentProcess();
        let (target, opened) = if pid == GetCurrentProcessId() {
            (me, false)
        } else {
            let t = open_process(PROCESS_DUP_HANDLE, pid);
            if t == 0 {
                return fail(WSAEINVAL);
            }
            (t, true)
        };
        let mut d: Handle = 0;
        let ok = DuplicateHandle(me, source, target, &mut d, 0, 0, DUPLICATE_SAME_ACCESS);
        if opened {
            crate::CloseHandle(target);
        }
        if cloned {
            afd::close(source);
        }
        if ok == 0 {
            return fail(WSAEINVAL);
        }
        d
    };
    let rec =
        dup::encode(&dup::Duplicate { handle: dup_handle as u64, family: i.family, ty: i.ty, protocol: i.protocol });
    // SAFETY: the caller's WSAPROTOCOL_INFOW.
    unsafe { core::ptr::copy_nonoverlapping(rec.as_ptr(), info, rec.len()) };
    0
}

/// The sockets made from each `WSADuplicateSocketW` record of this process: (record handle,
/// socket). Windows ends every socket made from one record when one of them is closed (they are
/// handles to one socket whose last user ends it), and so do we.
struct MadeTable {
    locked: AtomicBool,
    list: core::cell::UnsafeCell<Vec<(Socket, isize)>>,
}

// SAFETY: access to `list` is serialised by `locked`.
unsafe impl Sync for MadeTable {}

static MADE: MadeTable = MadeTable { locked: AtomicBool::new(false), list: core::cell::UnsafeCell::new(Vec::new()) };

fn with_made<R>(f: impl FnOnce(&mut Vec<(Socket, isize)>) -> R) -> R {
    while MADE.locked.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
    // SAFETY: the lock is held.
    let list = unsafe { &mut *MADE.list.get() };
    let r = f(list);
    MADE.locked.store(false, Ordering::Release);
    r
}

/// `closesocket(s)` of a socket made from a record: the other sockets made from the same record,
/// which end with it (to be closed by the caller).
pub fn siblings_of(s: Socket) -> Vec<Socket> {
    with_made(|t| {
        let Some(record) = t.iter().find(|(k, _)| *k == s).map(|(_, r)| *r) else { return Vec::new() };
        let sibs: Vec<Socket> = t.iter().filter(|(k, r)| *r == record && *k != s).map(|(k, _)| *k).collect();
        t.retain(|(_, r)| *r != record);
        sibs
    })
}

/// `WSASocketW(FROM_PROTOCOL_INFO, ...)` with a record of [`WSADuplicateSocketW`]: the socket it names.
pub unsafe fn from_protocol_info(info: *const u8) -> Result<Socket, u32> {
    // SAFETY: the caller's WSAPROTOCOL_INFOW (628 bytes).
    let b = unsafe { core::slice::from_raw_parts(info, dup::PROTOCOL_INFO_SIZE) };
    let d = dup::decode(b).ok_or(WSAEINVAL)?;
    let handle = d.handle as Handle;
    if afd::info(handle).is_none() {
        return Err(WSAEINVAL);
    }
    // Each socket made from the record is a handle of its own to the duplicated socket (closing it
    // leaves the record usable), and, as on Windows, blocking again.
    let mut mine: Handle = 0;
    // SAFETY: plain kernel32 call on our own handles.
    let ok = unsafe {
        let me = GetCurrentProcess();
        DuplicateHandle(me, handle, me, &mut mine, 0, 1, DUPLICATE_SAME_ACCESS)
    };
    if ok == 0 {
        return Err(WSAEINVAL);
    }
    let _ = afd::ioctl(mine, proto::SET_NONBLOCKING, &0u32.to_le_bytes(), &mut []);
    with_made(|t| t.push((mine as Socket, handle)));
    Ok(mine as Socket)
}

// ----- WSALookupService* --------------------------------------------------------------------

/// The lookup handles handed out and not ended yet (never reused: the counter only goes up).
const LOOKUP_SLOTS: usize = 128;
static LOOKUP_LIVE: [AtomicUsize; LOOKUP_SLOTS] = [const { AtomicUsize::new(0) }; LOOKUP_SLOTS];
static LOOKUPS: AtomicUsize = AtomicUsize::new(0);
/// Per slot: 1 once the lookup's only result was handed out.
static LOOKUP_DONE: [AtomicUsize; LOOKUP_SLOTS] = [const { AtomicUsize::new(0) }; LOOKUP_SLOTS];
/// The name of the one network every lookup of the `NS_ALL`/`NS_NLA` name spaces finds, and the size of a
/// `WSAQUERYSETW` (x64).
const LOOKUP_RESULT_NAME: &str = "Poly Network";
const QUERY_SET_SIZE: usize = 120;
/// The tag of a lookup handle: small distinct non-zero values, so they are not mistaken for others.
const LOOKUP_BASE: usize = 0x4C55_0000;

/// `WSALookupServiceBeginW(lpqsRestrictions, dwControlFlags, lphLookup)`: no name-space provider of
/// Poly's resolves service queries, so the query (of the name spaces `poly_winsock::resolve::lookup_begin`
/// accepts) starts a lookup and `WSALookupServiceNextW` ends it (`WSA_E_NO_MORE`) at once. The
/// query set is `WSAQUERYSETW` (x64): `dwSize` at 0, `dwNameSpace` at 40.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WSALookupServiceBeginW(query: *const u8, flags: u32, lookup: *mut Handle) -> i32 {
    if let Err(e) = check_started() {
        return fail(e);
    }
    if query.is_null() || lookup.is_null() {
        return fail(WSAEFAULT);
    }
    // SAFETY: checked non-null: the caller's WSAQUERYSETW.
    let (size, namespace) = unsafe {
        (core::ptr::read_unaligned(query.cast::<u32>()), core::ptr::read_unaligned(query.add(40).cast::<u32>()))
    };
    if let Err(e) = poly_winsock::resolve::lookup_begin(size, namespace, flags) {
        return fail(e);
    }
    let h = LOOKUP_BASE + LOOKUPS.fetch_add(1, Ordering::AcqRel) + 1;
    let Some(slot) =
        LOOKUP_LIVE.iter().position(|s| s.compare_exchange(0, h, Ordering::AcqRel, Ordering::Acquire).is_ok())
    else {
        return fail(WSAEMFILE);
    };
    LOOKUP_DONE[slot].store(0, Ordering::Release);
    // SAFETY: checked non-null.
    unsafe { lookup.write(h as Handle) };
    0
}

/// `WSALookupServiceNextW(hLookup, dwControlFlags, lpdwBufferLength, lpqsResults)`: the first call gives
/// one result, a `WSAQUERYSETW` of the one network Poly has (observed on Windows: a first result with
/// `dwSize` of at least 80); a buffer too small gets `WSAEFAULT` and the length needed; after that
/// `WSA_E_NO_MORE`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WSALookupServiceNextW(
    lookup: Handle,
    _flags: u32,
    len: *mut u32,
    results: *mut u8,
) -> i32 {
    if let Err(e) = check_started() {
        return fail(e);
    }
    let v = lookup as usize;
    let Some(slot) = (if v == 0 { None } else { LOOKUP_LIVE.iter().position(|s| s.load(Ordering::Acquire) == v) })
    else {
        return fail(WSA_INVALID_HANDLE);
    };
    if LOOKUP_DONE[slot].load(Ordering::Acquire) != 0 {
        return fail(WSA_E_NO_MORE);
    }
    if len.is_null() {
        return fail(WSAEFAULT);
    }
    let name: Vec<u16> = LOOKUP_RESULT_NAME.encode_utf16().chain([0]).collect();
    let need = QUERY_SET_SIZE + name.len() * 2;
    // SAFETY: checked non-null: the caller's length.
    let have = unsafe { len.read() } as usize;
    // SAFETY: as above.
    unsafe { len.write(need as u32) };
    if results.is_null() || have < need {
        return fail(WSAEFAULT);
    }
    // SAFETY: the caller's buffer of `have >= need` bytes.
    unsafe {
        core::ptr::write_bytes(results, 0, QUERY_SET_SIZE);
        results.cast::<u32>().write_unaligned(QUERY_SET_SIZE as u32);
        let text = results.add(QUERY_SET_SIZE);
        core::ptr::copy_nonoverlapping(name.as_ptr().cast::<u8>(), text, name.len() * 2);
        results.add(8).cast::<*mut u8>().write_unaligned(text);
        // `dwNameSpace`: NS_NLA.
        results.add(40).cast::<u32>().write_unaligned(15);
    }
    LOOKUP_DONE[slot].store(1, Ordering::Release);
    0
}

/// `WSALookupServiceEnd(hLookup)`: the handle is dead afterwards.
#[unsafe(no_mangle)]
pub extern "system" fn WSALookupServiceEnd(lookup: Handle) -> i32 {
    if let Err(e) = check_started() {
        return fail(e);
    }
    let v = lookup as usize;
    let ended =
        v != 0 && LOOKUP_LIVE.iter().any(|s| s.compare_exchange(v, 0, Ordering::AcqRel, Ordering::Acquire).is_ok());
    if !ended {
        return fail(WSA_INVALID_HANDLE);
    }
    0
}

fn is_lookup(h: Handle) -> bool {
    let v = h as usize;
    v != 0 && LOOKUP_LIVE.iter().any(|s| s.load(Ordering::Acquire) == v)
}

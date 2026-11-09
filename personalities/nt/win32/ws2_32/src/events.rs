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

#[link(name = "kernel32", kind = "raw-dylib")]
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

/// `recv` ran on `s`.
pub fn after_recv(s: Socket) {
    hook(s, Registration::after_recv);
}

/// `send` failed with `WSAEWOULDBLOCK` on `s`.
pub fn after_send_blocked(s: Socket) {
    hook(s, Registration::after_send_blocked);
}

/// `accept` ran on `s`.
pub fn after_accept(s: Socket) {
    hook(s, Registration::after_accept);
}

/// A non-blocking `connect` on `s` is under way.
pub fn connecting(s: Socket) {
    hook(s, Registration::connecting);
}

/// `connect` on `s` finished at once.
pub fn connected(s: Socket) {
    hook(s, |r| {
        r.connected();
        signal(r);
    });
}

/// `closesocket(s)`: the association ends.
pub fn forget(s: Socket) {
    if COUNT.load(Ordering::Acquire) == 0 {
        return;
    }
    with_table(|t| t.retain(|(k, _)| *k != s));
}

/// Set the event of a registration that has something pending (with the table locked).
fn signal(r: &Registration) {
    if r.pending != 0 && r.event != 0 {
        // SAFETY: the program's event handle; a bad one fails harmlessly.
        unsafe { crate::SetEvent(r.event) };
    }
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

unsafe extern "system" fn helper(_parameter: *mut u8) -> u32 {
    loop {
        let snap: Vec<(Socket, u32)> = with_table(|t| {
            t.iter()
                .map(|(s, r)| {
                    let (read, write) = r.wants();
                    (*s, poll::events_interest(read, write))
                })
                .collect()
        });
        if snap.is_empty() {
            // SAFETY: plain kernel32 call.
            unsafe { Sleep(50) };
            continue;
        }
        let entries: Vec<(u64, u32)> = snap.iter().map(|&(s, i)| (s as u64, i)).collect();
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
            let (readable, writable, error, hangup) = poll::flags(r);
            if !(readable || writable || error || hangup) {
                continue;
            }
            let inputs = Inputs {
                readable,
                writable,
                error,
                hangup,
                bytes: if readable { bytes_readable(s) } else { 0 },
                so_error: if error { so_error(s) } else { 0 },
            };
            with_table(|t| {
                if let Some(reg) = find(t, s) {
                    let before = reg.pending;
                    if reg.step(&inputs) {
                        if reg.pending != before {
                            moved = true;
                        }
                        signal(reg);
                    }
                }
            });
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

/// `WSAEnumNetworkEvents(s, hEventObject, lpNetworkEvents)`: the events recorded since the last
/// call and their error codes; the record and the event object are reset.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WSAEnumNetworkEvents(s: Socket, event: Handle, out: *mut u8) -> i32 {
    if let Err(e) = check_started() {
        return fail(e);
    }
    if out.is_null() {
        return fail(WSAEFAULT);
    }
    if afd::info(s as Handle).is_none() {
        return fail(WSAENOTSOCK);
    }
    // Reset the event and take the record in one step, so an event recorded meanwhile is not lost.
    let (events, errors) = with_table(|t| {
        if event != 0 {
            // SAFETY: the program's event handle.
            unsafe { crate::ResetEvent(event) };
        }
        find(t, s).map_or((0, [0; ev::FD_MAX_EVENTS]), |r| r.take())
    });
    let b = ev::encode_network_events(events, &errors);
    // SAFETY: the caller's WSANETWORKEVENTS.
    unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), out, b.len()) };
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

/// `WSADuplicateSocketW(s, dwProcessId, lpProtocolInfo)`: the endpoint's handle is duplicated into
/// the target process and the record names it (`WSASocketW` with `FROM_PROTOCOL_INFO` in the
/// target takes it up).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WSADuplicateSocketW(s: Socket, pid: u32, info: *mut u8) -> i32 {
    if let Err(e) = check_started() {
        return fail(e);
    }
    if info.is_null() {
        return fail(WSAEFAULT);
    }
    let Some(i) = afd::info(s as Handle) else { return fail(WSAENOTSOCK) };
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
        let ok = DuplicateHandle(me, s as Handle, target, &mut d, 0, 0, DUPLICATE_SAME_ACCESS);
        if opened {
            crate::CloseHandle(target);
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

/// `WSASocketW(FROM_PROTOCOL_INFO, ...)` with a record of [`WSADuplicateSocketW`]: the socket it names.
pub unsafe fn from_protocol_info(info: *const u8) -> Result<Socket, u32> {
    // SAFETY: the caller's WSAPROTOCOL_INFOW (628 bytes).
    let b = unsafe { core::slice::from_raw_parts(info, dup::PROTOCOL_INFO_SIZE) };
    let d = dup::decode(b).ok_or(WSAEINVAL)?;
    let s = d.handle as Socket;
    match afd::info(s as Handle) {
        Some(_) => Ok(s),
        None => Err(WSAEINVAL),
    }
}

// ----- WSALookupService* --------------------------------------------------------------------

/// The lookup handles handed out (never reused); a query finds nothing.
static LOOKUPS: AtomicUsize = AtomicUsize::new(0);

/// `WSALookupServiceBeginW(lpqsRestrictions, dwControlFlags, lphLookup)`: no name-space provider of
/// Poly's resolves service queries, so the query is accepted and `WSALookupServiceNextW` ends it
/// (`WSA_E_NO_MORE`) at once.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WSALookupServiceBeginW(query: *const u8, flags: u32, lookup: *mut Handle) -> i32 {
    if let Err(e) = check_started() {
        return fail(e);
    }
    if query.is_null() || lookup.is_null() {
        return fail(WSAEFAULT);
    }
    if flags == 0 {
        return fail(WSAEINVAL);
    }
    let h = LOOKUPS.fetch_add(1, Ordering::AcqRel) + 1;
    // SAFETY: checked non-null. Handles are small distinct non-zero values tagged so they are not mistaken for others.
    unsafe { lookup.write((0x4C55_0000 + h) as Handle) };
    0
}

/// `WSALookupServiceNextW(hLookup, dwControlFlags, lpdwBufferLength, lpqsResults)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WSALookupServiceNextW(
    lookup: Handle,
    _flags: u32,
    _len: *mut u32,
    _results: *mut u8,
) -> i32 {
    if let Err(e) = check_started() {
        return fail(e);
    }
    if !is_lookup(lookup) {
        return fail(WSA_INVALID_HANDLE);
    }
    fail(WSA_E_NO_MORE)
}

/// `WSALookupServiceEnd(hLookup)`.
#[unsafe(no_mangle)]
pub extern "system" fn WSALookupServiceEnd(lookup: Handle) -> i32 {
    if let Err(e) = check_started() {
        return fail(e);
    }
    if !is_lookup(lookup) {
        return fail(WSA_INVALID_HANDLE);
    }
    0
}

fn is_lookup(h: Handle) -> bool {
    let v = h as usize;
    v > 0x4C55_0000 && v <= 0x4C55_0000 + LOOKUPS.load(Ordering::Acquire)
}

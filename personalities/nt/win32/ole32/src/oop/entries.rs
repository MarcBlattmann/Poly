//! The B2a exports (ADR 0220) and the hooks the apartment code calls: class objects, the server
//! count, initialisation spies, `CoDisconnectObject`.

use super::*;
use crate::{IID_IUNKNOWN, add_ref_unknown, com_usable, current_owner, release_unknown, thread_id};
use alloc::vec::Vec;
use poly_com::classtable::{ClassTable, Revoked};
use poly_com::git::Owner;
use poly_com::hresult::{CO_E_NOTINITIALIZED, E_INVALIDARG, E_NOINTERFACE, S_OK};
use poly_com::wire::Request;

/// Withdrawals waiting for the sender thread, in order.
static WITHDRAWALS: Locked<Vec<Request>> = Locked::new(Vec::new());
/// The sender thread's wake-up event (0 until it runs).
static SENDER_EVENT: core::sync::atomic::AtomicIsize = core::sync::atomic::AtomicIsize::new(0);

unsafe extern "system" fn sender(_: *mut c_void) -> u32 {
    loop {
        let ev = SENDER_EVENT.load(core::sync::atomic::Ordering::Acquire);
        // SAFETY: our own event.
        unsafe { crate::WaitForSingleObject(ev as usize, 0xFFFF_FFFF) };
        loop {
            let next = WITHDRAWALS.with(|q| if q.is_empty() { None } else { Some(q.remove(0)) });
            let Some(req) = next else { break };
            let _ = pipe::activation(&req, false);
        }
    }
}

/// The sender thread's event, the thread started on first use. A process starts it when it first
/// publishes a class, so that a withdrawal later (`CoReleaseServerProcess` reaching 0 inside an
/// object's last `Release`) costs a queue entry and an event, not a new thread.
fn sender_event() -> isize {
    use core::sync::atomic::Ordering::{AcqRel, Acquire};
    let ev = SENDER_EVENT.load(Acquire);
    if ev != 0 {
        return ev;
    }
    // SAFETY: an unnamed auto-reset event, kept for the life of the process.
    let made = unsafe { CreateEventW(core::ptr::null(), 0, 0, core::ptr::null()) };
    match SENDER_EVENT.compare_exchange(0, made, AcqRel, Acquire) {
        Ok(_) => {
            let mut tid = 0u32;
            // SAFETY: the sender runs for the life of the process.
            let t = unsafe { CreateThread(core::ptr::null(), 0, sender, core::ptr::null_mut(), 0, &mut tid) };
            close(t);
            made
        }
        Err(other) => {
            close(made);
            other
        }
    }
}

/// Tell the activation process to forget a registration, without waiting: the factory is already
/// disconnected (a client served it meanwhile is not handed it: the activation process checks),
/// and `CoReleaseServerProcess`, `CoRevokeClassObject` and `CoUninitialize` return at once, as on
/// Windows.
fn withdraw_later(req: Request) {
    WITHDRAWALS.with(|q| q.push(req));
    // SAFETY: the sender's event.
    unsafe { SetEvent(sender_event()) };
}

/// Tell the activation process what changed in the class table: newly servable registrations are
/// published (their factories exported), the others withdrawn.
pub(crate) fn publish_changes() {
    let p = CLASSES.with(|t| t.sync());
    if p.is_empty() {
        return;
    }
    let me = pid();
    for cookie in p.withdraw {
        exporter::withdraw_factory(cookie);
        withdraw_later(Request::Revoke { pid: me, cookie });
    }
    if !p.publish.is_empty() {
        sender_event();
    }
    for (cookie, clsid, object, single) in p.publish {
        let owner = CLASSES.with(|t| t.get(cookie)).map_or(Owner::Mta, |r| r.owner);
        let factory = exporter::export_factory(cookie, object, owner);
        let _ = pipe::activation(&Request::Register { clsid, pid: me, cookie, factory, single }, true);
    }
}

/// `CoRegisterClassObject` (sheet `CoRegisterClassObject`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CoRegisterClassObject(
    clsid: *const Guid,
    object: *mut c_void,
    ctx: u32,
    flags: u32,
    cookie: *mut u32,
) -> Hresult {
    if !com_usable() {
        return CO_E_NOTINITIALIZED;
    }
    if clsid.is_null() || cookie.is_null() {
        return E_INVALIDARG;
    }
    let hr = ClassTable::check(object as usize, ctx, flags);
    if hr != S_OK {
        // The recorded run shows the cookie written as 0 for bad arguments (not for a thread
        // without COM, above).
        // SAFETY: the caller's out parameter.
        unsafe { *cookie = 0 };
        return hr;
    }
    let owner = current_owner();
    if matches!(owner, Owner::Sta(_)) && ctx & poly_com::classtable::CLSCTX_LOCAL_SERVER != 0 {
        // Calls from other apartments and processes reach this STA through its window.
        apt::ensure_window();
    }
    // SAFETY: the caller's object and pointers; the registration holds one reference.
    unsafe {
        add_ref_unknown(object);
        let c = CLASSES.with(|t| t.register(*clsid, object as usize, ctx, flags, owner));
        *cookie = c;
    }
    publish_changes();
    S_OK
}

/// `CoRevokeClassObject` (sheet `CoRevokeClassObject`): unknown cookies are `S_OK`.
#[unsafe(no_mangle)]
pub extern "system" fn CoRevokeClassObject(cookie: u32) -> Hresult {
    if !com_usable() {
        return CO_E_NOTINITIALIZED;
    }
    if let Revoked::Removed { object, .. } = CLASSES.with(|t| t.revoke(cookie, current_owner())) {
        exporter::withdraw_factory(cookie);
        // SAFETY: the registration's reference.
        unsafe { release_unknown(object as *mut c_void) };
    }
    publish_changes();
    S_OK
}

/// `CoResumeClassObjects` (sheet `CoResumeClassObjects`).
#[unsafe(no_mangle)]
pub extern "system" fn CoResumeClassObjects() -> Hresult {
    let hr = CLASSES.with(|t| t.resume());
    publish_changes();
    hr
}

/// `CoSuspendClassObjects` (sheet `CoSuspendClassObjects`): always `S_OK`.
#[unsafe(no_mangle)]
pub extern "system" fn CoSuspendClassObjects() -> Hresult {
    CLASSES.with(|t| t.suspend());
    publish_changes();
    S_OK
}

/// `CoAddRefServerProcess`: the new count.
#[unsafe(no_mangle)]
pub extern "system" fn CoAddRefServerProcess() -> u32 {
    COUNT.with(|c| c.add())
}

/// `CoReleaseServerProcess`: the new count; at 0 the local-server registrations stop being served.
#[unsafe(no_mangle)]
pub extern "system" fn CoReleaseServerProcess() -> u32 {
    let (n, zero) = COUNT.with(|c| c.release());
    if zero {
        CLASSES.with(|t| t.close_local());
        publish_changes();
    }
    n
}

/// `CoDisconnectObject`: the object's exports go; proxies in other processes are disconnected.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CoDisconnectObject(object: *mut c_void, _reserved: u32) -> Hresult {
    if object.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's COM object.
    let (_, id) = unsafe { query(object, &IID_IUNKNOWN) };
    if !id.is_null() {
        marshal::disconnect(id as usize);
        exporter::disconnect(id as usize);
        // SAFETY: the query's reference.
        unsafe { release_unknown(id) };
    }
    S_OK
}

// ----- initialisation spies ------------------------------------------------------------

/// `IInitializeSpy`'s methods (slots 3 to 6, `objidl.h`).
unsafe fn spy_slot(spy: usize, slot: usize) -> usize {
    // SAFETY: an IInitializeSpy pointer the table holds.
    unsafe { *(*(spy as *const *const usize)).add(slot) }
}

/// `CoRegisterInitializeSpy` (sheet `CoRegisterInitializeSpy`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CoRegisterInitializeSpy(spy: *mut c_void, cookie: *mut u64) -> Hresult {
    if spy.is_null() || cookie.is_null() {
        return E_INVALIDARG;
    }
    // Ask for the interface before anything is written: a refusal leaves the cookie as it was
    // (sheet, rule `bad_arguments`). Only `S_OK` with a pointer counts as having it.
    let mut p: *mut c_void = core::ptr::null_mut();
    // SAFETY: the caller's object; slot 0 is QueryInterface; the reference is the registration's.
    let hr = unsafe {
        let vtbl =
            *(spy as *const *const unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> Hresult);
        (*vtbl)(spy, &IID_IINITIALIZE_SPY, &mut p)
    };
    if hr != S_OK || p.is_null() {
        if hr >= 0 && !p.is_null() {
            // SAFETY: a reference the query handed out.
            unsafe { release_unknown(p) };
        }
        return E_NOINTERFACE;
    }
    let me = thread_id();
    let c = SPIES.with(|s| s.register(me, p as usize));
    // SAFETY: the caller's out parameter (`ULARGE_INTEGER`).
    unsafe { cookie.write_unaligned(c) };
    S_OK
}

/// `CoRevokeInitializeSpy` (sheet `CoRevokeInitializeSpy`): only the registering thread's cookies.
#[unsafe(no_mangle)]
pub extern "system" fn CoRevokeInitializeSpy(cookie: u64) -> Hresult {
    let me = thread_id();
    match SPIES.with(|s| s.revoke(me, cookie)) {
        Some(p) => {
            // SAFETY: the registration's reference.
            unsafe { release_unknown(p as *mut c_void) };
            S_OK
        }
        None => E_INVALIDARG,
    }
}

/// The current thread's spies, newest first, each with a reference held for the call.
fn my_spies() -> Vec<usize> {
    let me = thread_id();
    let v = SPIES.with(|s| if s.any_for(me) { s.of_thread(me) } else { Vec::new() });
    for &p in &v {
        // SAFETY: a registered spy (the registration holds a reference).
        unsafe { add_ref_unknown(p as *mut c_void) };
    }
    v
}

fn let_go(spies: Vec<usize>) {
    for p in spies {
        // SAFETY: the reference `my_spies` took.
        unsafe { release_unknown(p as *mut c_void) };
    }
}

/// Around an initialisation (`CoInitializeEx`, `OleInitialize` as `COINIT_APARTMENTTHREADED`):
/// `PreInitialize(coinit, count)`, the initialisation, `PostInitialize(hr, coinit, count after)`,
/// whose result is the call's. For `OleInitialize` (`ole`) the spies see the result of its COM
/// part (`S_FALSE` on a thread that already has its STA) and the call keeps its own result.
pub(crate) fn spied_initialize(coinit: u32, ole: bool, init: impl FnOnce() -> Hresult) -> Hresult {
    let spies = my_spies();
    if spies.is_empty() {
        return init();
    }
    let state = crate::state();
    let before = state.count;
    let had_sta = before > 0 && state.model() == Some(poly_com::apartment::Model::Sta);
    for &s in &spies {
        // SAFETY: IInitializeSpy::PreInitialize(DWORD dwCoInit, DWORD dwCurThreadAptRefs).
        unsafe {
            let f: unsafe extern "system" fn(usize, u32, u32) -> Hresult = core::mem::transmute(spy_slot(s, 3));
            f(s, coinit, before);
        }
    }
    let own = init();
    let mut hr = if ole && had_sta && own >= 0 { poly_com::hresult::S_FALSE } else { own };
    let after = crate::state().count;
    for &s in &spies {
        // SAFETY: IInitializeSpy::PostInitialize(HRESULT hrCoInit, DWORD dwCoInit, DWORD dwNewThreadAptRefs).
        unsafe {
            let f: unsafe extern "system" fn(usize, Hresult, u32, u32) -> Hresult =
                core::mem::transmute(spy_slot(s, 4));
            hr = f(s, hr, coinit, after);
        }
    }
    let_go(spies);
    if ole { own } else { hr }
}

/// Around an uninitialisation: `PreUninitialize(count)`, it, `PostUninitialize(count after)`.
pub(crate) fn spied_uninitialize(uninit: impl FnOnce()) {
    let spies = my_spies();
    if spies.is_empty() {
        uninit();
        return;
    }
    let before = crate::state().count;
    for &s in &spies {
        // SAFETY: IInitializeSpy::PreUninitialize(DWORD dwCurThreadAptRefs).
        unsafe {
            let f: unsafe extern "system" fn(usize, u32) -> Hresult = core::mem::transmute(spy_slot(s, 5));
            f(s, before);
        }
    }
    uninit();
    let after = crate::state().count;
    for &s in &spies {
        // SAFETY: IInitializeSpy::PostUninitialize(DWORD dwNewThreadAptRefs).
        unsafe {
            let f: unsafe extern "system" fn(usize, u32) -> Hresult = core::mem::transmute(spy_slot(s, 6));
            f(s, after);
        }
    }
    let_go(spies);
}

/// COM was initialised in the process (a release of the server count at 0 wraps from now on).
pub(crate) fn com_initialised() {
    COUNT.with(|c| c.arm());
}

/// The apartment `owner` ended: its registrations and exports go.
pub(crate) fn apartment_ended(owner: Owner) {
    let gone = CLASSES.with(|t| t.end_apartment(owner));
    marshal::apartment_ended(owner);
    exporter::end_apartment(owner);
    for o in gone {
        // SAFETY: the registrations' references.
        unsafe { release_unknown(o as *mut c_void) };
    }
    if matches!(owner, Owner::Sta(_)) {
        apt::drop_window();
    }
    publish_changes();
}

/// A thread ends (`DLL_THREAD_DETACH`): its spies are released.
pub(crate) fn thread_ended() {
    let me = thread_id();
    let gone = SPIES.with(|s| if s.any_for(me) { s.end_thread(me) } else { Vec::new() });
    for p in gone {
        // SAFETY: the registrations' references.
        unsafe { release_unknown(p as *mut c_void) };
    }
}

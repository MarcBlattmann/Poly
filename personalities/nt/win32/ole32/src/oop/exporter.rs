//! The object exporter of this process (ADR 0220): a named pipe on which other processes call the
//! objects this process handed out — registered class factories (exported while their registration
//! is published) and the objects their `CreateInstance` made.
//!
//! Each connection carries one call; it runs in the object's apartment ([`super::apt::run_in`]).

use super::*;
use crate::{IID_IUNKNOWN, release_unknown};
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use poly_com::git::Owner;
use poly_com::hresult::{E_UNEXPECTED, S_OK};
use poly_com::wire::{ObjRef, Op, Reply, Request, frame};

/// The object ids of registered factories: this bit and the registration's cookie.
const FACTORY_OID: u64 = 1 << 63;

/// One exported object.
#[derive(Clone, Copy)]
struct Export {
    oid: u64,
    /// The object: the identity (`IUnknown`) of a made object, the registered pointer of a factory.
    object: usize,
    owner: Owner,
    /// References proxies hold (made objects only; the export holds one COM reference while > 0).
    refs: u32,
    factory: bool,
}

static EXPORTS: Locked<Vec<Export>> = Locked::new(Vec::new());
static NEXT_OID: AtomicU64 = AtomicU64::new(1);
/// 0: no listener; 1: starting; 2: listening.
static STATE: AtomicU32 = AtomicU32::new(0);
static ENDPOINT: Locked<Option<String>> = Locked::new(None);

/// The pipe name of this process's exporter; the listener starts on first use.
pub(crate) fn endpoint() -> String {
    loop {
        match STATE.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => {
                let mut r = [0u8; 8];
                // SAFETY: a buffer of 8 bytes.
                unsafe { crate::ProcessPrng(r.as_mut_ptr(), 8) };
                let name = alloc::format!(r"\\.\pipe\poly-com-ox-{}-{:016x}", pid(), u64::from_le_bytes(r));
                ENDPOINT.with(|e| *e = Some(name.clone()));
                let mut tid = 0u32;
                // SAFETY: the listener runs for the life of the process.
                let t = unsafe { CreateThread(core::ptr::null(), 0, listener, core::ptr::null_mut(), 0, &mut tid) };
                close(t);
                STATE.store(2, Ordering::Release);
                return name;
            }
            Err(2) => return ENDPOINT.with(|e| e.clone()).unwrap_or_default(),
            Err(_) => core::hint::spin_loop(),
        }
    }
}

unsafe extern "system" fn listener(_: *mut c_void) -> u32 {
    let name = wide(&ENDPOINT.with(|e| e.clone()).unwrap_or_default());
    loop {
        // SAFETY: a NUL-terminated name; byte mode, blocking.
        let h = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                pipe::PIPE_ACCESS_DUPLEX,
                0,
                pipe::PIPE_UNLIMITED_INSTANCES,
                65536,
                65536,
                0,
                core::ptr::null(),
            )
        };
        if h == INVALID_HANDLE {
            // SAFETY: a plain sleep.
            unsafe { Sleep(20) };
            continue;
        }
        // SAFETY: a pipe instance of ours.
        let ok = unsafe { ConnectNamedPipe(h, core::ptr::null_mut()) } != 0
            // SAFETY: no arguments.
            || unsafe { GetLastError() } == pipe::ERROR_PIPE_CONNECTED;
        if !ok {
            close(h);
            continue;
        }
        let mut tid = 0u32;
        // SAFETY: the connection thread owns the handle from here.
        let t = unsafe { CreateThread(core::ptr::null(), 0, connection, h as *mut c_void, 0, &mut tid) };
        if t == 0 {
            close(h);
        } else {
            close(t);
        }
    }
}

unsafe extern "system" fn connection(arg: *mut c_void) -> u32 {
    let h = arg as Handle;
    if let Some(payload) = pipe::read_frame(h) {
        let reply = match Request::decode(&payload) {
            Ok(Request::Call { oid, op }) => dispatch(oid, op),
            _ => Reply::hr(E_UNEXPECTED),
        };
        if pipe::write_all(h, &frame(&reply.encode())) {
            // Wait for the client to read the reply and close its end.
            let mut b = [0u8; 1];
            pipe::read_exact(h, &mut b);
        }
    }
    close(h);
    0
}

fn find(oid: u64) -> Option<Export> {
    EXPORTS.with(|x| x.iter().find(|e| e.oid == oid).copied())
}

/// A registration became published: its factory can be called. The registration holds the
/// factory's reference; the export borrows it until [`withdraw_factory`].
pub(crate) fn export_factory(cookie: u32, object: usize, owner: Owner) -> ObjRef {
    let oid = FACTORY_OID | u64::from(cookie);
    EXPORTS.with(|x| {
        if !x.iter().any(|e| e.oid == oid) {
            x.push(Export { oid, object, owner, refs: 0, factory: true });
        }
    });
    ObjRef { endpoint: endpoint(), oid }
}

/// The registration was withdrawn: calls on its factory get `RPC_E_DISCONNECTED` from now on.
pub(crate) fn withdraw_factory(cookie: u32) {
    let oid = FACTORY_OID | u64::from(cookie);
    EXPORTS.with(|x| x.retain(|e| e.oid != oid));
}

/// Export the object `identity` (its `IUnknown`, one reference the export takes over) of apartment
/// `owner` for one more proxy reference.
pub(crate) fn export_object(identity: usize, owner: Owner) -> ObjRef {
    let (oid, extra) = EXPORTS.with(|x| {
        if let Some(e) = x.iter_mut().find(|e| !e.factory && e.object == identity && e.refs > 0) {
            e.refs += 1;
            (e.oid, true)
        } else {
            let oid = NEXT_OID.fetch_add(1, Ordering::Relaxed);
            x.push(Export { oid, object: identity, owner, refs: 1, factory: false });
            (oid, false)
        }
    });
    if extra {
        // SAFETY: the reference the caller handed over; the export already holds one.
        unsafe { release_unknown(identity as *mut c_void) };
    }
    ObjRef { endpoint: endpoint(), oid }
}

/// Give back `n` proxy references of `oid`; the last one releases the object.
fn release_refs(oid: u64, n: u32) {
    let gone = EXPORTS.with(|x| {
        let i = x.iter().position(|e| e.oid == oid && !e.factory)?;
        x[i].refs = x[i].refs.saturating_sub(n);
        if x[i].refs == 0 { Some(x.remove(i).object) } else { None }
    });
    if let Some(o) = gone {
        // SAFETY: the export's own reference.
        unsafe { release_unknown(o as *mut c_void) };
    }
}

/// The apartment `owner` ended: what it exported is disconnected.
pub(crate) fn end_apartment(owner: Owner) {
    let gone: Vec<Export> = EXPORTS.with(|x| {
        let (gone, keep) = x.drain(..).partition(|e| e.owner == owner);
        *x = keep;
        gone
    });
    for e in gone {
        if !e.factory {
            // SAFETY: the export's own reference.
            unsafe { release_unknown(e.object as *mut c_void) };
        }
    }
}

/// `CoDisconnectObject`: every export of the object goes; its proxies get `RPC_E_DISCONNECTED`.
pub(crate) fn disconnect(identity: usize) {
    let gone: Vec<Export> = EXPORTS.with(|x| {
        let (gone, keep) = x.drain(..).partition(|e| e.object == identity && !e.factory);
        *x = keep;
        gone
    });
    for e in gone {
        // SAFETY: the export's own reference.
        unsafe { release_unknown(e.object as *mut c_void) };
    }
}

/// Run one call.
fn dispatch(oid: u64, op: Op) -> Reply {
    let Some(e) = find(oid) else { return Reply::hr(RPC_E_DISCONNECTED) };
    let object = e.object as *mut c_void;
    match op {
        Op::Release(n) => {
            release_refs(oid, n);
            Reply::hr(S_OK)
        }
        Op::QueryInterface(iid) => {
            let hr = apt::run_in(e.owner, || {
                // SAFETY: an exported object; the reference the query added is given back at once.
                let (hr, p) = unsafe { query(object, &iid) };
                if !p.is_null() {
                    // SAFETY: as above.
                    unsafe { release_unknown(p) };
                }
                hr
            });
            Reply::hr(hr)
        }
        Op::LockServer(lock) => {
            let hr = apt::run_in(e.owner, || {
                // SAFETY: an exported factory; the reference the query added is given back.
                unsafe {
                    let (hr, f) = query(object, &IID_ICLASS_FACTORY);
                    if f.is_null() {
                        return hr;
                    }
                    let r = factory_lock(f, lock);
                    release_unknown(f);
                    r
                }
            });
            Reply::hr(hr)
        }
        Op::CreateInstance(iid) => {
            // The server count is raised while an activation from another process runs.
            COUNT.with(|c| c.enter_call());
            let (hr, identity) = apt::run_in(e.owner, || {
                // SAFETY: an exported factory; the made object's references are counted below.
                unsafe {
                    let (hr, f) = query(object, &IID_ICLASS_FACTORY);
                    if f.is_null() {
                        return (hr, core::ptr::null_mut());
                    }
                    let (hr, o) = factory_create(f, core::ptr::null_mut(), &iid);
                    release_unknown(f);
                    if o.is_null() {
                        return (hr, core::ptr::null_mut());
                    }
                    let (_, id) = query(o, &IID_IUNKNOWN);
                    release_unknown(o);
                    (hr, id)
                }
            });
            COUNT.with(|c| c.leave_call());
            if identity.is_null() {
                return Reply::hr(if hr >= 0 { E_UNEXPECTED } else { hr });
            }
            Reply { hr, obj: Some(export_object(identity as usize, e.owner)) }
        }
    }
}

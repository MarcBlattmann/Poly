//! The object exporter of this process (ADR 0220): a named pipe on which other processes call the
//! objects this process handed out — registered class factories (exported while their registration
//! is published) and the objects their `CreateInstance` made.
//!
//! Each connection carries one call; it runs in the object's apartment ([`super::apt::run_in`]).
//! The client's process comes from the pipe; a call the process's security does not admit is
//! refused with `E_ACCESSDENIED`, and an admitted one runs with its call context (B2c,
//! [`super::security`]).

use super::*;
use crate::{IID_IUNKNOWN, release_unknown};
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use poly_com::git::Owner;
use poly_com::hresult::{E_UNEXPECTED, S_OK};
use poly_com::security::E_ACCESSDENIED;
use poly_com::wire::{Caller, ObjRef, Op, Reply, Request, frame};

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
static OXID: AtomicU64 = AtomicU64::new(0);
static ENDPOINT: Locked<Option<String>> = Locked::new(None);

/// The pipe name of this process's exporter; the listener starts on first use, and the pipe exists
/// when this returns.
pub(crate) fn endpoint() -> String {
    loop {
        match STATE.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => {
                let mut r = [0u8; 8];
                // SAFETY: a buffer of 8 bytes.
                unsafe { crate::ProcessPrng(r.as_mut_ptr(), 8) };
                let oxid = u64::from_le_bytes(r);
                OXID.store(oxid, Ordering::Release);
                let name = poly_com::marshal::endpoint_for(pid(), oxid);
                ENDPOINT.with(|e| *e = Some(name.clone()));
                // The first instance exists before the name is handed out: whoever is told the
                // endpoint (the activation process checking a registration's factory) finds it.
                let first = new_instance(&wide(&name));
                let mut tid = 0u32;
                // SAFETY: the listener runs for the life of the process and owns `first`.
                let t = unsafe { CreateThread(core::ptr::null(), 0, listener, first as *mut c_void, 0, &mut tid) };
                close(t);
                STATE.store(2, Ordering::Release);
                return name;
            }
            Err(2) => return ENDPOINT.with(|e| e.clone()).unwrap_or_default(),
            Err(_) => core::hint::spin_loop(),
        }
    }
}

/// This process's exporter OXID (starts the exporter listener on first use).
pub(crate) fn oxid() -> u64 {
    let _ = endpoint();
    OXID.load(Ordering::Acquire)
}

/// A new listening instance of the exporter's pipe (`INVALID_HANDLE` on failure).
fn new_instance(name: &[u16]) -> Handle {
    // SAFETY: a NUL-terminated name; byte mode, blocking.
    unsafe {
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
    }
}

unsafe extern "system" fn listener(first: *mut c_void) -> u32 {
    let name = wide(&ENDPOINT.with(|e| e.clone()).unwrap_or_default());
    let mut h = first as Handle;
    loop {
        if h == INVALID_HANDLE {
            // SAFETY: a plain sleep.
            unsafe { Sleep(20) };
            h = new_instance(&name);
            continue;
        }
        // SAFETY: a pipe instance of ours.
        let ok = unsafe { ConnectNamedPipe(h, core::ptr::null_mut()) } != 0
            // SAFETY: no arguments.
            || unsafe { GetLastError() } == pipe::ERROR_PIPE_CONNECTED;
        if !ok {
            close(h);
            h = new_instance(&name);
            continue;
        }
        // The next instance listens before this connection is handed on, so that a client never
        // finds the pipe without a listening instance (calls come one connection each).
        let connected = h;
        h = new_instance(&name);
        let mut tid = 0u32;
        // SAFETY: the connection thread owns the handle from here.
        let t = unsafe { CreateThread(core::ptr::null(), 0, connection, connected as *mut c_void, 0, &mut tid) };
        if t == 0 {
            close(connected);
        } else {
            close(t);
        }
    }
}

unsafe extern "system" fn connection(arg: *mut c_void) -> u32 {
    let h = arg as Handle;
    if let Some(payload) = pipe::read_frame(h) {
        // The call is delivered: from here on a connection that breaks is a call cut off by this
        // process's end (`pipe::transact`).
        if !pipe::write_all(h, &[poly_com::wire::RECEIVED]) {
            close(h);
            return 0;
        }
        let reply = match Request::decode(&payload) {
            Ok(Request::Call { oid, op, caller }) => dispatch(oid, op, super::security::pipe_client(h), caller),
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

/// Add `n` proxy references to the live export `oid`; whether it exists.
pub(crate) fn add_refs(oid: u64, n: u32) -> bool {
    EXPORTS.with(|x| match x.iter_mut().find(|e| e.oid == oid && !e.factory && e.refs > 0) {
        Some(e) => {
            e.refs += n;
            true
        }
        None => false,
    })
}

/// The object of the live export `oid` (its identity; the export keeps its reference).
pub(crate) fn object_of(oid: u64) -> Option<(usize, Owner)> {
    EXPORTS.with(|x| x.iter().find(|e| e.oid == oid && !e.factory && e.refs > 0).map(|e| (e.object, e.owner)))
}

/// Give back `n` proxy references of `oid`; the last one releases the object.
pub(crate) fn release_refs(oid: u64, n: u32) {
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

/// Run one call from process `client`.
fn dispatch(oid: u64, op: Op, client: u32, caller: Caller) -> Reply {
    let Some(e) = find(oid) else { return Reply::hr(RPC_E_DISCONNECTED) };
    let object = e.object as *mut c_void;
    // Reference counts are the proxies' own business; everything else is a call the process's
    // security decides on (sheet `CoInitializeSecurity`, rule `access_check`).
    if !matches!(op, Op::Release(_) | Op::AddRef { .. }) && !super::security::admits(client) {
        return Reply::hr(E_ACCESSDENIED);
    }
    match op {
        // A method of an interface marshalled through its proxy/stub class (B2b).
        Op::Invoke { iid, method, data } => {
            COUNT.with(|c| c.enter_call());
            let r = apt::run_in(e.owner, || {
                super::security::serve(client, caller, || {
                    let mut out = Err(guard::RPC_E_SERVERFAULT);
                    // An exception the process lets COM catch ends the call with a server fault.
                    let _ = guard::guarded(|| out = super::channel::invoke_stub(e.object, &iid, method, &data));
                    out
                })
            });
            COUNT.with(|c| c.leave_call());
            match r {
                Ok(data) => Reply { hr: S_OK, obj: None, data },
                Err(hr) => Reply::hr(hr),
            }
        }
        // References for a proxy another process marshalled on (B2b); with a token, only while
        // that table registration lives.
        Op::AddRef { n, token } => {
            if token != 0 && !super::marshal::table_has(token) {
                return Reply::hr(poly_com::hresult::CO_E_OBJNOTREG);
            }
            if add_refs(oid, n) { Reply::hr(S_OK) } else { Reply::hr(RPC_E_DISCONNECTED) }
        }
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
                super::security::serve(client, caller, || {
                    // SAFETY: an exported factory; the reference the query added is given back.
                    unsafe {
                        let (hr, f) = query(object, &IID_ICLASS_FACTORY);
                        if f.is_null() {
                            return hr;
                        }
                        let mut r = guard::RPC_E_SERVERFAULT;
                        let _ = guard::guarded(|| r = factory_lock(f, lock));
                        release_unknown(f);
                        r
                    }
                })
            });
            Reply::hr(hr)
        }
        Op::CreateInstance(iid) => {
            // The server count is raised while an activation from another process runs.
            COUNT.with(|c| c.enter_call());
            let (hr, identity) = apt::run_in(e.owner, || {
                super::security::serve(client, caller, || {
                    // SAFETY: an exported factory; the made object's references are counted below.
                    unsafe {
                        let (hr, f) = query(object, &IID_ICLASS_FACTORY);
                        if f.is_null() {
                            return (hr, core::ptr::null_mut());
                        }
                        let mut made = (guard::RPC_E_SERVERFAULT, core::ptr::null_mut());
                        let _ = guard::guarded(|| made = factory_create(f, core::ptr::null_mut(), &iid));
                        let (hr, o) = made;
                        release_unknown(f);
                        if o.is_null() {
                            return (hr, core::ptr::null_mut());
                        }
                        let (_, id) = query(o, &IID_IUNKNOWN);
                        release_unknown(o);
                        (hr, id)
                    }
                })
            });
            COUNT.with(|c| c.leave_call());
            if identity.is_null() {
                return Reply::hr(if hr >= 0 { E_UNEXPECTED } else { hr });
            }
            Reply { hr, obj: Some(export_object(identity as usize, e.owner)), data: Vec::new() }
        }
    }
}

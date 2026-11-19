//! Proxies (ADR 0220): what a caller gets for an object of another apartment or another process.
//!
//! One proxy per object and process (identity holds), with three interfaces in one block:
//! `IUnknown` (the identity), `IClassFactory` (when the object has it) and `IMarshal` (answered by
//! the proxy itself, as Windows' proxy manager does; its methods are not implemented in B2a).
//! `AddRef` and `Release` count locally; the last `Release` gives the object back. Other
//! interfaces need marshalling through a type library (B2b) and are `E_NOINTERFACE` today.

use super::*;
use crate::{IID_IMARSHAL, IID_IUNKNOWN, add_ref_unknown, release_unknown};
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};
use poly_com::git::Owner;
use poly_com::hresult::{CLASS_E_NOAGGREGATION, E_NOINTERFACE, E_NOTIMPL, E_POINTER, S_OK};
use poly_com::wire::{ObjRef, Op, Request};

/// Where the object is.
enum Target {
    /// Another apartment of this process: the object itself (the proxy holds one reference).
    Local { owner: Owner, object: usize },
    /// Another process: its exporter and the object's id there; the references the proxy holds.
    Remote { endpoint: String, oid: u64, remote_refs: AtomicU32 },
}

#[repr(C)]
struct Proxy {
    unk: *const UnkVtbl,
    fac: *const FacVtbl,
    mar: *const MarVtbl,
    refs: AtomicU32,
    /// Whether the object has `IClassFactory`: 0 not asked yet, 1 yes, 2 no.
    factory: AtomicU32,
    target: Target,
    /// Interface proxies made by proxy/stub classes (B2b): (interface id, interface pointer,
    /// its `IRpcProxyBuffer`). Their references count on this block.
    ifaces: Locked<Vec<(Guid, usize, usize)>>,
}

const PTR: usize = core::mem::size_of::<usize>();

type Qi = unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> Hresult;
type Ref = unsafe extern "system" fn(*mut c_void) -> u32;

#[repr(C)]
struct UnkVtbl {
    qi: Qi,
    add_ref: Ref,
    release: Ref,
}

#[repr(C)]
struct FacVtbl {
    qi: Qi,
    add_ref: Ref,
    release: Ref,
    create: unsafe extern "system" fn(*mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> Hresult,
    lock: unsafe extern "system" fn(*mut c_void, i32) -> Hresult,
}

#[repr(C)]
struct MarVtbl {
    qi: Qi,
    add_ref: Ref,
    release: Ref,
    get_unmarshal_class: unsafe extern "system" fn(
        *mut c_void,
        *const Guid,
        *mut c_void,
        u32,
        *mut c_void,
        u32,
        *mut Guid,
    ) -> Hresult,
    get_marshal_size_max: unsafe extern "system" fn(
        *mut c_void,
        *const Guid,
        *mut c_void,
        u32,
        *mut c_void,
        u32,
        *mut u32,
    ) -> Hresult,
    marshal_interface: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *const Guid,
        *mut c_void,
        u32,
        *mut c_void,
        u32,
    ) -> Hresult,
    unmarshal_interface: unsafe extern "system" fn(*mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> Hresult,
    release_marshal_data: unsafe extern "system" fn(*mut c_void, *mut c_void) -> Hresult,
    disconnect_object: unsafe extern "system" fn(*mut c_void, u32) -> Hresult,
}

// SAFETY: tables of function pointers.
unsafe impl Sync for UnkVtbl {}
// SAFETY: as above.
unsafe impl Sync for FacVtbl {}
// SAFETY: as above.
unsafe impl Sync for MarVtbl {}

static UNK: UnkVtbl = UnkVtbl { qi: unk_qi, add_ref: unk_add_ref, release: unk_release };
static FAC: FacVtbl =
    FacVtbl { qi: fac_qi, add_ref: fac_add_ref, release: fac_release, create: fac_create, lock: fac_lock };
static MAR: MarVtbl = MarVtbl {
    qi: mar_qi,
    add_ref: mar_add_ref,
    release: mar_release,
    get_unmarshal_class: marshal::proxy_get_unmarshal_class,
    get_marshal_size_max: marshal::proxy_get_marshal_size_max,
    marshal_interface: marshal::proxy_marshal_interface,
    unmarshal_interface: marshal::proxy_unmarshal_interface,
    release_marshal_data: marshal::proxy_release_marshal_data,
    disconnect_object: marshal::proxy_disconnect_object,
};

/// The proxies of the process, for identity: (object key, proxy).
#[derive(Clone, PartialEq, Eq)]
enum Key {
    Local(Owner, usize),
    Remote(String, u64),
}

static PROXIES: Locked<Vec<(Key, usize)>> = Locked::new(Vec::new());

fn key_of(p: &Proxy) -> Key {
    match &p.target {
        Target::Local { owner, object } => Key::Local(*owner, *object),
        Target::Remote { endpoint, oid, .. } => Key::Remote(endpoint.clone(), *oid),
    }
}

/// The `IUnknown` of the proxy for `key`, one reference added, if one exists.
fn existing(key: &Key, more_remote: bool) -> Option<*mut c_void> {
    PROXIES.with(|t| {
        let &(_, p) = t.iter().find(|(k, _)| k == key)?;
        let proxy = p as *const Proxy;
        // SAFETY: a live proxy (the table holds proxies whose count is above 0; a proxy at 0 is
        // removed under this lock before it is freed).
        unsafe {
            if (*proxy).refs.load(Ordering::Acquire) == 0 {
                return None;
            }
            (*proxy).refs.fetch_add(1, Ordering::AcqRel);
            if more_remote {
                if let Target::Remote { remote_refs, .. } = &(*proxy).target {
                    remote_refs.fetch_add(1, Ordering::AcqRel);
                }
            }
        }
        Some(p as *mut c_void)
    })
}

fn make(target: Target) -> *mut c_void {
    let p = Box::into_raw(Box::new(Proxy {
        unk: &UNK,
        fac: &FAC,
        mar: &MAR,
        refs: AtomicU32::new(1),
        factory: AtomicU32::new(0),
        target,
        ifaces: Locked::new(Vec::new()),
    }));
    // SAFETY: just made.
    let key = key_of(unsafe { &*p });
    PROXIES.with(|t| t.push((key, p as usize)));
    p.cast()
}

/// A proxy for `object` of the apartment `owner` (this process): its `IUnknown`, one reference.
pub(crate) fn local(owner: Owner, object: usize) -> *mut c_void {
    if let Some(p) = existing(&Key::Local(owner, object), false) {
        return p;
    }
    // SAFETY: the caller's live object; the proxy keeps this reference.
    unsafe { add_ref_unknown(object as *mut c_void) };
    make(Target::Local { owner, object })
}

/// A proxy for the exported object `r` (one reference of it, which the proxy takes over).
pub(crate) fn remote(r: ObjRef) -> *mut c_void {
    let key = Key::Remote(r.endpoint.clone(), r.oid);
    if let Some(p) = existing(&key, true) {
        return p;
    }
    make(Target::Remote { endpoint: r.endpoint, oid: r.oid, remote_refs: AtomicU32::new(1) })
}

/// Whether `p` is one of our proxies (any of its interfaces).
pub(crate) fn is_proxy(p: *mut c_void) -> bool {
    // SAFETY: a COM interface pointer: its first word is its vtable.
    let v = unsafe { *(p as *const usize) };
    v == (&raw const UNK) as usize || v == (&raw const FAC) as usize || v == (&raw const MAR) as usize
}

/// Marshalling target of one of our proxies.
pub(crate) enum ProxyMarshalInfo {
    Local { owner: Owner, object: usize },
    Remote { endpoint: String, oid: u64 },
}

/// If `p` is one of our proxies, return its underlying target for marshalling.
pub(crate) fn marshal_info(p: *mut c_void) -> Option<ProxyMarshalInfo> {
    if !is_proxy(p) {
        return None;
    }
    // SAFETY: one of our proxies' interfaces: find the block from its vtable.
    let base = unsafe { base_of(p) };
    // SAFETY: a live proxy.
    match unsafe { &(*base).target } {
        Target::Local { owner, object } => Some(ProxyMarshalInfo::Local { owner: *owner, object: *object }),
        Target::Remote { endpoint, oid, .. } => {
            Some(ProxyMarshalInfo::Remote { endpoint: endpoint.clone(), oid: *oid })
        }
    }
}

/// Whether `p` is a proxy of an object in another process.
pub(crate) fn is_remote(p: *mut c_void) -> bool {
    if !is_proxy(p) {
        return false;
    }
    // SAFETY: one of our proxies' interfaces: find the block from its vtable.
    let base = unsafe { base_of(p) };
    // SAFETY: a live proxy.
    matches!(unsafe { &(*base).target }, Target::Remote { .. })
}

/// The proxy block of one of its interface pointers.
unsafe fn base_of(p: *mut c_void) -> *mut Proxy {
    // SAFETY: the caller's interface pointer of a proxy.
    let v = unsafe { *(p as *const usize) };
    let off = if v == (&raw const FAC) as usize {
        PTR
    } else if v == (&raw const MAR) as usize {
        2 * PTR
    } else {
        0
    };
    (p as usize - off) as *mut Proxy
}

/// Call the remote object; the reply, or the failure as an `HRESULT`.
fn call(p: &Proxy, op: Op) -> Result<poly_com::wire::Reply, Hresult> {
    match &p.target {
        Target::Remote { endpoint, oid, .. } => {
            pipe::transact(endpoint, &Request::Call { oid: *oid, op }).map_err(pipe::Failure::hr)
        }
        Target::Local { .. } => Err(E_NOTIMPL),
    }
}

/// Whether the object of the proxy `px` (its `IUnknown`) can still be called: false when its
/// server let it go or ended. A remote factory proxy also learns whether it is a factory.
pub(crate) fn ping(px: *mut c_void) -> bool {
    // SAFETY: one of our proxies' IUnknown.
    let p = unsafe { &*base_of(px) };
    match call(p, Op::QueryInterface(IID_ICLASS_FACTORY)) {
        Ok(r) if r.hr == RPC_E_DISCONNECTED => false,
        Ok(r) => {
            p.factory.store(if r.hr == S_OK { 1 } else { 2 }, Ordering::Release);
            true
        }
        Err(_) => false,
    }
}

/// Whether the object has `IClassFactory` (asked once).
fn has_factory(p: &Proxy) -> bool {
    match p.factory.load(Ordering::Acquire) {
        1 => return true,
        2 => return false,
        _ => {}
    }
    let yes = match &p.target {
        Target::Local { owner, object } => {
            let o = *object as *mut c_void;
            apt::run_in(*owner, || {
                // SAFETY: the proxy's object; the reference the query added is given back.
                let (hr, f) = unsafe { query(o, &IID_ICLASS_FACTORY) };
                if !f.is_null() {
                    // SAFETY: as above.
                    unsafe { release_unknown(f) };
                }
                hr == S_OK
            })
        }
        Target::Remote { .. } => matches!(call(p, Op::QueryInterface(IID_ICLASS_FACTORY)), Ok(r) if r.hr == S_OK),
    };
    p.factory.store(if yes { 1 } else { 2 }, Ordering::Release);
    yes
}

unsafe fn qi(p: *mut Proxy, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter and IID; a live proxy.
    unsafe {
        *out = core::ptr::null_mut();
        if iid.is_null() {
            return E_POINTER;
        }
        let iid = *iid;
        let slot = if iid == IID_IUNKNOWN {
            0
        } else if iid == IID_IMARSHAL {
            2
        } else if iid == IID_ICLASS_FACTORY && has_factory(&*p) {
            1
        } else {
            return iface_proxy(p, &iid, out);
        };
        (*p).refs.fetch_add(1, Ordering::AcqRel);
        *out = (p as usize + slot * PTR) as *mut c_void;
    }
    S_OK
}

/// The proxy of interface `iid` of the object (B2b): one made earlier, or a new one from the
/// interface's proxy/stub class, connected to a channel to the object. `E_NOINTERFACE` when the
/// object does not have the interface or nothing can marshal it.
unsafe fn iface_proxy(p: *mut Proxy, iid: &Guid, out: *mut *mut c_void) -> Hresult {
    // SAFETY: a live proxy.
    let proxy = unsafe { &*p };
    if let Some(itf) = proxy.ifaces.with(|v| v.iter().find(|e| e.0 == *iid).map(|e| e.1)) {
        // SAFETY: an interface of this block: its AddRef counts on the block.
        unsafe {
            add_ref(p);
            *out = itf as *mut c_void;
        }
        return S_OK;
    }
    // Does the object have it?
    let hr = match &proxy.target {
        Target::Local { owner, object } => {
            let o = *object as *mut c_void;
            let g = *iid;
            apt::run_in(*owner, || {
                // SAFETY: the proxy's object; the query's reference is given back at once.
                let (hr, q) = unsafe { query(o, &g) };
                if !q.is_null() {
                    // SAFETY: as above.
                    unsafe { release_unknown(q) };
                }
                hr
            })
        }
        Target::Remote { .. } => match call(proxy, Op::QueryInterface(*iid)) {
            Ok(r) => r.hr,
            Err(hr) => hr,
        },
    };
    if hr < 0 {
        return if hr == RPC_E_DISCONNECTED || hr == RPC_S_SERVER_UNAVAILABLE { hr } else { E_NOINTERFACE };
    }
    let Ok(f) = super::channel::ps_factory(iid) else { return E_NOINTERFACE };
    let mut buf: *mut c_void = core::ptr::null_mut();
    let mut pv: *mut c_void = core::ptr::null_mut();
    // SAFETY: IPSFactoryBuffer::CreateProxy (slot 3) with this block's IUnknown as the outer object.
    let hr = unsafe {
        let v = *(f as *const *const usize);
        let create: unsafe extern "system" fn(*mut c_void, *mut c_void, *const Guid, *mut *mut c_void, *mut *mut c_void) -> Hresult =
            core::mem::transmute(*v.add(3));
        let hr = create(f, p.cast(), iid, &mut buf, &mut pv);
        release_unknown(f);
        hr
    };
    if hr < 0 || buf.is_null() {
        return E_NOINTERFACE;
    }
    // The interface pointer: asked of the proxy buffer (its AddRef goes to this block).
    // SAFETY: an IRpcProxyBuffer.
    let (qhr, itf) = unsafe { query(buf, iid) };
    if qhr < 0 || itf.is_null() {
        // SAFETY: the buffer's reference.
        unsafe { release_unknown(buf) };
        return E_NOINTERFACE;
    }
    let target = match &proxy.target {
        Target::Local { owner, object } => super::channel::Target::Local { owner: *owner, object: *object },
        Target::Remote { endpoint, oid, .. } => super::channel::Target::Remote { endpoint: endpoint.clone(), oid: *oid },
    };
    let chan = super::channel::new_channel(*iid, target);
    // SAFETY: IRpcProxyBuffer::Connect (slot 3); the channel keeps its own reference.
    unsafe {
        let v = *(buf as *const *const usize);
        let connect: unsafe extern "system" fn(*mut c_void, *mut c_void) -> Hresult = core::mem::transmute(*v.add(3));
        connect(buf, chan);
        release_unknown(chan);
    }
    let raced = proxy.ifaces.with(|v| {
        if let Some(e) = v.iter().find(|e| e.0 == *iid) {
            Some(e.1)
        } else {
            v.push((*iid, itf as usize, buf as usize));
            None
        }
    });
    if let Some(other) = raced {
        // Another thread made it first: use that one, drop ours.
        // SAFETY: our interface reference (on this block) and our buffer.
        unsafe {
            release_unknown(itf);
            disconnect_buffer(buf as usize);
            add_ref(p);
            *out = other as *mut c_void;
        }
        return S_OK;
    }
    // SAFETY: the caller's out pointer; the query added this block's reference.
    unsafe { *out = itf };
    S_OK
}

/// Disconnect and release an interface proxy's buffer.
unsafe fn disconnect_buffer(buf: usize) {
    let b = buf as *mut c_void;
    // SAFETY: an IRpcProxyBuffer: Disconnect (slot 4), then its reference.
    unsafe {
        let v = *(b as *const *const usize);
        let disconnect: unsafe extern "system" fn(*mut c_void) = core::mem::transmute(*v.add(4));
        disconnect(b);
        release_unknown(b);
    }
}

unsafe fn add_ref(p: *mut Proxy) -> u32 {
    // SAFETY: a live proxy.
    unsafe { (*p).refs.fetch_add(1, Ordering::AcqRel) + 1 }
}

unsafe fn release(p: *mut Proxy) -> u32 {
    // SAFETY: a live proxy.
    let n = unsafe { (*p).refs.fetch_sub(1, Ordering::AcqRel) } - 1;
    if n != 0 {
        return n;
    }
    // Out of the table first (a lookup that raced us and raised the count keeps it alive).
    let gone = PROXIES.with(|t| {
        // SAFETY: as above.
        if unsafe { (*p).refs.load(Ordering::Acquire) } != 0 {
            return false;
        }
        t.retain(|(_, q)| *q != p as usize);
        true
    });
    if gone {
        // SAFETY: the last reference of a proxy made by `make`.
        let b = unsafe { Box::from_raw(p) };
        for (_, _, buf) in b.ifaces.with(core::mem::take) {
            // SAFETY: the block's interface proxies (their pointers' references were all on it).
            unsafe { disconnect_buffer(buf) };
        }
        match &b.target {
            Target::Local { object, .. } => {
                // SAFETY: the proxy's reference of the object.
                unsafe { release_unknown(*object as *mut c_void) };
            }
            Target::Remote { remote_refs, .. } => {
                let n = remote_refs.load(Ordering::Acquire);
                let _ = call(&b, Op::Release(n));
            }
        }
    }
    0
}

unsafe fn create(p: *mut Proxy, outer: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = core::ptr::null_mut() };
    if iid.is_null() {
        return E_POINTER;
    }
    if !outer.is_null() {
        // A proxy cannot be aggregated; the server is not asked (sheet: aggregation_remote).
        return CLASS_E_NOAGGREGATION;
    }
    // SAFETY: the caller's IID; a live proxy.
    let (iid, proxy) = unsafe { (*iid, &*p) };
    let made = match &proxy.target {
        Target::Local { owner, object } => {
            let o = *object as *mut c_void;
            let owner = *owner;
            let (hr, id) = apt::run_in(owner, || {
                // SAFETY: the proxy's object: a class factory; the made object's identity is kept.
                unsafe {
                    let (hr, f) = query(o, &IID_ICLASS_FACTORY);
                    if f.is_null() {
                        return (hr, 0usize);
                    }
                    let (hr, made) = factory_create(f, core::ptr::null_mut(), &iid);
                    release_unknown(f);
                    if made.is_null() {
                        return (hr, 0);
                    }
                    let (_, id) = query(made, &IID_IUNKNOWN);
                    release_unknown(made);
                    (hr, id as usize)
                }
            });
            if id == 0 {
                return hr;
            }
            let px = local(owner, id);
            // SAFETY: the identity's reference from the query; the proxy holds its own.
            unsafe { release_unknown(id as *mut c_void) };
            px
        }
        // The server is asked for IUnknown; the interface comes from a query (recorded runs: the
        // servers log `create IUnknown` whatever the client asked for).
        Target::Remote { .. } => match call(proxy, Op::CreateInstance(IID_IUNKNOWN)) {
            Ok(r) => match r.obj {
                Some(o) if r.hr >= 0 => remote(o),
                _ => return r.hr,
            },
            Err(hr) => return hr,
        },
    };
    // The new object's proxy, as the interface asked for.
    // SAFETY: `made` is a proxy's IUnknown with one reference, given back after the query.
    unsafe {
        let hr = qi(made.cast(), &iid, out);
        release(made.cast());
        hr
    }
}

unsafe fn lock(p: *mut Proxy, l: i32) -> Hresult {
    // SAFETY: a live proxy.
    let proxy = unsafe { &*p };
    match &proxy.target {
        Target::Local { owner, object } => {
            let o = *object as *mut c_void;
            apt::run_in(*owner, || {
                // SAFETY: the proxy's object; the reference the query added is given back.
                unsafe {
                    let (hr, f) = query(o, &IID_ICLASS_FACTORY);
                    if f.is_null() {
                        return hr;
                    }
                    let r = factory_lock(f, l != 0);
                    release_unknown(f);
                    r
                }
            })
        }
        // The sheet saw S_OK also through the proxy of a factory its server had revoked.
        Target::Remote { .. } => match call(proxy, Op::LockServer(l != 0)) {
            Ok(r) if r.hr != RPC_E_DISCONNECTED => r.hr,
            _ => S_OK,
        },
    }
}

unsafe extern "system" fn unk_qi(this: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    // SAFETY: our IUnknown at offset 0.
    unsafe { qi(this.cast(), iid, out) }
}
unsafe extern "system" fn unk_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: as above.
    unsafe { add_ref(this.cast()) }
}
unsafe extern "system" fn unk_release(this: *mut c_void) -> u32 {
    // SAFETY: as above.
    unsafe { release(this.cast()) }
}
unsafe extern "system" fn fac_qi(this: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    // SAFETY: our IClassFactory one pointer into the block.
    unsafe { qi((this as usize - PTR) as *mut Proxy, iid, out) }
}
unsafe extern "system" fn fac_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: as above.
    unsafe { add_ref((this as usize - PTR) as *mut Proxy) }
}
unsafe extern "system" fn fac_release(this: *mut c_void) -> u32 {
    // SAFETY: as above.
    unsafe { release((this as usize - PTR) as *mut Proxy) }
}
unsafe extern "system" fn fac_create(
    this: *mut c_void,
    outer: *mut c_void,
    iid: *const Guid,
    out: *mut *mut c_void,
) -> Hresult {
    // SAFETY: as above.
    unsafe { create((this as usize - PTR) as *mut Proxy, outer, iid, out) }
}
unsafe extern "system" fn fac_lock(this: *mut c_void, l: i32) -> Hresult {
    // SAFETY: as above.
    unsafe { lock((this as usize - PTR) as *mut Proxy, l) }
}
unsafe extern "system" fn mar_qi(this: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    // SAFETY: our IMarshal two pointers into the block.
    unsafe { qi((this as usize - 2 * PTR) as *mut Proxy, iid, out) }
}
unsafe extern "system" fn mar_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: as above.
    unsafe { add_ref((this as usize - 2 * PTR) as *mut Proxy) }
}
unsafe extern "system" fn mar_release(this: *mut c_void) -> u32 {
    // SAFETY: as above.
    unsafe { release((this as usize - 2 * PTR) as *mut Proxy) }
}

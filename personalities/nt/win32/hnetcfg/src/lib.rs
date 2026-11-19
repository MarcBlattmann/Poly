//! Our `hnetcfg.dll`: the Windows Firewall's COM objects as Chrome's installer uses them (create a
//! `NetFwRule`, set its properties, `INetFwPolicy2::get_Rules` and `INetFwRules::Add` it, later
//! `Remove` it by name). Poly has no host firewall: the rules live in a table of the process and
//! enforce nothing, which the spec sheet allows (`specs/windows/hnetcfg.dll/NetFwPolicy2.yaml`).
//! The values each property takes are `poly_kernelbase::netfw`'s; this crate is the COM shell
//! around them. The classes reach `CoCreateInstance` through `poly_com::inproc` (ADR 0223).
//!
//! The interfaces are dual (`IDispatch`-derived); their methods follow `IDispatch`'s seven in the
//! order of `netfw.h` (Learn: "INetFwPolicy2", "INetFwRules", "INetFwRule"). Late binding
//! (`IDispatch::Invoke`) is not offered: it answers `DISP_E_MEMBERNOTFOUND`.

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicI16, AtomicI32, AtomicU32, AtomicUsize, Ordering};
use poly_com::guid::Guid;
use poly_com::inproc::{CLSID_NET_FW_POLICY2, CLSID_NET_FW_RULE, IID_ICLASS_FACTORY, IID_IUNKNOWN};
use poly_kernelbase::netfw::{self, Rule};

poly_win32_rt::process_heap_allocator!();

type Hresult = i32;

const S_OK: Hresult = 0;
const S_FALSE: Hresult = 1;
const E_NOTIMPL: Hresult = 0x8000_4001_u32 as i32;
const E_NOINTERFACE: Hresult = 0x8000_4002_u32 as i32;
const E_POINTER: Hresult = netfw::E_POINTER as i32;
const E_INVALIDARG: Hresult = netfw::E_INVALIDARG as i32;
const E_OUTOFMEMORY: Hresult = 0x8007_000E_u32 as i32;
const CLASS_E_NOAGGREGATION: Hresult = 0x8004_0110_u32 as i32;
const CLASS_E_CLASSNOTAVAILABLE: Hresult = 0x8004_0111_u32 as i32;
/// `DISP_E_UNKNOWNNAME`, `DISP_E_MEMBERNOTFOUND` (`winerror.h`).
const DISP_E_UNKNOWNNAME: Hresult = 0x8002_0006_u32 as i32;
const DISP_E_MEMBERNOTFOUND: Hresult = 0x8002_0003_u32 as i32;
/// `VT_DISPATCH` (`wtypes.h`).
const VT_DISPATCH: u16 = 9;

const fn guid(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> Guid {
    Guid { data1, data2, data3, data4 }
}

// `oaidl.h`, `netfw.h`.
const IID_IDISPATCH: Guid = guid(0x0002_0400, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IENUM_VARIANT: Guid = guid(0x0002_0404, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_INET_FW_POLICY2: Guid = guid(0x9832_5047, 0xC671, 0x4174, [0x8D, 0x81, 0xDE, 0xFC, 0xD3, 0xF0, 0x31, 0x86]);
const IID_INET_FW_RULES: Guid = guid(0x9C4C_6277, 0x5027, 0x441E, [0xAF, 0xAE, 0xCA, 0x1F, 0x54, 0x2D, 0xA0, 0x09]);
const IID_INET_FW_RULE: Guid = guid(0xAF23_0D27, 0xBABA, 0x4E42, [0xAC, 0xED, 0xF5, 0x24, 0xF2, 0x2C, 0xFC, 0xE2]);

#[cfg_attr(target_arch = "x86", link(name = "oleaut32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "oleaut32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn SysAllocStringLen(text: *const u16, len: u32) -> *mut u16;
    fn SysStringLen(b: *const u16) -> u32;
}

// ----- a spin lock (the objects may be used from any thread of the multithreaded apartment)

struct Lock<T> {
    taken: AtomicBool,
    value: UnsafeCell<T>,
}

// SAFETY: access to the value is serialised by `taken`.
unsafe impl<T: Send> Sync for Lock<T> {}

impl<T> Lock<T> {
    const fn new(v: T) -> Lock<T> {
        Lock { taken: AtomicBool::new(false), value: UnsafeCell::new(v) }
    }

    fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        while self.taken.swap(true, Ordering::Acquire) {
            core::hint::spin_loop();
        }
        // SAFETY: we hold the lock.
        let r = f(unsafe { &mut *self.value.get() });
        self.taken.store(false, Ordering::Release);
        r
    }
}

/// A rule's properties, shared by the rule objects that show it and the table that holds it (a
/// change made on the object after `Add` is the table's change too).
type Shared = Arc<Lock<Rule>>;

/// The process's rules (Poly's built-in ones until the first call changes them). Locked before a
/// rule's own lock, never after.
static TABLE: Lock<Option<Vec<Shared>>> = Lock::new(None);

fn builtin_table() -> Vec<Shared> {
    netfw::builtin_rules().into_iter().map(|r| Arc::new(Lock::new(r))).collect()
}

fn with_table<R>(f: impl FnOnce(&mut Vec<Shared>) -> R) -> R {
    TABLE.with(|t| f(t.get_or_insert_with(builtin_table)))
}

// ----- objects

/// A method table: `N` function pointers.
#[repr(transparent)]
struct Vtbl<const N: usize>([*const c_void; N]);
// SAFETY: function pointers only, never written.
unsafe impl<const N: usize> Sync for Vtbl<N> {}

/// A COM object: its method table, its references, the interfaces it answers, its data.
#[repr(C)]
struct Object<T> {
    vtbl: *const c_void,
    refs: AtomicU32,
    iids: &'static [Guid],
    data: T,
}

fn new_object<T, const N: usize>(vtbl: &'static Vtbl<N>, iids: &'static [Guid], data: T) -> *mut c_void {
    Box::into_raw(Box::new(Object { vtbl: (vtbl as *const Vtbl<N>).cast(), refs: AtomicU32::new(1), iids, data }))
        .cast()
}

unsafe fn data<'a, T>(p: *mut c_void) -> &'a T {
    // SAFETY: (caller) `p` is an `Object<T>` of ours.
    unsafe { &(*p.cast::<Object<T>>()).data }
}

unsafe extern "system" fn query<T>(p: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's pointers; `p` is our object.
    unsafe {
        *out = core::ptr::null_mut();
        if iid.is_null() {
            return E_INVALIDARG;
        }
        let iid = iid.read_unaligned();
        if !(*p.cast::<Object<T>>()).iids.contains(&iid) {
            return E_NOINTERFACE;
        }
        add_ref::<T>(p);
        *out = p;
    }
    S_OK
}

unsafe extern "system" fn add_ref<T>(p: *mut c_void) -> u32 {
    // SAFETY: our object.
    unsafe { (*p.cast::<Object<T>>()).refs.fetch_add(1, Ordering::AcqRel) + 1 }
}

unsafe extern "system" fn release<T>(p: *mut c_void) -> u32 {
    // SAFETY: our object; freed with its last reference.
    unsafe {
        let left = (*p.cast::<Object<T>>()).refs.fetch_sub(1, Ordering::AcqRel) - 1;
        if left == 0 {
            drop(Box::from_raw(p.cast::<Object<T>>()));
        }
        left
    }
}

/// Hand out a fresh object as `iid` (the object's own reference is given back).
unsafe fn hand_out<T>(object: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    // SAFETY: our fresh object and the caller's pointers.
    unsafe {
        let hr = query::<T>(object, iid, out);
        release::<T>(object);
        hr
    }
}

// ----- IDispatch (slots 3 to 6): no type information, no late binding

unsafe extern "system" fn type_info_count(_: *mut c_void, count: *mut u32) -> Hresult {
    if count.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *count = 0 };
    S_OK
}

unsafe extern "system" fn type_info(_: *mut c_void, _: u32, _: u32, out: *mut *mut c_void) -> Hresult {
    if !out.is_null() {
        // SAFETY: the caller's out parameter.
        unsafe { *out = core::ptr::null_mut() };
    }
    E_NOTIMPL
}

unsafe extern "system" fn ids_of_names(
    _: *mut c_void,
    _: *const Guid,
    _: *const *const u16,
    _: u32,
    _: u32,
    _: *mut i32,
) -> Hresult {
    DISP_E_UNKNOWNNAME
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn invoke(
    _: *mut c_void,
    _: i32,
    _: *const Guid,
    _: u32,
    _: u16,
    _: *mut c_void,
    _: *mut c_void,
    _: *mut c_void,
    _: *mut u32,
) -> Hresult {
    DISP_E_MEMBERNOTFOUND
}

macro_rules! dispatch_slots {
    ($t:ty) => {
        [
            query::<$t> as *const c_void,
            add_ref::<$t> as *const c_void,
            release::<$t> as *const c_void,
            type_info_count as *const c_void,
            type_info as *const c_void,
            ids_of_names as *const c_void,
            invoke as *const c_void,
        ]
    };
}

/// The first `N` slots of `head`, the rest from `tail`.
const fn join<const A: usize, const B: usize, const N: usize>(
    head: [*const c_void; A],
    tail: [*const c_void; B],
) -> Vtbl<N> {
    let mut v = [core::ptr::null(); N];
    let mut i = 0;
    while i < A {
        v[i] = head[i];
        i += 1;
    }
    while i < N {
        v[i] = tail[i - A];
        i += 1;
    }
    Vtbl(v)
}

// ----- BSTRs and out parameters

/// A `BSTR` argument as its characters (`None` for NULL).
unsafe fn bstr_in(b: *const u16) -> Option<Vec<u16>> {
    if b.is_null() {
        return None;
    }
    // SAFETY: the caller's BSTR: `SysStringLen` characters at `b`.
    Some(unsafe { core::slice::from_raw_parts(b, SysStringLen(b) as usize) }.to_vec())
}

/// A `BSTR` out parameter: a new string, or NULL for `None`.
unsafe fn bstr_out(out: *mut *mut u16, v: Option<&[u16]>) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter; `v` is `len` characters.
    unsafe {
        *out = core::ptr::null_mut();
        if let Some(v) = v {
            let b = SysAllocStringLen(v.as_ptr(), v.len() as u32);
            if b.is_null() {
                return E_OUTOFMEMORY;
            }
            *out = b;
        }
    }
    S_OK
}

unsafe fn put<T>(out: *mut T, v: T) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { out.write_unaligned(v) };
    S_OK
}

fn hr(r: Result<(), u32>) -> Hresult {
    match r {
        Ok(()) => S_OK,
        Err(e) => e as i32,
    }
}

fn variant_bool(b: bool) -> i16 {
    if b { netfw::VARIANT_TRUE } else { 0 }
}

/// `VARIANT` (`oaidl.h`, x64: 24 bytes; the type at 0, the value at 8).
#[repr(C)]
struct Variant {
    vt: u16,
    reserved: [u16; 3],
    value: usize,
    extra: usize,
}

// ----- NetFwRule (INetFwRule)

/// The rule object's data.
type RuleData = Shared;

const RULE_IIDS: &[Guid] = &[IID_IUNKNOWN, IID_IDISPATCH, IID_INET_FW_RULE];

fn new_rule_object(shared: Shared) -> *mut c_void {
    new_object(&RULE_VTBL, RULE_IIDS, shared)
}

unsafe fn with_rule<R>(p: *mut c_void, f: impl FnOnce(&mut Rule) -> R) -> R {
    // SAFETY: (caller) one of our rule objects.
    unsafe { data::<RuleData>(p) }.with(f)
}

/// The string properties, by number: `Name` 0, `Description` 1, `ApplicationName` 2,
/// `ServiceName` 3, `LocalPorts` 4, `RemotePorts` 5, `LocalAddresses` 6, `RemoteAddresses` 7,
/// `IcmpTypesAndCodes` 8, `InterfaceTypes` 9, `Grouping` 10.
fn text_field(r: &mut Rule, f: usize) -> &mut Option<Vec<u16>> {
    match f {
        0 => &mut r.name,
        1 => &mut r.description,
        2 => &mut r.application,
        3 => &mut r.service,
        4 => &mut r.local_ports,
        5 => &mut r.remote_ports,
        6 => &mut r.local_addresses,
        7 => &mut r.remote_addresses,
        8 => &mut r.icmp_types,
        9 => &mut r.interface_types,
        _ => &mut r.grouping,
    }
}

unsafe extern "system" fn get_text<const F: usize>(p: *mut c_void, out: *mut *mut u16) -> Hresult {
    // SAFETY: our rule object; the caller's out parameter.
    unsafe { with_rule(p, |r| bstr_out(out, text_field(r, F).as_deref())) }
}

unsafe extern "system" fn put_text<const F: usize>(p: *mut c_void, v: *const u16) -> Hresult {
    // SAFETY: the caller's BSTR.
    let v = unsafe { bstr_in(v) };
    // SAFETY: our rule object.
    unsafe {
        with_rule(p, |r| match F {
            4 | 5 => hr(r.set_ports(F == 4, v.as_deref())),
            _ => {
                *text_field(r, F) = v;
                S_OK
            }
        })
    }
}

/// The number properties: `Protocol` 0, `Direction` 1, `Profiles` 2, `Action` 3.
unsafe extern "system" fn get_number<const F: usize>(p: *mut c_void, out: *mut i32) -> Hresult {
    // SAFETY: our rule object.
    let v = unsafe {
        with_rule(p, |r| match F {
            0 => r.protocol,
            1 => r.direction,
            2 => r.profiles,
            _ => r.action,
        })
    };
    // SAFETY: the caller's out parameter.
    unsafe { put(out, v) }
}

unsafe extern "system" fn put_number<const F: usize>(p: *mut c_void, v: i32) -> Hresult {
    // SAFETY: our rule object.
    unsafe {
        with_rule(p, |r| {
            hr(match F {
                0 => r.set_protocol(v),
                1 => r.set_direction(v),
                2 => {
                    r.set_profiles(v);
                    Ok(())
                }
                _ => r.set_action(v),
            })
        })
    }
}

/// The flags: `Enabled` 0, `EdgeTraversal` 1. Only `VARIANT_TRUE` is true.
unsafe extern "system" fn get_flag<const F: usize>(p: *mut c_void, out: *mut i16) -> Hresult {
    // SAFETY: our rule object.
    let v = unsafe { with_rule(p, |r| if F == 0 { r.enabled } else { r.edge_traversal }) };
    // SAFETY: the caller's out parameter.
    unsafe { put(out, variant_bool(v)) }
}

unsafe extern "system" fn put_flag<const F: usize>(p: *mut c_void, v: i16) -> Hresult {
    let on = v == netfw::VARIANT_TRUE;
    // SAFETY: our rule object.
    unsafe {
        with_rule(p, |r| {
            if F == 0 {
                r.enabled = on;
            } else {
                r.edge_traversal = on;
            }
        })
    };
    S_OK
}

/// `Interfaces`: no interface list (an empty `VARIANT`); a list put is accepted and not kept.
unsafe extern "system" fn get_interfaces(_: *mut c_void, out: *mut Variant) -> Hresult {
    // SAFETY: the caller's out parameter.
    unsafe { put(out, Variant { vt: 0, reserved: [0; 3], value: 0, extra: 0 }) }
}

unsafe extern "system" fn put_interfaces(_: *mut c_void, _: *const Variant) -> Hresult {
    S_OK
}

/// `INetFwRule`: `Name` 7/8, `Description` 9/10, `ApplicationName` 11/12, `ServiceName` 13/14,
/// `Protocol` 15/16, `LocalPorts` 17/18, `RemotePorts` 19/20, `LocalAddresses` 21/22,
/// `RemoteAddresses` 23/24, `IcmpTypesAndCodes` 25/26, `Direction` 27/28, `Interfaces` 29/30,
/// `InterfaceTypes` 31/32, `Enabled` 33/34, `Grouping` 35/36, `Profiles` 37/38,
/// `EdgeTraversal` 39/40, `Action` 41/42.
static RULE_VTBL: Vtbl<43> = join(
    dispatch_slots!(RuleData),
    [
        get_text::<0> as *const c_void,
        put_text::<0> as *const c_void,
        get_text::<1> as *const c_void,
        put_text::<1> as *const c_void,
        get_text::<2> as *const c_void,
        put_text::<2> as *const c_void,
        get_text::<3> as *const c_void,
        put_text::<3> as *const c_void,
        get_number::<0> as *const c_void,
        put_number::<0> as *const c_void,
        get_text::<4> as *const c_void,
        put_text::<4> as *const c_void,
        get_text::<5> as *const c_void,
        put_text::<5> as *const c_void,
        get_text::<6> as *const c_void,
        put_text::<6> as *const c_void,
        get_text::<7> as *const c_void,
        put_text::<7> as *const c_void,
        get_text::<8> as *const c_void,
        put_text::<8> as *const c_void,
        get_number::<1> as *const c_void,
        put_number::<1> as *const c_void,
        get_interfaces as *const c_void,
        put_interfaces as *const c_void,
        get_text::<9> as *const c_void,
        put_text::<9> as *const c_void,
        get_flag::<0> as *const c_void,
        put_flag::<0> as *const c_void,
        get_text::<10> as *const c_void,
        put_text::<10> as *const c_void,
        get_number::<2> as *const c_void,
        put_number::<2> as *const c_void,
        get_flag::<1> as *const c_void,
        put_flag::<1> as *const c_void,
        get_number::<3> as *const c_void,
        put_number::<3> as *const c_void,
    ],
);

/// Whether `p` is one of our rule objects.
unsafe fn is_our_rule(p: *mut c_void) -> bool {
    // SAFETY: (caller) `p` is a live interface pointer: its first field is its method table.
    unsafe { *p.cast::<*const c_void>() == (&RULE_VTBL as *const Vtbl<43>).cast() }
}

// ----- INetFwRules

const RULES_IIDS: &[Guid] = &[IID_IUNKNOWN, IID_IDISPATCH, IID_INET_FW_RULES];

unsafe extern "system" fn rules_count(_: *mut c_void, out: *mut i32) -> Hresult {
    let n = with_table(|t| t.len()) as i32;
    // SAFETY: the caller's out parameter.
    unsafe { put(out, n) }
}

unsafe extern "system" fn rules_add(_: *mut c_void, rule: *mut c_void) -> Hresult {
    if rule.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's interface pointer.
    if !unsafe { is_our_rule(rule) } {
        return E_INVALIDARG;
    }
    // SAFETY: one of our rule objects.
    let shared = unsafe { data::<RuleData>(rule) }.clone();
    if let Err(e) = shared.with(|r| r.check_add()) {
        return e as i32;
    }
    with_table(|t| t.push(shared));
    S_OK
}

unsafe extern "system" fn rules_remove(_: *mut c_void, name: *const u16) -> Hresult {
    // SAFETY: the caller's BSTR.
    let Some(name) = (unsafe { bstr_in(name) }) else { return E_INVALIDARG };
    with_table(|t| {
        if let Some(i) = t.iter().position(|r| r.with(|r| r.is_named(&name))) {
            t.remove(i);
        }
    });
    S_OK
}

unsafe extern "system" fn rules_item(_: *mut c_void, name: *const u16, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter and BSTR.
    unsafe {
        *out = core::ptr::null_mut();
        let Some(name) = bstr_in(name) else { return E_INVALIDARG };
        match with_table(|t| t.iter().find(|r| r.with(|r| r.is_named(&name))).cloned()) {
            Some(shared) => {
                *out = new_rule_object(shared);
                S_OK
            }
            None => netfw::HR_NOT_FOUND as i32,
        }
    }
}

unsafe extern "system" fn rules_enum(_: *mut c_void, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    let items = with_table(|t| t.clone());
    // SAFETY: the caller's out parameter.
    unsafe { *out = new_object(&ENUM_VTBL, ENUM_IIDS, Enum { items, at: AtomicUsize::new(0) }) };
    S_OK
}

/// `INetFwRules`: `get_Count` 7, `Add` 8, `Remove` 9, `Item` 10, `get__NewEnum` 11.
static RULES_VTBL: Vtbl<12> = join(
    dispatch_slots!(()),
    [
        rules_count as *const c_void,
        rules_add as *const c_void,
        rules_remove as *const c_void,
        rules_item as *const c_void,
        rules_enum as *const c_void,
    ],
);

// ----- the rules' enumerator (IEnumVARIANT of VT_DISPATCH rule objects)

struct Enum {
    items: Vec<Shared>,
    at: AtomicUsize,
}

const ENUM_IIDS: &[Guid] = &[IID_IUNKNOWN, IID_IENUM_VARIANT];

unsafe extern "system" fn enum_next(p: *mut c_void, count: u32, out: *mut Variant, fetched: *mut u32) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: our enumerator.
    let e = unsafe { data::<Enum>(p) };
    let mut n = 0u32;
    while n < count {
        let at = e.at.fetch_add(1, Ordering::AcqRel);
        let Some(shared) = e.items.get(at) else {
            e.at.store(e.items.len(), Ordering::Release);
            break;
        };
        let object = new_rule_object(shared.clone());
        // SAFETY: the caller's array of `count` VARIANTs.
        unsafe {
            out.add(n as usize).write_unaligned(Variant {
                vt: VT_DISPATCH,
                reserved: [0; 3],
                value: object as usize,
                extra: 0,
            })
        };
        n += 1;
    }
    if !fetched.is_null() {
        // SAFETY: the caller's count.
        unsafe { *fetched = n };
    }
    if n == count { S_OK } else { S_FALSE }
}

unsafe extern "system" fn enum_skip(p: *mut c_void, count: u32) -> Hresult {
    // SAFETY: our enumerator.
    let e = unsafe { data::<Enum>(p) };
    let at = e.at.load(Ordering::Acquire).saturating_add(count as usize);
    e.at.store(at.min(e.items.len()), Ordering::Release);
    if at <= e.items.len() { S_OK } else { S_FALSE }
}

unsafe extern "system" fn enum_reset(p: *mut c_void) -> Hresult {
    // SAFETY: our enumerator.
    unsafe { data::<Enum>(p) }.at.store(0, Ordering::Release);
    S_OK
}

unsafe extern "system" fn enum_clone(p: *mut c_void, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: our enumerator; the caller's out parameter.
    unsafe {
        let e = data::<Enum>(p);
        let copy = Enum { items: e.items.clone(), at: AtomicUsize::new(e.at.load(Ordering::Acquire)) };
        *out = new_object(&ENUM_VTBL, ENUM_IIDS, copy);
    }
    S_OK
}

/// `IEnumVARIANT`: `Next` 3, `Skip` 4, `Reset` 5, `Clone` 6.
static ENUM_VTBL: Vtbl<7> = Vtbl([
    query::<Enum> as *const c_void,
    add_ref::<Enum> as *const c_void,
    release::<Enum> as *const c_void,
    enum_next as *const c_void,
    enum_skip as *const c_void,
    enum_reset as *const c_void,
    enum_clone as *const c_void,
]);

// ----- NetFwPolicy2 (INetFwPolicy2)

const POLICY_IIDS: &[Guid] = &[IID_IUNKNOWN, IID_IDISPATCH, IID_INET_FW_POLICY2];

/// The per-profile switches (`FirewallEnabled` 0, `BlockAllInboundTraffic` 1,
/// `NotificationsDisabled` 2, `UnicastResponsesToMulticastBroadcastDisabled` 3) of the domain,
/// private and public profiles. They read back what was put and change nothing: Poly has no
/// firewall, so it starts disabled.
static SWITCHES: [[AtomicI16; 3]; 4] = [const { [const { AtomicI16::new(0) }; 3] }; 4];
/// The default inbound (block) and outbound (allow) actions per profile.
static DEFAULT_ACTIONS: [[AtomicI32; 3]; 2] =
    [[const { AtomicI32::new(netfw::ACTION_BLOCK) }; 3], [const { AtomicI32::new(netfw::ACTION_ALLOW) }; 3]];

/// The index of a single profile (`netfw::check_profile` passed).
fn profile_index(profile: i32) -> usize {
    match profile {
        netfw::PROFILE_DOMAIN => 0,
        netfw::PROFILE_PRIVATE => 1,
        _ => 2,
    }
}

unsafe extern "system" fn current_profiles(_: *mut c_void, out: *mut i32) -> Hresult {
    // SAFETY: the caller's out parameter.
    unsafe { put(out, netfw::CURRENT_PROFILES) }
}

unsafe extern "system" fn get_switch<const K: usize>(_: *mut c_void, profile: i32, out: *mut i16) -> Hresult {
    if let Err(e) = netfw::check_profile(profile) {
        return e as i32;
    }
    let v = SWITCHES[K][profile_index(profile)].load(Ordering::Acquire);
    // SAFETY: the caller's out parameter.
    unsafe { put(out, v) }
}

unsafe extern "system" fn put_switch<const K: usize>(_: *mut c_void, profile: i32, v: i16) -> Hresult {
    if let Err(e) = netfw::check_profile(profile) {
        return e as i32;
    }
    SWITCHES[K][profile_index(profile)].store(variant_bool(v != 0), Ordering::Release);
    S_OK
}

unsafe extern "system" fn get_excluded(_: *mut c_void, profile: i32, out: *mut Variant) -> Hresult {
    if let Err(e) = netfw::check_profile(profile) {
        return e as i32;
    }
    // SAFETY: the caller's out parameter.
    unsafe { get_interfaces(core::ptr::null_mut(), out) }
}

unsafe extern "system" fn put_excluded(_: *mut c_void, profile: i32, _: *const Variant) -> Hresult {
    hr(netfw::check_profile(profile))
}

unsafe extern "system" fn get_rules(_: *mut c_void, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = new_object(&RULES_VTBL, RULES_IIDS, ()) };
    S_OK
}

unsafe extern "system" fn service_restriction(_: *mut c_void, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = core::ptr::null_mut() };
    E_NOTIMPL
}

/// Whether every rule of a group is enabled (and the group has rules).
fn group_enabled(group: &[u16]) -> bool {
    with_table(|t| {
        let mut any = false;
        let all = t.iter().all(|r| {
            r.with(|r| {
                let member = r.grouping.as_deref() == Some(group);
                any |= member;
                !member || r.enabled
            })
        });
        any && all
    })
}

unsafe extern "system" fn enable_group(_: *mut c_void, _profiles: i32, group: *const u16, on: i16) -> Hresult {
    // SAFETY: the caller's BSTR.
    let Some(group) = (unsafe { bstr_in(group) }) else { return E_INVALIDARG };
    with_table(|t| {
        for r in t.iter() {
            r.with(|r| {
                if r.grouping.as_deref() == Some(&group[..]) {
                    r.enabled = on != 0;
                }
            });
        }
    });
    S_OK
}

unsafe extern "system" fn is_group_enabled(
    _: *mut c_void,
    _profiles: i32,
    group: *const u16,
    out: *mut i16,
) -> Hresult {
    // SAFETY: the caller's BSTR and out parameter.
    unsafe { group_currently_enabled(core::ptr::null_mut(), group, out) }
}

unsafe extern "system" fn group_currently_enabled(_: *mut c_void, group: *const u16, out: *mut i16) -> Hresult {
    // SAFETY: the caller's BSTR.
    let Some(group) = (unsafe { bstr_in(group) }) else { return E_INVALIDARG };
    // SAFETY: the caller's out parameter.
    unsafe { put(out, variant_bool(group_enabled(&group))) }
}

unsafe extern "system" fn restore_defaults(_: *mut c_void) -> Hresult {
    with_table(|t| *t = builtin_table());
    S_OK
}

unsafe extern "system" fn get_default_action<const K: usize>(_: *mut c_void, profile: i32, out: *mut i32) -> Hresult {
    if let Err(e) = netfw::check_profile(profile) {
        return e as i32;
    }
    let v = DEFAULT_ACTIONS[K][profile_index(profile)].load(Ordering::Acquire);
    // SAFETY: the caller's out parameter.
    unsafe { put(out, v) }
}

unsafe extern "system" fn put_default_action<const K: usize>(_: *mut c_void, profile: i32, v: i32) -> Hresult {
    if let Err(e) = netfw::check_profile(profile) {
        return e as i32;
    }
    if v != netfw::ACTION_BLOCK && v != netfw::ACTION_ALLOW {
        return E_INVALIDARG;
    }
    DEFAULT_ACTIONS[K][profile_index(profile)].store(v, Ordering::Release);
    S_OK
}

/// `NET_FW_MODIFY_STATE_OK` (`icftypes.h`): changes to the local policy take effect.
unsafe extern "system" fn modify_state(_: *mut c_void, out: *mut i32) -> Hresult {
    // SAFETY: the caller's out parameter.
    unsafe { put(out, 0) }
}

/// `INetFwPolicy2`: `get_CurrentProfileTypes` 7, `FirewallEnabled` 8/9, `ExcludedInterfaces`
/// 10/11, `BlockAllInboundTraffic` 12/13, `NotificationsDisabled` 14/15,
/// `UnicastResponsesToMulticastBroadcastDisabled` 16/17, `get_Rules` 18,
/// `get_ServiceRestriction` 19, `EnableRuleGroup` 20, `IsRuleGroupEnabled` 21,
/// `RestoreLocalFirewallDefaults` 22, `DefaultInboundAction` 23/24, `DefaultOutboundAction`
/// 25/26, `get_IsRuleGroupCurrentlyEnabled` 27, `get_LocalPolicyModifyState` 28.
static POLICY_VTBL: Vtbl<29> = join(
    dispatch_slots!(()),
    [
        current_profiles as *const c_void,
        get_switch::<0> as *const c_void,
        put_switch::<0> as *const c_void,
        get_excluded as *const c_void,
        put_excluded as *const c_void,
        get_switch::<1> as *const c_void,
        put_switch::<1> as *const c_void,
        get_switch::<2> as *const c_void,
        put_switch::<2> as *const c_void,
        get_switch::<3> as *const c_void,
        put_switch::<3> as *const c_void,
        get_rules as *const c_void,
        service_restriction as *const c_void,
        enable_group as *const c_void,
        is_group_enabled as *const c_void,
        restore_defaults as *const c_void,
        get_default_action::<0> as *const c_void,
        put_default_action::<0> as *const c_void,
        get_default_action::<1> as *const c_void,
        put_default_action::<1> as *const c_void,
        group_currently_enabled as *const c_void,
        modify_state as *const c_void,
    ],
);

// ----- the class objects

/// A class object (`IClassFactory`: `CreateInstance` 3, `LockServer` 4).
#[repr(C)]
struct Factory {
    vtbl: *const c_void,
}
// SAFETY: immutable.
unsafe impl Sync for Factory {}

static POLICY_FACTORY_VTBL: Vtbl<5> = Vtbl([
    factory_query_interface as *const c_void,
    factory_count as *const c_void,
    factory_count as *const c_void,
    factory_create::<false> as *const c_void,
    factory_lock as *const c_void,
]);
static RULE_FACTORY_VTBL: Vtbl<5> = Vtbl([
    factory_query_interface as *const c_void,
    factory_count as *const c_void,
    factory_count as *const c_void,
    factory_create::<true> as *const c_void,
    factory_lock as *const c_void,
]);
static POLICY_FACTORY: Factory = Factory { vtbl: (&POLICY_FACTORY_VTBL as *const Vtbl<5>).cast() };
static RULE_FACTORY: Factory = Factory { vtbl: (&RULE_FACTORY_VTBL as *const Vtbl<5>).cast() };

unsafe extern "system" fn factory_query_interface(p: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's pointers.
    unsafe {
        *out = core::ptr::null_mut();
        if iid.is_null() {
            return E_INVALIDARG;
        }
        let iid = iid.read_unaligned();
        if iid != IID_IUNKNOWN && iid != IID_ICLASS_FACTORY {
            return E_NOINTERFACE;
        }
        *out = p;
    }
    S_OK
}

/// The class objects live for ever: `AddRef` and `Release` count nothing.
unsafe extern "system" fn factory_count(_: *mut c_void) -> u32 {
    1
}

unsafe extern "system" fn factory_lock(_: *mut c_void, _: i32) -> Hresult {
    S_OK
}

unsafe extern "system" fn factory_create<const RULE: bool>(
    _: *mut c_void,
    outer: *mut c_void,
    iid: *const Guid,
    out: *mut *mut c_void,
) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter; a fresh object handed out through QueryInterface.
    unsafe {
        *out = core::ptr::null_mut();
        if !outer.is_null() {
            return CLASS_E_NOAGGREGATION;
        }
        if RULE {
            hand_out::<RuleData>(new_rule_object(Arc::new(Lock::new(Rule::default()))), iid, out)
        } else {
            hand_out::<()>(new_object(&POLICY_VTBL, POLICY_IIDS, ()), iid, out)
        }
    }
}

/// `DllGetClassObject(rclsid, riid, ppv)`: the class objects of `CLSID_NetFwPolicy2` and
/// `CLSID_NetFwRule`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(
    clsid: *const Guid,
    iid: *const Guid,
    out: *mut *mut c_void,
) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's pointers.
    unsafe {
        *out = core::ptr::null_mut();
        if clsid.is_null() {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let factory = match clsid.read_unaligned() {
            c if c == CLSID_NET_FW_POLICY2 => &POLICY_FACTORY,
            c if c == CLSID_NET_FW_RULE => &RULE_FACTORY,
            _ => return CLASS_E_CLASSNOTAVAILABLE,
        };
        factory_query_interface((factory as *const Factory).cast_mut().cast(), iid, out)
    }
}

/// `DllCanUnloadNow`: the DLL stays (`S_FALSE`).
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> Hresult {
    S_FALSE
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn hnetcfg_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}

//! `Shell.Application` (`CLSID_Shell`) and the desktop's `ShellWindows` (`CLSID_ShellWindows`),
//! as far as a program goes to start another one "from the desktop": the sheet
//! `shell32.dll/ShellDispatch.ShellExecute.yaml` (`IShellWindows::FindWindowSW` for the desktop,
//! its `IServiceProvider`, the top-level `IShellBrowser`, the active `IShellView`, the view's
//! background object `IShellFolderViewDual`, its `Application`, and late-bound `ShellExecute`
//! through `IDispatch`).
//!
//! On Windows `ShellWindows` lives in Explorer, so a program started through that chain is
//! Explorer's child with the user's own token. Poly's desktop is no Windows process: the objects
//! live in the caller (`poly_com::inproc`'s desktop classes) and `ShellExecute` starts the program
//! with `ShellExecuteExW`, as `Shell.Application` created in the caller does. Starting it from
//! Poly's session with the user's token instead is the session manager's work (the sheet's
//! `not_covered`: B5).
//!
//! Interface ids, method orders and constants: `exdisp.h`, `shldisp.h`, `shobjidl_core.h`,
//! `servprov.h`, `oaidl.h`, `winerror.h` (the sheet's `interface`).

use crate::com::*;
use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};

#[cfg_attr(target_arch = "x86", link(name = "user32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "user32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetDesktopWindow() -> usize;
}

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetLastError() -> u32;
}

/// `IID_IDispatch` (`oaidl.h`).
const IID_IDISPATCH: Guid = guid(0x0002_0400, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
/// `IID_IShellWindows` (`exdisp.h`).
const IID_ISHELL_WINDOWS: Guid = guid(0x85CB_6900, 0x4D95, 0x11CF, [0x96, 0x0C, 0x00, 0x80, 0xC7, 0xF4, 0xEE, 0x85]);
/// `IID_IServiceProvider` (`servprov.h`).
const IID_ISERVICE_PROVIDER: Guid = guid(0x6D51_40C1, 0x7436, 0x11CE, [0x80, 0x34, 0x00, 0xAA, 0x00, 0x60, 0x09, 0xFA]);
/// `SID_STopLevelBrowser` (`shlguid.h`).
const SID_STOP_LEVEL_BROWSER: Guid =
    guid(0x4C96_BE40, 0x915C, 0x11CF, [0x99, 0xD3, 0x00, 0xAA, 0x00, 0x4A, 0xE8, 0x37]);
/// `IID_IOleWindow` (`oleidl.h`), `IID_IShellBrowser`, `IID_IShellView` (`shobjidl_core.h`).
const IID_IOLE_WINDOW: Guid = guid(0x0000_0114, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_ISHELL_BROWSER: Guid = guid(0x0002_14E2, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_ISHELL_VIEW: Guid = guid(0x0002_14E3, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
/// `IID_IShellFolderViewDual` (`shldisp.h`).
const IID_ISHELL_FOLDER_VIEW_DUAL: Guid =
    guid(0xE7A1_AF80, 0x4D96, 0x11CF, [0x96, 0x0C, 0x00, 0x80, 0xC7, 0xF4, 0xEE, 0x85]);

/// `SWC_DESKTOP` (`exdisp.h`), `SWFO_NEEDDISPATCH`.
const SWC_DESKTOP: i32 = 8;
const SWFO_NEEDDISPATCH: i32 = 1;
/// `SVGIO_BACKGROUND` (`shobjidl_core.h`): the view's background object.
const SVGIO_BACKGROUND: u32 = 0;
/// `DISPATCH_METHOD` (`oaidl.h`).
const DISPATCH_METHOD: u16 = 1;
/// `DISPID_UNKNOWN` (`oaidl.h`).
const DISPID_UNKNOWN: i32 = -1;
/// `DISP_E_*` (`winerror.h`).
const DISP_E_MEMBERNOTFOUND: Hresult = 0x8002_0003_u32 as i32;
const DISP_E_PARAMNOTFOUND: u32 = 0x8002_0004;
const DISP_E_UNKNOWNNAME: Hresult = 0x8002_0006_u32 as i32;
const DISP_E_BADPARAMCOUNT: Hresult = 0x8002_000E_u32 as i32;
const DISP_E_TYPEMISMATCH: Hresult = 0x8002_0005_u32 as i32;
/// `VARTYPE`s (`wtypes.h`).
const VT_EMPTY: u16 = 0;
const VT_I2: u16 = 2;
const VT_I4: u16 = 3;
const VT_BSTR: u16 = 8;
const VT_ERROR: u16 = 10;
const VT_VARIANT: u16 = 12;
const VT_UI2: u16 = 18;
const VT_UI4: u16 = 19;
const VT_INT: u16 = 22;
const VT_UINT: u16 = 23;
const VT_BYREF: u16 = 0x4000;
/// `SW_SHOWNORMAL` (`winuser.h`): `ShellExecute`'s show command when the caller gives none.
const SW_SHOWNORMAL: i32 = 1;

/// `VARIANT` (`oaidl.h`): `vt`, three reserved `WORD`s, then the value (16 bytes on x86, 24 on
/// x64). Methods that take one by value receive this type, so their argument sizes are right.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Variant {
    vt: u16,
    reserved: [u16; 3],
    value: usize,
    record: usize,
}

/// `DISPPARAMS` (`oaidl.h`).
#[repr(C)]
struct DispParams {
    args: *const Variant,
    named: *const i32,
    count: u32,
    named_count: u32,
}

/// The `DISPID` of `ShellExecute` on `Shell.Application`'s `IDispatch`.
const DISPID_SHELL_EXECUTE: i32 = 1;

// ----- IDispatch with nothing to call --------------------------------------------------------

unsafe extern "system" fn type_info_count(_this: *mut c_void, count: *mut u32) -> Hresult {
    if count.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *count = 0 };
    S_OK
}

unsafe extern "system" fn type_info(_this: *mut c_void, _index: u32, _lcid: u32, out: *mut *mut c_void) -> Hresult {
    if !out.is_null() {
        // SAFETY: the caller's out parameter.
        unsafe { *out = core::ptr::null_mut() };
    }
    E_NOTIMPL
}

/// `GetIDsOfNames`: every name unknown.
unsafe extern "system" fn no_ids(
    _this: *mut c_void,
    _iid: *const Guid,
    _names: *const *const u16,
    count: u32,
    _lcid: u32,
    ids: *mut i32,
) -> Hresult {
    if !ids.is_null() {
        for i in 0..count as usize {
            // SAFETY: the caller's array of `count` ids.
            unsafe { *ids.add(i) = DISPID_UNKNOWN };
        }
    }
    DISP_E_UNKNOWNNAME
}

#[allow(clippy::too_many_arguments)] // IDispatch::Invoke's eight
unsafe extern "system" fn no_invoke(
    _this: *mut c_void,
    _id: i32,
    _iid: *const Guid,
    _lcid: u32,
    _flags: u16,
    _params: *mut DispParams,
    _result: *mut Variant,
    _exception: *mut c_void,
    _arg_err: *mut u32,
) -> Hresult {
    DISP_E_MEMBERNOTFOUND
}

unsafe extern "system" fn not_impl_0(_: *mut c_void) -> Hresult {
    E_NOTIMPL
}
unsafe extern "system" fn not_impl_1(_: *mut c_void, _: usize) -> Hresult {
    E_NOTIMPL
}
unsafe extern "system" fn not_impl_2(_: *mut c_void, _: usize, _: usize) -> Hresult {
    E_NOTIMPL
}
unsafe extern "system" fn not_impl_3(_: *mut c_void, _: usize, _: usize, _: usize) -> Hresult {
    E_NOTIMPL
}
unsafe extern "system" fn not_impl_5(_: *mut c_void, _: usize, _: usize, _: usize, _: usize, _: usize) -> Hresult {
    E_NOTIMPL
}
unsafe extern "system" fn ok_1(_: *mut c_void, _: usize) -> Hresult {
    S_OK
}
unsafe extern "system" fn ok_2(_: *mut c_void, _: usize, _: usize) -> Hresult {
    S_OK
}

/// A pointer out parameter that returns nothing (`E_NOTIMPL`).
unsafe extern "system" fn no_object(_: *mut c_void, out: *mut *mut c_void) -> Hresult {
    if !out.is_null() {
        // SAFETY: the caller's out parameter.
        unsafe { *out = core::ptr::null_mut() };
    }
    E_NOTIMPL
}

// ----- Shell.Application ----------------------------------------------------------------------

#[repr(C)]
struct ShellApp {
    head: Head,
}

static APP_IIDS: [Guid; 1] = [IID_IDISPATCH];

/// `IDispatch` after `IUnknown`: `GetTypeInfoCount` 3, `GetTypeInfo` 4, `GetIDsOfNames` 5,
/// `Invoke` 6.
static APP_VTBL: Vtbl<7> = Vtbl([
    query_interface as *const c_void,
    add_ref as *const c_void,
    release as *const c_void,
    type_info_count as *const c_void,
    type_info as *const c_void,
    app_ids as *const c_void,
    app_invoke as *const c_void,
]);

/// The class factory's `CreateInstance` for `CLSID_Shell`, and `Application` of the objects below.
pub unsafe fn create_shell_app(iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    let object = into_raw(ShellApp { head: Head::new(&APP_VTBL, &APP_IIDS, drop_box::<ShellApp>) });
    // SAFETY: a fresh object; the caller's out parameter.
    unsafe { hand_out(object.cast(), iid, out) }
}

/// Whether `name` is `ShellExecute`, without regard to ASCII case (as late binding compares names).
fn is_shell_execute(name: &[u16]) -> bool {
    let want = b"shellexecute";
    name.len() == want.len()
        && name.iter().zip(want).all(|(&c, &w)| {
            let c = if (u16::from(b'A')..=u16::from(b'Z')).contains(&c) { c + 32 } else { c };
            c == u16::from(w)
        })
}

unsafe extern "system" fn app_ids(
    _this: *mut c_void,
    _iid: *const Guid,
    names: *const *const u16,
    count: u32,
    _lcid: u32,
    ids: *mut i32,
) -> Hresult {
    if names.is_null() || ids.is_null() || count == 0 {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's arrays of `count` names and ids.
    unsafe {
        for i in 0..count as usize {
            *ids.add(i) = DISPID_UNKNOWN;
        }
        if !is_shell_execute(wstr(*names)) || count > 1 {
            return DISP_E_UNKNOWNNAME;
        }
        *ids = DISPID_SHELL_EXECUTE;
    }
    S_OK
}

/// One argument of a late-bound call: absent (`VT_EMPTY`, or `VT_ERROR` with
/// `DISP_E_PARAMNOTFOUND`), a string or a number; `VT_BYREF|VT_VARIANT` is followed.
enum Arg {
    Missing,
    Text(alloc::vec::Vec<u16>),
    Number(i32),
}

/// # Safety
/// `v` is a valid `VARIANT` of the caller.
unsafe fn arg_of(v: &Variant) -> Result<Arg, Hresult> {
    // SAFETY: the value field is what `vt` says it is.
    unsafe {
        Ok(match v.vt {
            VT_EMPTY => Arg::Missing,
            VT_ERROR if v.value as u32 == DISP_E_PARAMNOTFOUND => Arg::Missing,
            VT_BSTR => Arg::Text(wstr(v.value as *const u16).to_vec()),
            VT_I2 => Arg::Number(i32::from(v.value as u16 as i16)),
            VT_UI2 => Arg::Number(i32::from(v.value as u16)),
            VT_I4 | VT_UI4 | VT_INT | VT_UINT => Arg::Number(v.value as u32 as i32),
            t if t == VT_BYREF | VT_VARIANT && v.value != 0 => return arg_of(&*(v.value as *const Variant)),
            t if t == VT_BYREF | VT_BSTR && v.value != 0 => Arg::Text(wstr(*(v.value as *const *const u16)).to_vec()),
            t if t == VT_BYREF | VT_I4 && v.value != 0 => Arg::Number(*(v.value as *const i32)),
            _ => return Err(DISP_E_TYPEMISMATCH),
        })
    }
}

/// `IShellDispatch2::ShellExecute(File, vArgs, vDir, vOperation, vShow)`: the arguments come in
/// reverse order in `DISPPARAMS`.
unsafe fn shell_execute(params: &DispParams) -> Hresult {
    let count = params.count as usize;
    if count == 0 || count > 5 || params.named_count != 0 || params.args.is_null() {
        return DISP_E_BADPARAMCOUNT;
    }
    let mut args = [const { None::<alloc::vec::Vec<u16>> }; 4];
    let mut show = SW_SHOWNORMAL;
    for i in 0..count {
        // SAFETY: the caller's array of `count` VARIANTs; argument `i` is at `count - 1 - i`.
        let arg = match unsafe { arg_of(&*params.args.add(count - 1 - i)) } {
            Ok(a) => a,
            Err(hr) => return hr,
        };
        match (i, arg) {
            (_, Arg::Missing) => {}
            (4, Arg::Number(n)) => show = n,
            (0..=3, Arg::Text(t)) => args[i] = Some(t),
            _ => return DISP_E_TYPEMISMATCH,
        }
    }
    let [Some(file), params_text, dir, verb] = args else { return DISP_E_TYPEMISMATCH };
    let file = nul(&file);
    let params_text = params_text.map(|t| nul(&t));
    let dir = dir.map(|t| nul(&t));
    let verb = verb.map(|t| nul(&t));
    let ptr = |v: &Option<alloc::vec::Vec<u16>>| v.as_ref().map_or(core::ptr::null(), |t| t.as_ptr());
    // SAFETY: NUL-terminated strings (or NULL) that live through the call.
    let inst = unsafe { crate::exec::ShellExecuteW(0, ptr(&verb), file.as_ptr(), ptr(&params_text), ptr(&dir), show) };
    // Above 32 is success (Learn: "ShellExecuteW").
    if inst > 32 {
        S_OK
    } else {
        // SAFETY: a plain call.
        let e = unsafe { GetLastError() };
        if e == 0 { E_FAIL } else { (0x8007_0000 | (e & 0xFFFF)) as Hresult }
    }
}

#[allow(clippy::too_many_arguments)] // IDispatch::Invoke's eight
unsafe extern "system" fn app_invoke(
    _this: *mut c_void,
    id: i32,
    _iid: *const Guid,
    _lcid: u32,
    flags: u16,
    params: *mut DispParams,
    result: *mut Variant,
    _exception: *mut c_void,
    _arg_err: *mut u32,
) -> Hresult {
    if id != DISPID_SHELL_EXECUTE || flags & DISPATCH_METHOD == 0 {
        return DISP_E_MEMBERNOTFOUND;
    }
    if params.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's DISPPARAMS and optional result.
    unsafe {
        if !result.is_null() {
            (*result).vt = VT_EMPTY;
        }
        shell_execute(&*params)
    }
}

// ----- ShellWindows ---------------------------------------------------------------------------

#[repr(C)]
struct ShellWindows {
    head: Head,
}

static WINDOWS_IIDS: [Guid; 2] = [IID_IDISPATCH, IID_ISHELL_WINDOWS];

/// `IShellWindows` (`exdisp.h`) after `IDispatch`: `get_Count` 7, `Item` 8, `_NewEnum` 9,
/// `Register` 10, `RegisterPending` 11, `Revoke` 12, `OnNavigate` 13, `OnActivated` 14,
/// `FindWindowSW` 15, `OnCreated` 16, `ProcessAttachDetach` 17.
static WINDOWS_VTBL: Vtbl<18> = Vtbl([
    query_interface as *const c_void,
    add_ref as *const c_void,
    release as *const c_void,
    type_info_count as *const c_void,
    type_info as *const c_void,
    no_ids as *const c_void,
    no_invoke as *const c_void,
    windows_count as *const c_void,
    windows_item as *const c_void,
    no_object as *const c_void,
    not_impl_5 as *const c_void,
    not_impl_5 as *const c_void,
    ok_1 as *const c_void,
    ok_2 as *const c_void,
    ok_2 as *const c_void,
    find_window as *const c_void,
    ok_2 as *const c_void,
    ok_1 as *const c_void,
]);

/// The class factory's `CreateInstance` for `CLSID_ShellWindows`.
pub unsafe fn create_shell_windows(iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    let object = into_raw(ShellWindows { head: Head::new(&WINDOWS_VTBL, &WINDOWS_IIDS, drop_box::<ShellWindows>) });
    // SAFETY: a fresh object; the caller's out parameter.
    unsafe { hand_out(object.cast(), iid, out) }
}

/// `get_Count`: Poly's desktop has no Windows folder or browser windows to list.
unsafe extern "system" fn windows_count(_this: *mut c_void, count: *mut i32) -> Hresult {
    if count.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *count = 0 };
    S_OK
}

/// `Item(index)`: no window (`S_FALSE`).
unsafe extern "system" fn windows_item(_this: *mut c_void, _index: Variant, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = core::ptr::null_mut() };
    S_FALSE
}

/// `FindWindowSW(pvarLoc, pvarLocRoot, swClass, phwnd, swfwOptions, ppdispOut)`: the desktop
/// (`SWC_DESKTOP`) is found, with its automation object when `SWFO_NEEDDISPATCH` asks for it;
/// no other window is (`S_FALSE`).
unsafe extern "system" fn find_window(
    _this: *mut c_void,
    _location: *const Variant,
    _root: *const Variant,
    class: i32,
    hwnd: *mut i32,
    options: i32,
    out: *mut *mut c_void,
) -> Hresult {
    // SAFETY: the caller's out parameters (either may be NULL).
    unsafe {
        if !out.is_null() {
            *out = core::ptr::null_mut();
        }
        if !hwnd.is_null() {
            *hwnd = 0;
        }
        if class != SWC_DESKTOP {
            return S_FALSE;
        }
        if !hwnd.is_null() {
            // A `long`: window handles fit in 32 bits.
            *hwnd = GetDesktopWindow() as i32;
        }
        if options & SWFO_NEEDDISPATCH != 0 {
            if out.is_null() {
                return E_POINTER;
            }
            let desktop = Desktop::new();
            let hr = desktop_query(desktop.cast(), &IID_IDISPATCH, out);
            desktop_release(desktop.cast());
            return hr;
        }
    }
    S_OK
}

// ----- the desktop: its automation object, service provider, browser and view -----------------

/// The desktop window's shell objects, one object with four interfaces (one table pointer each,
/// in this order; `IUnknown` is the first).
#[repr(C)]
struct Desktop {
    dispatch: *const c_void,
    provider: *const c_void,
    browser: *const c_void,
    view: *const c_void,
    refs: AtomicU32,
}

impl Desktop {
    fn new() -> *mut Desktop {
        into_raw(Desktop {
            dispatch: (&DESKTOP_DISPATCH_VTBL as *const Vtbl<7>).cast(),
            provider: (&DESKTOP_PROVIDER_VTBL as *const Vtbl<4>).cast(),
            browser: (&DESKTOP_BROWSER_VTBL as *const Vtbl<18>).cast(),
            view: (&DESKTOP_VIEW_VTBL as *const Vtbl<16>).cast(),
            refs: AtomicU32::new(1),
        })
    }
}

/// The object behind interface pointer `this`, which is its slot `S`.
fn desktop_of<const S: usize>(this: *mut c_void) -> *mut Desktop {
    this.cast::<*const c_void>().wrapping_sub(S).cast()
}

/// `QueryInterface` on the object: each interface is its own slot.
unsafe fn desktop_query(d: *mut Desktop, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: a live desktop object; the caller's pointers.
    unsafe {
        *out = core::ptr::null_mut();
        if iid.is_null() {
            return E_INVALIDARG;
        }
        let iid = iid.read_unaligned();
        let slot: *mut *const c_void = if iid == poly_com::inproc::IID_IUNKNOWN || iid == IID_IDISPATCH {
            &raw mut (*d).dispatch
        } else if iid == IID_ISERVICE_PROVIDER {
            &raw mut (*d).provider
        } else if iid == IID_ISHELL_BROWSER || iid == IID_IOLE_WINDOW {
            &raw mut (*d).browser
        } else if iid == IID_ISHELL_VIEW {
            &raw mut (*d).view
        } else {
            return E_NOINTERFACE;
        };
        (*d).refs.fetch_add(1, Ordering::AcqRel);
        *out = slot.cast();
    }
    S_OK
}

unsafe fn desktop_release(d: *mut Desktop) -> u32 {
    // SAFETY: a live desktop object, freed when its last reference goes.
    unsafe {
        let left = (*d).refs.fetch_sub(1, Ordering::AcqRel) - 1;
        if left == 0 {
            drop(alloc::boxed::Box::from_raw(d));
        }
        left
    }
}

unsafe extern "system" fn d_query<const S: usize>(
    this: *mut c_void,
    iid: *const Guid,
    out: *mut *mut c_void,
) -> Hresult {
    // SAFETY: `this` is slot `S` of a live desktop object.
    unsafe { desktop_query(desktop_of::<S>(this), iid, out) }
}

unsafe extern "system" fn d_add_ref<const S: usize>(this: *mut c_void) -> u32 {
    // SAFETY: `this` is slot `S` of a live desktop object.
    unsafe { (*desktop_of::<S>(this)).refs.fetch_add(1, Ordering::AcqRel) + 1 }
}

unsafe extern "system" fn d_release<const S: usize>(this: *mut c_void) -> u32 {
    // SAFETY: `this` is slot `S` of a live desktop object.
    unsafe { desktop_release(desktop_of::<S>(this)) }
}

/// The desktop's automation object: an `IDispatch` with nothing to call.
static DESKTOP_DISPATCH_VTBL: Vtbl<7> = Vtbl([
    d_query::<0> as *const c_void,
    d_add_ref::<0> as *const c_void,
    d_release::<0> as *const c_void,
    type_info_count as *const c_void,
    type_info as *const c_void,
    no_ids as *const c_void,
    no_invoke as *const c_void,
]);

/// `IServiceProvider` after `IUnknown`: `QueryService` 3.
static DESKTOP_PROVIDER_VTBL: Vtbl<4> = Vtbl([
    d_query::<1> as *const c_void,
    d_add_ref::<1> as *const c_void,
    d_release::<1> as *const c_void,
    query_service as *const c_void,
]);

/// `QueryService(guidService, riid, ppv)`: the top-level browser is the desktop itself.
unsafe extern "system" fn query_service(
    this: *mut c_void,
    service: *const Guid,
    iid: *const Guid,
    out: *mut *mut c_void,
) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's pointers; `this` is slot 1 of a live desktop object.
    unsafe {
        *out = core::ptr::null_mut();
        if service.is_null() || service.read_unaligned() != SID_STOP_LEVEL_BROWSER {
            return E_NOINTERFACE;
        }
        desktop_query(desktop_of::<1>(this), iid, out)
    }
}

/// `GetWindow(phwnd)` (`IOleWindow`): the desktop window.
unsafe fn window_of(hwnd: *mut usize) -> Hresult {
    if hwnd.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *hwnd = GetDesktopWindow() };
    S_OK
}

unsafe extern "system" fn browser_window(_this: *mut c_void, hwnd: *mut usize) -> Hresult {
    // SAFETY: the caller's out parameter.
    unsafe { window_of(hwnd) }
}

unsafe extern "system" fn view_window(_this: *mut c_void, hwnd: *mut usize) -> Hresult {
    // SAFETY: the caller's out parameter.
    unsafe { window_of(hwnd) }
}

/// `IShellBrowser` after `IUnknown` (`shobjidl_core.h`): `IOleWindow`'s `GetWindow` 3,
/// `ContextSensitiveHelp` 4; `InsertMenusSB` 5, `SetMenuSB` 6, `RemoveMenusSB` 7,
/// `SetStatusTextSB` 8, `EnableModelessSB` 9, `TranslateAcceleratorSB` 10, `BrowseObject` 11,
/// `GetViewStateStream` 12, `GetControlWindow` 13, `SendControlMsg` 14, `QueryActiveShellView`
/// 15, `OnViewWindowActive` 16, `SetToolbarItems` 17.
static DESKTOP_BROWSER_VTBL: Vtbl<18> = Vtbl([
    d_query::<2> as *const c_void,
    d_add_ref::<2> as *const c_void,
    d_release::<2> as *const c_void,
    browser_window as *const c_void,
    not_impl_1 as *const c_void,
    not_impl_2 as *const c_void,
    not_impl_3 as *const c_void,
    not_impl_1 as *const c_void,
    not_impl_1 as *const c_void,
    not_impl_1 as *const c_void,
    not_impl_2 as *const c_void,
    not_impl_2 as *const c_void,
    not_impl_2 as *const c_void,
    not_impl_2 as *const c_void,
    not_impl_5 as *const c_void,
    active_view as *const c_void,
    not_impl_1 as *const c_void,
    not_impl_3 as *const c_void,
]);

/// `QueryActiveShellView(ppshv)`: the desktop's view.
unsafe extern "system" fn active_view(this: *mut c_void, out: *mut *mut c_void) -> Hresult {
    // SAFETY: `this` is slot 2 of a live desktop object; the caller's out parameter.
    unsafe { desktop_query(desktop_of::<2>(this), &IID_ISHELL_VIEW, out) }
}

/// `IShellView` after `IUnknown` (`shobjidl_core.h`): `IOleWindow`'s `GetWindow` 3,
/// `ContextSensitiveHelp` 4; `TranslateAccelerator` 5, `EnableModeless` 6, `UIActivate` 7,
/// `Refresh` 8, `CreateViewWindow` 9, `DestroyViewWindow` 10, `GetCurrentInfo` 11,
/// `AddPropertySheetPages` 12, `SaveViewState` 13, `SelectItem` 14, `GetItemObject` 15.
static DESKTOP_VIEW_VTBL: Vtbl<16> = Vtbl([
    d_query::<3> as *const c_void,
    d_add_ref::<3> as *const c_void,
    d_release::<3> as *const c_void,
    view_window as *const c_void,
    not_impl_1 as *const c_void,
    not_impl_1 as *const c_void,
    not_impl_1 as *const c_void,
    not_impl_1 as *const c_void,
    not_impl_0 as *const c_void,
    not_impl_5 as *const c_void,
    not_impl_0 as *const c_void,
    not_impl_1 as *const c_void,
    not_impl_3 as *const c_void,
    not_impl_0 as *const c_void,
    not_impl_2 as *const c_void,
    item_object as *const c_void,
]);

/// `GetItemObject(uItem, riid, ppv)`: the background object (`SVGIO_BACKGROUND`), a new
/// `IShellFolderViewDual`; nothing for the items (the desktop shows none to Windows programs).
unsafe extern "system" fn item_object(
    _this: *mut c_void,
    item: u32,
    iid: *const Guid,
    out: *mut *mut c_void,
) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = core::ptr::null_mut() };
    if item & 0xF != SVGIO_BACKGROUND {
        return E_NOTIMPL;
    }
    let object = into_raw(FolderView { head: Head::new(&FOLDER_VIEW_VTBL, &FOLDER_VIEW_IIDS, drop_box::<FolderView>) });
    // SAFETY: a fresh object; the caller's out parameter.
    unsafe { hand_out(object.cast(), iid, out) }
}

// ----- the view's background object -----------------------------------------------------------

#[repr(C)]
struct FolderView {
    head: Head,
}

static FOLDER_VIEW_IIDS: [Guid; 2] = [IID_IDISPATCH, IID_ISHELL_FOLDER_VIEW_DUAL];

/// `IShellFolderViewDual` (`shldisp.h`) after `IDispatch`: `get_Application` 7, `get_Parent` 8,
/// `get_Folder` 9, `SelectedItems` 10, `get_FocusedItem` 11, `SelectItem` 12, `PopupItemMenu`
/// 13, `get_Script` 14, `get_ViewOptions` 15.
static FOLDER_VIEW_VTBL: Vtbl<16> = Vtbl([
    query_interface as *const c_void,
    add_ref as *const c_void,
    release as *const c_void,
    type_info_count as *const c_void,
    type_info as *const c_void,
    no_ids as *const c_void,
    no_invoke as *const c_void,
    application as *const c_void,
    no_object as *const c_void,
    no_object as *const c_void,
    no_object as *const c_void,
    no_object as *const c_void,
    not_impl_2 as *const c_void,
    popup_item_menu as *const c_void,
    no_object as *const c_void,
    no_object as *const c_void,
]);

/// `get_Application(ppid)`: `Shell.Application`.
unsafe extern "system" fn application(_this: *mut c_void, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { create_shell_app(&IID_IDISPATCH, out) }
}

/// `PopupItemMenu(pfi, vx, vy, pbs)`: no menus.
unsafe extern "system" fn popup_item_menu(
    _this: *mut c_void,
    _item: usize,
    _x: Variant,
    _y: Variant,
    _out: usize,
) -> Hresult {
    E_NOTIMPL
}

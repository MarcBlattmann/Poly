//! `ITypeInfo::Invoke`: a dispatch call (`IDispatch::Invoke` of an object whose type information is
//! ours) turned into a call of the interface's vtable method (sheet `DllGetClassObject`, rule
//! `invoke`): arguments in reverse order, converted to the parameters' types
//! (`DISP_E_TYPEMISMATCH` with the argument's index), the count checked
//! (`DISP_E_BADPARAMCOUNT`), the member found by id and by kind (`DISP_E_MEMBERNOTFOUND`), the
//! `[out, retval]` parameter returned as the result, and a failing `HRESULT` reported as
//! `DISP_E_EXCEPTION` with it in `scode`. Also `DispCallFunc`.

use super::objects::{TypeInfo, view_of};
use crate::call;
use crate::com::*;
use crate::variant::{self, DISP_E_TYPEMISMATCH, Variant, set, value_of, vt_of};
use alloc::vec::Vec;
use core::ffi::c_void;
use poly_com::typelib::view::vtable_funcs;
use poly_com::typelib::*;
use poly_kernelbase::oleaut::{VARIANT_SIZE, VT_ARRAY, VT_BYREF};

pub(crate) const DISP_E_MEMBERNOTFOUND: Hresult = 0x8002_0003_u32 as i32;
pub(crate) const DISP_E_BADPARAMCOUNT: Hresult = 0x8002_000E_u32 as i32;
pub(crate) const DISP_E_EXCEPTION: Hresult = 0x8002_0009_u32 as i32;
pub(crate) const DISP_E_NONAMEDARGS: Hresult = 0x8002_0007_u32 as i32;
const DISPID_PROPERTYPUT: i32 = -3;

/// `DISPPARAMS`.
#[repr(C)]
pub(crate) struct DispParams {
    pub args: *mut Variant,
    pub named: *mut i32,
    pub c_args: u32,
    pub c_named: u32,
}

/// `EXCEPINFO`.
#[repr(C)]
pub(crate) struct ExcepInfo {
    pub code: u16,
    pub reserved: u16,
    pub source: *mut u16,
    pub description: *mut u16,
    pub help_file: *mut u16,
    pub help_context: u32,
    pub reserved2: *mut c_void,
    pub deferred: *mut c_void,
    pub scode: i32,
}

/// The variant type a value of `td` travels as in a dispatch call (`None`: not one Poly converts).
pub(crate) fn variant_type(td: &TypeDesc) -> Option<u16> {
    Some(match td {
        TypeDesc::Base(VT_INT) => VT_I4,
        TypeDesc::Base(VT_UINT) => VT_UI4,
        TypeDesc::Base(VT_HRESULT) => VT_ERROR,
        TypeDesc::Base(VT_LPWSTR) | TypeDesc::Base(VT_LPSTR) => VT_BSTR,
        TypeDesc::Base(vt) => *vt,
        TypeDesc::SafeArray(inner) => VT_ARRAY | variant_type(inner)?,
        TypeDesc::Ptr(inner) => match &**inner {
            TypeDesc::UserDefined(_) => VT_UNKNOWN,
            _ => return None,
        },
        TypeDesc::UserDefined(_) => VT_UNKNOWN,
        _ => return None,
    })
}

/// Where a call's out value lives until it becomes the result.
struct Out {
    slot: Variant,
    td: TypeDesc,
}

/// `ITypeInfo::Invoke(pvInstance, memid, wFlags, pDispParams, pVarResult, pExcepInfo, puArgErr)`.
pub(crate) unsafe extern "system" fn ti_invoke(
    this: *mut c_void,
    instance: *mut c_void,
    memid: i32,
    flags: u16,
    params: *mut DispParams,
    result: *mut Variant,
    excep: *mut ExcepInfo,
    arg_err: *mut u32,
) -> Hresult {
    if instance.is_null() || params.is_null() {
        return E_INVALIDARG;
    }
    let ti = this as *mut TypeInfo;
    let Some((lib, v)) = view_of(ti) else { return E_UNEXPECTED };
    // SAFETY: a live block.
    let index = unsafe { (*ti).index };
    let Some(df) = v.funcs.iter().find(|f| f.memid == memid && f.invkind & i32::from(flags) != 0) else {
        return DISP_E_MEMBERNOTFOUND;
    };
    // The method as the vtable has it (with its HRESULT and [out, retval]).
    let raw = match vtable_funcs(&lib, index) {
        Some(fs) => fs.into_iter().find(|f| f.memid == df.memid && f.invkind == df.invkind),
        None => None,
    };
    let Some(f) = raw else { return DISP_E_MEMBERNOTFOUND };
    // SAFETY: the caller's DISPPARAMS.
    let dp = unsafe { &*params };
    if dp.c_named > 0
        && !(dp.c_named == 1 && flags & 0xC != 0 && !dp.named.is_null() && unsafe { *dp.named } == DISPID_PROPERTYPUT)
    {
        return DISP_E_NONAMEDARGS;
    }
    let retval =
        f.params.last().is_some_and(|p| p.flags & PARAMFLAG_FRETVAL != 0) && f.ret == TypeDesc::Base(VT_HRESULT);
    let n_in = f.params.len() - usize::from(retval);
    let given = dp.c_args as usize;
    let required = n_in - f.params[..n_in].iter().filter(|p| p.flags & PARAMFLAG_FOPT != 0).count();
    if given > n_in || given < required {
        return DISP_E_BADPARAMCOUNT;
    }
    let mut args: Vec<u64> = alloc::vec![instance as u64];
    let mut temps: Vec<Variant> = Vec::with_capacity(f.params.len());
    let mut outs: Vec<Out> = Vec::new();
    // `temps` must not move while its pointers are in `args`.
    temps.resize(f.params.len(), [0; VARIANT_SIZE]);
    let mut status = S_OK;
    for (i, p) in f.params.iter().enumerate() {
        if i == n_in {
            // the [out, retval] parameter: a slot of ours
            let inner = p.td.inner().cloned().unwrap_or(TypeDesc::Base(VT_VARIANT));
            outs.push(Out { slot: [0; VARIANT_SIZE], td: inner });
            args.push(0);
            continue;
        }
        if i >= given {
            // a missing optional argument: VT_ERROR / DISP_E_PARAMNOTFOUND in a variant
            set(temps[i].as_mut_ptr(), VT_ERROR, 0x8002_0004);
            args.push(if p.td == TypeDesc::Base(VT_VARIANT) { temps[i].as_ptr() as u64 } else { 0 });
            continue;
        }
        let src = dp.args.wrapping_add(given - 1 - i) as *const u8;
        let arg_index = (given - 1 - i) as u32;
        match &p.td {
            TypeDesc::Base(VT_VARIANT) => {
                // passed as a pointer to a VARIANT (the caller's)
                args.push(src as u64);
            }
            TypeDesc::Ptr(inner) => {
                // [out] or [in, out]: a by-reference variant gives its pointer; else a slot of ours
                let want = match &**inner {
                    TypeDesc::Base(VT_VARIANT) => Some(VT_VARIANT),
                    t => variant_type(t),
                };
                if vt_of(src) & VT_BYREF != 0 && want.is_some_and(|w| vt_of(src) & !VT_BYREF == w) {
                    args.push(value_of(src));
                } else if want == Some(VT_VARIANT) {
                    args.push(src as u64);
                } else {
                    // SAFETY: our temporary and the caller's variant.
                    let hr = unsafe { variant::change_type(temps[i].as_mut_ptr(), src, want.unwrap_or(VT_I4)) };
                    if hr < 0 {
                        status = hr;
                        put(arg_err, arg_index);
                        break;
                    }
                    args.push(temps[i].as_ptr() as u64 + 8);
                }
            }
            td => {
                let Some(want) = variant_type(td) else {
                    status = DISP_E_TYPEMISMATCH;
                    put(arg_err, arg_index);
                    break;
                };
                // SAFETY: our temporary and the caller's variant.
                let hr = unsafe { variant::change_type(temps[i].as_mut_ptr(), src, want) };
                if hr < 0 {
                    status = if hr == variant::DISP_E_OVERFLOW { hr } else { DISP_E_TYPEMISMATCH };
                    put(arg_err, arg_index);
                    break;
                }
                args.push(value_of(temps[i].as_ptr()));
            }
        }
    }
    if status < 0 {
        clear_all(&mut temps);
        return status;
    }
    // The retval slot's pointer (after `outs` is complete, so it does not move).
    if retval {
        let at = args.len() - 1;
        args[at] = outs[0].slot.as_mut_ptr() as u64 + 8;
        if matches!(outs[0].td, TypeDesc::Base(VT_VARIANT)) {
            args[at] = outs[0].slot.as_mut_ptr() as u64;
        }
    }
    // SAFETY: the instance's vtable; the method at its offset.
    let fptr = unsafe { *(*(instance as *const *const usize)).add(f.ovft as usize / core::mem::size_of::<usize>()) };
    if !call::AVAILABLE {
        clear_all(&mut temps);
        return E_NOTIMPL;
    }
    // SAFETY: a method of the stated parameters.
    let hr = unsafe { call::call(fptr, &args, core::ptr::null_mut()) } as u32 as i32;
    clear_all(&mut temps);
    if hr < 0 {
        if !excep.is_null() {
            // SAFETY: the caller's EXCEPINFO.
            unsafe {
                core::ptr::write_bytes(excep, 0, 1);
                (*excep).scode = hr;
                fill_from_error_info(&mut *excep);
            }
        }
        for o in &mut outs {
            free_out(o);
        }
        return DISP_E_EXCEPTION;
    }
    for mut o in outs {
        if result.is_null() {
            free_out(&mut o);
            continue;
        }
        to_result(&mut o, result as *mut u8);
    }
    S_OK
}

fn clear_all(temps: &mut [Variant]) {
    for t in temps {
        if vt_of(t.as_ptr()) != 0 {
            // SAFETY: our temporaries (initialised variants).
            unsafe { crate::VariantClear(t.as_mut_ptr()) };
        }
    }
}

/// The variant result for an out value of type `td` (its slot holds the value at offset 8, or the
/// whole variant for a `VARIANT`).
fn to_result(o: &mut Out, result: *mut u8) {
    // SAFETY: the caller's result variant.
    unsafe { crate::VariantClear(result) };
    match &o.td {
        TypeDesc::Base(VT_VARIANT) => {
            // SAFETY: both variants.
            unsafe { core::ptr::copy_nonoverlapping(o.slot.as_ptr(), result, VARIANT_SIZE) };
        }
        td => {
            let vt = variant_type(td).unwrap_or(VT_I4);
            let bits = value_of(o.slot.as_ptr());
            let bits = match vt {
                VT_I2 | VT_UI2 | VT_BOOL => bits & 0xFFFF,
                VT_I1 | VT_UI1 => bits & 0xFF,
                VT_I4 | VT_UI4 | VT_ERROR | VT_R4 => bits & 0xFFFF_FFFF,
                _ => bits,
            };
            set(result, vt, bits);
        }
    }
}

fn free_out(o: &mut Out) {
    let td = o.td.clone();
    let mut tmp: Variant = [0; VARIANT_SIZE];
    to_result(o, tmp.as_mut_ptr());
    let _ = td;
    // SAFETY: our variant.
    unsafe { crate::VariantClear(tmp.as_mut_ptr()) };
}

/// The thread's error information, if the method set some, into an `EXCEPINFO` (the object stays
/// set for the caller).
fn fill_from_error_info(e: &mut ExcepInfo) {
    let Some(ei) = crate::errinfo::peek() else { return };
    let mut s: *mut u16 = core::ptr::null_mut();
    vcall!(ei, 4, (*mut *mut u16) -> Hresult, &mut s);
    e.source = s;
    let mut d: *mut u16 = core::ptr::null_mut();
    vcall!(ei, 5, (*mut *mut u16) -> Hresult, &mut d);
    e.description = d;
    let mut h: *mut u16 = core::ptr::null_mut();
    vcall!(ei, 6, (*mut *mut u16) -> Hresult, &mut h);
    e.help_file = h;
    let mut c = 0u32;
    vcall!(ei, 7, (*mut u32) -> Hresult, &mut c);
    e.help_context = c;
    release(ei);
}

/// `DispCallFunc(pvInstance, oVft, cc, vtReturn, cActuals, prgvt, prgpvarg, pvargResult)`: call a
/// function (a vtable method of `pvInstance`, or the function at `oVft` when it is NULL) with the
/// values of the given variants.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DispCallFunc(
    instance: *mut c_void,
    ovft: usize,
    _cc: i32,
    vt_ret: u16,
    n: u32,
    vts: *const u16,
    args: *const *const u8,
    result: *mut u8,
) -> Hresult {
    if !call::AVAILABLE {
        return E_NOTIMPL;
    }
    if n > 0 && (vts.is_null() || args.is_null()) {
        return E_INVALIDARG;
    }
    let f = if instance.is_null() {
        ovft
    } else {
        // SAFETY: the instance's vtable.
        unsafe { *(*(instance as *const *const usize)).add(ovft / core::mem::size_of::<usize>()) }
    };
    let mut a: Vec<u64> = Vec::new();
    if !instance.is_null() {
        a.push(instance as u64);
    }
    for i in 0..n as usize {
        // SAFETY: the caller's arrays of `n` entries.
        let (vt, v) = unsafe { (*vts.add(i), *args.add(i)) };
        a.push(if vt == VT_VARIANT { v as u64 } else { value_of(v) });
    }
    let mut fp = 0u64;
    // SAFETY: the caller's function of these arguments.
    let r = unsafe { call::call(f, &a, &mut fp) };
    if !result.is_null() {
        let bits = match vt_ret {
            VT_R4 | VT_R8 | VT_DATE => fp,
            _ => r,
        };
        if vt_ret != VT_EMPTY && vt_ret != VT_VOID {
            set(result, vt_ret, bits);
        } else {
            set(result, VT_EMPTY, 0);
        }
    }
    S_OK
}

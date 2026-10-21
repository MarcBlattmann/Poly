//! The registry API (ADR 0083): `RegOpenKeyExW`, `RegCreateKeyExW`,
//! `RegQueryValueExW`, `RegSetValueExW`, `RegGetValueW`, `RegEnumKeyExW`,
//! `RegEnumValueW`, `RegDeleteKeyExW`, `RegDeleteValueW`, `RegCloseKey`,
//! `RegFlushKey`, `RegQueryInfoKeyW` and the `A` forms of the common ones,
//! over our `ntdll`'s `Nt*Key` services (ADR 0066). `advapi32` forwards
//! them here through `api-ms-win-core-registry-l1-1-0`.
//!
//! Every function returns its Win32 error (Learn: "If the function fails,
//! the return value is a nonzero error code defined in Winerror.h") and
//! leaves the thread's last error alone (to be confirmed by the spec agent).
//! The predefined keys are opened once per process and cached
//! (`poly_kernelbase::registry` has the paths and the `HKEY_CLASSES_ROOT`
//! view).

use crate::base::{self, ansi_arg, close, wide_arg};
use crate::nt::{Handle, OBJ_CASE_INSENSITIVE, OBJ_INHERIT, ObjectAttributes, UnicodeString};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicIsize, Ordering};
use poly_kernelbase::error::*;
use poly_kernelbase::registry::{self as reg, Predefined};
use poly_ntobj::NtStatus;

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtOpenKeyEx(handle: *mut Handle, access: u32, attributes: *const ObjectAttributes, options: u32) -> NtStatus;
    fn NtCreateKey(
        handle: *mut Handle,
        access: u32,
        attributes: *const ObjectAttributes,
        title: u32,
        class: *const UnicodeString,
        options: u32,
        disposition: *mut u32,
    ) -> NtStatus;
    fn NtDeleteKey(handle: Handle) -> NtStatus;
    fn NtDeleteValueKey(handle: Handle, name: *const UnicodeString) -> NtStatus;
    fn NtEnumerateKey(handle: Handle, index: u32, class: u32, info: *mut u8, length: u32, ret: *mut u32) -> NtStatus;
    fn NtEnumerateValueKey(
        handle: Handle,
        index: u32,
        class: u32,
        info: *mut u8,
        length: u32,
        ret: *mut u32,
    ) -> NtStatus;
    fn NtQueryKey(handle: Handle, class: u32, info: *mut u8, length: u32, ret: *mut u32) -> NtStatus;
    fn NtQueryValueKey(
        handle: Handle,
        name: *const UnicodeString,
        class: u32,
        info: *mut u8,
        length: u32,
        ret: *mut u32,
    ) -> NtStatus;
    fn NtSetValueKey(
        handle: Handle,
        name: *const UnicodeString,
        title: u32,
        kind: u32,
        data: *const u8,
        size: u32,
    ) -> NtStatus;
    fn NtFlushKey(handle: Handle) -> NtStatus;
}

type HKey = isize;

/// `STATUS_BUFFER_OVERFLOW`, `STATUS_BUFFER_TOO_SMALL`,
/// `STATUS_OBJECT_NAME_NOT_FOUND` (`ntstatus.h`).
const STATUS_BUFFER_OVERFLOW: NtStatus = NtStatus(0x8000_0005);
const STATUS_BUFFER_TOO_SMALL: NtStatus = NtStatus(0xC000_0023);
const STATUS_OBJECT_NAME_NOT_FOUND: NtStatus = NtStatus(0xC000_0034);

/// `KEY_NODE_INFORMATION`, `KEY_FULL_INFORMATION`,
/// `KEY_VALUE_FULL_INFORMATION`, `KEY_VALUE_PARTIAL_INFORMATION` (`wdm.h`).
const KEY_NODE_INFORMATION: u32 = 1;
const KEY_FULL_INFORMATION: u32 = 2;
const KEY_VALUE_FULL_INFORMATION: u32 = 1;
const KEY_VALUE_PARTIAL_INFORMATION: u32 = 2;

/// `SECURITY_ATTRIBUTES.bInheritHandle` (offset 16 on x64, `minwinbase.h`).
const SA_INHERIT_OFFSET: usize = 16;

fn err(status: NtStatus) -> u32 {
    poly_kernelbase::error::from_status(status)
}

/// An `OBJECT_ATTRIBUTES` naming `name` below `root`.
fn attributes(root: Handle, name: &UnicodeString, inherit: bool) -> ObjectAttributes {
    let mut a = ObjectAttributes::new(Some(name), OBJ_CASE_INSENSITIVE | if inherit { OBJ_INHERIT } else { 0 });
    a.root = root;
    a
}

/// Open `name` below `root` (an empty name: `root` itself again).
fn nt_open(root: Handle, name: &[u16], access: u32, options: u32) -> Result<Handle, NtStatus> {
    let s = UnicodeString::new(name);
    let a = attributes(root, &s, false);
    let mut h = 0;
    // SAFETY: the attributes point at live locals.
    let st = unsafe { NtOpenKeyEx(&mut h, reg::key_access(access), &a, options) };
    if st.is_success() { Ok(h) } else { Err(st) }
}

// ----- predefined keys -----------------------------------------------------------------

/// The cached handles of the predefined keys (0: not opened yet), and of
/// the user's half of `HKEY_CLASSES_ROOT` (-1: it does not exist).
static ROOTS: [AtomicIsize; 10] = [const { AtomicIsize::new(0) }; 10];
static USER_CLASSES: AtomicIsize = AtomicIsize::new(0);

/// The token user's SID as a string (`HKEY_CURRENT_USER` is
/// `\REGISTRY\USER\<it>`).
fn user_sid() -> alloc::string::String {
    crate::security::token_user().map_or_else(
        || poly_security::sddl::to_string(&poly_security::poly_user_sid()),
        |s| poly_security::sddl::to_string(&s),
    )
}

/// Keep `h` in `slot` unless another thread got there first.
fn cache(slot: &AtomicIsize, h: Handle) -> Handle {
    match slot.compare_exchange(0, h, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => h,
        Err(other) => {
            if h > 0 {
                close(h);
            }
            other
        }
    }
}

fn open_absolute(path: &str) -> Result<Handle, NtStatus> {
    let units: Vec<u16> = path.encode_utf16().collect();
    nt_open(0, &units, reg::KEY_ALL_ACCESS, 0)
}

/// The handle of a predefined key (opened at first use).
fn root(p: Predefined) -> Result<Handle, u32> {
    let slot = &ROOTS[p.index()];
    let h = slot.load(Ordering::Acquire);
    if h != 0 {
        return Ok(h);
    }
    let path = reg::root_path(p, &user_sid()).ok_or(ERROR_INVALID_HANDLE)?;
    let h = open_absolute(&path).map_err(err)?;
    Ok(cache(slot, h))
}

/// The user's half of `HKEY_CLASSES_ROOT`, if it exists.
fn user_classes() -> Option<Handle> {
    let h = USER_CLASSES.load(Ordering::Acquire);
    if h != 0 {
        return (h > 0).then_some(h);
    }
    let h = open_absolute(&reg::user_classes(&user_sid())).unwrap_or(-1);
    let h = cache(&USER_CLASSES, h);
    (h > 0).then_some(h)
}

/// The NT handle an `HKEY` stands for.
fn handle_of(hkey: HKey) -> Result<Handle, u32> {
    match reg::predefined(hkey as u64) {
        Some(p) => root(p),
        None if hkey == 0 => Err(ERROR_INVALID_HANDLE),
        None => Ok(hkey),
    }
}

/// Open `sub` below `hkey` (through the `HKEY_CLASSES_ROOT` view), the
/// caller closing the result.
fn open_sub(hkey: HKey, sub: &[u16], access: u32, options: u32) -> Result<Handle, u32> {
    let sub = reg::normalized(sub);
    if reg::predefined(hkey as u64) == Some(Predefined::ClassesRoot)
        && !sub.is_empty()
        && let Some(user) = user_classes()
    {
        match nt_open(user, &sub, access, options) {
            Ok(h) => return Ok(h),
            Err(STATUS_OBJECT_NAME_NOT_FOUND) => {}
            Err(e) => return Err(err(e)),
        }
    }
    nt_open(handle_of(hkey)?, &sub, access, options).map_err(err)
}

/// Run `f` on the key `sub` names below `hkey` (or `hkey` itself), closing
/// what was opened.
fn with_key<R>(
    hkey: HKey,
    sub: Option<&[u16]>,
    access: u32,
    f: impl FnOnce(Handle) -> Result<R, u32>,
) -> Result<R, u32> {
    match sub {
        Some(s) if !reg::components(s).is_empty() => {
            let h = open_sub(hkey, s, access, 0)?;
            let r = f(h);
            close(h);
            r
        }
        _ => f(handle_of(hkey)?),
    }
}

fn result(r: Result<(), u32>) -> u32 {
    r.err().unwrap_or(ERROR_SUCCESS)
}

// ----- open, create, close -------------------------------------------------------------

/// `RegOpenKeyExW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RegOpenKeyExW(
    hkey: HKey,
    sub: *const u16,
    options: u32,
    sam: u32,
    out: *mut HKey,
) -> u32 {
    if out.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    // SAFETY: the caller's NUL-terminated string or null.
    let sub = unsafe { wide_arg(sub) }.unwrap_or(&[]);
    let r = if reg::components(sub).is_empty() && reg::predefined(hkey as u64).is_some() {
        // Learn: an empty subkey of a predefined key returns that key.
        handle_of(hkey).map(|_| hkey)
    } else {
        open_sub(hkey, sub, sam, options & reg::REG_OPTION_OPEN_LINK)
    };
    match r {
        Ok(h) => {
            // SAFETY: checked non-null; the caller's variable.
            unsafe { out.write(h) };
            ERROR_SUCCESS
        }
        Err(e) => e,
    }
}

/// `RegOpenKeyExA`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RegOpenKeyExA(hkey: HKey, sub: *const u8, options: u32, sam: u32, out: *mut HKey) -> u32 {
    // SAFETY: the caller's string or null.
    let sub = unsafe { ansi_arg(sub) }.map(|s| base::with_nul(&s));
    // SAFETY: a NUL-terminated copy or null.
    unsafe { RegOpenKeyExW(hkey, sub.as_ref().map_or(core::ptr::null(), |s| s.as_ptr()), options, sam, out) }
}

/// Create (or open) one component below `parent`.
fn create_one(
    parent: Handle,
    name: &[u16],
    access: u32,
    class: Option<&[u16]>,
    options: u32,
    inherit: bool,
) -> Result<(Handle, u32), NtStatus> {
    let s = UnicodeString::new(name);
    let a = attributes(parent, &s, inherit);
    let class = class.map(UnicodeString::new);
    let class_ptr = class.as_ref().map_or(core::ptr::null(), |c| c as *const UnicodeString);
    let (mut h, mut disposition) = (0, 0);
    // SAFETY: every pointer is to a live local.
    let st = unsafe { NtCreateKey(&mut h, reg::key_access(access), &a, 0, class_ptr, options, &mut disposition) };
    if st.is_success() { Ok((h, disposition)) } else { Err(st) }
}

/// `RegCreateKeyExW`: every missing key on the path is created (Learn:
/// "The RegCreateKeyEx function creates all missing keys in the specified
/// path"), with the same options; the class goes to the last one only.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RegCreateKeyExW(
    hkey: HKey,
    sub: *const u16,
    _reserved: u32,
    class: *const u16,
    options: u32,
    sam: u32,
    security: *const u8,
    out: *mut HKey,
    disposition: *mut u32,
) -> u32 {
    if out.is_null() || sub.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    // SAFETY: checked non-null; NUL-terminated per the contract.
    let sub = unsafe { wide_arg(sub) }.unwrap_or(&[]);
    // SAFETY: the caller's string or null.
    let class = unsafe { wide_arg(class) };
    // SAFETY: a SECURITY_ATTRIBUTES or null.
    let inherit = !security.is_null() && unsafe { (security.add(SA_INHERIT_OFFSET) as *const i32).read() } != 0;
    let parts = reg::components(sub);
    let options = options & (reg::REG_OPTION_VOLATILE | reg::REG_OPTION_CREATE_LINK | reg::REG_OPTION_BACKUP_RESTORE);
    let start = if reg::predefined(hkey as u64) == Some(Predefined::ClassesRoot) && !parts.is_empty() {
        // The view: an existing key under the user's classes is used;
        // anything new goes under HKLM\Software\Classes (Learn).
        match user_classes().map(|u| nt_open(u, &reg::normalized(sub), sam, 0)) {
            Some(Ok(h)) => return finish_create(out, disposition, h, reg::REG_OPENED_EXISTING_KEY),
            _ => handle_of(hkey),
        }
    } else {
        handle_of(hkey)
    };
    let mut parent = match start {
        Ok(h) => h,
        Err(e) => return e,
    };
    if parts.is_empty() {
        // Learn: an empty subkey gives a new handle to hKey.
        return match nt_open(parent, &[], sam, 0) {
            Ok(h) => finish_create(out, disposition, h, reg::REG_OPENED_EXISTING_KEY),
            Err(e) => err(e),
        };
    }
    let mut opened: Option<Handle> = None;
    let mut last = 0;
    for (i, name) in parts.iter().enumerate() {
        let final_part = i + 1 == parts.len();
        let access = if final_part { sam } else { reg::KEY_ALL_ACCESS };
        let r = create_one(parent, name, access, if final_part { class } else { None }, options, inherit && final_part);
        if let Some(h) = opened.take() {
            close(h);
        }
        match r {
            Ok((h, d)) => {
                parent = h;
                opened = Some(h);
                last = d;
            }
            Err(e) => return err(e),
        }
    }
    finish_create(out, disposition, parent, last)
}

fn finish_create(out: *mut HKey, disposition: *mut u32, h: Handle, d: u32) -> u32 {
    // SAFETY: `out` was checked non-null; `disposition` is the caller's
    // optional variable.
    unsafe {
        out.write(h);
        if !disposition.is_null() {
            disposition.write(d);
        }
    }
    ERROR_SUCCESS
}

/// `RegCreateKeyExA`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RegCreateKeyExA(
    hkey: HKey,
    sub: *const u8,
    reserved: u32,
    class: *const u8,
    options: u32,
    sam: u32,
    security: *const u8,
    out: *mut HKey,
    disposition: *mut u32,
) -> u32 {
    // SAFETY: the caller's strings or null.
    let (sub, class) =
        unsafe { (ansi_arg(sub).map(|s| base::with_nul(&s)), ansi_arg(class).map(|s| base::with_nul(&s))) };
    let p = |s: &Option<Vec<u16>>| s.as_ref().map_or(core::ptr::null(), |s| s.as_ptr());
    // SAFETY: NUL-terminated copies or null; the rest is the caller's.
    unsafe { RegCreateKeyExW(hkey, p(&sub), reserved, p(&class), options, sam, security, out, disposition) }
}

/// `RegCloseKey`: a predefined key stays open (cached).
#[unsafe(no_mangle)]
pub extern "system" fn RegCloseKey(hkey: HKey) -> u32 {
    if reg::predefined(hkey as u64).is_some() {
        return ERROR_SUCCESS;
    }
    // SAFETY: closing a handle value is memory-safe; the kernel checks it.
    let st = unsafe { crate::nt::NtClose(hkey) };
    if st.is_success() { ERROR_SUCCESS } else { err(st) }
}

/// `RegFlushKey`.
#[unsafe(no_mangle)]
pub extern "system" fn RegFlushKey(hkey: HKey) -> u32 {
    result(handle_of(hkey).and_then(|h| {
        // SAFETY: a handle value; the kernel checks it.
        let st = unsafe { NtFlushKey(h) };
        if st.is_success() { Ok(()) } else { Err(err(st)) }
    }))
}

// ----- delete ---------------------------------------------------------------------------

/// `RegDeleteKeyExW`: open the subkey for `DELETE` and delete it (it must
/// have no subkeys: the kernel refuses with `STATUS_CANNOT_DELETE`,
/// `ERROR_ACCESS_DENIED`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RegDeleteKeyExW(hkey: HKey, sub: *const u16, sam: u32, _reserved: u32) -> u32 {
    // SAFETY: the caller's string or null.
    let Some(sub) = (unsafe { wide_arg(sub) }) else { return ERROR_INVALID_PARAMETER };
    let access = reg::DELETE | (sam & reg::KEY_WOW64_RES);
    let h = match open_sub(hkey, sub, access, 0) {
        Ok(h) => h,
        Err(e) => return e,
    };
    // SAFETY: a key handle we opened.
    let st = unsafe { NtDeleteKey(h) };
    close(h);
    if st.is_success() { ERROR_SUCCESS } else { err(st) }
}

/// `RegDeleteKeyExA`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RegDeleteKeyExA(hkey: HKey, sub: *const u8, sam: u32, reserved: u32) -> u32 {
    // SAFETY: the caller's string or null.
    let sub = unsafe { ansi_arg(sub) }.map(|s| base::with_nul(&s));
    // SAFETY: a NUL-terminated copy or null.
    unsafe { RegDeleteKeyExW(hkey, sub.as_ref().map_or(core::ptr::null(), |s| s.as_ptr()), sam, reserved) }
}

/// `RegDeleteValueW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RegDeleteValueW(hkey: HKey, name: *const u16) -> u32 {
    // SAFETY: the caller's string or null (the unnamed value).
    let name = unsafe { wide_arg(name) }.unwrap_or(&[]);
    result(handle_of(hkey).and_then(|h| {
        let s = UnicodeString::new(name);
        // SAFETY: a live UNICODE_STRING.
        let st = unsafe { NtDeleteValueKey(h, &s) };
        if st.is_success() { Ok(()) } else { Err(err(st)) }
    }))
}

/// `RegDeleteValueA`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RegDeleteValueA(hkey: HKey, name: *const u8) -> u32 {
    // SAFETY: the caller's string or null.
    let name = unsafe { ansi_arg(name) }.map(|s| base::with_nul(&s));
    // SAFETY: a NUL-terminated copy or null.
    unsafe { RegDeleteValueW(hkey, name.as_ref().map_or(core::ptr::null(), |s| s.as_ptr())) }
}

// ----- values ---------------------------------------------------------------------------

/// Call an `Nt*` information service with a buffer that grows to what it
/// reports.
fn query_info(mut call: impl FnMut(*mut u8, u32, *mut u32) -> NtStatus) -> Result<Vec<u8>, NtStatus> {
    let mut buf: Vec<u8> = alloc::vec![0; 256];
    loop {
        let mut need = 0u32;
        let st = call(buf.as_mut_ptr(), buf.len() as u32, &mut need);
        if st.is_success() {
            buf.truncate(need as usize);
            return Ok(buf);
        }
        if (st == STATUS_BUFFER_OVERFLOW || st == STATUS_BUFFER_TOO_SMALL) && need as usize > buf.len() {
            buf.resize(need as usize, 0);
            continue;
        }
        return Err(st);
    }
}

/// A value's type and data.
fn query_value(h: Handle, name: &[u16]) -> Result<(u32, Vec<u8>), u32> {
    let s = UnicodeString::new(name);
    let info = query_info(|buf, len, ret| {
        // SAFETY: `buf` holds `len` bytes; `s` is live.
        unsafe { NtQueryValueKey(h, &s, KEY_VALUE_PARTIAL_INFORMATION, buf, len, ret) }
    })
    .map_err(err)?;
    reg::parse_value_partial(&info).ok_or(ERROR_INVALID_PARAMETER)
}

/// Hand a value back by the `RegQueryValueExW` rule.
///
/// # Safety
/// `kind`, `data` and `size` are the caller's optional out-parameters;
/// `data` holds `*size` bytes.
unsafe fn give_value(kind: u32, value: &[u8], kind_out: *mut u32, data: *mut u8, size: *mut u32) -> u32 {
    // SAFETY: the caller's contract.
    unsafe {
        if !kind_out.is_null() {
            kind_out.write(kind);
        }
        let capacity = if size.is_null() { 0 } else { size.read() };
        let r = reg::data_fit(value.len(), !data.is_null(), capacity);
        if !size.is_null() {
            size.write(value.len() as u32);
        }
        match r {
            Ok(true) => {
                core::ptr::copy_nonoverlapping(value.as_ptr(), data, value.len());
                ERROR_SUCCESS
            }
            Ok(false) => ERROR_SUCCESS,
            Err(e) => e,
        }
    }
}

/// `RegQueryValueExW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RegQueryValueExW(
    hkey: HKey,
    name: *const u16,
    reserved: *mut u32,
    kind: *mut u32,
    data: *mut u8,
    size: *mut u32,
) -> u32 {
    if !reserved.is_null() || (!data.is_null() && size.is_null()) {
        return ERROR_INVALID_PARAMETER;
    }
    // SAFETY: the caller's string or null (the unnamed value).
    let name = unsafe { wide_arg(name) }.unwrap_or(&[]);
    match handle_of(hkey).and_then(|h| query_value(h, name)) {
        // SAFETY: the caller's out-parameters.
        Ok((k, v)) => unsafe { give_value(k, &v, kind, data, size) },
        Err(e) => e,
    }
}

/// `RegQueryValueExA`: strings converted to the ANSI code page (Learn), and
/// their sizes with them.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RegQueryValueExA(
    hkey: HKey,
    name: *const u8,
    reserved: *mut u32,
    kind: *mut u32,
    data: *mut u8,
    size: *mut u32,
) -> u32 {
    if !reserved.is_null() || (!data.is_null() && size.is_null()) {
        return ERROR_INVALID_PARAMETER;
    }
    // SAFETY: the caller's string or null.
    let name = unsafe { ansi_arg(name) }.unwrap_or_default();
    match handle_of(hkey).and_then(|h| query_value(h, &name)) {
        // SAFETY: the caller's out-parameters.
        Ok((k, v)) => unsafe { give_value(k, &reg::to_ansi_data(k, &v), kind, data, size) },
        Err(e) => e,
    }
}

fn set_value(hkey: HKey, name: &[u16], kind: u32, data: &[u8]) -> u32 {
    result(handle_of(hkey).and_then(|h| {
        let s = UnicodeString::new(name);
        // SAFETY: `data` is a live slice of `data.len()` bytes.
        let st = unsafe { NtSetValueKey(h, &s, 0, kind, data.as_ptr(), data.len() as u32) };
        if st.is_success() { Ok(()) } else { Err(err(st)) }
    }))
}

/// `RegSetValueExW`. `Reserved` must be 0, and a null `lpData` comes with
/// a zero `cbData` (Learn), else `ERROR_INVALID_PARAMETER`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RegSetValueExW(
    hkey: HKey,
    name: *const u16,
    reserved: u32,
    kind: u32,
    data: *const u8,
    size: u32,
) -> u32 {
    if reserved != 0 || (data.is_null() && size != 0) {
        return ERROR_INVALID_PARAMETER;
    }
    // SAFETY: the caller's string or null.
    let name = unsafe { wide_arg(name) }.unwrap_or(&[]);
    // SAFETY: `data` holds `size` bytes.
    let data = if data.is_null() { &[][..] } else { unsafe { core::slice::from_raw_parts(data, size as usize) } };
    set_value(hkey, name, kind, data)
}

/// `RegSetValueExA`: string data is converted to UTF-16 (Learn).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RegSetValueExA(
    hkey: HKey,
    name: *const u8,
    reserved: u32,
    kind: u32,
    data: *const u8,
    size: u32,
) -> u32 {
    if reserved != 0 || (data.is_null() && size != 0) {
        return ERROR_INVALID_PARAMETER;
    }
    // SAFETY: the caller's string or null.
    let name = unsafe { ansi_arg(name) }.unwrap_or_default();
    // SAFETY: `data` holds `size` bytes.
    let data = if data.is_null() { &[][..] } else { unsafe { core::slice::from_raw_parts(data, size as usize) } };
    set_value(hkey, &name, kind, &reg::from_ansi_data(kind, data))
}

/// The process environment's expansion (`ExpandEnvironmentStringsW`).
fn expand(s: &[u16]) -> Vec<u16> {
    poly_kernelbase::env::expand(s, &|n| crate::process::env_var(n))
}

/// `RegGetValueW` and `RegGetValueA` (`ansi`).
///
/// # Safety
/// The caller's arguments, as for `RegGetValueW`.
#[allow(clippy::too_many_arguments)] // RegGetValueW's seven, and which form
unsafe fn get_value(
    hkey: HKey,
    sub: Option<&[u16]>,
    name: &[u16],
    flags: u32,
    kind: *mut u32,
    data: *mut u8,
    size: *mut u32,
    ansi: bool,
) -> u32 {
    // SAFETY: the caller's optional size variable.
    let capacity = if size.is_null() { 0 } else { unsafe { size.read() } };
    let r = (|| {
        if !data.is_null() && size.is_null() {
            return Err(ERROR_INVALID_PARAMETER);
        }
        reg::check_get_flags(flags)?;
        let (k, v) = with_key(hkey, sub, reg::KEY_QUERY_VALUE, |h| query_value(h, name))?;
        let (k, v) = reg::get_value(flags, k, &v, &expand)?;
        Ok((k, if ansi { reg::get_value_ansi(k, &v) } else { v }))
    })();
    // SAFETY: the caller's out-parameters (`data` holds `capacity` bytes).
    unsafe {
        let e = match r {
            Ok((k, v)) => give_value(k, &v, kind, data, size),
            Err(e) => e,
        };
        if e != ERROR_SUCCESS && flags & reg::RRF_ZEROONFAILURE != 0 && !data.is_null() {
            core::ptr::write_bytes(data, 0, capacity as usize);
        }
        e
    }
}

/// `RegGetValueW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RegGetValueW(
    hkey: HKey,
    sub: *const u16,
    name: *const u16,
    flags: u32,
    kind: *mut u32,
    data: *mut u8,
    size: *mut u32,
) -> u32 {
    // SAFETY: the caller's strings or null.
    let (sub, name) = unsafe { (wide_arg(sub), wide_arg(name).unwrap_or(&[])) };
    // SAFETY: the caller's out-parameters.
    unsafe { get_value(hkey, sub, name, flags, kind, data, size, false) }
}

/// `RegGetValueA`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RegGetValueA(
    hkey: HKey,
    sub: *const u8,
    name: *const u8,
    flags: u32,
    kind: *mut u32,
    data: *mut u8,
    size: *mut u32,
) -> u32 {
    // SAFETY: the caller's strings or null.
    let (sub, name) = unsafe { (ansi_arg(sub), ansi_arg(name).unwrap_or_default()) };
    // SAFETY: the caller's out-parameters.
    unsafe { get_value(hkey, sub.as_deref(), &name, flags, kind, data, size, true) }
}

// ----- enumeration and information ----------------------------------------------------

/// Copy a name by the enumeration rule: it fits when the buffer holds it
/// and a NUL; the count then becomes its length.
///
/// # Safety
/// `buf` holds `*count` units.
unsafe fn give_name(name: &[u16], buf: *mut u16, count: *mut u32) -> Result<(), u32> {
    // SAFETY: the caller's contract.
    unsafe {
        if (count.read() as usize) <= name.len() {
            return Err(ERROR_MORE_DATA);
        }
        base::put_wide(buf, name);
        count.write(name.len() as u32);
    }
    Ok(())
}

/// `RegEnumKeyExW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RegEnumKeyExW(
    hkey: HKey,
    index: u32,
    name: *mut u16,
    name_len: *mut u32,
    reserved: *mut u32,
    class: *mut u16,
    class_len: *mut u32,
    last_write: *mut u64,
) -> u32 {
    if name.is_null() || name_len.is_null() || !reserved.is_null() || (!class.is_null() && class_len.is_null()) {
        return ERROR_INVALID_PARAMETER;
    }
    let r = handle_of(hkey).and_then(|h| {
        let info = query_info(|buf, len, ret| {
            // SAFETY: `buf` holds `len` bytes.
            unsafe { NtEnumerateKey(h, index, KEY_NODE_INFORMATION, buf, len, ret) }
        })
        .map_err(err)?;
        reg::parse_node_information(&info).ok_or(ERROR_INVALID_PARAMETER)
    });
    let (time, key, key_class) = match r {
        Ok(v) => v,
        Err(e) => return e,
    };
    // SAFETY: the caller's buffers and counts, checked above.
    unsafe {
        if let Err(e) = give_name(&key, name, name_len) {
            return e;
        }
        if !class.is_null()
            && let Err(e) = give_name(&key_class, class, class_len)
        {
            return e;
        }
        if !last_write.is_null() {
            last_write.write(time);
        }
    }
    ERROR_SUCCESS
}

/// `RegEnumValueW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RegEnumValueW(
    hkey: HKey,
    index: u32,
    name: *mut u16,
    name_len: *mut u32,
    reserved: *mut u32,
    kind: *mut u32,
    data: *mut u8,
    size: *mut u32,
) -> u32 {
    if name.is_null() || name_len.is_null() || !reserved.is_null() || (!data.is_null() && size.is_null()) {
        return ERROR_INVALID_PARAMETER;
    }
    let r = handle_of(hkey).and_then(|h| {
        let info = query_info(|buf, len, ret| {
            // SAFETY: `buf` holds `len` bytes.
            unsafe { NtEnumerateValueKey(h, index, KEY_VALUE_FULL_INFORMATION, buf, len, ret) }
        })
        .map_err(err)?;
        reg::parse_value_full(&info).ok_or(ERROR_INVALID_PARAMETER)
    });
    let (k, value_name, value) = match r {
        Ok(v) => v,
        Err(e) => return e,
    };
    // SAFETY: the caller's buffers and counts, checked above.
    unsafe {
        if let Err(e) = give_name(&value_name, name, name_len) {
            return e;
        }
        give_value(k, &value, kind, data, size)
    }
}

/// `RegQueryInfoKeyW`. The key's security descriptor size is reported as 0
/// (keys have none in Poly, ADR 0066; to be confirmed by the spec agent).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RegQueryInfoKeyW(
    hkey: HKey,
    class: *mut u16,
    class_len: *mut u32,
    reserved: *mut u32,
    subkeys: *mut u32,
    max_subkey_len: *mut u32,
    max_class_len: *mut u32,
    values: *mut u32,
    max_value_name_len: *mut u32,
    max_value_len: *mut u32,
    security_len: *mut u32,
    last_write: *mut u64,
) -> u32 {
    if !reserved.is_null() || (!class.is_null() && class_len.is_null()) {
        return ERROR_INVALID_PARAMETER;
    }
    let r = handle_of(hkey).and_then(|h| {
        let info = query_info(|buf, len, ret| {
            // SAFETY: `buf` holds `len` bytes.
            unsafe { NtQueryKey(h, KEY_FULL_INFORMATION, buf, len, ret) }
        })
        .map_err(err)?;
        reg::parse_full_information(&info).ok_or(ERROR_INVALID_PARAMETER)
    });
    let info = match r {
        Ok(i) => i,
        Err(e) => return e,
    };
    let put = |p: *mut u32, v: u32| {
        if !p.is_null() {
            // SAFETY: the caller's optional variable.
            unsafe { p.write(v) };
        }
    };
    put(subkeys, info.subkeys);
    put(max_subkey_len, info.max_subkey_len);
    put(max_class_len, info.max_class_len);
    put(values, info.values);
    put(max_value_name_len, info.max_value_name_len);
    put(max_value_len, info.max_value_len);
    put(security_len, 0);
    if !last_write.is_null() {
        // SAFETY: the caller's optional FILETIME.
        unsafe { last_write.write(info.last_write) };
    }
    if !class.is_null() {
        // SAFETY: checked above; `class` holds `*class_len` units.
        if let Err(e) = unsafe { give_name(&info.class, class, class_len) } {
            // Learn: the variable then holds the class's length.
            put(class_len, info.class.len() as u32);
            return e;
        }
    } else {
        put(class_len, info.class.len() as u32);
    }
    ERROR_SUCCESS
}

//! `ITaskService`, `ITaskFolder`, `IRegisteredTask`, `IRunningTask` and their collections.
//!
//! The service object connects to the scheduler process; a folder or task object holds the path
//! text it was asked with (its `Name` and `Path` are that text) and asks the scheduler on every
//! call, so the object of a deleted task still answers its name but no longer its state
//! (sheets `ITaskFolder.GetTask`, `ITaskService.GetFolder`). Collections are snapshots of names;
//! their items are live objects.

use crate::client::call;
use crate::defn::{definition_of, new_definition};
use crate::*;
use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_void;
use poly_taskschd::parse::parse_task;
use poly_taskschd::path::{child_text, last, same};
use poly_taskschd::wire::{InstanceInfo, Request};

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetComputerNameW(buf: *mut u16, n: *mut u32) -> i32;
    fn GetCurrentProcess() -> isize;
    fn CloseHandle(h: isize) -> i32;
    fn LocalFree(p: *mut c_void) -> *mut c_void;
}

#[cfg_attr(target_arch = "x86", link(name = "advapi32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "advapi32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetUserNameW(buf: *mut u16, n: *mut u32) -> i32;
    fn OpenProcessToken(process: isize, access: u32, token: *mut isize) -> i32;
    fn GetTokenInformation(token: isize, class: u32, buf: *mut u8, len: u32, ret: *mut u32) -> i32;
    fn ConvertSidToStringSidW(sid: *mut c_void, out: *mut *mut u16) -> i32;
}

/// Poly's user's SID (`poly_registry::defaults::POLY_USER_SID`), when the token cannot be read.
const POLY_USER_SID: &str = "S-1-5-21-520193847-884726105-1936024571-1002";
/// `TOKEN_QUERY`, `TokenUser`.
const TOKEN_QUERY: u32 = 8;
const TOKEN_USER: u32 = 1;

fn computer_name() -> String {
    let mut b = [0u16; 64];
    let mut n = 63u32;
    // SAFETY: a 64-unit buffer and its size.
    if unsafe { GetComputerNameW(b.as_mut_ptr(), &mut n) } == 0 {
        return String::from("POLY");
    }
    String::from_utf16_lossy(&b[..n as usize])
}

fn user_name() -> String {
    let mut b = [0u16; 128];
    let mut n = 127u32;
    // SAFETY: a 128-unit buffer and its size (which counts the NUL on return).
    if unsafe { GetUserNameW(b.as_mut_ptr(), &mut n) } == 0 || n == 0 {
        return String::from("poly");
    }
    let n = (n as usize).min(127);
    let end = b[..n].iter().position(|&u| u == 0).unwrap_or(n);
    String::from_utf16_lossy(&b[..end])
}

/// The SID of the calling process's user, as text.
fn user_sid() -> String {
    let mut token = 0isize;
    // SAFETY: the pseudo handle of this process; the token is closed below.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return String::from(POLY_USER_SID);
    }
    let mut buf = [0u8; 256];
    let mut len = 0u32;
    // SAFETY: a 256-byte buffer for `TOKEN_USER` (a SID pointer, attributes, the SID).
    let ok = unsafe { GetTokenInformation(token, TOKEN_USER, buf.as_mut_ptr(), buf.len() as u32, &mut len) };
    // SAFETY: our token handle.
    unsafe { CloseHandle(token) };
    if ok == 0 {
        return String::from(POLY_USER_SID);
    }
    // SAFETY: `TOKEN_USER` starts with the SID's address (`SID_AND_ATTRIBUTES`).
    let sid = unsafe { *(buf.as_ptr() as *const *mut c_void) };
    let mut text: *mut u16 = core::ptr::null_mut();
    // SAFETY: a SID in `buf`; the text is freed below.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 || text.is_null() {
        return String::from(POLY_USER_SID);
    }
    let mut n = 0;
    // SAFETY: a NUL-terminated string the call allocated.
    while unsafe { *text.add(n) } != 0 && n < 256 {
        n += 1;
    }
    // SAFETY: as above.
    let s = String::from_utf16_lossy(unsafe { core::slice::from_raw_parts(text, n) });
    // SAFETY: allocated by ConvertSidToStringSidW with LocalAlloc.
    unsafe { LocalFree(text as *mut c_void) };
    s
}

// ------------------------------------------------------------------------------- the service

struct ServiceObj {
    /// The target server while connected.
    target: Lock<Option<String>>,
}

pub fn new_service() -> *mut c_void {
    make(SERVICE_VTBL.ptr(), SERVICE_IIDS, ServiceObj { target: Lock::new(None) })
}

fn connected(this: *mut c_void) -> bool {
    // SAFETY: a service object.
    unsafe { data::<ServiceObj>(this) }.target.with(|t| t.is_some())
}

unsafe extern "system" fn sv_get_folder(this: *mut c_void, path: Bstr, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = core::ptr::null_mut() };
    if !connected(this) {
        return NOT_CONNECTED;
    }
    let p = from_bstr(path).filter(|p| !p.is_empty()).unwrap_or_else(|| String::from("\\"));
    let r = call(&Request::GetFolder { folder: String::from("\\"), rel: p.clone() });
    if r.hr < 0 {
        return r.hr;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = folder_object(p) };
    S_OK
}

unsafe extern "system" fn sv_running(this: *mut c_void, flags: i32, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = core::ptr::null_mut() };
    if !connected(this) {
        return NOT_CONNECTED;
    }
    let r = call(&Request::Running { flags });
    if r.hr < 0 {
        return r.hr;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = make(RUNCOLL_VTBL.ptr(), RUNCOLL_IIDS, RunColl { items: r.instances }) };
    S_OK
}

unsafe extern "system" fn sv_new_task(_this: *mut c_void, flags: u32, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = core::ptr::null_mut() };
    if flags != 0 {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = new_definition(poly_taskschd::def::Definition::default()) };
    S_OK
}

/// Whether `s` names this machine (sheet `ITaskService.Connect`: the computer's name, `localhost`,
/// `127.0.0.1`, an empty name).
fn local(s: &str) -> bool {
    let name = computer_name();
    let s = s.strip_prefix("\\\\").unwrap_or(s);
    s.is_empty() || same(s, "localhost") || s == "127.0.0.1" || same(s, &name)
}

unsafe extern "system" fn sv_connect(this: *mut c_void, server: Variant, _user: Variant, _domain: Variant, _pw: Variant) -> HRESULT {
    // SAFETY: a service object.
    let o = unsafe { data::<ServiceObj>(this) };
    let target = if server.is_empty() {
        computer_name()
    } else {
        match server.text() {
            Some(None) => String::new(),
            Some(Some(s)) if local(&s) => s,
            _ => return BAD_NETPATH,
        }
    };
    let r = call(&Request::Ping);
    if r.hr < 0 {
        return r.hr;
    }
    o.target.with(|t| *t = Some(target));
    S_OK
}

unsafe extern "system" fn sv_get_connected(this: *mut c_void, out: *mut i16) -> HRESULT {
    // SAFETY: the caller's out parameter.
    unsafe { put_bool(out, if connected(this) { -1 } else { 0 }) }
}

unsafe extern "system" fn sv_get_target(this: *mut c_void, out: *mut Bstr) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: a service object.
    match unsafe { data::<ServiceObj>(this) }.target.with(|t| t.clone()) {
        // SAFETY: the caller's out parameter.
        Some(t) => unsafe { put_text(out, &Some(t)) },
        None => NOT_CONNECTED,
    }
}

unsafe extern "system" fn sv_get_user(this: *mut c_void, out: *mut Bstr) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    if !connected(this) {
        return NOT_CONNECTED;
    }
    // SAFETY: the caller's out parameter.
    unsafe { put_text(out, &Some(user_name())) }
}

unsafe extern "system" fn sv_get_domain(this: *mut c_void, out: *mut Bstr) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    if !connected(this) {
        return NOT_CONNECTED;
    }
    // SAFETY: the caller's out parameter.
    unsafe { put_text(out, &Some(computer_name())) }
}

unsafe extern "system" fn sv_get_version(this: *mut c_void, out: *mut u32) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    if !connected(this) {
        return NOT_CONNECTED;
    }
    // Task Scheduler 1.6 (sheet `ITaskService.HighestVersion`).
    // SAFETY: the caller's out parameter.
    unsafe { put_val(out, 0x0001_0006u32) }
}

static SERVICE_VTBL: Vt<16> = dual![
    sv_get_folder,
    sv_running,
    sv_new_task,
    sv_connect,
    sv_get_connected,
    sv_get_target,
    sv_get_user,
    sv_get_domain,
    sv_get_version,
];
static SERVICE_IIDS: &[Guid] = &[IID_ITASKSERVICE];

// ------------------------------------------------------------------------------- folders

struct FolderObj {
    path: String,
}

fn folder_object(path: String) -> *mut c_void {
    make(FOLDER_VTBL.ptr(), FOLDER_IIDS, FolderObj { path })
}

fn folder<'a>(this: *mut c_void) -> &'a FolderObj {
    // SAFETY: a folder object.
    unsafe { data::<FolderObj>(this) }
}

unsafe extern "system" fn fo_get_name(this: *mut c_void, out: *mut Bstr) -> HRESULT {
    // SAFETY: the caller's out parameter.
    unsafe { put_text(out, &Some(last(&folder(this).path))) }
}

unsafe extern "system" fn fo_get_path(this: *mut c_void, out: *mut Bstr) -> HRESULT {
    // SAFETY: the caller's out parameter.
    unsafe { put_text(out, &Some(folder(this).path.clone())) }
}

unsafe extern "system" fn fo_get_folder(this: *mut c_void, rel: Bstr, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = core::ptr::null_mut() };
    let f = folder(this);
    let rel = from_bstr(rel).unwrap_or_default();
    let r = call(&Request::GetFolder { folder: f.path.clone(), rel: rel.clone() });
    if r.hr < 0 {
        return r.hr;
    }
    let p = if rel.is_empty() || rel == "\\" { f.path.clone() } else { child_text(&f.path, &rel) };
    // SAFETY: the caller's out parameter.
    unsafe { *out = folder_object(p) };
    S_OK
}

unsafe extern "system" fn fo_get_folders(this: *mut c_void, flags: i32, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = core::ptr::null_mut() };
    let f = folder(this);
    let r = call(&Request::Folders { folder: f.path.clone(), flags });
    if r.hr < 0 {
        return r.hr;
    }
    let c = NameColl { base: f.path.clone(), names: r.names };
    // SAFETY: the caller's out parameter.
    unsafe { *out = make(FOLDERCOLL_VTBL.ptr(), FOLDERCOLL_IIDS, c) };
    S_OK
}

unsafe extern "system" fn fo_create_folder(this: *mut c_void, rel: Bstr, sddl: Variant, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: the caller's out parameter.
    unsafe { clear(out) };
    let f = folder(this);
    let rel = from_bstr(rel);
    let sddl = sddl.opt_text();
    let r = call(&Request::CreateFolder { folder: f.path.clone(), rel: rel.clone(), sddl });
    if r.hr < 0 {
        return r.hr;
    }
    let p = child_text(&f.path, &rel.unwrap_or_default());
    // SAFETY: our new object, handed out or released.
    unsafe { hand_out(out, folder_object(p)) };
    S_OK
}

unsafe extern "system" fn fo_delete_folder(this: *mut c_void, rel: Bstr, _flags: i32) -> HRESULT {
    let f = folder(this);
    call(&Request::DeleteFolder { folder: f.path.clone(), rel: from_bstr(rel) }).hr
}

unsafe extern "system" fn fo_get_task(this: *mut c_void, rel: Bstr, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = core::ptr::null_mut() };
    let Some(rel) = from_bstr(rel) else { return E_INVALIDARG };
    let f = folder(this);
    let r = call(&Request::GetTask { folder: f.path.clone(), rel: rel.clone() });
    if r.hr < 0 {
        return r.hr;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = task_object(child_text(&f.path, &rel)) };
    S_OK
}

unsafe extern "system" fn fo_get_tasks(this: *mut c_void, flags: i32, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = core::ptr::null_mut() };
    let f = folder(this);
    let r = call(&Request::Tasks { folder: f.path.clone(), flags });
    if r.hr < 0 {
        return r.hr;
    }
    let c = NameColl { base: f.path.clone(), names: r.names };
    // SAFETY: the caller's out parameter.
    unsafe { *out = make(TASKCOLL_VTBL.ptr(), TASKCOLL_IIDS, c) };
    S_OK
}

unsafe extern "system" fn fo_delete_task(this: *mut c_void, rel: Bstr, flags: i32) -> HRESULT {
    let f = folder(this);
    call(&Request::DeleteTask { folder: f.path.clone(), rel: from_bstr(rel), flags }).hr
}

/// What both registration calls share: send the definition and hand out the task.
#[allow(clippy::too_many_arguments)]
unsafe fn register(
    this: *mut c_void,
    path: Bstr,
    def: poly_taskschd::def::Definition,
    from_xml: bool,
    flags: i32,
    user: Variant,
    logon: i32,
    sddl: Variant,
    out: *mut *mut c_void,
) -> HRESULT {
    let f = folder(this);
    let typed = from_bstr(path);
    let r = call(&Request::Register {
        folder: f.path.clone(),
        def,
        path: typed.clone(),
        flags,
        user: user.opt_text(),
        logon_type: logon,
        sddl: sddl.opt_text(),
        from_xml,
        caller_name: alloc::format!("{}\\{}", computer_name(), user_name()),
        caller_sid: user_sid(),
    });
    if r.hr < 0 {
        return r.hr;
    }
    let Some(name) = r.names.first() else { return r.hr };
    let p = match typed.as_deref().filter(|t| !t.is_empty()) {
        Some(t) => child_text(&f.path, t),
        None => child_text("\\", name),
    };
    // SAFETY: our new object, handed out or released.
    unsafe { hand_out(out, task_object(p)) };
    r.hr
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn fo_register_xml(
    this: *mut c_void,
    path: Bstr,
    xml: Bstr,
    flags: i32,
    user: Variant,
    _pw: Variant,
    logon: i32,
    sddl: Variant,
    out: *mut *mut c_void,
) -> HRESULT {
    // SAFETY: the caller's out parameter.
    unsafe { clear(out) };
    let def = match parse_task(&from_bstr(xml).unwrap_or_default()) {
        Ok(d) => d,
        Err(hr) => return hr,
    };
    // SAFETY: as the method's own arguments.
    unsafe { register(this, path, def, true, flags, user, logon, sddl, out) }
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn fo_register_def(
    this: *mut c_void,
    path: Bstr,
    def: *mut c_void,
    flags: i32,
    user: Variant,
    _pw: Variant,
    logon: i32,
    sddl: Variant,
    out: *mut *mut c_void,
) -> HRESULT {
    // SAFETY: the caller's out parameter.
    unsafe { clear(out) };
    if def.is_null() {
        return NULL_REF_POINTER;
    }
    let Some(d) = definition_of(def) else { return E_INVALIDARG };
    // SAFETY: as the method's own arguments.
    unsafe { register(this, path, d, false, flags, user, logon, sddl, out) }
}

unsafe extern "system" fn fo_get_sd(_this: *mut c_void, _info: i32, out: *mut Bstr) -> HRESULT {
    // SAFETY: the caller's out parameter.
    unsafe { put_text(out, &Some(String::from("D:(A;;FA;;;BA)(A;;FA;;;SY)"))) }
}

unsafe extern "system" fn fo_set_sd(_this: *mut c_void, _sddl: Bstr, _flags: i32) -> HRESULT {
    S_OK
}

static FOLDER_VTBL: Vt<20> = dual![
    fo_get_name,
    fo_get_path,
    fo_get_folder,
    fo_get_folders,
    fo_create_folder,
    fo_delete_folder,
    fo_get_task,
    fo_get_tasks,
    fo_delete_task,
    fo_register_xml,
    fo_register_def,
    fo_get_sd,
    fo_set_sd,
];
static FOLDER_IIDS: &[Guid] = &[IID_ITASKFOLDER];

// ------------------------------------------------------------------------------- collections of names

struct NameColl {
    base: String,
    names: Vec<String>,
}

unsafe extern "system" fn nc_count(this: *mut c_void, out: *mut i32) -> HRESULT {
    // SAFETY: a collection of names.
    let c = unsafe { data::<NameColl>(this) };
    // SAFETY: the caller's out parameter.
    unsafe { put_val(out, c.names.len() as i32) }
}

/// The name an item argument stands for: a 1-based position or a name (case does not count; the
/// item gets the name as asked).
fn pick(c: &NameColl, v: &Variant) -> Result<String, HRESULT> {
    if let Some(i) = v.int() {
        return if i >= 1 && (i as usize) <= c.names.len() { Ok(c.names[i as usize - 1].clone()) } else { Err(E_INVALIDARG) };
    }
    match v.text() {
        Some(t) => {
            let t = t.unwrap_or_default();
            if c.names.iter().any(|n| same(n, &t)) { Ok(t) } else { Err(PATH_NOT_FOUND) }
        }
        None => Err(E_INVALIDARG),
    }
}

unsafe extern "system" fn tc_item(this: *mut c_void, v: Variant, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = core::ptr::null_mut() };
    // SAFETY: a task collection.
    let c = unsafe { data::<NameColl>(this) };
    match pick(c, &v) {
        Ok(n) => {
            // SAFETY: the caller's out parameter.
            unsafe { *out = task_object(child_text(&c.base, &n)) };
            S_OK
        }
        Err(hr) => hr,
    }
}

unsafe extern "system" fn fc_item(this: *mut c_void, v: Variant, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = core::ptr::null_mut() };
    // SAFETY: a folder collection.
    let c = unsafe { data::<NameColl>(this) };
    match pick(c, &v) {
        Ok(n) => {
            // SAFETY: the caller's out parameter.
            unsafe { *out = folder_object(child_text(&c.base, &n)) };
            S_OK
        }
        Err(hr) => hr,
    }
}

unsafe extern "system" fn tc_enum(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: a task collection.
    let c = unsafe { data::<NameColl>(this) };
    let items = c.names.iter().map(|n| task_object(child_text(&c.base, n))).collect();
    // SAFETY: new objects handed to the enumerator.
    unsafe { new_enum(items, out) }
}

unsafe extern "system" fn fc_enum(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: a folder collection.
    let c = unsafe { data::<NameColl>(this) };
    let items = c.names.iter().map(|n| folder_object(child_text(&c.base, n))).collect();
    // SAFETY: new objects handed to the enumerator.
    unsafe { new_enum(items, out) }
}

static TASKCOLL_VTBL: Vt<10> = dual![nc_count, tc_item, tc_enum];
static TASKCOLL_IIDS: &[Guid] = &[IID_IREGISTEREDTASKCOLLECTION];
static FOLDERCOLL_VTBL: Vt<10> = dual![nc_count, fc_item, fc_enum];
static FOLDERCOLL_IIDS: &[Guid] = &[IID_ITASKFOLDERCOLLECTION];

// ------------------------------------------------------------------------------- registered tasks

struct TaskObj {
    path: String,
}

fn task_object(path: String) -> *mut c_void {
    make(TASK_VTBL.ptr(), TASK_IIDS, TaskObj { path })
}

fn task<'a>(this: *mut c_void) -> &'a TaskObj {
    // SAFETY: a registered task object.
    unsafe { data::<TaskObj>(this) }
}

fn info(this: *mut c_void) -> Result<poly_taskschd::wire::Info, HRESULT> {
    let r = call(&Request::Info { path: task(this).path.clone() });
    if r.hr < 0 {
        return Err(r.hr);
    }
    r.info.ok_or(E_UNEXPECTED)
}

unsafe extern "system" fn rt_get_name(this: *mut c_void, out: *mut Bstr) -> HRESULT {
    // SAFETY: the caller's out parameter.
    unsafe { put_text(out, &Some(last(&task(this).path))) }
}

unsafe extern "system" fn rt_get_path(this: *mut c_void, out: *mut Bstr) -> HRESULT {
    // SAFETY: the caller's out parameter.
    unsafe { put_text(out, &Some(task(this).path.clone())) }
}

unsafe extern "system" fn rt_get_state(this: *mut c_void, out: *mut i32) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    let (hr, v) = match info(this) {
        Ok(i) => (S_OK, i.state),
        Err(hr) => (hr, 0),
    };
    // SAFETY: the caller's out parameter.
    unsafe { *out = v };
    hr
}

unsafe extern "system" fn rt_get_enabled(this: *mut c_void, out: *mut i16) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    match info(this) {
        // SAFETY: the caller's out parameter.
        Ok(i) => unsafe { put_bool(out, if i.enabled { -1 } else { 0 }) },
        Err(hr) => hr,
    }
}

unsafe extern "system" fn rt_put_enabled(this: *mut c_void, v: i16) -> HRESULT {
    call(&Request::SetEnabled { path: task(this).path.clone(), on: v != 0 }).hr
}

unsafe extern "system" fn rt_run(this: *mut c_void, _params: Variant, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: the caller's out parameter.
    unsafe { clear(out) };
    let r = call(&Request::Run { path: task(this).path.clone() });
    if r.hr < 0 {
        return r.hr;
    }
    if let Some(i) = r.instances.into_iter().next() {
        // SAFETY: our new object, handed out or released.
        unsafe { hand_out(out, running_object(i)) };
    }
    S_OK
}

unsafe extern "system" fn rt_run_ex(
    this: *mut c_void,
    params: Variant,
    _flags: i32,
    _session: i32,
    _user: Bstr,
    out: *mut *mut c_void,
) -> HRESULT {
    // SAFETY: as the method's own arguments.
    unsafe { rt_run(this, params, out) }
}

unsafe extern "system" fn rt_instances(this: *mut c_void, _flags: i32, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = core::ptr::null_mut() };
    let r = call(&Request::Instances { path: task(this).path.clone() });
    if r.hr < 0 {
        return r.hr;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = make(RUNCOLL_VTBL.ptr(), RUNCOLL_IIDS, RunColl { items: r.instances }) };
    S_OK
}

unsafe extern "system" fn rt_last_run(this: *mut c_void, out: *mut f64) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    match info(this) {
        // SAFETY: the caller's out parameter.
        Ok(i) => unsafe { put_val(out, i.last_run) },
        Err(hr) => hr,
    }
}

unsafe extern "system" fn rt_last_result(this: *mut c_void, out: *mut i32) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    match info(this) {
        Ok(i) => {
            // SAFETY: the caller's out parameter.
            unsafe { *out = i.last_result };
            // A task that never ran answers with that code itself (sheet `IRegisteredTask.Run`).
            if i.last_result == SCHED_S_TASK_HAS_NOT_RUN { i.last_result } else { S_OK }
        }
        Err(hr) => hr,
    }
}

unsafe extern "system" fn rt_missed(this: *mut c_void, out: *mut i32) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    match info(this) {
        // SAFETY: the caller's out parameter.
        Ok(i) => unsafe { put_val(out, i.missed) },
        Err(hr) => hr,
    }
}

unsafe extern "system" fn rt_next_run(this: *mut c_void, out: *mut f64) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    match info(this) {
        // SAFETY: the caller's out parameter.
        Ok(i) => unsafe { put_val(out, i.next_run) },
        Err(hr) => hr,
    }
}

unsafe extern "system" fn rt_definition(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = core::ptr::null_mut() };
    let r = call(&Request::Definition { path: task(this).path.clone() });
    if r.hr < 0 {
        return r.hr;
    }
    let Some(d) = r.def else { return E_UNEXPECTED };
    // SAFETY: the caller's out parameter.
    unsafe { *out = new_definition(d) };
    S_OK
}

unsafe extern "system" fn rt_xml(this: *mut c_void, out: *mut Bstr) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    let r = call(&Request::Xml { path: task(this).path.clone() });
    if r.hr < 0 {
        return r.hr;
    }
    // SAFETY: the caller's out parameter.
    unsafe { put_text(out, &Some(r.text)) }
}

unsafe extern "system" fn rt_get_sd(this: *mut c_void, _info: i32, out: *mut Bstr) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    let r = call(&Request::SecurityDescriptor { path: task(this).path.clone() });
    if r.hr < 0 {
        return r.hr;
    }
    // SAFETY: the caller's out parameter.
    unsafe { put_text(out, &Some(r.text)) }
}

unsafe extern "system" fn rt_set_sd(_this: *mut c_void, _sddl: Bstr, _flags: i32) -> HRESULT {
    S_OK
}

unsafe extern "system" fn rt_stop(this: *mut c_void, flags: i32) -> HRESULT {
    call(&Request::Stop { path: task(this).path.clone(), flags }).hr
}

unsafe extern "system" fn rt_run_times(_this: *mut c_void, _a: *mut c_void, _b: *mut c_void, _n: *mut u32, _out: *mut c_void) -> HRESULT {
    E_NOTIMPL
}

static TASK_VTBL: Vt<25> = dual![
    rt_get_name,
    rt_get_path,
    rt_get_state,
    rt_get_enabled,
    rt_put_enabled,
    rt_run,
    rt_run_ex,
    rt_instances,
    rt_last_run,
    rt_last_result,
    rt_missed,
    rt_next_run,
    rt_definition,
    rt_xml,
    rt_get_sd,
    rt_set_sd,
    rt_stop,
    rt_run_times,
];
static TASK_IIDS: &[Guid] = &[IID_IREGISTEREDTASK];

// ------------------------------------------------------------------------------- running instances

struct RunObj {
    i: Lock<InstanceInfo>,
}

fn running_object(i: InstanceInfo) -> *mut c_void {
    make(RUN_VTBL.ptr(), RUN_IIDS, RunObj { i: Lock::new(i) })
}

fn run_info(this: *mut c_void) -> InstanceInfo {
    // SAFETY: a running instance object.
    unsafe { data::<RunObj>(this) }.i.with(|i| i.clone())
}

unsafe extern "system" fn ri_name(this: *mut c_void, out: *mut Bstr) -> HRESULT {
    // SAFETY: the caller's out parameter.
    unsafe { put_text(out, &Some(run_info(this).name)) }
}

unsafe extern "system" fn ri_guid(this: *mut c_void, out: *mut Bstr) -> HRESULT {
    // SAFETY: the caller's out parameter.
    unsafe { put_text(out, &Some(run_info(this).guid)) }
}

unsafe extern "system" fn ri_path(this: *mut c_void, out: *mut Bstr) -> HRESULT {
    // SAFETY: the caller's out parameter.
    unsafe { put_text(out, &Some(run_info(this).path)) }
}

unsafe extern "system" fn ri_state(this: *mut c_void, out: *mut i32) -> HRESULT {
    // SAFETY: the caller's out parameter.
    unsafe { put_val(out, run_info(this).state) }
}

unsafe extern "system" fn ri_action(this: *mut c_void, out: *mut Bstr) -> HRESULT {
    // SAFETY: the caller's out parameter.
    unsafe { put_text(out, &Some(run_info(this).action)) }
}

unsafe extern "system" fn ri_stop(this: *mut c_void) -> HRESULT {
    call(&Request::StopInstance { guid: run_info(this).guid }).hr
}

unsafe extern "system" fn ri_refresh(this: *mut c_void) -> HRESULT {
    let i = run_info(this);
    let r = call(&Request::Instances { path: i.path.clone() });
    if r.hr >= 0
        && let Some(now) = r.instances.into_iter().find(|x| x.guid == i.guid)
    {
        // SAFETY: a running instance object.
        unsafe { data::<RunObj>(this) }.i.with(|x| *x = now);
    }
    S_OK
}

unsafe extern "system" fn ri_pid(this: *mut c_void, out: *mut u32) -> HRESULT {
    // SAFETY: the caller's out parameter.
    unsafe { put_val(out, run_info(this).pid) }
}

static RUN_VTBL: Vt<15> = dual![ri_name, ri_guid, ri_path, ri_state, ri_action, ri_stop, ri_refresh, ri_pid];
static RUN_IIDS: &[Guid] = &[IID_IRUNNINGTASK];

struct RunColl {
    items: Vec<InstanceInfo>,
}

unsafe extern "system" fn rc_count(this: *mut c_void, out: *mut i32) -> HRESULT {
    // SAFETY: a collection of instances.
    let c = unsafe { data::<RunColl>(this) };
    // SAFETY: the caller's out parameter.
    unsafe { put_val(out, c.items.len() as i32) }
}

unsafe extern "system" fn rc_item(this: *mut c_void, v: Variant, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = core::ptr::null_mut() };
    // SAFETY: a collection of instances.
    let c = unsafe { data::<RunColl>(this) };
    match v.int() {
        Some(i) if i >= 1 && (i as usize) <= c.items.len() => {
            // SAFETY: the caller's out parameter.
            unsafe { *out = running_object(c.items[i as usize - 1].clone()) };
            S_OK
        }
        _ => E_INVALIDARG,
    }
}

unsafe extern "system" fn rc_enum(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: a collection of instances.
    let c = unsafe { data::<RunColl>(this) };
    let items = c.items.iter().cloned().map(running_object).collect();
    // SAFETY: new objects handed to the enumerator.
    unsafe { new_enum(items, out) }
}

static RUNCOLL_VTBL: Vt<10> = dual![rc_count, rc_item, rc_enum];
static RUNCOLL_IIDS: &[Guid] = &[IID_IRUNNINGTASKCOLLECTION];

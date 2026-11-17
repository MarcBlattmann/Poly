//! The Common Item Dialog's Save object (`CLSID_FileSaveDialog`, `IFileSaveDialog`), as
//! Chrome's "Save as" and its "ask where to save" downloads use it (ADR 0223). The object
//! keeps what its setters were given ([`poly_comdlg::itemdlg::State`]); `Show` runs Poly's
//! one file dialog (`file::run`, the dialog `GetSaveFileNameW` shows) with those settings and
//! keeps the path the user chose for `GetResult`. Poly's dialog looks like Poly's; what the
//! object answers is the sheet's: `specs/windows/comdlg32.dll/IFileSaveDialog.SetFileName.yaml`.

use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};
use poly_comdlg::itemdlg::{self as st, FileType, State};

type Hresult = i32;

/// A `GUID` (`guiddef.h`).
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

const fn guid(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> Guid {
    Guid { data1, data2, data3, data4 }
}

// `unknwn.h`, `shobjidl_core.h`.
const IID_IUNKNOWN: Guid = guid(0, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_ICLASS_FACTORY: Guid = guid(1, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IMODAL_WINDOW: Guid = guid(0xB4DB_1657, 0x70D7, 0x485E, [0x8E, 0x3E, 0x6F, 0xCB, 0x5A, 0x5C, 0x18, 0x02]);
const IID_IFILE_DIALOG: Guid = guid(0x42F8_5136, 0xDB7E, 0x439C, [0x85, 0xF1, 0xE4, 0x07, 0x5D, 0x13, 0x5F, 0xC8]);
const IID_IFILE_SAVE_DIALOG: Guid = guid(0x84BC_CD23, 0x5FDE, 0x4CDB, [0xAE, 0xA4, 0xAF, 0x64, 0xB8, 0x3D, 0x78, 0xAB]);
const IID_ISHELL_ITEM: Guid = guid(0x4382_6D1E, 0xE718, 0x42EE, [0xBC, 0x55, 0xA1, 0xE2, 0x61, 0xC3, 0x7B, 0xFE]);
const CLSID_FILE_SAVE_DIALOG: Guid = guid(0xC0B4_E2F3, 0xBA21, 0x4773, [0x8D, 0xBA, 0x33, 0x5E, 0xC9, 0x46, 0xEB, 0x8B]);

const S_OK: Hresult = 0;
const E_NOINTERFACE: Hresult = 0x8000_4002_u32 as i32;
const E_POINTER: Hresult = 0x8000_4003_u32 as i32;
const E_OUTOFMEMORY: Hresult = 0x8007_000E_u32 as i32;
const CLASS_E_NOAGGREGATION: Hresult = 0x8004_0110_u32 as i32;
const CLASS_E_CLASSNOTAVAILABLE: Hresult = 0x8004_0111_u32 as i32;
const E_FAIL: Hresult = st::E_FAIL as i32;
const E_UNEXPECTED: Hresult = st::E_UNEXPECTED as i32;
const E_INVALIDARG: Hresult = st::E_INVALIDARG as i32;
/// `SIGDN_FILESYSPATH` (`shobjidl_core.h`).
const SIGDN_FILESYSPATH: u32 = 0x8005_8000;
/// `CSIDL_PERSONAL` (`shlobj_core.h`): the Documents folder, where a dialog with no folder set opens.
const CSIDL_PERSONAL: i32 = 5;
const MAX_PATH: usize = 260;
/// The file name buffer `Show` gives Poly's dialog, in characters.
const FILE_BUFFER: usize = 32768;

#[cfg_attr(target_arch = "x86", link(name = "ole32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ole32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn CoTaskMemAlloc(bytes: usize) -> *mut u8;
    fn CoTaskMemFree(p: *mut u8);
}

#[cfg_attr(target_arch = "x86", link(name = "shell32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "shell32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn ILCreateFromPathW(path: *const u16) -> *mut u8;
    fn ILFree(pidl: *mut u8);
    fn SHCreateItemFromIDList(pidl: *const u8, iid: *const Guid, out: *mut *mut c_void) -> Hresult;
    fn SHGetFolderPathW(hwnd: usize, csidl: i32, token: usize, flags: u32, path: *mut u16) -> Hresult;
}

/// A method table: `N` function pointers.
#[repr(transparent)]
struct Vtbl<const N: usize>([*const c_void; N]);
// SAFETY: function pointers only, never written.
unsafe impl<const N: usize> Sync for Vtbl<N> {}

#[repr(C)]
struct Dialog {
    vtbl: *const c_void,
    refs: AtomicU32,
    /// Used from the thread of the object's apartment only.
    state: UnsafeCell<State>,
}

unsafe fn state<'a>(p: *mut c_void) -> &'a mut State {
    // SAFETY: our object, used by one thread at a time.
    unsafe { &mut *(*p.cast::<Dialog>()).state.get() }
}

unsafe extern "system" fn query_interface(p: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
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
        if ![IID_IUNKNOWN, IID_IMODAL_WINDOW, IID_IFILE_DIALOG, IID_IFILE_SAVE_DIALOG].contains(&iid) {
            return E_NOINTERFACE;
        }
        add_ref(p);
        *out = p;
    }
    S_OK
}

unsafe extern "system" fn add_ref(p: *mut c_void) -> u32 {
    // SAFETY: our object.
    unsafe { (*p.cast::<Dialog>()).refs.fetch_add(1, Ordering::AcqRel) + 1 }
}

unsafe extern "system" fn release(p: *mut c_void) -> u32 {
    // SAFETY: our object; freed with its last reference.
    unsafe {
        let left = (*p.cast::<Dialog>()).refs.fetch_sub(1, Ordering::AcqRel) - 1;
        if left == 0 {
            drop(alloc::boxed::Box::from_raw(p.cast::<Dialog>()));
        }
        left
    }
}

/// A NUL-terminated string of the caller, without the NUL (`None` for NULL).
unsafe fn text(p: *const u16) -> Option<Vec<u16>> {
    if p.is_null() {
        return None;
    }
    Some(crate::util::read_wstr(p, FILE_BUFFER))
}

/// The file-system path of a shell item (its `GetDisplayName(SIGDN_FILESYSPATH)`, slot 5).
unsafe fn item_path(item: *mut c_void) -> Option<Vec<u16>> {
    if item.is_null() {
        return None;
    }
    // SAFETY: a live IShellItem; the string is task memory.
    unsafe {
        let vtbl = *item.cast::<*const usize>();
        let f: unsafe extern "system" fn(*mut c_void, u32, *mut *mut u16) -> Hresult = core::mem::transmute(*vtbl.add(5));
        let mut s: *mut u16 = core::ptr::null_mut();
        if f(item, SIGDN_FILESYSPATH, &mut s) != S_OK || s.is_null() {
            return None;
        }
        let path = crate::util::read_wstr(s, FILE_BUFFER);
        CoTaskMemFree(s.cast());
        Some(path)
    }
}

/// A shell item of `path` (which need not exist yet: the result of a Save dialog).
unsafe fn item_of(path: &[u16], out: *mut *mut c_void) -> Hresult {
    let mut name = path.to_vec();
    name.push(0);
    // SAFETY: a NUL-terminated path; the list is freed after the item is made.
    unsafe {
        let pidl = ILCreateFromPathW(name.as_ptr());
        if pidl.is_null() {
            return E_OUTOFMEMORY;
        }
        let hr = SHCreateItemFromIDList(pidl, &IID_ISHELL_ITEM, out);
        ILFree(pidl);
        hr
    }
}

/// The Documents folder (or `C:\`), where a dialog opens that was given no folder.
fn documents() -> Vec<u16> {
    let mut buf = [0u16; MAX_PATH];
    // SAFETY: a MAX_PATH buffer.
    if unsafe { SHGetFolderPathW(0, CSIDL_PERSONAL, 0, 0, buf.as_mut_ptr()) } == S_OK {
        let n = buf.iter().position(|&c| c == 0).unwrap_or(MAX_PATH);
        return buf[..n].to_vec();
    }
    "C:\\".encode_utf16().collect()
}

/// `OPENFILENAMEW` (`commdlg.h`): 152 bytes on x64.
#[repr(C)]
struct Ofn {
    size: u32,
    owner: usize,
    instance: usize,
    filter: *const u16,
    custom_filter: *mut u16,
    max_custom: u32,
    filter_index: u32,
    file: *mut u16,
    max_file: u32,
    file_title: *mut u16,
    max_file_title: u32,
    initial_dir: *const u16,
    title: *const u16,
    flags: u32,
    file_offset: u16,
    file_extension: u16,
    def_ext: *const u16,
    cust_data: isize,
    hook: usize,
    template: *const u16,
    reserved: usize,
    reserved2: u32,
    flags_ex: u32,
}

fn nul(v: &[u16]) -> Vec<u16> {
    let mut o = v.to_vec();
    o.push(0);
    o
}

#[cfg_attr(target_arch = "x86", link(name = "user32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "user32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetWindowThreadProcessId(hwnd: usize, pid: *mut u32) -> u32;
}

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetCurrentProcessId() -> u32;
}

/// The owner Poly's dialog gets: the caller's window if it is one of this process's; a window of
/// another process (Chrome shows the dialog from a utility process, owned by the browser's
/// window) is left out, so the dialog does not wait on that process.
fn own_owner(owner: usize) -> usize {
    let mut pid = 0u32;
    // SAFETY: a window handle query and a local.
    let same = unsafe { GetWindowThreadProcessId(owner, &mut pid) != 0 && pid == GetCurrentProcessId() };
    if same { owner } else { 0 }
}

/// `IModalWindow::Show(hwndOwner)`: Poly's file dialog with the object's settings; the chosen
/// path is kept for `GetResult`. Cancel: `HRESULT_FROM_WIN32(ERROR_CANCELLED)`.
unsafe extern "system" fn show(p: *mut c_void, owner: usize) -> Hresult {
    // SAFETY: our object.
    let s = unsafe { state(p) };
    poly_win32_rt::unimplemented_export_error("comdlg32: the Save dialog of IFileSaveDialog is shown", 0, 0);
    let fallback = documents();
    let folder = nul(s.start_folder(&fallback));
    let filter = s.filter();
    let def_ext = s.ofn_default_ext().map(|e| nul(&e));
    let title = s.title.as_ref().map(|t| nul(t));
    let mut file = alloc::vec![0u16; FILE_BUFFER];
    if let Some(n) = &s.name {
        let n = &n[..n.len().min(FILE_BUFFER - 1)];
        file[..n.len()].copy_from_slice(n);
    }
    let mut ofn = Ofn {
        size: core::mem::size_of::<Ofn>() as u32,
        owner: own_owner(owner),
        instance: 0,
        filter: filter.as_ref().map_or(core::ptr::null(), |f| f.as_ptr()),
        custom_filter: core::ptr::null_mut(),
        max_custom: 0,
        filter_index: s.type_index.max(1),
        file: file.as_mut_ptr(),
        max_file: FILE_BUFFER as u32,
        file_title: core::ptr::null_mut(),
        max_file_title: 0,
        initial_dir: folder.as_ptr(),
        title: title.as_ref().map_or(core::ptr::null(), |t| t.as_ptr()),
        flags: s.ofn_flags(),
        file_offset: 0,
        file_extension: 0,
        def_ext: def_ext.as_ref().map_or(core::ptr::null(), |e| e.as_ptr()),
        cust_data: 0,
        hook: 0,
        template: core::ptr::null(),
        reserved: 0,
        reserved2: 0,
        flags_ex: 0,
    };
    // A complete OPENFILENAMEW whose buffers live until the dialog returns.
    let ok = crate::file::run((&mut ofn as *mut Ofn).cast(), true);
    if ok == 0 {
        let line = alloc::format!(
            "comdlg32: the Save dialog of IFileSaveDialog was cancelled (error {:#x})",
            crate::CommDlgExtendedError()
        );
        poly_win32_rt::unimplemented_export_error(&line, 0, 0);
        s.result = None;
        return st::CANCELLED as i32;
    }
    let n = file.iter().position(|&c| c == 0).unwrap_or(FILE_BUFFER);
    poly_win32_rt::unimplemented_export_error("comdlg32: the Save dialog of IFileSaveDialog was answered", 0, 0);
    s.result = Some(file[..n].to_vec());
    if s.types.is_some() {
        s.type_index = ofn.filter_index;
    }
    S_OK
}

unsafe extern "system" fn set_file_types(p: *mut c_void, count: u32, specs: *const [*const u16; 2]) -> Hresult {
    // SAFETY: our object; `count` `COMDLG_FILTERSPEC`s of the caller.
    unsafe {
        let types = (!specs.is_null() && count > 0).then(|| {
            (0..count as usize)
                .map(|i| {
                    let [name, spec] = *specs.add(i);
                    FileType { name: text(name).unwrap_or_default(), spec: text(spec).unwrap_or_default() }
                })
                .collect()
        });
        state(p).set_file_types(types) as Hresult
    }
}

unsafe extern "system" fn set_file_type_index(p: *mut c_void, index: u32) -> Hresult {
    // SAFETY: our object.
    unsafe { state(p).set_type_index(index) as Hresult }
}

unsafe extern "system" fn get_file_type_index(p: *mut c_void, out: *mut u32) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: our object; the caller's out parameter.
    unsafe { *out = state(p).type_index };
    S_OK
}

unsafe extern "system" fn advise(_: *mut c_void, sink: *mut c_void, cookie: *mut u32) -> Hresult {
    if sink.is_null() || cookie.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's out parameter. Poly's dialog sends no events to it.
    unsafe { *cookie = 1 };
    S_OK
}

unsafe extern "system" fn unadvise(_: *mut c_void, cookie: u32) -> Hresult {
    if cookie == 0 { E_INVALIDARG } else { S_OK }
}

unsafe extern "system" fn set_options(p: *mut c_void, options: u32) -> Hresult {
    // SAFETY: our object.
    unsafe { state(p).set_options(options) as Hresult }
}

unsafe extern "system" fn get_options(p: *mut c_void, out: *mut u32) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: our object; the caller's out parameter.
    unsafe { *out = state(p).options };
    S_OK
}

unsafe extern "system" fn set_default_folder(p: *mut c_void, item: *mut c_void) -> Hresult {
    // SAFETY: our object; the caller's item.
    unsafe {
        let Some(path) = item_path(item) else { return E_INVALIDARG };
        state(p).default_folder = Some(path);
    }
    S_OK
}

unsafe extern "system" fn set_folder(p: *mut c_void, item: *mut c_void) -> Hresult {
    // SAFETY: our object; the caller's item.
    unsafe {
        let Some(path) = item_path(item) else { return E_INVALIDARG };
        state(p).folder = Some(path);
    }
    S_OK
}

unsafe extern "system" fn get_folder(p: *mut c_void, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: our object; the caller's out parameter.
    unsafe {
        *out = core::ptr::null_mut();
        let fallback = documents();
        let folder = state(p).start_folder(&fallback).to_vec();
        item_of(&folder, out)
    }
}

/// `GetResult`: the chosen file after `Show` (`E_UNEXPECTED` before).
unsafe extern "system" fn get_result(p: *mut c_void, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: our object; the caller's out parameter.
    unsafe {
        *out = core::ptr::null_mut();
        match state(p).result.clone() {
            Some(path) => item_of(&path, out),
            None => E_UNEXPECTED,
        }
    }
}

/// `GetCurrentSelection`: only while the dialog is shown (never, seen from the caller: Poly's
/// dialog sends no events), else the chosen file; `E_FAIL` before.
unsafe extern "system" fn get_current_selection(p: *mut c_void, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: our object; the caller's out parameter.
    unsafe {
        *out = core::ptr::null_mut();
        match state(p).result.clone() {
            Some(path) => item_of(&path, out),
            None => E_FAIL,
        }
    }
}

unsafe extern "system" fn set_file_name(p: *mut c_void, name: *const u16) -> Hresult {
    // SAFETY: our object; the caller's string.
    unsafe {
        let Some(name) = text(name) else { return E_INVALIDARG };
        state(p).set_file_name(&name) as Hresult
    }
}

unsafe extern "system" fn get_file_name(p: *mut c_void, out: *mut *mut u16) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: our object; the caller's out parameter gets task memory.
    unsafe {
        let (hr, name) = state(p).file_name();
        let block = CoTaskMemAlloc(2 * (name.len() + 1)).cast::<u16>();
        if block.is_null() {
            *out = core::ptr::null_mut();
            return E_OUTOFMEMORY;
        }
        core::ptr::copy_nonoverlapping(name.as_ptr(), block, name.len());
        *block.add(name.len()) = 0;
        *out = block;
        hr as Hresult
    }
}

unsafe extern "system" fn set_title(p: *mut c_void, title: *const u16) -> Hresult {
    // SAFETY: our object; the caller's string.
    unsafe { state(p).title = text(title) };
    S_OK
}

unsafe extern "system" fn set_default_extension(p: *mut c_void, ext: *const u16) -> Hresult {
    // SAFETY: our object; the caller's string.
    unsafe { state(p).default_ext = text(ext) };
    S_OK
}

/// `SetSaveAsItem`: the dialog opens in the item's folder with its name.
unsafe extern "system" fn set_save_as_item(p: *mut c_void, item: *mut c_void) -> Hresult {
    // SAFETY: our object; the caller's item.
    unsafe {
        let Some(path) = item_path(item) else { return E_INVALIDARG };
        let s = state(p);
        let cut = path.iter().rposition(|&c| c == u16::from(b'\\'));
        if let Some(i) = cut {
            s.folder = Some(path[..i.max(3).min(path.len())].to_vec());
            s.name = Some(path[i + 1..].to_vec());
        }
    }
    S_OK
}

unsafe extern "system" fn ok_1(_: *mut c_void, _: usize) -> Hresult {
    S_OK
}
unsafe extern "system" fn ok_2(_: *mut c_void, _: usize, _: usize) -> Hresult {
    S_OK
}
unsafe extern "system" fn ok_4(_: *mut c_void, _: usize, _: usize, _: usize, _: usize) -> Hresult {
    S_OK
}
unsafe extern "system" fn fail_0(_: *mut c_void) -> Hresult {
    E_FAIL
}
unsafe extern "system" fn get_properties(_: *mut c_void, out: *mut *mut c_void) -> Hresult {
    if !out.is_null() {
        // SAFETY: the caller's out parameter.
        unsafe { *out = core::ptr::null_mut() };
    }
    E_FAIL
}

/// `IModalWindow::Show` 3; `IFileDialog`: `SetFileTypes` 4 .. `SetFilter` 26; `IFileSaveDialog`:
/// `SetSaveAsItem` 27, `SetProperties` 28, `SetCollectedProperties` 29, `GetProperties` 30,
/// `ApplyProperties` 31.
static VTBL: Vtbl<32> = Vtbl([
    query_interface as *const c_void,
    add_ref as *const c_void,
    release as *const c_void,
    show as *const c_void,
    set_file_types as *const c_void,
    set_file_type_index as *const c_void,
    get_file_type_index as *const c_void,
    advise as *const c_void,
    unadvise as *const c_void,
    set_options as *const c_void,
    get_options as *const c_void,
    set_default_folder as *const c_void,
    set_folder as *const c_void,
    get_folder as *const c_void,
    get_current_selection as *const c_void,
    set_file_name as *const c_void,
    get_file_name as *const c_void,
    set_title as *const c_void,
    ok_1 as *const c_void, // SetOkButtonLabel
    ok_1 as *const c_void, // SetFileNameLabel
    get_result as *const c_void,
    ok_2 as *const c_void, // AddPlace(item, place)
    set_default_extension as *const c_void,
    ok_1 as *const c_void, // Close(hr)
    ok_1 as *const c_void, // SetClientGuid(guid)
    fail_0 as *const c_void, // ClearClientData: E_FAIL (the sheet)
    ok_1 as *const c_void, // SetFilter(filter)
    set_save_as_item as *const c_void,
    ok_1 as *const c_void, // SetProperties(store)
    ok_2 as *const c_void, // SetCollectedProperties(list, append)
    get_properties as *const c_void,
    ok_4 as *const c_void, // ApplyProperties(item, store, hwnd, sink)
]);

// ----- the class object -----

/// `IClassFactory`: `CreateInstance` 3, `LockServer` 4.
static FACTORY_VTBL: Vtbl<5> = Vtbl([
    factory_query_interface as *const c_void,
    factory_count as *const c_void,
    factory_count as *const c_void,
    factory_create as *const c_void,
    factory_lock as *const c_void,
]);

#[repr(C)]
struct Factory {
    vtbl: *const c_void,
}
// SAFETY: immutable.
unsafe impl Sync for Factory {}
static FACTORY: Factory = Factory { vtbl: (&FACTORY_VTBL as *const Vtbl<5>).cast() };

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

/// The class object lives for ever: `AddRef` and `Release` count nothing.
unsafe extern "system" fn factory_count(_: *mut c_void) -> u32 {
    1
}

unsafe extern "system" fn factory_lock(_: *mut c_void, _: i32) -> Hresult {
    S_OK
}

unsafe extern "system" fn factory_create(
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
        let object = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(Dialog {
            vtbl: (&VTBL as *const Vtbl<32>).cast(),
            refs: AtomicU32::new(1),
            state: UnsafeCell::new(State::new_save()),
        }))
        .cast::<c_void>();
        let hr = query_interface(object, iid, out);
        release(object);
        hr
    }
}

/// `DllGetClassObject(rclsid, riid, ppv)`: the class object of `CLSID_FileSaveDialog`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(clsid: *const Guid, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's pointers.
    unsafe {
        *out = core::ptr::null_mut();
        if clsid.is_null() || clsid.read_unaligned() != CLSID_FILE_SAVE_DIALOG {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        factory_query_interface((&FACTORY as *const Factory).cast_mut().cast(), iid, out)
    }
}

/// `DllCanUnloadNow`: the DLL stays (`S_FALSE`).
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> Hresult {
    1
}

//! The shell link object, `CLSID_ShellLink` (sheets `ShellLink.yaml`, `IShellLinkW.yaml`,
//! `ShellLink.IPersistFile.yaml`, `ShellLink.PropertyStore.yaml`): the `.lnk` files installers
//! write. The link's data and every rule are `poly_lnk`'s; this file holds the COM tables, turns
//! the callers' buffers into slices and back, and answers `poly_lnk::System` with Win32 calls.
//!
//! One object answers many interfaces, each with its own pointer (the sheet's `identity`: the
//! `IUnknown` pointer is none of the others). The object is an array of table pointers followed
//! by the count and the link; an interface pointer is the address of its table pointer, so the
//! object is found by stepping back by the interface's index. The interfaces beyond the ones the
//! sheets describe (the shell extension interfaces, `IShellLinkDataList`, `IMarshal`) are answered
//! by `QueryInterface` as on Windows, and their methods return `E_NOTIMPL`.

use crate::com::*;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};
use poly_lnk::{FileInfo, Link, PropKey, PropValue, System, Text, hr, idlist, props, text};

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetEnvironmentVariableW(name: *const u16, buf: *mut u16, size: u32) -> u32;
    fn GetFileAttributesW(name: *const u16) -> u32;
    fn GetFileAttributesExW(name: *const u16, level: u32, data: *mut u8) -> i32;
    fn FindFirstFileW(name: *const u16, data: *mut u8) -> usize;
    fn FindClose(find: usize) -> i32;
    fn GetVolumeInformationW(
        root: *const u16,
        name: *mut u16,
        name_size: u32,
        serial: *mut u32,
        max_component: *mut u32,
        flags: *mut u32,
        fs_name: *mut u16,
        fs_size: u32,
    ) -> i32;
    fn GetComputerNameW(buf: *mut u16, size: *mut u32) -> i32;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        sa: *mut c_void,
        disposition: u32,
        flags: u32,
        template: usize,
    ) -> usize;
    fn ReadFile(file: usize, buf: *mut u8, size: u32, read: *mut u32, overlapped: *mut u8) -> i32;
    fn WriteFile(file: usize, buf: *const u8, size: u32, written: *mut u32, overlapped: *mut u8) -> i32;
    fn GetFileSizeEx(file: usize, size: *mut i64) -> i32;
    fn CloseHandle(h: usize) -> i32;
    fn GetFullPathNameW(name: *const u16, size: u32, buf: *mut u16, file_part: *mut *mut u16) -> u32;
    fn GetLastError() -> u32;
    fn GetCurrentProcess() -> *mut u8;
    fn LocalFree(p: *mut u8) -> *mut u8;
}

#[cfg_attr(target_arch = "x86", link(name = "advapi32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "advapi32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn OpenProcessToken(process: *mut u8, access: u32, token: *mut *mut u8) -> i32;
    fn GetTokenInformation(token: *mut u8, class: u32, info: *mut u8, length: u32, ret: *mut u32) -> i32;
    fn ConvertSidToStringSidW(sid: *const u8, out: *mut *mut u16) -> i32;
}

#[cfg_attr(target_arch = "x86", link(name = "ole32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ole32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn CoTaskMemAlloc(bytes: usize) -> *mut u8;
}

// ----- interface ids (the sheet `ShellLink.yaml`, from shobjidl_core.h, objidl.h, propsys.h, ocidl.h,
// servprov.h, oleidl.h) ---------------------------------------------------------------------------

const fn ole(data1: u32) -> Guid {
    guid(data1, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46])
}
const IID_IUNKNOWN: Guid = ole(0);
const IID_IMARSHAL: Guid = ole(0x0000_0003);
const IID_IPERSIST_STREAM: Guid = ole(0x0000_0109);
const IID_IPERSIST_FILE: Guid = ole(0x0000_010B);
const IID_IPERSIST: Guid = ole(0x0000_010C);
const IID_IDROP_TARGET: Guid = ole(0x0000_0122);
const IID_ICONTEXT_MENU: Guid = ole(0x0002_14E4);
const IID_ISHELL_EXT_INIT: Guid = ole(0x0002_14E8);
const IID_IEXTRACT_ICON_A: Guid = ole(0x0002_14EB);
const IID_ISHELL_LINK_A: Guid = ole(0x0002_14EE);
const IID_ICONTEXT_MENU2: Guid = ole(0x0002_14F4);
const IID_ISHELL_LINK_W: Guid = ole(0x0002_14F9);
const IID_IEXTRACT_ICON_W: Guid = ole(0x0002_14FA);
const IID_IQUERY_INFO: Guid = ole(0x0002_1500);
const IID_ICONTEXT_MENU3: Guid = guid(0xBCFC_E0A0, 0xEC17, 0x11D0, [0x8D, 0x10, 0x00, 0xA0, 0xC9, 0x0F, 0x27, 0x19]);
const IID_IPROPERTY_STORE: Guid = guid(0x886D_8EEB, 0x8CF2, 0x4446, [0x8D, 0x02, 0xCD, 0xBA, 0x1D, 0xBD, 0xCF, 0x99]);
const IID_ISHELL_LINK_DATA_LIST: Guid =
    guid(0x45E2_B4AE, 0xB1C3, 0x11D0, [0xB9, 0x2F, 0x00, 0xA0, 0xC9, 0x03, 0x12, 0xE1]);
const IID_IOBJECT_WITH_SITE: Guid = guid(0xFC48_01A3, 0x2BA9, 0x11CF, [0xA2, 0x29, 0x00, 0xAA, 0x00, 0x3D, 0x73, 0x52]);
const IID_ISERVICE_PROVIDER: Guid = guid(0x6D51_40C1, 0x7436, 0x11CE, [0x80, 0x34, 0x00, 0xAA, 0x00, 0x60, 0x09, 0xFA]);

/// The interfaces and the index of each one's table in the object.
const UNKNOWN: usize = 0;
const LINK_W: usize = 1;
const LINK_A: usize = 2;
const PERSIST_FILE: usize = 3;
const PERSIST_STREAM: usize = 4;
const PROPERTY_STORE: usize = 5;
const INTERFACES: usize = 16;

static IIDS: [(Guid, usize); 19] = [
    (IID_IUNKNOWN, UNKNOWN),
    (IID_ISHELL_LINK_W, LINK_W),
    (IID_ISHELL_LINK_A, LINK_A),
    (IID_IPERSIST, PERSIST_FILE),
    (IID_IPERSIST_FILE, PERSIST_FILE),
    (IID_IPERSIST_STREAM, PERSIST_STREAM),
    (IID_IPROPERTY_STORE, PROPERTY_STORE),
    (IID_ISHELL_LINK_DATA_LIST, 6),
    (IID_ISHELL_EXT_INIT, 7),
    (IID_ICONTEXT_MENU, 8),
    (IID_ICONTEXT_MENU2, 8),
    (IID_ICONTEXT_MENU3, 8),
    (IID_IEXTRACT_ICON_A, 9),
    (IID_IEXTRACT_ICON_W, 10),
    (IID_IQUERY_INFO, 11),
    (IID_IDROP_TARGET, 12),
    (IID_IOBJECT_WITH_SITE, 13),
    (IID_ISERVICE_PROVIDER, 14),
    (IID_IMARSHAL, 15),
];

/// `SLGP_RAWPATH` (`shobjidl_core.h`).
const SLGP_RAWPATH: u32 = 4;
/// `STGM_*`-independent: `IPersistFile::Load` does not enforce its mode (the sheet's `load_ok`).
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_SHARE_READ_WRITE_DELETE: u32 = 7;
const OPEN_EXISTING: u32 = 3;
const CREATE_ALWAYS: u32 = 2;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
const INVALID_HANDLE: usize = usize::MAX;
const INVALID_ATTRIBUTES: u32 = u32::MAX;
/// `ERROR_PATH_NOT_FOUND`, `ERROR_ACCESS_DENIED`, `ERROR_INVALID_NAME` (`winerror.h`).
const ERROR_PATH_NOT_FOUND: u32 = 3;
const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_INVALID_NAME: u32 = 123;
/// The largest file `Load` reads.
const MAX_LINK_FILE: i64 = 16 << 20;

fn from_win32(e: u32) -> Hresult {
    if e == 0 { E_FAIL } else { (0x8007_0000 | (e & 0xFFFF)) as Hresult }
}

// ----- the object ------------------------------------------------------------------------------

struct State {
    link: Link,
    dirty: bool,
    /// The file `Save(path, TRUE)` or `Load` named (`GetCurFile`), as the caller wrote it.
    cur_file: Option<Text>,
    /// `SetPath` or `SetIDList` was called: `GetPath` without a path then clears the find data (a
    /// new link leaves it untouched, the sheet's `fresh_link` and sl_path).
    path_set: bool,
}

#[repr(C)]
struct ShellLink {
    tables: [*const c_void; INTERFACES],
    refs: AtomicU32,
    state: UnsafeCell<State>,
}

/// The object of interface pointer `this`, whose table is number `I`.
unsafe fn object<const I: usize>(this: *mut c_void) -> *mut ShellLink {
    // SAFETY: an interface pointer of a ShellLink is the address of its `I`th table pointer.
    unsafe { this.cast::<*const c_void>().sub(I).cast() }
}

unsafe fn state<'a, const I: usize>(this: *mut c_void) -> &'a mut State {
    // SAFETY: a live object, used from its apartment's thread (one call at a time).
    unsafe { &mut *(*object::<I>(this)).state.get() }
}

unsafe extern "system" fn qi<const I: usize>(this: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's pointers; a live object.
    unsafe {
        *out = core::ptr::null_mut();
        if iid.is_null() {
            return E_INVALIDARG;
        }
        let iid = iid.read_unaligned();
        let Some(&(_, index)) = IIDS.iter().find(|(g, _)| *g == iid) else { return E_NOINTERFACE };
        let obj = object::<I>(this);
        (*obj).refs.fetch_add(1, Ordering::AcqRel);
        *out = core::ptr::addr_of_mut!((*obj).tables[index]).cast();
    }
    S_OK
}

unsafe extern "system" fn add_ref<const I: usize>(this: *mut c_void) -> u32 {
    // SAFETY: a live object.
    unsafe { (*object::<I>(this)).refs.fetch_add(1, Ordering::AcqRel) + 1 }
}

unsafe extern "system" fn release<const I: usize>(this: *mut c_void) -> u32 {
    // SAFETY: a live object; freed when the last reference goes.
    unsafe {
        let obj = object::<I>(this);
        let left = (*obj).refs.fetch_sub(1, Ordering::AcqRel) - 1;
        if left == 0 {
            drop(alloc::boxed::Box::from_raw(obj));
        }
        left
    }
}

/// A method of an interface the object only answers: `E_NOTIMPL`, whatever the arguments (the
/// caller cleans them up).
unsafe extern "system" fn not_implemented(_this: *mut c_void) -> Hresult {
    E_NOTIMPL
}

macro_rules! unknown {
    ($i:expr) => {
        [qi::<{ $i }> as *const c_void, add_ref::<{ $i }> as *const c_void, release::<{ $i }> as *const c_void]
    };
}

/// A table of `IUnknown` and `N - 3` methods that are not implemented.
macro_rules! stub_table {
    ($name:ident, $i:expr, $n:expr) => {
        static $name: Vtbl<$n> = {
            let mut t = [not_implemented as *const c_void; $n];
            let u = unknown!($i);
            t[0] = u[0];
            t[1] = u[1];
            t[2] = u[2];
            Vtbl(t)
        };
    };
}

static UNKNOWN_VTBL: Vtbl<3> = Vtbl(unknown!(UNKNOWN));
stub_table!(DATA_LIST_VTBL, 6, 8);
stub_table!(EXT_INIT_VTBL, 7, 4);
stub_table!(CONTEXT_MENU_VTBL, 8, 8);
stub_table!(EXTRACT_A_VTBL, 9, 5);
stub_table!(EXTRACT_W_VTBL, 10, 5);
stub_table!(QUERY_INFO_VTBL, 11, 5);
stub_table!(DROP_TARGET_VTBL, 12, 7);
stub_table!(OBJECT_WITH_SITE_VTBL, 13, 5);
stub_table!(SERVICE_PROVIDER_VTBL, 14, 4);
stub_table!(MARSHAL_VTBL, 15, 9);

macro_rules! link_table {
    ($name:ident, $i:expr, $unit:ty) => {
        /// `IShellLinkW` / `IShellLinkA` (`shobjidl_core.h`; the sheet's `slots`).
        static $name: Vtbl<21> = {
            let u = unknown!($i);
            Vtbl([
                u[0],
                u[1],
                u[2],
                get_path::<{ $i }, $unit> as *const c_void,
                get_id_list::<{ $i }> as *const c_void,
                set_id_list::<{ $i }> as *const c_void,
                get_string::<{ $i }, $unit, DESCRIPTION> as *const c_void,
                set_string::<{ $i }, $unit, DESCRIPTION> as *const c_void,
                get_string::<{ $i }, $unit, WORKING_DIR> as *const c_void,
                set_string::<{ $i }, $unit, WORKING_DIR> as *const c_void,
                get_string::<{ $i }, $unit, ARGUMENTS> as *const c_void,
                set_string::<{ $i }, $unit, ARGUMENTS> as *const c_void,
                get_hotkey::<{ $i }> as *const c_void,
                set_hotkey::<{ $i }> as *const c_void,
                get_show_cmd::<{ $i }> as *const c_void,
                set_show_cmd::<{ $i }> as *const c_void,
                get_icon_location::<{ $i }, $unit> as *const c_void,
                set_icon_location::<{ $i }, $unit> as *const c_void,
                set_relative_path::<{ $i }, $unit> as *const c_void,
                resolve::<{ $i }> as *const c_void,
                set_path::<{ $i }, $unit> as *const c_void,
            ])
        };
    };
}
link_table!(LINK_W_VTBL, LINK_W, u16);
link_table!(LINK_A_VTBL, LINK_A, u8);

/// `IPersistFile` (`objidl.h`): `GetClassID` 3, `IsDirty` 4, `Load` 5, `Save` 6,
/// `SaveCompleted` 7, `GetCurFile` 8.
static PERSIST_FILE_VTBL: Vtbl<9> = {
    let u = unknown!(PERSIST_FILE);
    Vtbl([
        u[0],
        u[1],
        u[2],
        get_class_id as *const c_void,
        is_dirty::<PERSIST_FILE> as *const c_void,
        file_load as *const c_void,
        file_save as *const c_void,
        file_save_completed as *const c_void,
        file_get_cur_file as *const c_void,
    ])
};

/// `IPersistStream` (`objidl.h`): `GetClassID` 3, `IsDirty` 4, `Load` 5, `Save` 6, `GetSizeMax` 7.
static PERSIST_STREAM_VTBL: Vtbl<8> = {
    let u = unknown!(PERSIST_STREAM);
    Vtbl([
        u[0],
        u[1],
        u[2],
        get_class_id as *const c_void,
        is_dirty::<PERSIST_STREAM> as *const c_void,
        stream_load as *const c_void,
        stream_save as *const c_void,
        stream_size_max as *const c_void,
    ])
};

/// `IPropertyStore` (`propsys.h`): `GetCount` 3, `GetAt` 4, `GetValue` 5, `SetValue` 6, `Commit` 7.
static PROPERTY_STORE_VTBL: Vtbl<8> = {
    let u = unknown!(PROPERTY_STORE);
    Vtbl([
        u[0],
        u[1],
        u[2],
        store_count as *const c_void,
        store_key_at as *const c_void,
        store_get as *const c_void,
        store_set as *const c_void,
        store_commit as *const c_void,
    ])
};

fn table<const N: usize>(t: &'static Vtbl<N>) -> *const c_void {
    (t as *const Vtbl<N>).cast()
}

/// `IClassFactory::CreateInstance` of `CLSID_ShellLink`: a new, empty link as `iid`.
pub unsafe fn create(iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    let obj = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(ShellLink {
        tables: [
            table(&UNKNOWN_VTBL),
            table(&LINK_W_VTBL),
            table(&LINK_A_VTBL),
            table(&PERSIST_FILE_VTBL),
            table(&PERSIST_STREAM_VTBL),
            table(&PROPERTY_STORE_VTBL),
            table(&DATA_LIST_VTBL),
            table(&EXT_INIT_VTBL),
            table(&CONTEXT_MENU_VTBL),
            table(&EXTRACT_A_VTBL),
            table(&EXTRACT_W_VTBL),
            table(&QUERY_INFO_VTBL),
            table(&DROP_TARGET_VTBL),
            table(&OBJECT_WITH_SITE_VTBL),
            table(&SERVICE_PROVIDER_VTBL),
            table(&MARSHAL_VTBL),
        ],
        refs: AtomicU32::new(1),
        state: UnsafeCell::new(State { link: Link::new(), dirty: false, cur_file: None, path_set: false }),
    }));
    // SAFETY: a fresh object holding one reference, handed over to the asked interface.
    unsafe {
        let unknown = core::ptr::addr_of_mut!((*obj).tables[UNKNOWN]).cast::<c_void>();
        let hr = qi::<UNKNOWN>(unknown, iid, out);
        release::<UNKNOWN>(unknown);
        hr
    }
}

// ----- text in the callers' units ----------------------------------------------------------------

/// The code units of the two `IShellLink` flavours: UTF-16 for `IShellLinkW`, ANSI for
/// `IShellLinkA` (Poly's ANSI code page keeps the first 256 code points).
trait Unit: Copy + Default + PartialEq {
    fn from_text(t: &[u16]) -> Vec<Self>;
    fn to_text(units: &[Self]) -> Text;
    /// The `WIN32_FIND_DATA` size and its `cFileName` length (`minwinbase.h`).
    const FIND_DATA: usize;
}

impl Unit for u16 {
    fn from_text(t: &[u16]) -> Vec<u16> {
        t.to_vec()
    }
    fn to_text(units: &[u16]) -> Text {
        units.to_vec()
    }
    const FIND_DATA: usize = 592;
}

impl Unit for u8 {
    fn from_text(t: &[u16]) -> Vec<u8> {
        text::to_ansi(t)
    }
    fn to_text(units: &[u8]) -> Text {
        text::from_ansi(units)
    }
    const FIND_DATA: usize = 320;
}

/// A NUL-terminated string of the caller (`None` for NULL).
unsafe fn caller_text<T: Unit>(p: *const T) -> Option<Text> {
    if p.is_null() {
        return None;
    }
    let mut n = 0;
    // SAFETY: a NUL-terminated string of the caller.
    unsafe {
        while *p.add(n) != T::default() {
            n += 1;
        }
        Some(T::to_text(core::slice::from_raw_parts(p, n)))
    }
}

/// Copy `n` units of `t` and a NUL into the caller's buffer.
unsafe fn put<T: Unit>(buf: *mut T, t: &[T], n: usize) {
    // SAFETY: the caller's buffer holds at least `n + 1` units (the rules give `n < cch`).
    unsafe {
        core::ptr::copy_nonoverlapping(t.as_ptr(), buf, n);
        buf.add(n).write(T::default());
    }
}

// ----- the system, for poly_lnk ------------------------------------------------------------------

struct Win32;

fn z(t: &[u16]) -> Vec<u16> {
    nul(t)
}

fn find_info(d: &[u8]) -> FileInfo {
    let u32_at = |o: usize| u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]);
    let u64_at = |o: usize| u64::from(u32_at(o)) | (u64::from(u32_at(o + 4)) << 32);
    FileInfo {
        attributes: u32_at(0),
        creation: u64_at(4),
        access: u64_at(12),
        write: u64_at(20),
        size: (u64::from(u32_at(28)) << 32) | u64::from(u32_at(32)),
    }
}

impl System for Win32 {
    fn env(&self, name: &[u16]) -> Option<Text> {
        let name = z(name);
        let mut buf = alloc::vec![0u16; 1024];
        // SAFETY: a NUL-terminated name and a buffer of its length.
        let n = unsafe { GetEnvironmentVariableW(name.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
        (n > 0 && n < buf.len()).then(|| buf[..n].to_vec())
    }

    fn entry(&self, path: &[u16]) -> Option<(Text, FileInfo)> {
        if path.iter().any(|&c| c == u16::from(b'*') || c == u16::from(b'?')) {
            return None;
        }
        let p = z(path);
        let mut fd = [0u8; 592];
        // SAFETY: a NUL-terminated path and a WIN32_FIND_DATAW.
        let h = unsafe { FindFirstFileW(p.as_ptr(), fd.as_mut_ptr()) };
        if h != INVALID_HANDLE {
            // SAFETY: the search just opened.
            unsafe { FindClose(h) };
            let name: Text =
                fd[44..564].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
            return Some((name, find_info(&fd)));
        }
        // A drive's root has no entry of its own: its attributes.
        let mut data = [0u8; 36];
        // SAFETY: a NUL-terminated path and a WIN32_FILE_ATTRIBUTE_DATA (GetFileExInfoStandard).
        if unsafe { GetFileAttributesExW(p.as_ptr(), 0, data.as_mut_ptr()) } == 0 {
            return None;
        }
        let mut fd = [0u8; 36];
        fd[..28].copy_from_slice(&data[..28]);
        fd[28..36].copy_from_slice(&data[28..36]);
        Some((text::file_name(path).to_vec(), find_info(&fd)))
    }

    fn drive_exists(&self, letter: u16) -> bool {
        let root = [letter, u16::from(b':'), u16::from(b'\\'), 0];
        // SAFETY: a NUL-terminated path.
        unsafe { GetFileAttributesW(root.as_ptr()) != INVALID_ATTRIBUTES }
    }

    fn volume_serial(&self, letter: u16) -> u32 {
        let root = [letter, u16::from(b':'), u16::from(b'\\'), 0];
        let mut serial = 0;
        // SAFETY: a NUL-terminated root and one out variable; the others are NULL.
        unsafe {
            GetVolumeInformationW(
                root.as_ptr(),
                core::ptr::null_mut(),
                0,
                &mut serial,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                0,
            )
        };
        serial
    }

    fn machine_name(&self) -> Vec<u8> {
        let mut buf = [0u16; 64];
        let mut n = buf.len() as u32;
        // SAFETY: a buffer of `n` characters.
        if unsafe { GetComputerNameW(buf.as_mut_ptr(), &mut n) } == 0 {
            return Vec::new();
        }
        text::to_ansi(&buf[..(n as usize).min(64)])
    }

    fn user_sid(&self) -> Text {
        /// `TOKEN_QUERY`, `TokenUser` (`winnt.h`).
        const TOKEN_QUERY: u32 = 8;
        const TOKEN_USER: u32 = 1;
        let mut out = Text::new();
        // SAFETY: the process's own token, queried into a local buffer; the string is freed with
        // LocalFree as ConvertSidToStringSidW's contract says.
        unsafe {
            let mut token: *mut u8 = core::ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return out;
            }
            let mut buf = [0u64; 32];
            let mut ret = 0;
            let ok = GetTokenInformation(token, TOKEN_USER, buf.as_mut_ptr().cast(), 256, &mut ret);
            CloseHandle(token as usize);
            if ok == 0 {
                return out;
            }
            // TOKEN_USER: a SID_AND_ATTRIBUTES whose first field is the SID's address.
            let sid = buf[0] as usize as *const u8;
            let mut s: *mut u16 = core::ptr::null_mut();
            if ConvertSidToStringSidW(sid, &mut s) != 0 && !s.is_null() {
                out = wstr(s).to_vec();
                LocalFree(s.cast());
            }
        }
        out
    }
}

// ----- IShellLinkW / IShellLinkA -----------------------------------------------------------------

const DESCRIPTION: u8 = 0;
const WORKING_DIR: u8 = 1;
const ARGUMENTS: u8 = 2;

fn field(link: &mut Link, which: u8) -> &mut Option<Text> {
    match which {
        DESCRIPTION => &mut link.description,
        WORKING_DIR => &mut link.working_dir,
        _ => &mut link.arguments,
    }
}

unsafe extern "system" fn get_path<const I: usize, T: Unit>(
    this: *mut c_void,
    buf: *mut T,
    cch: i32,
    fd: *mut u8,
    flags: u32,
) -> Hresult {
    // SAFETY: a live object.
    let st = unsafe { state::<I>(this) };
    let shown = st.link.reported_path(flags & SLGP_RAWPATH != 0).map(T::from_text);
    let (hr, n) = text::get_path(shown.as_ref().map(Vec::len), cch);
    // SAFETY: the caller's buffer of `cch` units and find data.
    unsafe {
        if !buf.is_null()
            && let Some(n) = n
        {
            put(buf, shown.as_deref().unwrap_or(&[]), n);
        }
        if !fd.is_null()
            && let Some(p) = st.link.path.as_deref().or(st.link.env_path.as_deref())
        {
            core::ptr::write_bytes(fd, 0, T::FIND_DATA);
            let info = st.link.find;
            fd.cast::<u32>().write_unaligned(info.attributes);
            fd.add(4).cast::<u64>().write_unaligned(info.creation);
            fd.add(12).cast::<u64>().write_unaligned(info.access);
            fd.add(20).cast::<u64>().write_unaligned(info.write);
            fd.add(28).cast::<u32>().write_unaligned((info.size >> 32) as u32);
            fd.add(32).cast::<u32>().write_unaligned(info.size as u32);
            let name = T::from_text(text::file_name(p));
            let n = name.len().min(259);
            put(fd.add(44).cast::<T>(), &name, n);
        } else if !fd.is_null() && st.path_set {
            core::ptr::write_bytes(fd, 0, T::FIND_DATA);
        }
    }
    hr
}

unsafe extern "system" fn get_id_list<const I: usize>(this: *mut c_void, out: *mut *mut u8) -> Hresult {
    if out.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: a live object; the caller's out parameter.
    unsafe {
        let st = state::<I>(this);
        *out = core::ptr::null_mut();
        let Some(p) = st.link.path.as_deref() else { return S_FALSE };
        let list = crate::pidl_of_path(p);
        if list.is_null() {
            return E_OUTOFMEMORY;
        }
        *out = list;
    }
    S_OK
}

/// The bytes of an item list of the caller, with its terminator.
unsafe fn list_bytes<'a>(pidl: *const u8) -> &'a [u8] {
    let mut n = 0;
    // SAFETY: a list of the caller: sized items ended by a size of 0.
    unsafe {
        loop {
            let cb = usize::from(pidl.add(n).cast::<u16>().read_unaligned());
            n += 2;
            if cb < 2 {
                break;
            }
            n += cb - 2;
        }
        core::slice::from_raw_parts(pidl, n)
    }
}

unsafe extern "system" fn set_id_list<const I: usize>(this: *mut c_void, pidl: *const u8) -> Hresult {
    // SAFETY: a live object; the caller's list.
    unsafe {
        let st = state::<I>(this);
        let path = if pidl.is_null() { None } else { idlist::path_of(list_bytes(pidl)) };
        st.link.set_id_list_path(path, &Win32);
        st.path_set = true;
        st.dirty = true;
    }
    S_OK
}

unsafe extern "system" fn get_string<const I: usize, T: Unit, const F: u8>(
    this: *mut c_void,
    buf: *mut T,
    cch: i32,
) -> Hresult {
    if buf.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: a live object; the caller's buffer of `cch` units.
    unsafe {
        let st = state::<I>(this);
        let units = T::from_text(field(&mut st.link, F).as_deref().unwrap_or(&[]));
        if let Some(n) = text::truncated(units.len(), cch) {
            put(buf, &units, n);
        }
    }
    S_OK
}

unsafe extern "system" fn set_string<const I: usize, T: Unit, const F: u8>(this: *mut c_void, s: *const T) -> Hresult {
    // SAFETY: a live object; the caller's string.
    unsafe {
        let st = state::<I>(this);
        // NULL clears the arguments and leaves the other strings (the sheet's `null_strings`).
        match caller_text(s) {
            Some(t) => *field(&mut st.link, F) = Some(t),
            None if F == ARGUMENTS => st.link.arguments = None,
            None => {}
        }
        st.dirty = true;
    }
    S_OK
}

unsafe extern "system" fn get_hotkey<const I: usize>(this: *mut c_void, out: *mut u16) -> Hresult {
    if out.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: a live object; the caller's out parameter.
    unsafe { *out = state::<I>(this).link.hotkey };
    S_OK
}

unsafe extern "system" fn set_hotkey<const I: usize>(this: *mut c_void, key: u16) -> Hresult {
    // SAFETY: a live object.
    let st = unsafe { state::<I>(this) };
    st.link.hotkey = key;
    st.dirty = true;
    S_OK
}

unsafe extern "system" fn get_show_cmd<const I: usize>(this: *mut c_void, out: *mut i32) -> Hresult {
    if out.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: a live object; the caller's out parameter.
    unsafe { *out = state::<I>(this).link.show_command };
    S_OK
}

unsafe extern "system" fn set_show_cmd<const I: usize>(this: *mut c_void, show: i32) -> Hresult {
    // SAFETY: a live object.
    let st = unsafe { state::<I>(this) };
    st.link.show_command = show;
    st.dirty = true;
    S_OK
}

unsafe extern "system" fn get_icon_location<const I: usize, T: Unit>(
    this: *mut c_void,
    buf: *mut T,
    cch: i32,
    index: *mut i32,
) -> Hresult {
    if buf.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: a live object; the caller's buffer and out parameter.
    unsafe {
        let st = state::<I>(this);
        let units = T::from_text(st.link.icon_path.as_deref().unwrap_or(&[]));
        if let Some(n) = text::truncated(units.len(), cch) {
            put(buf, &units, n);
        }
        if !index.is_null() {
            *index = st.link.icon_index;
        }
    }
    S_OK
}

unsafe extern "system" fn set_icon_location<const I: usize, T: Unit>(
    this: *mut c_void,
    path: *const T,
    index: i32,
) -> Hresult {
    // SAFETY: a live object; the caller's string.
    unsafe {
        let st = state::<I>(this);
        if let Some(t) = caller_text(path) {
            st.link.icon_path = Some(t);
        }
        st.link.icon_index = index;
        st.dirty = true;
    }
    S_OK
}

unsafe extern "system" fn set_relative_path<const I: usize, T: Unit>(
    this: *mut c_void,
    path: *const T,
    reserved: u32,
) -> Hresult {
    if reserved != 0 {
        return E_INVALIDARG;
    }
    // SAFETY: a live object; the caller's string.
    unsafe {
        let st = state::<I>(this);
        st.link.relative_source = caller_text(path).filter(|t| !t.is_empty());
        st.dirty = true;
    }
    S_OK
}

unsafe extern "system" fn resolve<const I: usize>(this: *mut c_void, _hwnd: usize, _flags: u32) -> Hresult {
    // SAFETY: a live object.
    unsafe { state::<I>(this) }.link.resolve(&Win32)
}

unsafe extern "system" fn set_path<const I: usize, T: Unit>(this: *mut c_void, path: *const T) -> Hresult {
    // SAFETY: the caller's string; a live object.
    let (Some(t), st) = (unsafe { caller_text(path) }, unsafe { state::<I>(this) }) else { return E_INVALIDARG };
    let r = st.link.set_path(&t, &Win32);
    st.path_set = true;
    st.dirty = true;
    r.hresult()
}

// ----- IPersistFile / IPersistStream ---------------------------------------------------------------

unsafe extern "system" fn get_class_id(_this: *mut c_void, out: *mut Guid) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { out.write_unaligned(poly_lnk::CLSID_SHELL_LINK) };
    S_OK
}

unsafe extern "system" fn is_dirty<const I: usize>(this: *mut c_void) -> Hresult {
    // SAFETY: a live object.
    if unsafe { state::<I>(this) }.dirty { S_OK } else { S_FALSE }
}

/// The bytes of a file, or the `HRESULT` of the failure to read it.
fn read_file(name: &[u16]) -> Result<Vec<u8>, Hresult> {
    if name.is_empty() {
        return Err(from_win32(ERROR_PATH_NOT_FOUND));
    }
    let p = z(name);
    // SAFETY: a NUL-terminated name; the handle is closed below; the buffer is `size` bytes.
    unsafe {
        let attrs = GetFileAttributesW(p.as_ptr());
        if attrs != INVALID_ATTRIBUTES && attrs & poly_lnk::FILE_ATTRIBUTE_DIRECTORY != 0 {
            return Err(from_win32(ERROR_ACCESS_DENIED));
        }
        let h = CreateFileW(
            p.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ_WRITE_DELETE,
            core::ptr::null_mut(),
            OPEN_EXISTING,
            0,
            0,
        );
        if h == INVALID_HANDLE {
            return Err(from_win32(GetLastError()));
        }
        let mut size = 0i64;
        let mut data = Vec::new();
        let mut result = Ok(());
        if GetFileSizeEx(h, &mut size) == 0 || !(0..=MAX_LINK_FILE).contains(&size) {
            result = Err(E_FAIL);
        } else {
            data.resize(size as usize, 0);
            let mut got = 0u32;
            if ReadFile(h, data.as_mut_ptr(), size as u32, &mut got, core::ptr::null_mut()) == 0 {
                result = Err(from_win32(GetLastError()));
            }
            data.truncate(got as usize);
        }
        CloseHandle(h);
        result.map(|()| data)
    }
}

/// Write a file whole, or the `HRESULT` of the failure.
fn write_file(name: &[u16], data: &[u8]) -> Result<(), Hresult> {
    if name.is_empty() {
        return Err(from_win32(ERROR_PATH_NOT_FOUND));
    }
    let bad = |c: u16| {
        c == u16::from(b'<')
            || c == u16::from(b'>')
            || c == u16::from(b'*')
            || c == u16::from(b'?')
            || c == u16::from(b'|')
    };
    if name.iter().any(|&c| bad(c)) {
        return Err(from_win32(ERROR_INVALID_NAME));
    }
    let p = z(name);
    // SAFETY: a NUL-terminated name; the handle is closed below.
    unsafe {
        let attrs = GetFileAttributesW(p.as_ptr());
        if attrs != INVALID_ATTRIBUTES && attrs & poly_lnk::FILE_ATTRIBUTE_DIRECTORY != 0 {
            return Err(from_win32(ERROR_ACCESS_DENIED));
        }
        let h =
            CreateFileW(p.as_ptr(), GENERIC_WRITE, 0, core::ptr::null_mut(), CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, 0);
        if h == INVALID_HANDLE {
            return Err(from_win32(GetLastError()));
        }
        let mut done = 0u32;
        let ok = WriteFile(h, data.as_ptr(), data.len() as u32, &mut done, core::ptr::null_mut());
        let e = GetLastError();
        CloseHandle(h);
        if ok == 0 || done as usize != data.len() {
            return Err(from_win32(e));
        }
    }
    Ok(())
}

/// The full path of a file name (the base of the link's relative path).
fn full_path(name: &[u16]) -> Text {
    let p = z(name);
    let mut buf = alloc::vec![0u16; 1024];
    // SAFETY: a NUL-terminated name and a buffer of its length.
    let n = unsafe { GetFullPathNameW(p.as_ptr(), buf.len() as u32, buf.as_mut_ptr(), core::ptr::null_mut()) } as usize;
    if n == 0 || n >= buf.len() { name.to_vec() } else { buf[..n].to_vec() }
}

unsafe extern "system" fn file_load(this: *mut c_void, name: *const u16, _mode: u32) -> Hresult {
    // SAFETY: the caller's string; a live object.
    let (Some(name), st) = (unsafe { caller_text(name) }, unsafe { state::<PERSIST_FILE>(this) }) else {
        return E_INVALIDARG;
    };
    let data = match read_file(&name) {
        Ok(d) => d,
        Err(e) => return e,
    };
    match Link::load(&data, &Win32) {
        Ok(link) => {
            st.link = link;
            st.dirty = false;
            st.cur_file = Some(name);
            S_OK
        }
        Err(_) => E_FAIL,
    }
}

unsafe extern "system" fn file_save(this: *mut c_void, name: *const u16, remember: i32) -> Hresult {
    // SAFETY: the caller's string; a live object.
    let (name, st) = (unsafe { caller_text(name) }, unsafe { state::<PERSIST_FILE>(this) });
    let Some(target) = name.clone().or_else(|| st.cur_file.clone()) else {
        // Nothing to save to: S_OK (the sheet's `save_ok`).
        st.dirty = false;
        return S_OK;
    };
    let full = full_path(&target);
    let data = st.link.save(Some(&full), &Win32);
    if let Err(e) = write_file(&target, &data) {
        return e;
    }
    if name.is_some() && remember != 0 {
        st.cur_file = name;
    }
    st.dirty = false;
    S_OK
}

unsafe extern "system" fn file_save_completed(_this: *mut c_void, _name: *const u16) -> Hresult {
    S_OK
}

unsafe extern "system" fn file_get_cur_file(this: *mut c_void, out: *mut *mut u16) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: a live object; the caller's out parameter.
    unsafe {
        match &state::<PERSIST_FILE>(this).cur_file {
            Some(f) => task_string(f, out),
            None => {
                *out = core::ptr::null_mut();
                S_FALSE
            }
        }
    }
}

unsafe extern "system" fn stream_load(this: *mut c_void, stream: *mut c_void) -> Hresult {
    if stream.is_null() {
        return E_POINTER;
    }
    let mut data = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let mut got = 0u32;
        // SAFETY: `ISequentialStream::Read` (slot 3) of the caller's stream into a local buffer.
        let hr = unsafe {
            crate::vcall!(
                stream,
                3,
                fn(*mut u8, u32, *mut u32) -> Hresult,
                chunk.as_mut_ptr(),
                chunk.len() as u32,
                &mut got
            )
        };
        if hr < 0 {
            return hr;
        }
        data.extend_from_slice(&chunk[..(got as usize).min(chunk.len())]);
        if got == 0 || data.len() as i64 > MAX_LINK_FILE {
            break;
        }
    }
    // SAFETY: a live object.
    let st = unsafe { state::<PERSIST_STREAM>(this) };
    match Link::load(&data, &Win32) {
        Ok(link) => {
            st.link = link;
            st.dirty = false;
            S_OK
        }
        Err(_) => E_FAIL,
    }
}

unsafe extern "system" fn stream_save(this: *mut c_void, stream: *mut c_void, clear_dirty: i32) -> Hresult {
    if stream.is_null() {
        return E_POINTER;
    }
    // SAFETY: a live object.
    let st = unsafe { state::<PERSIST_STREAM>(this) };
    let data = st.link.save(None, &Win32);
    let mut done = 0u32;
    // SAFETY: `ISequentialStream::Write` (slot 4) of the caller's stream.
    let hr = unsafe {
        crate::vcall!(stream, 4, fn(*const u8, u32, *mut u32) -> Hresult, data.as_ptr(), data.len() as u32, &mut done)
    };
    if hr >= 0 && clear_dirty != 0 {
        st.dirty = false;
    }
    hr
}

unsafe extern "system" fn stream_size_max(this: *mut c_void, out: *mut u64) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: a live object; the caller's out parameter.
    unsafe { out.write_unaligned(state::<PERSIST_STREAM>(this).link.save(None, &Win32).len() as u64) };
    S_OK
}

// ----- IPropertyStore ------------------------------------------------------------------------------

unsafe extern "system" fn store_count(this: *mut c_void, out: *mut u32) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: a live object; the caller's out parameter.
    unsafe { *out = state::<PROPERTY_STORE>(this).link.property_count() as u32 };
    S_OK
}

unsafe extern "system" fn store_key_at(this: *mut c_void, index: u32, out: *mut [u8; 20]) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: a live object; the caller's PROPERTYKEY.
    unsafe {
        match state::<PROPERTY_STORE>(this).link.property_key_at(index as usize) {
            Some(k) => {
                out.write_unaligned(k.to_bytes());
                S_OK
            }
            None => E_INVALIDARG,
        }
    }
}

/// A copy of `bytes` in task memory.
fn task_copy(bytes: &[u8]) -> *mut u8 {
    // SAFETY: a fresh block of the right size.
    unsafe {
        let p = CoTaskMemAlloc(bytes.len().max(1));
        if !p.is_null() {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        }
        p
    }
}

unsafe extern "system" fn store_get(this: *mut c_void, key: *const [u8; 20], out: *mut [u8; 24]) -> Hresult {
    if key.is_null() || out.is_null() {
        return E_POINTER;
    }
    // SAFETY: a live object; the caller's key and PROPVARIANT.
    unsafe {
        let key = PropKey::from_bytes(&key.read_unaligned());
        let mut v = [0u8; 24];
        match state::<PROPERTY_STORE>(this).link.property(&key) {
            None | Some(PropValue::Empty) => {}
            Some(PropValue::Text(t)) => {
                let mut bytes: Vec<u8> = t.iter().flat_map(|c| c.to_le_bytes()).collect();
                bytes.extend_from_slice(&[0, 0]);
                let p = task_copy(&bytes);
                if p.is_null() {
                    return E_OUTOFMEMORY;
                }
                v[0..2].copy_from_slice(&props::VT_LPWSTR.to_le_bytes());
                v[8..16].copy_from_slice(&(p as usize as u64).to_le_bytes());
            }
            Some(PropValue::Clsid(g)) => {
                let p = task_copy(&idlist::guid_bytes(&g));
                if p.is_null() {
                    return E_OUTOFMEMORY;
                }
                v[0..2].copy_from_slice(&props::VT_CLSID.to_le_bytes());
                v[8..16].copy_from_slice(&(p as usize as u64).to_le_bytes());
            }
            Some(PropValue::Scalar(vt, bytes)) => {
                v[0..2].copy_from_slice(&vt.to_le_bytes());
                v[8..16].copy_from_slice(&bytes);
            }
        }
        out.write_unaligned(v);
    }
    S_OK
}

unsafe extern "system" fn store_set(this: *mut c_void, key: *const [u8; 20], value: *const [u8; 24]) -> Hresult {
    if key.is_null() || value.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's key and PROPVARIANT (whose pointers it owns).
    let (key, v) = unsafe { (PropKey::from_bytes(&key.read_unaligned()), value.read_unaligned()) };
    let vt = u16::from_le_bytes([v[0], v[1]]);
    let ptr = u64::from_le_bytes(v[8..16].try_into().expect("8 bytes")) as usize;
    let value = match vt {
        props::VT_EMPTY => PropValue::Empty,
        props::VT_LPWSTR | 8 => {
            // VT_BSTR (8) is a string too.
            // SAFETY: the variant's string.
            match unsafe { caller_text(ptr as *const u16) } {
                Some(t) => PropValue::Text(t),
                None => return E_INVALIDARG,
            }
        }
        props::VT_CLSID => {
            if ptr == 0 {
                return E_INVALIDARG;
            }
            // SAFETY: the variant's 16-byte GUID.
            let b = unsafe { core::slice::from_raw_parts(ptr as *const u8, 16) };
            PropValue::Clsid(idlist::guid_from(b).unwrap_or_default())
        }
        vt if props::scalar_size(vt).is_some() => PropValue::Scalar(vt, v[8..16].try_into().expect("8 bytes")),
        _ => return E_INVALIDARG,
    };
    // SAFETY: a live object.
    let st = unsafe { state::<PROPERTY_STORE>(this) };
    let r = st.link.set_property(&key, value);
    st.dirty = true;
    r
}

unsafe extern "system" fn store_commit(_this: *mut c_void) -> Hresult {
    // The values are in place as soon as they are set (the sheet's `set_get`).
    hr::S_OK
}

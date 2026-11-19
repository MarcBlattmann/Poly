//! What the type-library objects and the marshaller share (ADR 0220, B2b): a lock, COM calls
//! through vtables, strings, the registry and files, and the few `ole32` functions `oleaut32`
//! uses (looked up on first use: `ole32` itself imports `oleaut32`).

use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
pub(crate) use poly_com::guid::Guid;
pub(crate) use poly_com::hresult::Hresult;

pub(crate) const S_OK: Hresult = 0;
pub(crate) const S_FALSE: Hresult = 1;
pub(crate) const E_NOTIMPL: Hresult = 0x8000_4001_u32 as i32;
pub(crate) const E_NOINTERFACE: Hresult = 0x8000_4002_u32 as i32;
pub(crate) const E_POINTER: Hresult = 0x8000_4003_u32 as i32;
pub(crate) const E_FAIL: Hresult = 0x8000_4005_u32 as i32;
pub(crate) const E_UNEXPECTED: Hresult = 0x8000_FFFF_u32 as i32;
pub(crate) const E_INVALIDARG: Hresult = 0x8007_0057_u32 as i32;
pub(crate) const E_OUTOFMEMORY: Hresult = 0x8007_000E_u32 as i32;
pub(crate) const CLASS_E_NOAGGREGATION: Hresult = 0x8004_0110_u32 as i32;
pub(crate) const CLASS_E_CLASSNOTAVAILABLE: Hresult = 0x8004_0111_u32 as i32;
pub(crate) const REGDB_E_IIDNOTREG: Hresult = 0x8004_0155_u32 as i32;

const fn std_guid(d1: u32) -> Guid {
    Guid { data1: d1, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] }
}

pub(crate) const IID_IUNKNOWN: Guid = std_guid(0);
pub(crate) const IID_IMARSHAL: Guid = std_guid(3);
pub(crate) const IID_IDISPATCH: Guid = std_guid(0x0002_0400);
pub(crate) const IID_ITYPEINFO: Guid = std_guid(0x0002_0401);
pub(crate) const IID_ITYPELIB: Guid = std_guid(0x0002_0402);
pub(crate) const IID_ITYPECOMP: Guid = std_guid(0x0002_0403);
pub(crate) const IID_ICREATETYPEINFO: Guid = std_guid(0x0002_0405);
pub(crate) const IID_ICREATETYPELIB: Guid = std_guid(0x0002_0406);
pub(crate) const IID_ICREATETYPEINFO2: Guid =
    Guid { data1: 0x0002_040E, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
pub(crate) const IID_ICREATETYPELIB2: Guid =
    Guid { data1: 0x0002_040F, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
pub(crate) const IID_NULL: Guid = Guid { data1: 0, data2: 0, data3: 0, data4: [0; 8] };

/// A vtable: function pointers as untyped pointers.
#[repr(transparent)]
pub(crate) struct Vtbl<const N: usize>(pub [*const (); N]);
// SAFETY: tables of function pointers.
unsafe impl<const N: usize> Sync for Vtbl<N> {}

/// A spin lock around a value (no `std` here).
pub(crate) struct Locked<T> {
    flag: AtomicBool,
    value: core::cell::UnsafeCell<T>,
}

// SAFETY: the flag serialises every access to the value.
unsafe impl<T: Send> Sync for Locked<T> {}
// SAFETY: as above.
unsafe impl<T: Send> Send for Locked<T> {}

impl<T> Locked<T> {
    pub(crate) const fn new(value: T) -> Locked<T> {
        Locked { flag: AtomicBool::new(false), value: core::cell::UnsafeCell::new(value) }
    }

    pub(crate) fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        while self.flag.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            core::hint::spin_loop();
        }
        // SAFETY: the flag is held.
        let r = f(unsafe { &mut *self.value.get() });
        self.flag.store(false, Ordering::Release);
        r
    }
}

/// Call slot `$i` of the vtable of the COM pointer `$p` with the stated signature.
#[macro_export]
macro_rules! vcall {
    ($p:expr, $i:expr, ($($at:ty),*) -> $r:ty $(, $a:expr)*) => {{
        let p: *mut core::ffi::c_void = $p as *mut core::ffi::c_void;
        // SAFETY: the caller's COM pointer; the slot has the stated signature.
        unsafe {
            let slot = *(*(p as *mut *const usize)).add($i);
            let f: unsafe extern "system" fn(*mut core::ffi::c_void $(, $at)*) -> $r = core::mem::transmute(slot);
            f(p $(, $a)*)
        }
    }};
}

/// `IUnknown::QueryInterface`.
pub(crate) fn query(p: *mut c_void, iid: &Guid) -> (Hresult, *mut c_void) {
    let mut out: *mut c_void = core::ptr::null_mut();
    let hr = vcall!(p, 0, (*const Guid, *mut *mut c_void) -> Hresult, iid, &mut out);
    if hr < 0 {
        out = core::ptr::null_mut();
    }
    (hr, out)
}

pub(crate) fn add_ref(p: *mut c_void) -> u32 {
    if p.is_null() {
        return 0;
    }
    vcall!(p, 1, () -> u32)
}

pub(crate) fn release(p: *mut c_void) -> u32 {
    if p.is_null() {
        return 0;
    }
    vcall!(p, 2, () -> u32)
}

/// A NUL-terminated wide string's text (lossy).
///
/// # Safety
/// `p` is null or NUL-terminated.
pub(crate) unsafe fn from_wide(p: *const u16) -> Option<String> {
    if p.is_null() {
        return None;
    }
    let mut n = 0;
    // SAFETY: the caller's string.
    while unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    // SAFETY: `n` characters.
    Some(String::from_utf16_lossy(unsafe { core::slice::from_raw_parts(p, n) }))
}

/// A NUL-terminated UTF-16 copy.
pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(core::iter::once(0)).collect()
}

/// A new `BSTR` holding `s` (NULL on no memory).
pub(crate) fn bstr(s: &str) -> *mut u16 {
    let w: Vec<u16> = s.encode_utf16().collect();
    // SAFETY: `w` holds `len` characters.
    unsafe { crate::SysAllocStringLen(w.as_ptr(), w.len() as u32) }
}

/// A `BSTR`'s text (`None` for NULL).
///
/// # Safety
/// `b` is null or a `BSTR`.
pub(crate) unsafe fn bstr_text(b: *const u16) -> Option<String> {
    if b.is_null() {
        return None;
    }
    // SAFETY: a BSTR has its length before it.
    let n = unsafe { crate::SysStringLen(b as *mut u16) } as usize;
    // SAFETY: as above.
    Some(String::from_utf16_lossy(unsafe { core::slice::from_raw_parts(b, n) }))
}

/// Write `v` to an out pointer that may be NULL.
pub(crate) fn put<T>(slot: *mut T, v: T) {
    if !slot.is_null() {
        // SAFETY: the caller's out pointer.
        unsafe { slot.write(v) };
    }
}

// ----- kernel32, advapi32 ------------------------------------------------------------------------

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    pub(crate) fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        sa: *const c_void,
        disposition: u32,
        flags: u32,
        template: isize,
    ) -> isize;
    pub(crate) fn ReadFile(h: isize, buf: *mut u8, n: u32, read: *mut u32, ov: *mut c_void) -> i32;
    pub(crate) fn WriteFile(h: isize, buf: *const u8, n: u32, written: *mut u32, ov: *mut c_void) -> i32;
    pub(crate) fn CloseHandle(h: isize) -> i32;
    pub(crate) fn GetLastError() -> u32;
    pub(crate) fn GetFileAttributesW(name: *const u16) -> u32;
    pub(crate) fn LoadLibraryW(name: *const u16) -> usize;
    pub(crate) fn LoadLibraryExW(name: *const u16, file: isize, flags: u32) -> usize;
    pub(crate) fn FreeLibrary(m: usize) -> i32;
    pub(crate) fn GetProcAddress(m: usize, name: *const u8) -> usize;
    pub(crate) fn FindResourceW(m: usize, name: *const u16, kind: *const u16) -> usize;
    pub(crate) fn LoadResource(m: usize, res: usize) -> usize;
    pub(crate) fn LockResource(h: usize) -> *const u8;
    pub(crate) fn SizeofResource(m: usize, res: usize) -> u32;
    pub(crate) fn TlsAlloc() -> u32;
    pub(crate) fn TlsGetValue(i: u32) -> *mut c_void;
    pub(crate) fn TlsSetValue(i: u32, v: *mut c_void) -> i32;
    pub(crate) fn GetCurrentProcessId() -> u32;
    pub(crate) fn Sleep(ms: u32);
}

#[cfg_attr(target_arch = "x86", link(name = "advapi32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "advapi32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn RegOpenKeyExW(key: isize, sub: *const u16, options: u32, access: u32, out: *mut isize) -> i32;
    fn RegCreateKeyExW(
        key: isize,
        sub: *const u16,
        reserved: u32,
        class: *const u16,
        options: u32,
        access: u32,
        sa: *const c_void,
        out: *mut isize,
        disp: *mut u32,
    ) -> i32;
    fn RegSetValueExW(key: isize, name: *const u16, reserved: u32, kind: u32, data: *const u8, n: u32) -> i32;
    fn RegQueryValueExW(key: isize, name: *const u16, reserved: *mut u32, kind: *mut u32, data: *mut u8, n: *mut u32)
    -> i32;
    fn RegEnumKeyExW(
        key: isize,
        index: u32,
        name: *mut u16,
        n: *mut u32,
        reserved: *mut u32,
        class: *mut u16,
        nc: *mut u32,
        time: *mut u64,
    ) -> i32;
    fn RegDeleteKeyW(key: isize, sub: *const u16) -> i32;
    fn RegCloseKey(key: isize) -> i32;
}

pub(crate) const HKEY_CURRENT_USER: isize = 0x8000_0001_u32 as i32 as isize;
pub(crate) const HKEY_LOCAL_MACHINE: isize = 0x8000_0002_u32 as i32 as isize;
const KEY_READ: u32 = 0x2_0019;
const KEY_ALL: u32 = 0xF_003F;

/// The class roots, in lookup order: the user's classes, then the machine's (the merged view).
pub(crate) const CLASS_ROOTS: [(isize, &str); 2] =
    [(HKEY_CURRENT_USER, "Software\\Classes\\"), (HKEY_LOCAL_MACHINE, "SOFTWARE\\Classes\\")];

/// An open registry key, closed on drop.
pub(crate) struct Key(pub(crate) isize);

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: a key this module opened.
        unsafe { RegCloseKey(self.0) };
    }
}

impl Key {
    pub(crate) fn open(root: isize, path: &str) -> Option<Key> {
        let p = wide(path);
        let mut k = 0isize;
        // SAFETY: a NUL-terminated path, a live out pointer.
        (unsafe { RegOpenKeyExW(root, p.as_ptr(), 0, KEY_READ, &mut k) } == 0).then_some(Key(k))
    }

    pub(crate) fn create(root: isize, path: &str) -> Option<Key> {
        let p = wide(path);
        let mut k = 0isize;
        // SAFETY: as above.
        let r = unsafe {
            RegCreateKeyExW(root, p.as_ptr(), 0, core::ptr::null(), 0, KEY_ALL, core::ptr::null(), &mut k, core::ptr::null_mut())
        };
        (r == 0).then_some(Key(k))
    }

    /// A string value (`None`: the default value).
    pub(crate) fn get(&self, name: Option<&str>) -> Option<String> {
        let n = name.map(wide);
        let np = n.as_ref().map_or(core::ptr::null(), |v| v.as_ptr());
        let mut buf = alloc::vec![0u16; 1024];
        let mut bytes = (buf.len() * 2) as u32;
        let mut kind = 0u32;
        // SAFETY: a buffer of `bytes` bytes.
        let r = unsafe {
            RegQueryValueExW(self.0, np, core::ptr::null_mut(), &mut kind, buf.as_mut_ptr().cast(), &mut bytes)
        };
        if r != 0 || (kind != 1 && kind != 2) {
            return None;
        }
        let mut len = bytes as usize / 2;
        while len > 0 && buf[len - 1] == 0 {
            len -= 1;
        }
        Some(String::from_utf16_lossy(&buf[..len]))
    }

    pub(crate) fn set(&self, name: Option<&str>, value: &str) -> bool {
        let n = name.map(wide);
        let np = n.as_ref().map_or(core::ptr::null(), |v| v.as_ptr());
        let v = wide(value);
        // SAFETY: REG_SZ data with its NUL.
        unsafe { RegSetValueExW(self.0, np, 0, 1, v.as_ptr().cast(), (v.len() * 2) as u32) == 0 }
    }

    /// The names of the subkeys.
    pub(crate) fn subkeys(&self) -> Vec<String> {
        let mut out = Vec::new();
        for i in 0.. {
            let mut buf = [0u16; 260];
            let mut n = buf.len() as u32;
            // SAFETY: a buffer of `n` characters.
            let r = unsafe {
                RegEnumKeyExW(
                    self.0,
                    i,
                    buf.as_mut_ptr(),
                    &mut n,
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                )
            };
            if r != 0 {
                break;
            }
            out.push(String::from_utf16_lossy(&buf[..n as usize]));
        }
        out
    }
}

/// Delete `path` below `root` with everything in it (and the key itself).
pub(crate) fn delete_tree(root: isize, path: &str) {
    if let Some(k) = Key::open(root, path) {
        let kids = k.subkeys();
        drop(k);
        for sub in kids {
            delete_tree(root, &alloc::format!("{path}\\{sub}"));
        }
    }
    let p = wide(path);
    // SAFETY: a NUL-terminated path; deleting a key that is already gone fails harmlessly.
    unsafe { RegDeleteKeyW(root, p.as_ptr()) };
}

/// A class-root key: the user's when it exists, else the machine's.
pub(crate) fn open_class(path: &str) -> Option<Key> {
    CLASS_ROOTS.iter().find_map(|&(root, prefix)| Key::open(root, &alloc::format!("{prefix}{path}")))
}

// ----- files -------------------------------------------------------------------------------------

const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const OPEN_EXISTING: u32 = 3;
const CREATE_ALWAYS: u32 = 2;
const INVALID: isize = -1;

/// The bytes of a file, or the Win32 error.
pub(crate) fn read_file(path: &str) -> Result<Vec<u8>, u32> {
    let p = wide(path);
    // SAFETY: a NUL-terminated name; share read|write|delete.
    let h = unsafe { CreateFileW(p.as_ptr(), GENERIC_READ, 7, core::ptr::null(), OPEN_EXISTING, 0x80, 0) };
    if h == INVALID {
        // SAFETY: no arguments.
        return Err(unsafe { GetLastError() });
    }
    let mut out = Vec::new();
    let mut buf = alloc::vec![0u8; 65536];
    loop {
        let mut n = 0u32;
        // SAFETY: a buffer of its length.
        let ok = unsafe { ReadFile(h, buf.as_mut_ptr(), buf.len() as u32, &mut n, core::ptr::null_mut()) };
        if ok == 0 || n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n as usize]);
    }
    // SAFETY: our handle.
    unsafe { CloseHandle(h) };
    Ok(out)
}

/// Write `bytes` as the whole file; the Win32 error on failure.
pub(crate) fn write_file(path: &str, bytes: &[u8]) -> Result<(), u32> {
    let p = wide(path);
    // SAFETY: as above.
    let h = unsafe { CreateFileW(p.as_ptr(), GENERIC_WRITE, 0, core::ptr::null(), CREATE_ALWAYS, 0x80, 0) };
    if h == INVALID {
        // SAFETY: no arguments.
        return Err(unsafe { GetLastError() });
    }
    let mut at = 0;
    let mut ok = true;
    while at < bytes.len() {
        let mut n = 0u32;
        // SAFETY: the rest of `bytes`.
        if unsafe { WriteFile(h, bytes[at..].as_ptr(), (bytes.len() - at) as u32, &mut n, core::ptr::null_mut()) } == 0
            || n == 0
        {
            ok = false;
            break;
        }
        at += n as usize;
    }
    // SAFETY: our handle.
    unsafe { CloseHandle(h) };
    if ok { Ok(()) } else { Err(29) }
}

/// Whether a file (not a directory) exists.
pub(crate) fn file_exists(path: &str) -> bool {
    let p = wide(path);
    // SAFETY: a NUL-terminated name.
    let a = unsafe { GetFileAttributesW(p.as_ptr()) };
    a != u32::MAX && a & 0x10 == 0
}

// ----- ole32, looked up on first use ---------------------------------------------------------------

static OLE32: [AtomicUsize; 9] = [const { AtomicUsize::new(0) }; 9];
const OLE32_NAMES: [&[u8]; 9] = [
    b"CoTaskMemAlloc\0",
    b"CoTaskMemFree\0",
    b"CoMarshalInterface\0",
    b"CoUnmarshalInterface\0",
    b"CoReleaseMarshalData\0",
    b"CreateStreamOnHGlobal\0",
    b"CoGetPSClsid\0",
    b"CoGetMarshalSizeMax\0",
    b"GetHGlobalFromStream\0",
];
pub(crate) const CO_TASK_MEM_ALLOC: usize = 0;
pub(crate) const CO_TASK_MEM_FREE: usize = 1;
pub(crate) const CO_MARSHAL_INTERFACE: usize = 2;
pub(crate) const CO_UNMARSHAL_INTERFACE: usize = 3;
pub(crate) const CO_RELEASE_MARSHAL_DATA: usize = 4;
pub(crate) const CREATE_STREAM_ON_HGLOBAL: usize = 5;

/// The address of an `ole32` function (0 when it cannot be found).
pub(crate) fn ole32(which: usize) -> usize {
    let p = OLE32[which].load(Ordering::Acquire);
    if p != 0 {
        return p;
    }
    let name = wide("ole32.dll");
    // SAFETY: kernel32 calls with NUL-terminated names.
    let p = unsafe { GetProcAddress(LoadLibraryW(name.as_ptr()), OLE32_NAMES[which].as_ptr()) };
    OLE32[which].store(p, Ordering::Release);
    p
}

/// `CoTaskMemAlloc`.
pub(crate) fn task_alloc(n: usize) -> *mut c_void {
    let f = ole32(CO_TASK_MEM_ALLOC);
    if f == 0 {
        return core::ptr::null_mut();
    }
    // SAFETY: ole32's CoTaskMemAlloc.
    unsafe { core::mem::transmute::<usize, unsafe extern "system" fn(usize) -> *mut c_void>(f)(n) }
}

/// `CoTaskMemFree`.
pub(crate) fn task_free(p: *mut c_void) {
    let f = ole32(CO_TASK_MEM_FREE);
    if f != 0 && !p.is_null() {
        // SAFETY: ole32's CoTaskMemFree.
        unsafe { core::mem::transmute::<usize, unsafe extern "system" fn(*mut c_void)>(f)(p) }
    }
}

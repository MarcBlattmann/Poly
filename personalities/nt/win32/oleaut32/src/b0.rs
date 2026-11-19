//! What Google's Chrome installers and their updater import from `oleaut32.dll` by ordinal (batch
//! B0 of `docs/windows/chrome-installer-plan.md`): `SystemTimeToVariantTime` (184) and
//! `OleLoadPicturePath` (424); `LoadTypeLib` (161) is the type library loader's (ADR 0220,
//! `typelib/load.rs`). The decisions are `poly_kernelbase::oleauto`'s; the sheets are
//! `specs/windows/oleaut32.dll/{SystemTimeToVariantTime,OleLoadPicturePath}.yaml`.

use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};
use poly_kernelbase::oleauto::{self, SystemTime};

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetFileAttributesW(name: *const u16) -> u32;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        sa: *const u8,
        disp: u32,
        flags: u32,
        tpl: isize,
    ) -> isize;
    fn ReadFile(h: isize, buf: *mut u8, n: u32, read: *mut u32, ov: *mut u8) -> i32;
    fn GetFileSizeEx(h: isize, size: *mut i64) -> i32;
    fn CloseHandle(h: isize) -> i32;
    fn GetProcessHeap() -> *mut u8;
    fn HeapAlloc(heap: *mut u8, flags: u32, bytes: usize) -> *mut u8;
    fn HeapFree(heap: *mut u8, flags: u32, p: *mut u8) -> i32;
}

#[cfg_attr(target_arch = "x86", link(name = "gdi32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "gdi32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn CreateBitmap(w: i32, h: i32, planes: u32, bpp: u32, bits: *const u8) -> isize;
    fn DeleteObject(h: isize) -> i32;
}

const S_OK: u32 = 0;
const E_INVALIDARG: u32 = 0x8007_0057;
const E_POINTER: u32 = 0x8000_4003;
const E_NOINTERFACE: u32 = 0x8000_4002;
const E_NOTIMPL: u32 = 0x8000_4001;
const E_OUTOFMEMORY: u32 = 0x8007_000E;
/// `DISP_E_UNKNOWNNAME`, `DISP_E_MEMBERNOTFOUND` (`winerror.h`).
const DISP_E_UNKNOWNNAME: u32 = 0x8002_0006;
const DISP_E_MEMBERNOTFOUND: u32 = 0x8002_0003;
const INVALID_FILE_ATTRIBUTES: u32 = u32::MAX;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const GENERIC_READ: u32 = 0x8000_0000;
const OPEN_EXISTING: u32 = 3;
/// The largest picture file read (a guard; the installers' pictures are small).
const MAX_PICTURE_FILE: i64 = 64 << 20;

// ----- SystemTimeToVariantTime (ordinal 184) ---------------------------------------------------

/// `SystemTimeToVariantTime(lpSystemTime, pvtime)`: 1 and the date, or 0 with the output
/// untouched for fields out of range (or a NULL pointer).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SystemTimeToVariantTime(st: *const u16, out: *mut f64) -> i32 {
    if st.is_null() || out.is_null() {
        return 0;
    }
    let mut w = [0u16; 8];
    // SAFETY: the caller's SYSTEMTIME, eight WORDs (2-byte aligned).
    unsafe { core::ptr::copy_nonoverlapping(st, w.as_mut_ptr(), 8) };
    match oleauto::system_time_to_variant_time(&SystemTime::from_words(w)) {
        Some(v) => {
            // SAFETY: the caller's DOUBLE.
            unsafe { out.write_unaligned(v) };
            1
        }
        None => 0,
    }
}

/// A NUL-terminated UTF-16 string as a slice (without the NUL).
///
/// # Safety
/// `p` is a readable NUL-terminated string.
unsafe fn wide_slice<'a>(p: *const u16) -> &'a [u16] {
    let mut n = 0;
    // SAFETY: the caller's string ends in a NUL.
    unsafe {
        while *p.add(n) != 0 {
            n += 1;
        }
        core::slice::from_raw_parts(p, n)
    }
}

// ----- OleLoadPicturePath (ordinal 424) --------------------------------------------------------

type Guid = [u8; 16];

const fn guid(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> Guid {
    let a = d1.to_le_bytes();
    let b = d2.to_le_bytes();
    let c = d3.to_le_bytes();
    [a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d4[0], d4[1], d4[2], d4[3], d4[4], d4[5], d4[6], d4[7]]
}

const IID_IUNKNOWN: Guid = guid(0, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IDISPATCH: Guid = guid(0x0002_0400, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
/// `IID_IPicture` and `IID_IPictureDisp` (`ocidl.h`).
const IID_IPICTURE: Guid = guid(0x7BF8_0980, 0xBF32, 0x101A, [0x8B, 0xBB, 0x00, 0xAA, 0x00, 0x30, 0x0C, 0xAB]);
const IID_IPICTUREDISP: Guid = guid(0x7BF8_0981, 0xBF32, 0x101A, [0x8B, 0xBB, 0x00, 0xAA, 0x00, 0x30, 0x0C, 0xAB]);

/// A picture object: `IPicture` and `IPictureDisp` (an `IDispatch`) over one bitmap, one count.
#[repr(C)]
struct Picture {
    picture: *const PictureVtbl,
    dispatch: *const DispatchVtbl,
    refs: AtomicU32,
    bitmap: isize,
    width: i32,
    height: i32,
}

type Pic = *mut Picture;

/// `IPicture` (`ocidl.h`): `IUnknown`, then `get_Handle` (3) ... `get_Attributes` (16). Every slot
/// has its own arguments: a 32-bit `stdcall` callee removes them.
#[repr(C)]
struct PictureVtbl {
    qi: unsafe extern "system" fn(Pic, *const Guid, *mut *mut c_void) -> u32,
    add_ref: unsafe extern "system" fn(Pic) -> u32,
    release: unsafe extern "system" fn(Pic) -> u32,
    get_handle: unsafe extern "system" fn(Pic, *mut u32) -> u32,
    get_hpal: unsafe extern "system" fn(Pic, *mut u32) -> u32,
    get_type: unsafe extern "system" fn(Pic, *mut i16) -> u32,
    get_width: unsafe extern "system" fn(Pic, *mut i32) -> u32,
    get_height: unsafe extern "system" fn(Pic, *mut i32) -> u32,
    #[allow(clippy::type_complexity)]
    render: unsafe extern "system" fn(Pic, isize, i32, i32, i32, i32, i32, i32, i32, i32, *const u8) -> u32,
    set_hpal: unsafe extern "system" fn(Pic, u32) -> u32,
    get_cur_dc: unsafe extern "system" fn(Pic, *mut isize) -> u32,
    select_picture: unsafe extern "system" fn(Pic, isize, *mut isize, *mut u32) -> u32,
    get_keep: unsafe extern "system" fn(Pic, *mut i32) -> u32,
    put_keep: unsafe extern "system" fn(Pic, i32) -> u32,
    picture_changed: unsafe extern "system" fn(Pic) -> u32,
    save_as_file: unsafe extern "system" fn(Pic, *mut c_void, i32, *mut i32) -> u32,
    get_attributes: unsafe extern "system" fn(Pic, *mut u32) -> u32,
}

/// `IDispatch` of the same object (`this` is the second pointer of [`Picture`]).
#[repr(C)]
struct DispatchVtbl {
    qi: unsafe extern "system" fn(Pic, *const Guid, *mut *mut c_void) -> u32,
    add_ref: unsafe extern "system" fn(Pic) -> u32,
    release: unsafe extern "system" fn(Pic) -> u32,
    get_type_info_count: unsafe extern "system" fn(Pic, *mut u32) -> u32,
    get_type_info: unsafe extern "system" fn(Pic, u32, u32, *mut *mut c_void) -> u32,
    get_ids_of_names: unsafe extern "system" fn(Pic, *const Guid, *const *const u16, u32, u32, *mut i32) -> u32,
    #[allow(clippy::type_complexity)]
    invoke: unsafe extern "system" fn(
        Pic,
        i32,
        *const Guid,
        u32,
        u16,
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *mut u32,
    ) -> u32,
}

// SAFETY: tables of function pointers.
unsafe impl Sync for PictureVtbl {}
// SAFETY: as above.
unsafe impl Sync for DispatchVtbl {}

static PICTURE_VTBL: PictureVtbl = PictureVtbl {
    qi: pic_qi,
    add_ref: pic_add_ref,
    release: pic_release,
    get_handle: pic_handle,
    get_hpal: pic_hpal,
    get_type: pic_type,
    get_width: pic_width,
    get_height: pic_height,
    render: pic_render,
    set_hpal: pic_set_hpal,
    get_cur_dc: pic_cur_dc,
    select_picture: pic_select,
    get_keep: pic_get_keep,
    put_keep: pic_put_keep,
    picture_changed: pic_changed,
    save_as_file: pic_save,
    get_attributes: pic_attributes,
};

static DISPATCH_VTBL: DispatchVtbl = DispatchVtbl {
    qi: disp_qi,
    add_ref: disp_add_ref,
    release: disp_release,
    get_type_info_count: disp_count,
    get_type_info: disp_type_info,
    get_ids_of_names: disp_ids,
    invoke: disp_invoke,
};

/// The object of a dispatch pointer (the second field).
fn from_disp(this: Pic) -> Pic {
    (this as usize - core::mem::size_of::<usize>()) as Pic
}

unsafe extern "system" fn pic_qi(this: Pic, iid: *const Guid, out: *mut *mut c_void) -> u32 {
    if out.is_null() {
        return E_POINTER;
    }
    if iid.is_null() {
        // SAFETY: the caller's out-pointer.
        unsafe { out.write(core::ptr::null_mut()) };
        return E_INVALIDARG;
    }
    // SAFETY: the caller's GUID.
    let iid = unsafe { *iid };
    let p: *mut c_void = if iid == IID_IUNKNOWN || iid == IID_IPICTURE {
        this.cast()
    } else if iid == IID_IDISPATCH || iid == IID_IPICTUREDISP {
        // SAFETY: the object's second field.
        unsafe { core::ptr::addr_of_mut!((*this).dispatch).cast() }
    } else {
        // SAFETY: the caller's out-pointer.
        unsafe { out.write(core::ptr::null_mut()) };
        return E_NOINTERFACE;
    };
    // SAFETY: a live object.
    unsafe {
        (*this).refs.fetch_add(1, Ordering::AcqRel);
        out.write(p);
    }
    S_OK
}

unsafe extern "system" fn pic_add_ref(this: Pic) -> u32 {
    // SAFETY: a live object.
    unsafe { (*this).refs.fetch_add(1, Ordering::AcqRel) + 1 }
}

unsafe extern "system" fn pic_release(this: Pic) -> u32 {
    // SAFETY: a live object; the last reference frees it.
    unsafe {
        let n = (*this).refs.fetch_sub(1, Ordering::AcqRel) - 1;
        if n == 0 {
            DeleteObject((*this).bitmap);
            HeapFree(GetProcessHeap(), 0, this.cast());
        }
        n
    }
}

unsafe extern "system" fn pic_handle(this: Pic, out: *mut u32) -> u32 {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: a live object; the caller's OLE_HANDLE (a UINT: GDI handles fit in 32 bits).
    unsafe { out.write_unaligned((*this).bitmap as u32) };
    S_OK
}

unsafe extern "system" fn pic_hpal(_: Pic, out: *mut u32) -> u32 {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's OLE_HANDLE.
    unsafe { out.write_unaligned(0) };
    S_OK
}

unsafe extern "system" fn pic_type(_: Pic, out: *mut i16) -> u32 {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's SHORT.
    unsafe { out.write_unaligned(oleauto::PICTYPE_BITMAP) };
    S_OK
}

unsafe extern "system" fn pic_width(this: Pic, out: *mut i32) -> u32 {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: a live object; the caller's LONG.
    unsafe { out.write_unaligned((*this).width) };
    S_OK
}

unsafe extern "system" fn pic_height(this: Pic, out: *mut i32) -> u32 {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: a live object; the caller's LONG.
    unsafe { out.write_unaligned((*this).height) };
    S_OK
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn pic_render(
    _: Pic,
    _: isize,
    _: i32,
    _: i32,
    _: i32,
    _: i32,
    _: i32,
    _: i32,
    _: i32,
    _: i32,
    _: *const u8,
) -> u32 {
    E_NOTIMPL
}

unsafe extern "system" fn pic_set_hpal(_: Pic, _: u32) -> u32 {
    E_NOTIMPL
}

unsafe extern "system" fn pic_cur_dc(_: Pic, out: *mut isize) -> u32 {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's HDC.
    unsafe { out.write_unaligned(0) };
    S_OK
}

unsafe extern "system" fn pic_select(_: Pic, _: isize, _: *mut isize, _: *mut u32) -> u32 {
    E_NOTIMPL
}

unsafe extern "system" fn pic_get_keep(_: Pic, out: *mut i32) -> u32 {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's BOOL.
    unsafe { out.write_unaligned(0) };
    S_OK
}

unsafe extern "system" fn pic_put_keep(_: Pic, _: i32) -> u32 {
    S_OK
}

unsafe extern "system" fn pic_changed(_: Pic) -> u32 {
    S_OK
}

unsafe extern "system" fn pic_save(_: Pic, _: *mut c_void, _: i32, _: *mut i32) -> u32 {
    E_NOTIMPL
}

unsafe extern "system" fn pic_attributes(_: Pic, out: *mut u32) -> u32 {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's DWORD.
    unsafe { out.write_unaligned(0) };
    S_OK
}

unsafe extern "system" fn disp_qi(this: Pic, iid: *const Guid, out: *mut *mut c_void) -> u32 {
    // SAFETY: as `IPicture`'s.
    unsafe { pic_qi(from_disp(this), iid, out) }
}

unsafe extern "system" fn disp_add_ref(this: Pic) -> u32 {
    // SAFETY: as `IPicture`'s.
    unsafe { pic_add_ref(from_disp(this)) }
}

unsafe extern "system" fn disp_release(this: Pic) -> u32 {
    // SAFETY: as `IPicture`'s.
    unsafe { pic_release(from_disp(this)) }
}

unsafe extern "system" fn disp_count(_: Pic, out: *mut u32) -> u32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's UINT.
    unsafe { out.write_unaligned(0) };
    S_OK
}

unsafe extern "system" fn disp_type_info(_: Pic, _: u32, _: u32, out: *mut *mut c_void) -> u32 {
    if !out.is_null() {
        // SAFETY: the caller's out-pointer.
        unsafe { out.write(core::ptr::null_mut()) };
    }
    E_NOTIMPL
}

unsafe extern "system" fn disp_ids(_: Pic, _: *const Guid, _: *const *const u16, _: u32, _: u32, _: *mut i32) -> u32 {
    DISP_E_UNKNOWNNAME
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn disp_invoke(
    _: Pic,
    _: i32,
    _: *const Guid,
    _: u32,
    _: u16,
    _: *mut c_void,
    _: *mut c_void,
    _: *mut c_void,
    _: *mut u32,
) -> u32 {
    DISP_E_MEMBERNOTFOUND
}

/// The bytes of the file at `path` (NUL-terminated), or `None` when it cannot be read.
fn read_file(path: &[u16]) -> Option<alloc::vec::Vec<u8>> {
    // SAFETY: a NUL-terminated name; the handle is closed below.
    let h = unsafe { CreateFileW(path.as_ptr(), GENERIC_READ, 3, core::ptr::null(), OPEN_EXISTING, 0, 0) };
    if h == -1 {
        return None;
    }
    let mut size = 0i64;
    // SAFETY: an open handle and a local.
    let ok = unsafe { GetFileSizeEx(h, &mut size) } != 0 && (0..=MAX_PICTURE_FILE).contains(&size);
    let mut data = alloc::vec![0u8; if ok { size as usize } else { 0 }];
    let mut done = 0usize;
    while ok && done < data.len() {
        let mut got = 0u32;
        // SAFETY: the rest of `data`.
        let r = unsafe {
            ReadFile(h, data.as_mut_ptr().add(done), (data.len() - done) as u32, &mut got, core::ptr::null_mut())
        };
        if r == 0 || got == 0 {
            break;
        }
        done += got as usize;
    }
    // SAFETY: the handle opened above.
    unsafe { CloseHandle(h) };
    data.truncate(done);
    ok.then_some(data)
}

/// `OleLoadPicturePath(szURLorPath, punkCaller, dwReserved, clrReserved, riid, ppvRet)`: a picture
/// from a file named by its full path, of the format its content shows (a bitmap or a GIF, as a
/// `PICTYPE_BITMAP` picture sized in HIMETRIC at 96 DPI). A NULL name is `E_INVALIDARG` with
/// `*ppvRet` untouched; every other failure sets it to NULL: a name that is not a full path is a
/// URL of no protocol (`INET_E_UNKNOWN_PROTOCOL`), a missing file `INET_E_OBJECT_NOT_FOUND`, a
/// directory `INET_E_REDIRECT_TO_DIR`, a file that is no picture `E_FAIL`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn OleLoadPicturePath(
    path: *const u16,
    _caller: *mut c_void,
    _reserved: u32,
    _colour: u32,
    iid: *const Guid,
    out: *mut *mut c_void,
) -> u32 {
    if path.is_null() {
        return E_INVALIDARG;
    }
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out-pointer and NUL-terminated name.
    let name = unsafe {
        out.write(core::ptr::null_mut());
        wide_slice(path)
    };
    if !oleauto::picture_path_is_file(name) {
        return oleauto::INET_E_UNKNOWN_PROTOCOL;
    }
    // SAFETY: the caller's NUL-terminated name.
    let attrs = unsafe { GetFileAttributesW(path) };
    if attrs == INVALID_FILE_ATTRIBUTES {
        return oleauto::INET_E_OBJECT_NOT_FOUND;
    }
    if attrs & FILE_ATTRIBUTE_DIRECTORY != 0 {
        return oleauto::INET_E_REDIRECT_TO_DIR;
    }
    let mut z = name.to_vec();
    z.push(0);
    let Some(data) = read_file(&z) else { return oleauto::INET_E_OBJECT_NOT_FOUND };
    let Some(picture) = oleauto::decode_picture(&data) else { return oleauto::E_FAIL };
    // SAFETY: 32-bit rows of the picture's size.
    let bitmap =
        unsafe { CreateBitmap(picture.width as i32, picture.height as i32, 1, 32, picture.pixels.as_ptr().cast()) };
    if bitmap == 0 {
        return E_OUTOFMEMORY;
    }
    // SAFETY: the process heap; the object is written whole before use.
    let obj = unsafe { HeapAlloc(GetProcessHeap(), 0, core::mem::size_of::<Picture>()) }.cast::<Picture>();
    if obj.is_null() {
        // SAFETY: the bitmap made above.
        unsafe { DeleteObject(bitmap) };
        return E_OUTOFMEMORY;
    }
    // SAFETY: a fresh block of the object's size (the heap aligns to 16).
    unsafe {
        obj.write(Picture {
            picture: &PICTURE_VTBL,
            dispatch: &DISPATCH_VTBL,
            refs: AtomicU32::new(1),
            bitmap,
            width: oleauto::himetric(picture.width),
            height: oleauto::himetric(picture.height),
        });
        let hr = pic_qi(obj, iid, out);
        pic_release(obj);
        hr
    }
}

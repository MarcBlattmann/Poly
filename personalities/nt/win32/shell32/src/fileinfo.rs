//! `SHGetFileInfoW` (sheet `specs/windows/shell32.dll/SHGetFileInfoW.yaml`): the type name, the
//! display name, the attributes, the icon and the executable type of a file, folder or drive root.
//! The rules are `poly_kernelbase::fileinfo`'s; this file looks the path up, keeps the system image
//! lists (one of 32 x 32 icons, one of 16 x 16, with the same index for each kind of file) and
//! fills the caller's `SHFILEINFOW`.
//!
//! The image lists are `comctl32`'s, loaded at the first call that needs them (`shell32` does not
//! import `comctl32`, so programs that never ask for an icon do not load it).

use crate::com::wstr;
use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};
use poly_kernelbase::fileinfo::{self as logic, Icon, Kind, shgfi};

/// `SHFILEINFOW` (`shellapi.h`).
#[repr(C)]
pub struct ShFileInfoW {
    icon: usize,
    icon_index: i32,
    attributes: u32,
    display_name: [u16; 260],
    type_name: [u16; 80],
}

/// `ICONINFO` (`winuser.h`).
#[repr(C)]
struct IconInfo {
    is_icon: i32,
    hotspot_x: u32,
    hotspot_y: u32,
    mask: usize,
    color: usize,
}

/// `WIN32_FIND_DATAW` (`minwinbase.h`), 592 bytes: the attributes come first, the name at 44.
#[repr(C)]
struct FindData {
    attributes: u32,
    times_and_sizes: [u32; 10],
    name: [u16; 260],
    short_name: [u16; 14],
    rest: [u32; 3],
}

const INVALID_FILE_ATTRIBUTES: u32 = u32::MAX;
const INVALID_HANDLE_VALUE: usize = usize::MAX;
/// `ILC_MASK | ILC_COLOR32` (`commctrl.h`).
const ILC_FLAGS: u32 = 0x1 | 0x20;

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetFileAttributesW(name: *const u16) -> u32;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        sa: *mut c_void,
        disposition: u32,
        flags: u32,
        template: usize,
    ) -> usize;
    fn ReadFile(file: usize, buffer: *mut u8, to_read: u32, read: *mut u32, overlapped: *mut u8) -> i32;
    fn CloseHandle(handle: usize) -> i32;
    fn FindFirstFileW(name: *const u16, data: *mut u8) -> usize;
    fn FindNextFileW(find: usize, data: *mut u8) -> i32;
    fn FindClose(find: usize) -> i32;
    fn LoadLibraryW(name: *const u16) -> usize;
    fn GetProcAddress(module: usize, name: *const u8) -> usize;
    fn AcquireSRWLockExclusive(lock: *mut usize);
    fn ReleaseSRWLockExclusive(lock: *mut usize);
}

#[cfg_attr(target_arch = "x86", link(name = "user32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "user32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn CreateIconIndirect(info: *const IconInfo) -> usize;
    fn DestroyIcon(icon: usize) -> i32;
}

#[cfg_attr(target_arch = "x86", link(name = "gdi32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "gdi32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn CreateBitmap(width: i32, height: i32, planes: u32, bits_per_pixel: u32, bits: *const c_void) -> usize;
    fn DeleteObject(object: usize) -> i32;
}

type ImageListCreate = unsafe extern "system" fn(i32, i32, u32, i32, i32) -> usize;
type ImageListAddIcon = unsafe extern "system" fn(usize, usize) -> i32;
type ImageListGetIcon = unsafe extern "system" fn(usize, i32, u32) -> usize;

/// The system image lists (32 x 32, 16 x 16) and `ImageList_GetIcon`, once made.
static LARGE: AtomicUsize = AtomicUsize::new(0);
static SMALL: AtomicUsize = AtomicUsize::new(0);
static GET_ICON: AtomicUsize = AtomicUsize::new(0);
/// An SRW lock that makes them once.
static mut LOCK: usize = 0;

/// `name` with a NUL, as ASCII.
fn wide_z<const N: usize>(name: &str) -> [u16; N] {
    let mut out = [0u16; N];
    for (o, b) in out.iter_mut().zip(name.bytes()) {
        *o = u16::from(b);
    }
    out
}

/// An icon of `size` x `size` pixels with the picture of `icon`; the caller destroys it.
fn make_icon(icon: Icon, size: u32) -> usize {
    let px = logic::icon_pixels(icon, size);
    // The mask: 1 (screen shows through) where the picture is transparent; rows of whole words.
    let stride = (size as usize).div_ceil(16) * 2;
    let mut mask = alloc::vec![0u8; stride * size as usize];
    for (i, &c) in px.iter().enumerate() {
        if c >> 24 == 0 {
            let (x, y) = (i % size as usize, i / size as usize);
            mask[y * stride + x / 8] |= 0x80 >> (x % 8);
        }
    }
    // SAFETY: gdi32 copies the bits (size * size 32-bit pixels, the mask's rows).
    let (color, mono) = unsafe {
        (
            CreateBitmap(size as i32, size as i32, 1, 32, px.as_ptr().cast()),
            CreateBitmap(size as i32, size as i32, 1, 1, mask.as_ptr().cast()),
        )
    };
    let info = IconInfo { is_icon: 1, hotspot_x: size / 2, hotspot_y: size / 2, mask: mono, color };
    // SAFETY: an ICONINFO with two bitmaps; the icon keeps copies of them.
    let h = unsafe { CreateIconIndirect(&info) };
    // SAFETY: the bitmaps made above.
    unsafe {
        DeleteObject(color);
        DeleteObject(mono);
    }
    h
}

/// The system image list of 16 x 16 icons (`small`) or 32 x 32 ones, made at the first call: one
/// icon for each [`Icon`], at its index. 0 if `comctl32` cannot be had.
fn image_list(small: bool) -> usize {
    let slot = if small { &SMALL } else { &LARGE };
    let have = slot.load(Ordering::Acquire);
    if have != 0 {
        return have;
    }
    // SAFETY: the SRW lock of this module, released below.
    unsafe { AcquireSRWLockExclusive(&raw mut LOCK) };
    if LARGE.load(Ordering::Acquire) == 0 {
        make_image_lists();
    }
    // SAFETY: acquired above.
    unsafe { ReleaseSRWLockExclusive(&raw mut LOCK) };
    slot.load(Ordering::Acquire)
}

/// Load `comctl32` and fill [`LARGE`], [`SMALL`] and [`GET_ICON`] (with the lock held).
fn make_image_lists() {
    let name: [u16; 13] = wide_z("comctl32.dll");
    // SAFETY: a NUL-terminated name.
    let module = unsafe { LoadLibraryW(name.as_ptr()) };
    if module == 0 {
        return;
    }
    // SAFETY: NUL-terminated names in a loaded module.
    let (create, add, get) = unsafe {
        (
            GetProcAddress(module, c"ImageList_Create".as_ptr().cast()),
            GetProcAddress(module, c"ImageList_AddIcon".as_ptr().cast()),
            GetProcAddress(module, c"ImageList_GetIcon".as_ptr().cast()),
        )
    };
    if create == 0 || add == 0 || get == 0 {
        return;
    }
    // SAFETY: the exports have the prototypes of `commctrl.h`.
    let (create, add): (ImageListCreate, ImageListAddIcon) =
        unsafe { (core::mem::transmute(create), core::mem::transmute(add)) };
    let mut lists = [0usize; 2];
    for (list, size) in lists.iter_mut().zip([32u32, 16]) {
        // SAFETY: comctl32's image list calls on a list made here and icons made here.
        unsafe {
            *list = create(size as i32, size as i32, ILC_FLAGS, Icon::ALL.len() as i32, 4);
            if *list == 0 {
                return;
            }
            for &icon in &Icon::ALL {
                let h = make_icon(icon, size);
                add(*list, h);
                DestroyIcon(h);
            }
        }
    }
    GET_ICON.store(get, Ordering::Release);
    SMALL.store(lists[1], Ordering::Release);
    LARGE.store(lists[0], Ordering::Release);
}

/// An icon of the system image list (the caller destroys it), or 0.
fn icon_of(index: i32, small: bool) -> usize {
    let list = image_list(small);
    let get = GET_ICON.load(Ordering::Acquire);
    if list == 0 || get == 0 {
        return 0;
    }
    // SAFETY: `ImageList_GetIcon` of comctl32 (`commctrl.h`).
    let get: ImageListGetIcon = unsafe { core::mem::transmute(get) };
    // SAFETY: a list made by `make_image_lists`; ILD_NORMAL.
    unsafe { get(list, index, 0) }
}

/// Whether the directory `path` has a subdirectory.
fn has_subfolder(path: &[u16]) -> bool {
    let mut pattern: alloc::vec::Vec<u16> = logic::trimmed(path).to_vec();
    if pattern.last() != Some(&u16::from(b'\\')) {
        pattern.push(u16::from(b'\\'));
    }
    pattern.extend([u16::from(b'*'), 0]);
    // SAFETY: zeroed is a valid WIN32_FIND_DATAW.
    let mut data: FindData = unsafe { core::mem::zeroed() };
    // SAFETY: a NUL-terminated pattern and a WIN32_FIND_DATAW.
    let find = unsafe { FindFirstFileW(pattern.as_ptr(), (&raw mut data).cast()) };
    if find == INVALID_HANDLE_VALUE {
        return false;
    }
    let mut found = false;
    loop {
        let len = data.name.iter().position(|&c| c == 0).unwrap_or(data.name.len());
        let name = &data.name[..len];
        let dots = !name.is_empty() && name.len() <= 2 && name.iter().all(|&c| c == u16::from(b'.'));
        if data.attributes & logic::FILE_ATTRIBUTE_DIRECTORY != 0 && !dots {
            found = true;
            break;
        }
        // SAFETY: the search handle and the structure.
        if unsafe { FindNextFileW(find, (&raw mut data).cast()) } == 0 {
            break;
        }
    }
    // SAFETY: the search handle, closed once.
    unsafe { FindClose(find) };
    found
}

/// The first 4 KiB of the file `path` (NUL-terminated).
fn head_of(path: *const u16) -> alloc::vec::Vec<u8> {
    // SAFETY: a NUL-terminated path; GENERIC_READ, all sharing, OPEN_EXISTING.
    let f = unsafe { CreateFileW(path, 0x8000_0000, 7, core::ptr::null_mut(), 3, 0x80, 0) };
    if f == INVALID_HANDLE_VALUE || f == 0 {
        return alloc::vec::Vec::new();
    }
    let mut buf = alloc::vec![0u8; 4096];
    let mut got = 0u32;
    // SAFETY: the open file and a buffer of 4096 bytes.
    let ok = unsafe { ReadFile(f, buf.as_mut_ptr(), 4096, &mut got, core::ptr::null_mut()) };
    // SAFETY: the handle, closed once.
    unsafe { CloseHandle(f) };
    buf.truncate(if ok != 0 { got as usize } else { 0 });
    buf
}

/// `text` into a fixed field with a NUL (cut to fit).
fn put(field: &mut [u16], text: &[u16]) {
    let n = text.len().min(field.len() - 1);
    field[..n].copy_from_slice(&text[..n]);
    field[n] = 0;
}

/// `SHGetFileInfoW(pszPath, dwFileAttributes, psfi, cbFileInfo, uFlags)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SHGetFileInfoW(
    path: *const u16,
    file_attributes: u32,
    info: *mut ShFileInfoW,
    _cb: u32,
    flags: u32,
) -> usize {
    use shgfi::*;
    if path.is_null() || flags & PIDL != 0 {
        return 0;
    }
    // SAFETY: the caller's NUL-terminated path.
    let p = unsafe { wstr(path) };
    let given = flags & USEFILEATTRIBUTES != 0;
    // The two names are written empty first, whatever is asked and also when the path is not
    // found (the sheet's `flags_results` and `missing_without_attributes`).
    // SAFETY: the caller's SHFILEINFOW, or NULL.
    if let Some(info) = unsafe { info.as_mut() } {
        info.display_name[0] = 0;
        info.type_name[0] = 0;
    }
    let asked = flags & (ICON | DISPLAYNAME | TYPENAME | ATTRIBUTES | ICONLOCATION | EXETYPE | SYSICONINDEX);
    if p.is_empty() {
        return usize::from(asked != 0 || given);
    }
    let directory = if given {
        file_attributes & logic::FILE_ATTRIBUTE_DIRECTORY != 0
    } else {
        // SAFETY: a NUL-terminated path.
        let a = unsafe { GetFileAttributesW(path) };
        if a == INVALID_FILE_ATTRIBUTES {
            return 0;
        }
        a & logic::FILE_ATTRIBUTE_DIRECTORY != 0
    };
    if flags & EXETYPE != 0 {
        return if given || directory { 0 } else { logic::exe_type(&head_of(path)) };
    }
    let kind = logic::kind(p, directory);
    let icon = logic::icon(p, kind);
    let small = flags & SMALLICON != 0;
    // SAFETY: the caller's SHFILEINFOW (NULL: nothing is written); `cbFileInfo` is not checked
    // (the sheet's `cb`).
    if let Some(info) = unsafe { info.as_mut() } {
        if flags & TYPENAME != 0 {
            put(&mut info.type_name, &logic::type_name(p, kind));
        }
        if flags & DISPLAYNAME != 0 {
            put(&mut info.display_name, &logic::display_name(p, kind));
        }
        if flags & ATTRIBUTES != 0 {
            let sub = kind == Kind::Folder && !given && has_subfolder(p);
            info.attributes = logic::attributes(kind, !given, sub);
        }
        if flags & ICONLOCATION != 0 {
            if icon == Icon::Program {
                put(&mut info.display_name, p);
                info.icon_index = 0;
            } else {
                let shell: [u16; 32] = wide_z("C:\\Windows\\System32\\shell32.dll");
                put(&mut info.display_name, &shell[..31]);
                info.icon_index = icon.index();
            }
        }
        if flags & (SYSICONINDEX | ICON) != 0 {
            info.icon_index = icon.index();
        }
        if flags & ICON != 0 {
            info.icon = icon_of(icon.index(), small);
        }
    }
    if flags & SYSICONINDEX != 0 {
        return image_list(small);
    }
    usize::from(asked != 0 || given)
}

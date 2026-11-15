//! `SHGetFileInfoW`: the icon, the system image list index, the display name, the type name, the
//! attributes and the executable type of a file or folder (`specs/windows/shell32.dll/
//! SHGetFileInfoW.yaml`). The rules are `poly_kernelbase::fileinfo`'s and the icons
//! `poly_kernelbase::shellicon`'s (both host-tested); this file asks the file system and keeps
//! the two system image lists (16 and 32 pixels) of the process.

use super::folders::{attributes, has_subdirectory, z};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};
use poly_kernelbase::fileinfo::{self as fi, Kind};
use poly_kernelbase::shellicon;

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *const u8, disposition: u32, flags: u32, template: isize)
    -> isize;
    fn ReadFile(h: isize, buf: *mut u8, n: u32, read: *mut u32, overlapped: *mut u8) -> i32;
    fn CloseHandle(h: isize) -> i32;
}

#[cfg_attr(target_arch = "x86", link(name = "comctl32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "comctl32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn ImageList_Create(cx: i32, cy: i32, flags: u32, initial: i32, grow: i32) -> usize;
    fn ImageList_AddIcon(himl: usize, icon: usize) -> i32;
    fn ImageList_GetIcon(himl: usize, index: i32, flags: u32) -> usize;
}

#[cfg_attr(target_arch = "x86", link(name = "user32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "user32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn CreateIconIndirect(info: *const u8) -> usize;
    fn DestroyIcon(icon: usize) -> i32;
}

#[cfg_attr(target_arch = "x86", link(name = "gdi32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "gdi32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn CreateBitmap(w: i32, h: i32, planes: u32, bpp: u32, bits: *const u8) -> usize;
    fn DeleteObject(h: usize) -> i32;
}

const ILC_MASK: u32 = 0x1;
const ILC_COLOR32: u32 = 0x20;
const GENERIC_READ: u32 = 0x8000_0000;
const FILE_SHARE_READ: u32 = 1;
const FILE_SHARE_WRITE: u32 = 2;
const OPEN_EXISTING: u32 = 3;

/// An icon made from the pixels of `shellicon::render`.
fn make_icon(kind: i32, size: i32) -> usize {
    let px = shellicon::render(kind, size);
    let mut color = Vec::with_capacity(px.len() * 4);
    for p in &px {
        color.extend_from_slice(&p.to_le_bytes());
    }
    let stride = (size as usize).div_ceil(16) * 2;
    let mut mask = alloc::vec![0u8; stride * size as usize];
    for y in 0..size as usize {
        for x in 0..size as usize {
            if px[y * size as usize + x] >> 24 == 0 {
                mask[y * stride + x / 8] |= 0x80 >> (x % 8);
            }
        }
    }
    // SAFETY: gdi32 copies the bits.
    let (hc, hm) = unsafe { (CreateBitmap(size, size, 1, 32, color.as_ptr()), CreateBitmap(size, size, 1, 1, mask.as_ptr())) };
    // ICONINFO: fIcon, the hot spot, the mask and the colour bitmap, each aligned to the pointer size.
    let ptr = core::mem::size_of::<usize>();
    let mut info = [0u8; 32];
    info[..4].copy_from_slice(&1u32.to_le_bytes());
    let at = (12 + ptr - 1) / ptr * ptr;
    info[at..at + ptr].copy_from_slice(&hm.to_le_bytes()[..ptr]);
    info[at + ptr..at + 2 * ptr].copy_from_slice(&hc.to_le_bytes()[..ptr]);
    // SAFETY: an ICONINFO with two bitmaps; the icon keeps copies.
    let icon = unsafe { CreateIconIndirect(info.as_ptr()) };
    // SAFETY: bitmaps made above.
    unsafe {
        DeleteObject(hc);
        DeleteObject(hm);
    }
    icon
}

static SMALL: AtomicUsize = AtomicUsize::new(0);
static LARGE: AtomicUsize = AtomicUsize::new(0);

/// The system image list of the process at `size` pixels (16 or 32): the seven icons of
/// `fileinfo::icon`, made at the first call.
pub fn system_image_list(small: bool) -> usize {
    let (slot, size) = if small { (&SMALL, 16) } else { (&LARGE, 32) };
    let h = slot.load(Ordering::Acquire);
    if h != 0 {
        return h;
    }
    // SAFETY: comctl32's documented call.
    let himl = unsafe { ImageList_Create(size, size, ILC_COLOR32 | ILC_MASK, fi::icon::COUNT, 8) };
    if himl == 0 {
        return 0;
    }
    for kind in 0..fi::icon::COUNT {
        let icon = make_icon(kind, size);
        if icon != 0 {
            // SAFETY: the list and the icon above; the list keeps its own copy.
            unsafe {
                ImageList_AddIcon(himl, icon);
                DestroyIcon(icon);
            }
        }
    }
    match slot.compare_exchange(0, himl, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => himl,
        Err(other) => other,
    }
}

fn exe_type_of(path: &[u16]) -> u32 {
    // SAFETY: a NUL-terminated path.
    let h = unsafe {
        CreateFileW(z(path).as_ptr(), GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE, core::ptr::null(), OPEN_EXISTING, 0, 0)
    };
    if h == -1 {
        return 0;
    }
    let mut head = [0u8; 1024];
    let mut n = 0u32;
    // SAFETY: a buffer of 1024 bytes.
    unsafe { ReadFile(h, head.as_mut_ptr(), 1024, &mut n, core::ptr::null_mut()) };
    // SAFETY: the handle above.
    unsafe { CloseHandle(h) };
    fi::exe_type(&head[..n as usize])
}

fn put_text(out: *mut u8, at: usize, units: usize, text: &[u16]) {
    let n = text.len().min(units - 1);
    for (i, &c) in text.iter().take(n).enumerate() {
        // SAFETY: the structure has `units` units at `at`.
        unsafe { core::ptr::write_unaligned(out.add(at + 2 * i).cast::<u16>(), c) };
    }
    // SAFETY: as above.
    unsafe { core::ptr::write_unaligned(out.add(at + 2 * n).cast::<u16>(), 0) };
}

/// `SHGetFileInfoW(pszPath, dwFileAttributes, psfi, cbFileInfo, uFlags)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SHGetFileInfoW(
    path: *const u16,
    file_attributes: u32,
    psfi: *mut u8,
    _size: u32,
    flags: u32,
) -> usize {
    if path.is_null() || psfi.is_null() || flags & fi::SHGFI_PIDL != 0 {
        return 0;
    }
    let ptr = core::mem::size_of::<usize>();
    let l = fi::layout(ptr);
    let mut n = 0;
    // SAFETY: a NUL-terminated path.
    while unsafe { *path.add(n) } != 0 {
        n += 1;
    }
    // SAFETY: `n` units before the NUL.
    let text: Vec<u16> = unsafe { core::slice::from_raw_parts(path, n) }.to_vec();
    // The two texts are empty unless something is put there, whatever happens.
    put_text(psfi, l.display_name, fi::DISPLAY_NAME_UNITS, &[]);
    put_text(psfi, l.type_name, fi::TYPE_NAME_UNITS, &[]);
    if flags == 0 {
        return 0;
    }
    let used = flags & fi::SHGFI_USEFILEATTRIBUTES != 0;
    let kind = if text.is_empty() {
        Kind::File
    } else if used {
        match (file_attributes & fi::FILE_ATTRIBUTE_DIRECTORY != 0, fi::is_drive_root(&text)) {
            (true, true) => Kind::DriveRoot,
            (true, false) => Kind::Directory,
            _ => Kind::File,
        }
    } else {
        let a = attributes(&text);
        if a == u32::MAX {
            return 0;
        } else if fi::is_drive_root(&text) {
            Kind::DriveRoot
        } else if a & fi::FILE_ATTRIBUTE_DIRECTORY != 0 {
            Kind::Directory
        } else {
            Kind::File
        }
    };
    let mut ret = 1usize;
    if flags & fi::SHGFI_ATTRIBUTES != 0 {
        let sub = !used && kind == Kind::Directory && has_subdirectory(&text);
        let a = fi::attributes(kind, sub, used);
        // SAFETY: the structure's dwAttributes.
        unsafe { core::ptr::write_unaligned(psfi.add(l.attributes).cast::<u32>(), a) };
    }
    if flags & fi::SHGFI_DISPLAYNAME != 0 {
        put_text(psfi, l.display_name, fi::DISPLAY_NAME_UNITS, &fi::display_name(&text, kind, &[]));
    }
    if flags & fi::SHGFI_TYPENAME != 0 {
        put_text(psfi, l.type_name, fi::TYPE_NAME_UNITS, &fi::type_name(&text, kind));
    }
    if flags & (fi::SHGFI_ICON | fi::SHGFI_SYSICONINDEX) != 0 {
        let index = fi::icon_index(&text, kind);
        let small = flags & fi::SHGFI_SMALLICON != 0 && flags & fi::SHGFI_SHELLICONSIZE == 0;
        let himl = system_image_list(small);
        // SAFETY: the structure's iIcon.
        unsafe { core::ptr::write_unaligned(psfi.add(l.icon_index).cast::<i32>(), index) };
        if flags & fi::SHGFI_ICON != 0 {
            // SAFETY: the list above; the structure's hIcon.
            let icon = if himl != 0 { unsafe { ImageList_GetIcon(himl, index, 0) } } else { 0 };
            // SAFETY: the structure's hIcon.
            unsafe { core::ptr::write_unaligned(psfi.cast::<usize>(), icon) };
            if icon == 0 {
                return 0;
            }
        }
        if flags & fi::SHGFI_SYSICONINDEX != 0 {
            ret = himl;
        }
    }
    if flags & fi::SHGFI_ICONLOCATION != 0 {
        // The file with the icon: a program has its own, every other kind has one of the shell's.
        let (file, index) = match kind {
            Kind::File if fi::icon_index(&text, kind) == fi::icon::APPLICATION => (text.clone(), 0),
            Kind::Directory | Kind::DriveRoot => ("C:\\Windows\\System32\\shell32.dll".encode_utf16().collect(), 3),
            Kind::File => ("C:\\Windows\\System32\\shell32.dll".encode_utf16().collect(), 0),
        };
        put_text(psfi, l.display_name, fi::DISPLAY_NAME_UNITS, &file);
        // SAFETY: the structure's iIcon.
        unsafe { core::ptr::write_unaligned(psfi.add(l.icon_index).cast::<i32>(), index) };
        ret = 1;
    }
    if flags & fi::SHGFI_EXETYPE != 0 {
        ret = if kind == Kind::File && !used { exe_type_of(&text) as usize } else { 0 };
    }
    ret
}

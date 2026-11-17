//! The PE resource API (`libloaderapi.h`, `winbase.h`): `FindResourceW`, `FindResourceExW`,
//! `LoadResource`, `LockResource`, `SizeofResource`, `FreeResource`, `EnumResourceNamesW`,
//! `EnumResourceTypesW`, and the `LOAD_LIBRARY_AS_DATAFILE` / `LOAD_LIBRARY_AS_IMAGE_RESOURCE`
//! modes of `LoadLibraryExW`, which the resource functions read as well as an image's.
//!
//! The rules are `poly_kernelbase::resource`'s (host-tested; the spec sheets of
//! `specs/windows/kernel32.dll/FindResourceW.yaml` and its relatives). A module handle is the
//! image base, with bit 0 set for a data file and bit 1 for an image resource (Learn,
//! `LoadLibraryExW`): the three kinds give the same answers.
//!
//! **Data files.** A file loaded as a data file or image resource is not run through the image
//! loader: its headers and the section that holds its resource directory are copied to a private
//! allocation of the image's size at the same relative addresses, so the resource tree's RVAs
//! work as in an image, and nothing else is read (a 100 MB executable costs its `.rsrc`).

use crate::base::*;
use crate::file::{CreateFileW, ReadFile, SetFilePointerEx};
use crate::memory::{VirtualAlloc, VirtualFree, VirtualQuery};
use crate::nt::Handle;
use alloc::vec::Vec;
use poly_kernelbase::error::*;
use poly_kernelbase::resource::{self as res, Id, Tree};

/// `LOAD_LIBRARY_AS_DATAFILE`, `LOAD_LIBRARY_AS_IMAGE_RESOURCE`, `LOAD_LIBRARY_AS_DATAFILE_EXCLUSIVE`
/// (`libloaderapi.h`).
pub const LOAD_LIBRARY_AS_DATAFILE: u32 = 0x2;
pub const LOAD_LIBRARY_AS_DATAFILE_EXCLUSIVE: u32 = 0x40;
pub const LOAD_LIBRARY_AS_IMAGE_RESOURCE: u32 = 0x20;

const MEM_COMMIT: u32 = 0x1000;
const MEM_RESERVE: u32 = 0x2000;
const MEM_RELEASE: u32 = 0x8000;
const PAGE_READWRITE: u32 = 0x04;
const PAGE_NOACCESS: u32 = 0x01;
const PAGE_GUARD: u32 = 0x100;
const PAGE: usize = 0x1000;

/// A module whose resource tree can be read.
struct Module {
    base: u64,
    tree_rva: u32,
    tree_len: u32,
}

impl Module {
    fn tree(&self) -> Tree<'static> {
        // SAFETY: `module_of` checked that the tree is committed memory of the module.
        Tree(unsafe {
            core::slice::from_raw_parts((self.base + u64::from(self.tree_rva)) as *const u8, self.tree_len as usize)
        })
    }

    fn tree_va(&self) -> u64 {
        self.base + u64::from(self.tree_rva)
    }
}

/// `MEMORY_BASIC_INFORMATION` (`winnt.h`): the fields used here, as offsets and the structure's size. The
/// base and the region size are pointer sized: `AllocationBase` at 8 (64-bit) or 4, `State` at 32 or 16,
/// `Protect` at 36 or 20, 48 or 28 bytes in all.
#[cfg(target_pointer_width = "64")]
mod mbi {
    pub const SIZE: usize = 48;
    pub const ALLOCATION_BASE: usize = 8;
    pub const POINTER: usize = 8;
    pub const STATE: usize = 32;
    pub const PROTECT: usize = 36;
}
#[cfg(target_pointer_width = "32")]
mod mbi {
    pub const SIZE: usize = 28;
    pub const ALLOCATION_BASE: usize = 4;
    pub const POINTER: usize = 4;
    pub const STATE: usize = 16;
    pub const PROTECT: usize = 20;
}

/// Whether `address` is committed, readable memory; with `allocation_base`, also the start of its
/// allocation.
fn readable(address: u64, allocation_base: Option<u64>) -> bool {
    let mut info = [0u8; mbi::SIZE];
    // SAFETY: a buffer of the structure's size.
    let n = unsafe { VirtualQuery(address as usize as *const u8, info.as_mut_ptr(), info.len()) };
    if n < info.len() {
        return false;
    }
    let u32_at = |o: usize| u32::from_le_bytes(info[o..o + 4].try_into().unwrap());
    let alloc = if mbi::POINTER == 8 {
        u64::from_le_bytes(info[mbi::ALLOCATION_BASE..mbi::ALLOCATION_BASE + 8].try_into().unwrap())
    } else {
        u64::from(u32_at(mbi::ALLOCATION_BASE))
    };
    let protect = u32_at(mbi::PROTECT);
    u32_at(mbi::STATE) == MEM_COMMIT
        && protect != 0
        && protect & (PAGE_NOACCESS | PAGE_GUARD) == 0
        && allocation_base.is_none_or(|b| alloc == b)
}

/// The module a handle names and its resource tree, or `ERROR_RESOURCE_DATA_NOT_FOUND` (no module,
/// or one without resources). NULL stands for the executable; the low two bits of a handle are the
/// data-file and image-resource marks.
fn module_of(handle: *mut u8) -> Result<Module, u32> {
    let none = ERROR_RESOURCE_DATA_NOT_FOUND;
    let base = if handle.is_null() { crate::process::exe_base() } else { (handle as u64) & !3 };
    if base == 0 || base & 0xFFFF != 0 || !readable(base, Some(base)) {
        return Err(none);
    }
    // SAFETY: the first page of an allocation base that is committed and readable.
    let (e_lfanew, mz) =
        unsafe { (((base + 0x3C) as *const u32).read_unaligned() as u64, (base as *const u16).read_unaligned()) };
    if mz != 0x5A4D || e_lfanew > 0xF00 {
        return Err(none);
    }
    let pe = base + e_lfanew;
    // SAFETY: inside the first page (`e_lfanew` is at most 0xF00 and the headers are read up to
    // the data directories, 0x108 bytes at most).
    let (sig, magic, size_of_image) = unsafe {
        (
            (pe as *const u32).read_unaligned(),
            ((pe + 24) as *const u16).read_unaligned(),
            ((pe + 24 + 56) as *const u32).read_unaligned(),
        )
    };
    if sig != 0x0000_4550 {
        return Err(none);
    }
    let dirs = pe
        + 24
        + if magic == 0x20B {
            112
        } else if magic == 0x10B {
            96
        } else {
            return Err(none);
        };
    // SAFETY: as above: the resource directory is the third data directory (8 bytes each).
    let (rva, size) =
        unsafe { (((dirs + 16) as *const u32).read_unaligned(), ((dirs + 20) as *const u32).read_unaligned()) };
    if rva == 0 || size == 0 || u64::from(rva) + u64::from(size) > u64::from(size_of_image) {
        return Err(none);
    }
    if !readable(base + u64::from(rva), None) {
        return Err(none);
    }
    Ok(Module { base, tree_rva: rva, tree_len: size })
}

/// A type or name argument: an integer id (`MAKEINTRESOURCE`) or a string.
///
/// # Safety
/// `p` is an integer id or a NUL-terminated string.
unsafe fn request_id(p: *const u16) -> Result<Id, u32> {
    if (p as usize) >> 16 == 0 {
        return Ok(Id::Num(p as usize as u32));
    }
    // SAFETY: the caller's contract.
    res::parse_string_id(unsafe { poly_kernelbase::text::wide_at(p) })
}

/// `FindResourceExW(hModule, lpType, lpName, wLanguage)`: the `HRSRC`, the address of the
/// resource's data entry, or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn FindResourceExW(module: *mut u8, ty: *const u16, name: *const u16, lang: u16) -> *mut u8 {
    let m = match module_of(module) {
        Ok(m) => m,
        Err(e) => return fail(e, core::ptr::null_mut()),
    };
    // SAFETY: the caller's arguments.
    let (ty, name) = match unsafe { (request_id(ty), request_id(name)) } {
        (Ok(t), Ok(n)) => (t, n),
        (Err(e), _) | (_, Err(e)) => return fail(e, core::ptr::null_mut()),
    };
    match m.tree().find(&ty, &name, lang) {
        Ok(at) => (m.tree_va() + at as u64) as *mut u8,
        Err(e) => fail(e, core::ptr::null_mut()),
    }
}

/// `FindResourceW(hModule, lpName, lpType)`: [`FindResourceExW`] with the neutral language.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn FindResourceW(module: *mut u8, name: *const u16, ty: *const u16) -> *mut u8 {
    // SAFETY: the caller's arguments.
    unsafe { FindResourceExW(module, ty, name, res::LANG_NEUTRAL) }
}

/// The data entry an `HRSRC` names, `(rva, size)`, checked to lie in the module's tree.
fn data_entry(m: &Module, hres: *mut u8) -> Result<(u32, u32), u32> {
    let va = hres as u64;
    if va < m.tree_va() || va + res::DATA_ENTRY as u64 > m.tree_va() + u64::from(m.tree_len) {
        return Err(ERROR_INVALID_PARAMETER);
    }
    m.tree().data((va - m.tree_va()) as usize).ok_or(ERROR_INVALID_PARAMETER)
}

/// `LoadResource(hModule, hResInfo)`: a pointer to the resource's data in the module (not a real
/// global memory handle: `LockResource` takes it as it is).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LoadResource(module: *mut u8, hres: *mut u8) -> *mut u8 {
    if hres.is_null() {
        return fail(ERROR_INVALID_PARAMETER, core::ptr::null_mut());
    }
    let m = match module_of(module) {
        Ok(m) => m,
        Err(e) => return fail(e, core::ptr::null_mut()),
    };
    match data_entry(&m, hres) {
        Ok((rva, _)) => (m.base + u64::from(rva)) as *mut u8,
        Err(e) => fail(e, core::ptr::null_mut()),
    }
}

/// `LockResource(hResData)`: the pointer `LoadResource` returned.
#[unsafe(no_mangle)]
pub extern "system" fn LockResource(data: *mut u8) -> *mut u8 {
    data
}

/// `FreeResource(hResData)`: nothing to free (a resource lives as long as its module); 0.
#[unsafe(no_mangle)]
pub extern "system" fn FreeResource(_data: *mut u8) -> i32 {
    FALSE
}

/// `SizeofResource(hModule, hResInfo)`: the size in bytes; 0 with an error for a bad handle.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SizeofResource(module: *mut u8, hres: *mut u8) -> u32 {
    if hres.is_null() {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    let m = match module_of(module) {
        Ok(m) => m,
        Err(e) => return fail(e, 0),
    };
    match data_entry(&m, hres) {
        Ok((_, size)) => size,
        Err(e) => fail(e, 0),
    }
}

/// The callback of `EnumResourceNamesW`: `BOOL (HMODULE, LPCWSTR type, LPWSTR name, LONG_PTR)`.
type NameProc = unsafe extern "system" fn(*mut u8, *const u16, *mut u16, isize) -> i32;
/// The callback of `EnumResourceTypesW`: `BOOL (HMODULE, LPWSTR type, LONG_PTR)`.
type TypeProc = unsafe extern "system" fn(*mut u8, *mut u16, isize) -> i32;

/// A listed id as the callback gets it: the integer, or a pointer to a NUL-terminated copy of the
/// name in `text` (kept alive by the caller during the call).
fn listed(id: &Id, text: &mut Vec<u16>) -> *mut u16 {
    match id {
        Id::Num(n) => *n as usize as *mut u16,
        Id::Name(n) => {
            text.clear();
            text.extend_from_slice(n);
            text.push(0);
            text.as_mut_ptr()
        }
    }
}

/// `EnumResourceNamesW(hModule, lpType, lpEnumFunc, lParam)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn EnumResourceNamesW(
    module: *mut u8,
    ty: *const u16,
    f: Option<NameProc>,
    lparam: isize,
) -> i32 {
    let Some(f) = f else { return fail(ERROR_INVALID_PARAMETER, FALSE) };
    let m = match module_of(module) {
        Ok(m) => m,
        Err(e) => return fail(e, FALSE),
    };
    // SAFETY: the caller's argument.
    let want = match unsafe { request_id(ty) } {
        Ok(t) => t,
        Err(e) => return fail(e, FALSE),
    };
    let names = match m.tree().names_of(&want) {
        Ok(n) => n,
        Err(e) => return fail(e, FALSE),
    };
    let mut text = Vec::new();
    for id in &names {
        let p = listed(id, &mut text);
        // SAFETY: the caller's callback, called with its own arguments and a valid name.
        if unsafe { f(module, ty, p, lparam) } == 0 {
            return fail(ERROR_RESOURCE_ENUM_USER_STOP, FALSE);
        }
    }
    TRUE
}

/// `ERROR_RESOURCE_ENUM_USER_STOP` (`winerror.h`).
const ERROR_RESOURCE_ENUM_USER_STOP: u32 = res::ERROR_RESOURCE_ENUM_USER_STOP;
const ERROR_RESOURCE_DATA_NOT_FOUND: u32 = res::ERROR_RESOURCE_DATA_NOT_FOUND;

/// `EnumResourceTypesW(hModule, lpEnumFunc, lParam)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn EnumResourceTypesW(module: *mut u8, f: Option<TypeProc>, lparam: isize) -> i32 {
    let Some(f) = f else { return fail(ERROR_INVALID_PARAMETER, FALSE) };
    let m = match module_of(module) {
        Ok(m) => m,
        Err(e) => return fail(e, FALSE),
    };
    let types = match m.tree().types() {
        Ok(t) => t,
        Err(e) => return fail(e, FALSE),
    };
    let mut text = Vec::new();
    for id in &types {
        let p = listed(id, &mut text);
        // SAFETY: the caller's callback.
        if unsafe { f(module, p, lparam) } == 0 {
            return fail(ERROR_RESOURCE_ENUM_USER_STOP, FALSE);
        }
    }
    TRUE
}

// ----- data files -------------------------------------------------------------------------------------

/// Reads `buf.len()` bytes at `offset` of an open file; false if fewer came.
fn read_at(file: Handle, offset: u64, buf: &mut [u8]) -> bool {
    let mut got = 0u32;
    // SAFETY: valid handle and buffer.
    unsafe {
        SetFilePointerEx(file, offset as i64, core::ptr::null_mut(), 0) != 0
            && ReadFile(file, buf.as_mut_ptr(), buf.len() as u32, &mut got, core::ptr::null_mut()) != 0
            && got as usize == buf.len()
    }
}

fn round_up(n: usize, to: usize) -> usize {
    n.div_ceil(to) * to
}

/// The part of a PE file a data-file load needs: sizes and the section table.
struct Headers {
    size_of_image: usize,
    size_of_headers: usize,
    rsrc: (u32, u32),
    /// `(virtual address, virtual size, file offset, raw size)` of each section.
    sections: Vec<(u32, u32, u32, u32)>,
}

fn parse_headers(h: &[u8]) -> Option<Headers> {
    let u16_at = |o: usize| h.get(o..o + 2).map(|b| u16::from_le_bytes(b.try_into().unwrap()));
    let u32_at = |o: usize| h.get(o..o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    if u16_at(0)? != 0x5A4D {
        return None;
    }
    let pe = u32_at(0x3C)? as usize;
    if u32_at(pe)? != 0x0000_4550 {
        return None;
    }
    let sections = usize::from(u16_at(pe + 6)?);
    let opt_size = usize::from(u16_at(pe + 20)?);
    let opt = pe + 24;
    let dirs = opt
        + match u16_at(opt)? {
            0x20B => 112,
            0x10B => 96,
            _ => return None,
        };
    let rsrc = (u32_at(dirs + 16)?, u32_at(dirs + 20)?);
    let mut list = Vec::new();
    for i in 0..sections {
        let s = opt + opt_size + i * 40;
        list.push((u32_at(s + 12)?, u32_at(s + 8)?, u32_at(s + 20)?, u32_at(s + 16)?));
    }
    Some(Headers {
        size_of_image: u32_at(opt + 56)? as usize,
        size_of_headers: u32_at(opt + 60)? as usize,
        rsrc,
        sections: list,
    })
}

/// Maps the headers and the resource section of the PE file at `path` (a NUL-terminated Win32
/// path) as described in the module comment; the handle is the base with the mark of `flags`.
fn load_datafile(path: &[u16], flags: u32) -> *mut u8 {
    let null = core::ptr::null_mut();
    // SAFETY: a NUL-terminated path.
    let file = unsafe { CreateFileW(path.as_ptr(), 0x8000_0000, 7, core::ptr::null(), 3, 0, 0) };
    if file == INVALID_HANDLE_VALUE {
        return null;
    }
    let result = (|| {
        let mut first = alloc::vec![0u8; 0x1000];
        // A file shorter than a page: read what there is.
        let mut got = 0u32;
        // SAFETY: valid handle and buffer.
        let ok = unsafe { ReadFile(file, first.as_mut_ptr(), first.len() as u32, &mut got, core::ptr::null_mut()) };
        if ok == 0 {
            return None;
        }
        first.truncate(got as usize);
        let mut hdr = parse_headers(&first)?;
        let mut head = first;
        if hdr.size_of_headers > head.len() {
            head.resize(hdr.size_of_headers, 0);
            if !read_at(file, 0, &mut head) {
                return None;
            }
            hdr = parse_headers(&head)?;
        }
        if hdr.size_of_image == 0 || hdr.size_of_image > 0x4000_0000 {
            return None;
        }
        // SAFETY: reserves address space only.
        let base = unsafe {
            VirtualAlloc(core::ptr::null_mut(), round_up(hdr.size_of_image, 0x10000), MEM_RESERVE, PAGE_READWRITE)
        };
        if base.is_null() {
            return None;
        }
        let unmap = |at: *mut u8| {
            // SAFETY: the allocation made above.
            unsafe { VirtualFree(at, 0, MEM_RELEASE) };
        };
        // The headers.
        let hsize = round_up(hdr.size_of_headers.max(head.len()).min(hdr.size_of_image), PAGE);
        // SAFETY: commits inside the reservation.
        if unsafe { VirtualAlloc(base, hsize, MEM_COMMIT, PAGE_READWRITE) }.is_null() {
            unmap(base);
            return None;
        }
        // SAFETY: `hsize` committed bytes at `base`; `head` is at most that long.
        unsafe { core::ptr::copy_nonoverlapping(head.as_ptr(), base, head.len().min(hsize)) };
        // The sections that hold the resource directory.
        let (rva, size) = (hdr.rsrc.0 as usize, hdr.rsrc.1 as usize);
        if rva != 0 && size != 0 {
            for &(va, vsize, raw, raw_size) in &hdr.sections {
                let (va, span) = (va as usize, (vsize.max(raw_size)) as usize);
                if va >= rva + size || va + span <= rva || va + span > hdr.size_of_image {
                    continue;
                }
                let len = round_up(span, PAGE);
                // SAFETY: commits inside the reservation.
                let at = unsafe { base.add(va) };
                if unsafe { VirtualAlloc(at, len, MEM_COMMIT, PAGE_READWRITE) }.is_null() {
                    unmap(base);
                    return None;
                }
                // SAFETY: `len` committed bytes at `at`, at least `raw_size`.
                let dst = unsafe { core::slice::from_raw_parts_mut(at, (raw_size as usize).min(len)) };
                if !read_at(file, u64::from(raw), dst) {
                    unmap(base);
                    return None;
                }
            }
        }
        with_state(|s| s.datafiles.push(base as u64));
        Some(base)
    })();
    close(file);
    match result {
        Some(base) => {
            let mark = if flags & LOAD_LIBRARY_AS_IMAGE_RESOURCE != 0
                && flags & (LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_DATAFILE_EXCLUSIVE) == 0
            {
                2
            } else {
                1
            };
            (base as usize | mark) as *mut u8
        }
        None => fail(ERROR_BAD_EXE_FORMAT, null),
    }
}

/// `LoadLibraryExW` with `LOAD_LIBRARY_AS_DATAFILE`, `..._AS_IMAGE_RESOURCE` or
/// `..._AS_DATAFILE_EXCLUSIVE`: `name` is the file's name; a plain file name is looked for in the
/// current directory and the executable's directory.
pub fn load_library_datafile(name: &[u16], flags: u32) -> *mut u8 {
    let has_path = name.iter().any(|&c| c == u16::from(b'\\') || c == u16::from(b'/'));
    let mut candidates: Vec<Vec<u16>> = Vec::new();
    if has_path {
        match full_path(name) {
            Ok((p, _)) => candidates.push(p),
            Err(e) => return fail(e, core::ptr::null_mut()),
        }
    } else {
        let mut cur = current_dir();
        cur.push(u16::from(b'\\'));
        cur.extend_from_slice(name);
        candidates.push(cur);
        // Then the system directories, as the standard search order of `LoadLibraryEx` (Learn):
        // `version.dll` as a data file is the one in `System32` (the sheet of
        // `LdrRegisterDllNotification` loads it so).
        for dir in ["C:\\Windows\\System32\\", "C:\\Windows\\"] {
            let mut p: Vec<u16> = dir.encode_utf16().collect();
            p.extend_from_slice(name);
            candidates.push(p);
        }
    }
    let mut last = core::ptr::null_mut();
    for c in candidates {
        let c = with_nul(&c);
        last = load_datafile(&c, flags);
        if !last.is_null() {
            return last;
        }
    }
    last
}

/// `FreeLibrary` of a data-file or image-resource handle: the mapping goes. `None` when the handle
/// is not one of ours.
pub fn free_datafile(handle: *mut u8) -> Option<i32> {
    if (handle as usize) & 3 == 0 {
        return None;
    }
    let base = (handle as u64) & !3;
    let known = with_state(|s| match s.datafiles.iter().position(|&b| b == base) {
        Some(i) => {
            s.datafiles.swap_remove(i);
            true
        }
        None => false,
    });
    if !known {
        return Some(fail(ERROR_INVALID_HANDLE, FALSE));
    }
    // SAFETY: the allocation `load_datafile` made.
    unsafe { VirtualFree(base as *mut u8, 0, MEM_RELEASE) };
    Some(TRUE)
}

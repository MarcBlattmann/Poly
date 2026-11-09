//! Reading font files from disk and listing the fonts directory, through
//! `kernel32` (`CreateFileW`, `ReadFile`, `FindFirstFileW`, ...).

use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_void;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        sa: *mut c_void,
        disp: u32,
        flags: u32,
        tmpl: *mut c_void,
    ) -> isize;
    fn GetFileSizeEx(h: isize, size: *mut i64) -> i32;
    fn ReadFile(h: isize, buf: *mut u8, n: u32, read: *mut u32, ov: *mut c_void) -> i32;
    fn CloseHandle(h: isize) -> i32;
    fn GetFileTime(h: isize, created: *mut u64, accessed: *mut u64, written: *mut u64) -> i32;
    fn GetFileAttributesW(name: *const u16) -> u32;
    fn FindFirstFileW(pattern: *const u16, data: *mut WIN32_FIND_DATAW) -> isize;
    fn FindNextFileW(h: isize, data: *mut WIN32_FIND_DATAW) -> i32;
    fn FindClose(h: isize) -> i32;
}

const GENERIC_READ: u32 = 0x8000_0000;
const FILE_SHARE_READ_WRITE: u32 = 3;
const OPEN_EXISTING: u32 = 3;
const INVALID_HANDLE: isize = -1;
const INVALID_FILE_ATTRIBUTES: u32 = 0xFFFF_FFFF;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;

/// `WIN32_FIND_DATAW` (`minwinbase.h`).
#[repr(C)]
struct WIN32_FIND_DATAW {
    attributes: u32,
    // `FILETIME`s are two `u32`s (4-byte aligned), not a `u64`.
    created: [u32; 2],
    accessed: [u32; 2],
    written: [u32; 2],
    size_high: u32,
    size_low: u32,
    reserved: [u32; 2],
    name: [u16; 260],
    alt_name: [u16; 14],
}

/// A NUL-terminated copy of `path`.
fn z(path: &[u16]) -> Vec<u16> {
    let mut v = path.to_vec();
    v.push(0);
    v
}

/// What a path names.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Missing,
    Directory,
    File,
}

pub fn kind(path: &[u16]) -> Kind {
    if path.is_empty() || path.contains(&0) {
        return Kind::Missing;
    }
    let p = z(path);
    // SAFETY: `p` is NUL-terminated.
    let a = unsafe { GetFileAttributesW(p.as_ptr()) };
    if a == INVALID_FILE_ATTRIBUTES {
        Kind::Missing
    } else if a & FILE_ATTRIBUTE_DIRECTORY != 0 {
        Kind::Directory
    } else {
        Kind::File
    }
}

/// The whole file, and its last write time (a `FILETIME`).
pub fn read_file(path: &[u16]) -> Result<(Vec<u8>, u64), i32> {
    let p = z(path);
    // SAFETY: `p` is NUL-terminated.
    let h = unsafe {
        CreateFileW(
            p.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ_WRITE,
            core::ptr::null_mut(),
            OPEN_EXISTING,
            0,
            core::ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE || h == 0 {
        return Err(crate::com::E_FILE_NOT_FOUND);
    }
    let mut size = 0i64;
    // SAFETY: `h` is an open file.
    if unsafe { GetFileSizeEx(h, &mut size) } == 0 || size < 0 || size > 0x4000_0000 {
        // SAFETY: closing our handle.
        unsafe { CloseHandle(h) };
        return Err(crate::com::E_FAIL);
    }
    let mut buf = alloc::vec![0u8; size as usize];
    let mut done = 0usize;
    while done < buf.len() {
        let mut n = 0u32;
        let want = (buf.len() - done).min(1 << 20) as u32;
        // SAFETY: `buf[done..]` has room for `want` bytes.
        let ok = unsafe { ReadFile(h, buf.as_mut_ptr().add(done), want, &mut n, core::ptr::null_mut()) };
        if ok == 0 || n == 0 {
            break;
        }
        done += n as usize;
    }
    let mut written = 0u64;
    // SAFETY: `h` is an open file.
    unsafe {
        GetFileTime(h, core::ptr::null_mut(), core::ptr::null_mut(), &mut written);
        CloseHandle(h);
    }
    buf.truncate(done);
    Ok((buf, written))
}

/// The last write time of a file, 0 when unknown.
pub fn write_time(path: &[u16]) -> u64 {
    let p = z(path);
    // SAFETY: `p` is NUL-terminated.
    let h = unsafe {
        CreateFileW(
            p.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ_WRITE,
            core::ptr::null_mut(),
            OPEN_EXISTING,
            0x0200_0000,
            core::ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE || h == 0 {
        return 0;
    }
    let mut written = 0u64;
    // SAFETY: `h` is an open file or directory.
    unsafe {
        GetFileTime(h, core::ptr::null_mut(), core::ptr::null_mut(), &mut written);
        CloseHandle(h);
    }
    written
}

/// The names of the plain files of a directory (`dir` without a trailing
/// separator).
pub fn list_dir(dir: &str) -> Vec<String> {
    let mut pattern: Vec<u16> = dir.encode_utf16().collect();
    pattern.extend("\\*".encode_utf16());
    pattern.push(0);
    let mut out = Vec::new();
    // SAFETY: zero is a valid `WIN32_FIND_DATAW`.
    let mut data: WIN32_FIND_DATAW = unsafe { core::mem::zeroed() };
    // SAFETY: `pattern` is NUL-terminated and `data` is writable.
    let h = unsafe { FindFirstFileW(pattern.as_ptr(), &mut data) };
    if h == INVALID_HANDLE || h == 0 {
        return out;
    }
    loop {
        if data.attributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
            let n = data.name.iter().position(|&c| c == 0).unwrap_or(data.name.len());
            out.push(char::decode_utf16(data.name[..n].iter().copied()).map(|r| r.unwrap_or('?')).collect());
        }
        // SAFETY: `h` is a find handle.
        if unsafe { FindNextFileW(h, &mut data) } == 0 {
            break;
        }
    }
    // SAFETY: closing the find handle.
    unsafe { FindClose(h) };
    out.sort();
    out
}

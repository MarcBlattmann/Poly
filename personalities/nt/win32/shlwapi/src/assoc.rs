//! `AssocQueryStringW`: the rules are `poly_kernelbase::assoc`'s (the sheet
//! `shlwapi.dll/AssocQueryStringW.yaml`); this file reads the registry and the programs' version
//! resources for them and fills the caller's buffer.

use alloc::vec::Vec;
use core::ffi::c_void;
use poly_kernelbase::assoc::{self as logic, Hive};

#[cfg_attr(target_arch = "x86", link(name = "advapi32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "advapi32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn RegGetValueW(
        key: isize,
        sub: *const u16,
        value: *const u16,
        flags: u32,
        kind: *mut u32,
        data: *mut u8,
        size: *mut u32,
    ) -> u32;
    fn RegOpenKeyExW(key: isize, sub: *const u16, options: u32, access: u32, out: *mut isize) -> u32;
    fn RegEnumValueW(
        key: isize,
        index: u32,
        name: *mut u16,
        name_len: *mut u32,
        reserved: *mut u32,
        kind: *mut u32,
        data: *mut u8,
        size: *mut u32,
    ) -> u32;
    fn RegCloseKey(key: isize) -> u32;
}

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetWindowsDirectoryW(buf: *mut u16, size: u32) -> u32;
}

#[cfg_attr(target_arch = "x86", link(name = "version", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "version", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetFileVersionInfoSizeW(name: *const u16, handle: *mut u32) -> u32;
    fn GetFileVersionInfoW(name: *const u16, handle: u32, len: u32, data: *mut u8) -> i32;
    fn VerQueryValueW(block: *const u8, sub: *const u16, out: *mut *mut c_void, len: *mut u32) -> i32;
}

/// `HKEY_CURRENT_USER`, `HKEY_LOCAL_MACHINE` (`winreg.h`), sign-extended as the header's cast makes them.
const HKEY_CURRENT_USER: isize = 0x8000_0001_u32 as i32 as isize;
const HKEY_LOCAL_MACHINE: isize = 0x8000_0002_u32 as i32 as isize;
/// `RRF_RT_REG_SZ` (`winreg.h`): a `REG_SZ`, or a `REG_EXPAND_SZ` that comes expanded (as a `REG_SZ`).
const RRF_STRINGS: u32 = 0x2;
/// `KEY_READ` (`winnt.h`).
const KEY_READ: u32 = 0x2_0019;

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn nul(text: &[u16]) -> Vec<u16> {
    let mut v = text.to_vec();
    v.push(0);
    v
}

/// The units of a NUL-terminated string at `p` (at most `max`).
///
/// # Safety
/// `p` points to at least `max` readable units or to a NUL before them.
unsafe fn units(p: *const u16, max: usize) -> Vec<u16> {
    let mut n = 0;
    // SAFETY: the caller's contract.
    unsafe {
        while n < max && *p.add(n) != 0 {
            n += 1;
        }
        core::slice::from_raw_parts(p, n).to_vec()
    }
}

/// The registry root and the path below it of a key of `hive`.
fn place(hive: Hive, key: &[u16]) -> (isize, Vec<u16>) {
    let (root, base) = match hive {
        Hive::UserClasses => (HKEY_CURRENT_USER, "Software\\Classes"),
        Hive::MachineClasses => (HKEY_LOCAL_MACHINE, "SOFTWARE\\Classes"),
        Hive::Machine => (HKEY_LOCAL_MACHINE, ""),
        Hive::User => (HKEY_CURRENT_USER, ""),
    };
    let mut path = w(base);
    if !path.is_empty() && !key.is_empty() {
        path.push(u16::from(b'\\'));
    }
    path.extend_from_slice(key);
    (root, path)
}

/// Poly's registry and files.
struct System;

impl logic::Registry for System {
    fn value(&self, hive: Hive, key: &[u16], name: Option<&[u16]>) -> Option<Vec<u16>> {
        let (root, path) = place(hive, key);
        let path = nul(&path);
        let name = name.map(nul);
        let name_ptr = name.as_ref().map_or(core::ptr::null(), |v| v.as_ptr());
        let mut buf = alloc::vec![0u8; 4096];
        let mut size = buf.len() as u32;
        let mut kind = 0u32;
        // SAFETY: NUL-terminated names; the buffer and its size.
        let r =
            unsafe { RegGetValueW(root, path.as_ptr(), name_ptr, RRF_STRINGS, &mut kind, buf.as_mut_ptr(), &mut size) };
        if r != 0 {
            return None;
        }
        Some(
            buf[..(size as usize).min(buf.len())]
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .take_while(|&c| c != 0)
                .collect(),
        )
    }

    fn has_key(&self, hive: Hive, key: &[u16]) -> bool {
        let (root, path) = place(hive, key);
        let path = nul(&path);
        let mut h: isize = 0;
        // SAFETY: a NUL-terminated path and an out pointer; the key is closed again.
        unsafe {
            if RegOpenKeyExW(root, path.as_ptr(), 0, KEY_READ, &mut h) != 0 {
                return false;
            }
            RegCloseKey(h);
        }
        true
    }

    fn value_names(&self, hive: Hive, key: &[u16]) -> Vec<Vec<u16>> {
        let (root, path) = place(hive, key);
        let path = nul(&path);
        let mut names = Vec::new();
        let mut h: isize = 0;
        // SAFETY: a NUL-terminated path, an out pointer, our name buffer with its size; the key is
        // closed again.
        unsafe {
            if RegOpenKeyExW(root, path.as_ptr(), 0, KEY_READ, &mut h) != 0 {
                return names;
            }
            let mut name = [0u16; 512];
            for index in 0..1024 {
                let mut len = name.len() as u32;
                let r = RegEnumValueW(
                    h,
                    index,
                    name.as_mut_ptr(),
                    &mut len,
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                );
                if r != 0 {
                    break;
                }
                names.push(name[..(len as usize).min(name.len())].to_vec());
            }
            RegCloseKey(h);
        }
        names
    }

    fn file_description(&self, path: &[u16]) -> Option<Vec<u16>> {
        let name = nul(path);
        let mut handle = 0u32;
        // SAFETY: a NUL-terminated path; the block is ours, of the size asked for.
        unsafe {
            let size = GetFileVersionInfoSizeW(name.as_ptr(), &mut handle);
            if size == 0 {
                return None;
            }
            let mut block = alloc::vec![0u8; size as usize];
            if GetFileVersionInfoW(name.as_ptr(), 0, size, block.as_mut_ptr()) == 0 {
                return None;
            }
            // The first language and code page of the translation table (Learn: "VerQueryValueW").
            let mut p: *mut c_void = core::ptr::null_mut();
            let mut len = 0u32;
            let translation = nul(&w("\\VarFileInfo\\Translation"));
            let (lang, cp) = if VerQueryValueW(block.as_ptr(), translation.as_ptr(), &mut p, &mut len) != 0 && len >= 4
            {
                let pair = p.cast::<u16>();
                (pair.read_unaligned(), pair.add(1).read_unaligned())
            } else {
                (0x0409, 0x04B0)
            };
            let sub = alloc::format!("\\StringFileInfo\\{lang:04x}{cp:04x}\\FileDescription");
            let sub = nul(&w(&sub));
            if VerQueryValueW(block.as_ptr(), sub.as_ptr(), &mut p, &mut len) == 0 || p.is_null() {
                return None;
            }
            Some(units(p.cast::<u16>(), len as usize))
        }
    }
}

/// The Windows directory (`C:\Windows`).
fn windows_directory() -> Vec<u16> {
    let mut buf = [0u16; 260];
    // SAFETY: a buffer of its size.
    let n = unsafe { GetWindowsDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if n == 0 || n >= buf.len() { w("C:\\Windows") } else { buf[..n].to_vec() }
}

/// `AssocQueryStringW(flags, str, pszAssoc, pszExtra, pszOut, pcchOut)`: one string of the
/// handler of a file type, ProgID or program; `*pcchOut` counts units with the NUL. A NULL
/// `pszOut` asks for the size (`S_FALSE`); a buffer too small gets the start of the text
/// (`HRESULT_FROM_WIN32(ERROR_INSUFFICIENT_BUFFER)`) or nothing with `ASSOCF_NOTRUNCATE`
/// (`E_POINTER`); a failure leaves an empty string.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn AssocQueryStringW(
    flags: u32,
    what: u32,
    assoc: *const u16,
    extra: *const u16,
    out: *mut u16,
    count: *mut u32,
) -> u32 {
    if assoc.is_null() {
        return crate::E_INVALIDARG;
    }
    if count.is_null() {
        return crate::E_POINTER;
    }
    // SAFETY: the caller's NUL-terminated strings (`extra` may be NULL) and its count.
    let (assoc, verb, capacity) =
        unsafe { (crate::wide(assoc), if extra.is_null() { None } else { Some(crate::wide(extra)) }, count.read()) };
    let text = match logic::query_string(&System, flags, what, assoc, verb, &windows_directory()) {
        Ok(t) => t,
        Err(hr) => {
            if !out.is_null() && capacity > 0 {
                // SAFETY: the caller's buffer holds at least one unit.
                unsafe { out.write(0) };
            }
            return hr;
        }
    };
    let (hr, reported, written) =
        logic::buffer_result(text.len(), capacity, !out.is_null(), flags & logic::ASSOCF_NOTRUNCATE != 0);
    // SAFETY: the caller's count, and its buffer of `capacity` units, of which `written + 1` are
    // filled (`buffer_result` keeps that within the capacity).
    unsafe {
        count.write(reported);
        if let Some(n) = written {
            core::ptr::copy_nonoverlapping(text.as_ptr(), out, n);
            out.add(n).write(0);
        }
    }
    hr
}

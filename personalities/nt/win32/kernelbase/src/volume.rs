//! `GetVolumeInformationW` and `GetVolumePathNameW` (sheets
//! `kernel32.dll/GetVolumeInformationW`, `kernel32.dll/GetVolumePathNameW`):
//! which root a name means is `poly_kernelbase::volume`; the volume's facts
//! are `NtQueryVolumeInformationFile`'s (`FileFsVolumeInformation`,
//! `FileFsAttributeInformation`) on its root directory.

use crate::base::*;
use crate::nt::{self, Handle, IoStatusBlock};
use alloc::vec::Vec;
use poly_kernelbase::error::*;
use poly_kernelbase::file::{FILE_DIRECTORY_FILE, FILE_READ_ATTRIBUTES, FILE_SYNCHRONOUS_IO_NONALERT, SYNCHRONIZE};
use poly_kernelbase::volume::{self, Root};

/// `FileFsVolumeInformation` (`wdm.h`).
const FILE_FS_VOLUME_INFORMATION: u32 = 1;
/// `FileFsAttributeInformation` (`wdm.h`).
const FILE_FS_ATTRIBUTE_INFORMATION: u32 = 5;

/// The drive letter of the current directory (`C` when it has none).
fn current_letter() -> u16 {
    match current_dir().as_slice() {
        [l, c, ..] if *c == u16::from(b':') => *l,
        _ => u16::from(b'C'),
    }
}

/// Query class `class` of the root directory of drive `letter`
/// (`ERROR_PATH_NOT_FOUND` when there is no such drive).
fn query_root(letter: u16, class: u32) -> Result<Vec<u8>, u32> {
    let root = [letter, u16::from(b':'), u16::from(b'\\')];
    let name = nt_name(&root)?;
    let oa = name.attributes();
    let mut h: Handle = 0;
    let mut iosb = IoStatusBlock::default();
    // SAFETY: valid pointers.
    let st = unsafe {
        nt::NtOpenFile(
            &mut h,
            SYNCHRONIZE | FILE_READ_ATTRIBUTES,
            &oa,
            &mut iosb,
            7,
            FILE_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT,
        )
    };
    if !st.is_success() {
        return Err(ERROR_PATH_NOT_FOUND);
    }
    let mut info = alloc::vec![0u8; 512];
    // SAFETY: valid pointers; `info` holds 512 bytes.
    let st = unsafe { nt::NtQueryVolumeInformationFile(h, &mut iosb, info.as_mut_ptr(), 512, class) };
    close(h);
    if !st.is_success() {
        return Err(from_status(st));
    }
    info.truncate(iosb.information.min(512));
    Ok(info)
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    b.get(at..at + 4).map_or(0, |s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn units(b: &[u8], at: usize, bytes: usize) -> Vec<u16> {
    b.get(at..at + bytes).unwrap_or(&[]).chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect()
}

/// Write `s` and a NUL into `buf` of `capacity` units, cut to fit (`buf` may be NULL).
///
/// # Safety
/// `buf` is NULL or holds `capacity` units.
unsafe fn put_cut(buf: *mut u16, capacity: u32, s: &[u16]) {
    if buf.is_null() || capacity == 0 {
        return;
    }
    let n = s.len().min(capacity as usize - 1);
    // SAFETY: the caller's buffer of `capacity` units; `n + 1` fit.
    unsafe {
        core::ptr::copy_nonoverlapping(s.as_ptr(), buf, n);
        *buf.add(n) = 0;
    }
}

/// Write `v` through `p` unless it is null.
///
/// # Safety
/// `p` is null or writable.
unsafe fn put<T>(p: *mut T, v: T) {
    if !p.is_null() {
        // SAFETY: the caller's contract.
        unsafe { p.write_unaligned(v) };
    }
}

/// `GetVolumeInformationW`: the label, serial number, longest name
/// component, flags and file system name of the volume whose root
/// `lpRootPathName` names. Every output is optional; a file system name
/// buffer too small for the name is `ERROR_BAD_LENGTH`, and a failing call
/// writes nothing. A label longer than its buffer is cut.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetVolumeInformationW(
    root: *const u16,
    volume_name: *mut u16,
    volume_name_size: u32,
    serial: *mut u32,
    max_component: *mut u32,
    flags: *mut u32,
    fs_name: *mut u16,
    fs_name_size: u32,
) -> i32 {
    // SAFETY: the caller's string.
    let name = unsafe { wide_arg(root) };
    let letter = match volume::information_root(name) {
        Ok(r) => r.letter(current_letter()).unwrap_or(u16::from(b'C')),
        Err(e) => return fail(e, FALSE),
    };
    let (vol, attr) =
        match (query_root(letter, FILE_FS_VOLUME_INFORMATION), query_root(letter, FILE_FS_ATTRIBUTE_INFORMATION)) {
            (Ok(v), Ok(a)) => (v, a),
            (Err(e), _) | (_, Err(e)) => return fail(e, FALSE),
        };
    // FILE_FS_VOLUME_INFORMATION: serial 8, label length 12, label 18.
    let label = units(&vol, 18, u32_at(&vol, 12) as usize);
    // FILE_FS_ATTRIBUTE_INFORMATION: attributes 0, max component 4, name length 8, name 12.
    let fs = units(&attr, 12, u32_at(&attr, 8) as usize);
    if !fs_name.is_null() && (fs_name_size as usize) < fs.len() + 1 {
        return fail(ERROR_BAD_LENGTH, FALSE);
    }
    // SAFETY: the caller's buffers and out-pointers, each optional.
    unsafe {
        put_cut(volume_name, volume_name_size, &label);
        put(serial, u32_at(&vol, 8));
        put(max_component, u32_at(&attr, 4));
        put(flags, u32_at(&attr, 0));
        put_cut(fs_name, fs_name_size, &fs);
    }
    TRUE
}

/// `GetVolumePathNameW`: the root of the volume that holds a name (which
/// need not exist), with its trailing backslash. A drive that is not there
/// is `ERROR_FILE_NOT_FOUND`; the buffer rules are
/// [`poly_kernelbase::volume::path_into`]'s. A successful call leaves the
/// last error at `ERROR_MORE_DATA` (the sheet).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetVolumePathNameW(name: *const u16, buf: *mut u16, capacity: u32) -> i32 {
    // SAFETY: the caller's string.
    let Some(name) = (unsafe { wide_arg(name) }) else { return fail(ERROR_INVALID_PARAMETER, FALSE) };
    let root = match volume::path_root(name) {
        Ok(r) => r,
        Err(e) => return fail(e, FALSE),
    };
    let current = current_letter();
    if let Root::Drive { letter, .. } = root
        && query_root(letter, FILE_FS_VOLUME_INFORMATION).is_err()
    {
        return fail(ERROR_FILE_NOT_FOUND, FALSE);
    }
    match volume::path_into(&root.text(current), capacity as usize) {
        Ok(text) => {
            // SAFETY: `text` (with its NUL) fits the caller's `capacity` units.
            unsafe { core::ptr::copy_nonoverlapping(text.as_ptr(), buf, text.len()) };
            set_last_error(ERROR_MORE_DATA);
            TRUE
        }
        Err(e) => {
            if e == ERROR_FILENAME_EXCED_RANGE && !buf.is_null() {
                // SAFETY: at least one unit (capacity is not 0 here).
                unsafe { *buf = 0 };
            }
            fail(e, FALSE)
        }
    }
}

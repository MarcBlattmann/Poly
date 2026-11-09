//! Process and thread attribute lists (Learn: "InitializeProcThreadAttributeList",
//! "UpdateProcThreadAttribute", "DeleteProcThreadAttributeList"; the
//! attribute numbers are `winbase.h`'s; ADR 0169). The list is Poly's own
//! layout in the caller's memory: a header (capacity, count) and one
//! `(attribute, size, value)` record per attribute; `value` points at the
//! caller's data, which must outlive the list, as Learn says.

use crate::base::*;
use alloc::vec::Vec;
use poly_kernelbase::error::*;

pub const PARENT_PROCESS: usize = 0x0002_0000;
pub const HANDLE_LIST: usize = 0x0002_0002;
pub const MITIGATION_POLICY: usize = 0x0002_0007;
pub const SECURITY_CAPABILITIES: usize = 0x0002_0009;
pub const JOB_LIST: usize = 0x0002_000D;
pub const CHILD_PROCESS_POLICY: usize = 0x0002_000E;

/// The header and one record.
const HEADER: usize = 8;
const RECORD: usize = 24;

fn needed(count: u32) -> usize {
    HEADER + RECORD * count as usize
}

/// `InitializeProcThreadAttributeList`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn InitializeProcThreadAttributeList(
    list: *mut u8,
    count: u32,
    flags: u32,
    size: *mut usize,
) -> i32 {
    if flags != 0 || size.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    let want = needed(count.min(0xFFFF));
    // SAFETY: the caller's size.
    let have = unsafe { size.read() };
    // SAFETY: the caller's out-pointer.
    unsafe { size.write(want) };
    if list.is_null() || have < want {
        return fail(ERROR_INSUFFICIENT_BUFFER, FALSE);
    }
    // SAFETY: `want` bytes at `list`, which the caller's size allows.
    unsafe {
        core::ptr::write_bytes(list, 0, want);
        (list as *mut u32).write(count);
    }
    TRUE
}

/// `UpdateProcThreadAttribute`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn UpdateProcThreadAttribute(
    list: *mut u8,
    flags: u32,
    attribute: usize,
    value: *const u8,
    size: usize,
    previous: *mut u8,
    return_size: *mut usize,
) -> i32 {
    let _ = return_size;
    if list.is_null() || flags != 0 || !previous.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    let size_ok = match attribute {
        PARENT_PROCESS => Some(size == 8),
        HANDLE_LIST | JOB_LIST => Some(size != 0 && size % 8 == 0),
        MITIGATION_POLICY => Some(size == 4 || size == 8),
        CHILD_PROCESS_POLICY => Some(size == 4),
        SECURITY_CAPABILITIES => Some(size == 24),
        _ => None,
    };
    let Some(size_ok) = size_ok else { return fail(ERROR_NOT_SUPPORTED, FALSE) };
    // SAFETY: an initialised list: the header and `capacity` records.
    unsafe {
        let capacity = (list as *const u32).read() as usize;
        let count = (list as *const u32).add(1).read() as usize;
        let records = list.add(HEADER) as *mut usize;
        for i in 0..count {
            if records.add(i * 3).read() == attribute {
                return fail(ERROR_OBJECT_NAME_EXISTS, FALSE);
            }
        }
        if count >= capacity {
            return fail(ERROR_GEN_FAILURE, FALSE);
        }
        if !size_ok {
            return fail(ERROR_BAD_LENGTH, FALSE);
        }
        records.add(count * 3).write(attribute);
        records.add(count * 3 + 1).write(size);
        records.add(count * 3 + 2).write(value as usize);
        (list as *mut u32).add(1).write(count as u32 + 1);
    }
    TRUE
}

/// `DeleteProcThreadAttributeList`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DeleteProcThreadAttributeList(list: *mut u8) {
    if !list.is_null() {
        // SAFETY: an initialised list (the header).
        unsafe { (list as *mut u32).add(1).write(0) };
    }
}

/// What a list asks `CreateProcessW` for.
#[derive(Default)]
pub struct Parsed {
    pub handle_list: Option<Vec<usize>>,
    pub mitigation: u64,
    pub jobs: Vec<usize>,
}

/// The attributes `CreateProcessW` uses, read from `list`.
///
/// # Safety
/// `list` is null or a list made by [`InitializeProcThreadAttributeList`]
/// whose values are still valid.
pub unsafe fn parse(list: *const u8) -> Result<Parsed, u32> {
    let mut out = Parsed::default();
    if list.is_null() {
        return Ok(out);
    }
    // SAFETY: as the contract says.
    unsafe {
        let count = (list as *const u32).add(1).read() as usize;
        let records = list.add(HEADER) as *const usize;
        for i in 0..count {
            let (attribute, size, value) =
                (records.add(i * 3).read(), records.add(i * 3 + 1).read(), records.add(i * 3 + 2).read());
            let words = || -> Vec<usize> {
                (0..size / 8).map(|j| (value as *const usize).add(j).read_unaligned()).collect()
            };
            match attribute {
                HANDLE_LIST => out.handle_list = Some(words()),
                JOB_LIST => out.jobs = words(),
                MITIGATION_POLICY => {
                    if value == 0 {
                        return Err(ERROR_INVALID_PARAMETER);
                    }
                    out.mitigation = if size == 4 {
                        u64::from((value as *const u32).read_unaligned())
                    } else {
                        (value as *const u64).read_unaligned()
                    };
                }
                _ => {}
            }
        }
    }
    Ok(out)
}

/// Whether `v` holds only `PROCESS_CREATION_MITIGATION_POLICY_*` values
/// Windows defines (`winbase.h`): the one-bit flags at bits 0 to 2 and
/// two-bit fields at every fourth bit from 8 to 52.
pub fn mitigation_defined(v: u64) -> bool {
    let mut ok: u64 = 0b111;
    let mut shift = 8;
    while shift <= 52 {
        ok |= 0b11 << shift;
        shift += 4;
    }
    v & !ok == 0
}

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
/// `PROC_THREAD_ATTRIBUTE_COMPONENT_FILTER` (`winbase.h`, number 0x1A): accepted, nothing in the child shows it.
pub const COMPONENT_FILTER: usize = 0x0002_001A;

/// The header and one record.
const HEADER: usize = 8;
const RECORD: usize = 3 * core::mem::size_of::<usize>();

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
        // A HANDLE (8 bytes, 4 in a 32-bit process) or an array of them.
        PARENT_PROCESS => Some(size == core::mem::size_of::<usize>()),
        HANDLE_LIST | JOB_LIST => Some(size != 0 && size % core::mem::size_of::<usize>() == 0),
        MITIGATION_POLICY => Some(size == 4 || size == 8 || size == 16),
        CHILD_PROCESS_POLICY | COMPONENT_FILTER => Some(size == 4),
        // SECURITY_CAPABILITIES (`winnt.h`): two pointers and two DWORDs (24 bytes, 16 in a 32-bit process).
        SECURITY_CAPABILITIES => Some(size == 2 * core::mem::size_of::<usize>() + 8),
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
    /// The two mitigation words (the second is 0 for a value of 8 bytes or less) and whether the
    /// attribute was given.
    pub mitigation: [u64; 2],
    pub has_mitigation: bool,
    /// `PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY`'s value, if given.
    pub child_policy: Option<u32>,
    pub jobs: Vec<usize>,
    /// `PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES`: the caller's `SECURITY_CAPABILITIES` (ADR 0196).
    pub security_capabilities: Option<usize>,
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
                (0..size / core::mem::size_of::<usize>())
                    .map(|j| (value as *const usize).add(j).read_unaligned())
                    .collect()
            };
            match attribute {
                HANDLE_LIST => out.handle_list = Some(words()),
                JOB_LIST => out.jobs = words(),
                SECURITY_CAPABILITIES => {
                    if value == 0 {
                        return Err(ERROR_INVALID_PARAMETER);
                    }
                    out.security_capabilities = Some(value);
                }
                MITIGATION_POLICY => {
                    if value == 0 {
                        return Err(ERROR_INVALID_PARAMETER);
                    }
                    // 16 bytes are two policy words (`PROCESS_CREATION_MITIGATION_POLICY2`).
                    out.has_mitigation = true;
                    out.mitigation[0] = if size == 4 {
                        u64::from((value as *const u32).read_unaligned())
                    } else {
                        (value as *const u64).read_unaligned()
                    };
                    if size == 16 {
                        out.mitigation[1] = (value as *const u64).add(1).read_unaligned();
                    }
                }
                CHILD_PROCESS_POLICY => {
                    if value == 0 {
                        return Err(ERROR_INVALID_PARAMETER);
                    }
                    out.child_policy = Some((value as *const u32).read_unaligned());
                }
                _ => {}
            }
        }
    }
    Ok(out)
}

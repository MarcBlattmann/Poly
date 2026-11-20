//! `K32EnumProcessModules` (Learn, "EnumProcessModules" in `psapi.h`): the module handles (load
//! addresses) of a process, read from its loader's list with `NtReadVirtualMemory`. The fields
//! are `winternl.h`'s: `PROCESS_BASIC_INFORMATION.PebBaseAddress`, `PEB.Ldr`,
//! `PEB_LDR_DATA.InMemoryOrderModuleList` and in each `LDR_DATA_TABLE_ENTRY` its
//! `InMemoryOrderLinks` and `DllBase`. The executable is the first module (Poly's loader puts it
//! first, as Windows does).

use crate::base::*;
use crate::nt::{self, Handle};
/// `ERROR_PARTIAL_COPY` (`winerror.h`).
const ERROR_PARTIAL_COPY: u32 = 299;

const P: usize = core::mem::size_of::<usize>();
/// `PROCESS_BASIC_INFORMATION.PebBaseAddress`.
const PBI_PEB: usize = P;
/// `PEB.Ldr`.
const PEB_LDR: usize = if P == 4 { 0x0C } else { 0x18 };
/// `PEB_LDR_DATA.InMemoryOrderModuleList`.
const LDR_IN_MEMORY_ORDER: usize = if P == 4 { 0x14 } else { 0x20 };
/// `LDR_DATA_TABLE_ENTRY.InMemoryOrderLinks` and `.DllBase`.
const ENTRY_LINKS: usize = if P == 4 { 0x08 } else { 0x10 };
const ENTRY_DLL_BASE: usize = if P == 4 { 0x18 } else { 0x30 };
/// A list longer than this is taken as broken.
const MAX_MODULES: usize = 4096;

/// A pointer-sized value of `process` at `at`.
fn read_ptr(process: Handle, at: usize) -> Option<usize> {
    let mut v = 0usize;
    let mut done = 0usize;
    // SAFETY: a buffer of `P` bytes; the kernel checks the address in the target.
    let st = unsafe { nt::NtReadVirtualMemory(process, at as *const u8, (&raw mut v).cast(), P, &mut done) };
    (st.is_success() && done == P).then_some(v)
}

/// `K32EnumProcessModules`: as many module handles as `cb` bytes hold, the bytes all of them
/// need in `*needed`. A process whose loader has not started yet has no list:
/// `ERROR_PARTIAL_COPY`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn K32EnumProcessModules(
    process: Handle,
    modules: *mut usize,
    cb: u32,
    needed: *mut u32,
) -> i32 {
    let mut info = [0u8; 48];
    // SAFETY: valid pointers.
    let st =
        unsafe { nt::NtQueryInformationProcess(process, 0, info.as_mut_ptr(), 6 * P as u32, core::ptr::null_mut()) };
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    let mut peb = [0u8; P];
    peb.copy_from_slice(&info[PBI_PEB..PBI_PEB + P]);
    let peb = usize::from_le_bytes(peb);
    let Some(ldr) = read_ptr(process, peb + PEB_LDR).filter(|&l| l != 0) else {
        return fail(ERROR_PARTIAL_COPY, FALSE);
    };
    let head = ldr + LDR_IN_MEMORY_ORDER;
    let room = cb as usize / P;
    let mut count = 0usize;
    let mut link = match read_ptr(process, head) {
        Some(l) => l,
        None => return fail(ERROR_PARTIAL_COPY, FALSE),
    };
    while link != head && link != 0 && count < MAX_MODULES {
        let Some(base) = read_ptr(process, link - ENTRY_LINKS + ENTRY_DLL_BASE) else {
            return fail(ERROR_PARTIAL_COPY, FALSE);
        };
        if count < room && !modules.is_null() {
            // SAFETY: the caller's array holds `cb` bytes.
            unsafe { modules.add(count).write_unaligned(base) };
        }
        count += 1;
        link = match read_ptr(process, link) {
            Some(l) => l,
            None => return fail(ERROR_PARTIAL_COPY, FALSE),
        };
    }
    if !needed.is_null() {
        // SAFETY: the caller's out-pointer.
        unsafe { needed.write_unaligned((count * P) as u32) };
    }
    TRUE
}

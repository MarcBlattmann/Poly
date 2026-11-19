//! The tool help library's process snapshot (`tlhelp32.h`; sheets `kernel32.dll/CreateToolhelp32Snapshot`,
//! `Process32FirstW`, `Process32NextW`). Chrome's setup walks it to find running copies of the
//! browser. The snapshot is a section holding a copy of the system's process list
//! (`ntdll!PolyProcessList`) and the walk position; the rules are `poly_kernelbase::toolhelp`'s.

use crate::base::*;
use crate::mapping::{CreateFileMappingW, MapViewOfFile, UnmapViewOfFile};
use crate::nt::{self, Handle};
use alloc::vec::Vec;
use poly_kernelbase::error::*;
use poly_kernelbase::toolhelp::{self, EntryLayout, HEADER_SIZE, Step};

/// `PAGE_READWRITE`, `FILE_MAP_WRITE` (`winnt.h`, `memoryapi.h`).
const PAGE_READWRITE: u32 = 0x04;
const FILE_MAP_WRITE: u32 = 0x02;

/// The system's process list as `PolyProcessList` records.
fn process_records() -> Result<Vec<u8>, u32> {
    let mut buf: Vec<u8> = alloc::vec![0; 64 * toolhelp::RECORD_SIZE];
    loop {
        let mut need = 0u32;
        // SAFETY: a buffer of the stated length and a u32 out-value.
        let st = unsafe { nt::PolyProcessList(buf.as_mut_ptr(), buf.len() as u32, &mut need) };
        if st.is_success() {
            buf.truncate(need as usize);
            return Ok(buf);
        }
        if (need as usize) <= buf.len() {
            return Err(poly_kernelbase::error::from_status(st));
        }
        // The list grew between the calls: ask again with room for a few more.
        buf.resize(need as usize + 8 * toolhelp::RECORD_SIZE, 0);
    }
}

/// `CreateToolhelp32Snapshot(dwFlags, th32ProcessID)`: a snapshot of the process list for
/// `TH32CS_SNAPPROCESS` (the process id does not narrow it); any other flags give a snapshot that
/// holds no process. A module snapshot of a process that does not exist is `ERROR_INVALID_PARAMETER`.
/// The handle is inheritable with `TH32CS_INHERIT` only.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateToolhelp32Snapshot(flags: u32, process: u32) -> Handle {
    let wants_modules = flags & (toolhelp::TH32CS_SNAPMODULE | toolhelp::TH32CS_SNAPMODULE32) != 0;
    let records = if flags & toolhelp::TH32CS_SNAPPROCESS != 0 || (wants_modules && process != 0) {
        match process_records() {
            Ok(r) => r,
            Err(e) => return fail(e, INVALID_HANDLE_VALUE),
        }
    } else {
        Vec::new()
    };
    if wants_modules && process != 0 && !toolhelp::lists(&records, process) {
        return fail(ERROR_INVALID_PARAMETER, INVALID_HANDLE_VALUE);
    }
    let listed: &[u8] = if flags & toolhelp::TH32CS_SNAPPROCESS != 0 { &records } else { &[] };
    let image = toolhelp::snapshot_image(listed);
    let sa = SecurityAttributesRaw {
        length: core::mem::size_of::<SecurityAttributesRaw>() as u32,
        descriptor: core::ptr::null(),
        inherit: i32::from(flags & toolhelp::TH32CS_INHERIT != 0),
    };
    // SAFETY: a paging-file section of the image's size; the attributes live across the call.
    let section = unsafe {
        CreateFileMappingW(
            INVALID_HANDLE_VALUE,
            (&sa as *const SecurityAttributesRaw).cast(),
            PAGE_READWRITE,
            0,
            image.len() as u32,
            core::ptr::null(),
        )
    };
    if section == 0 {
        return INVALID_HANDLE_VALUE;
    }
    // SAFETY: a view of the whole new section.
    let view = unsafe { MapViewOfFile(section, FILE_MAP_WRITE, 0, 0, image.len()) };
    if view.is_null() {
        let e = last_error();
        close(section);
        return fail(e, INVALID_HANDLE_VALUE);
    }
    // SAFETY: the view holds `image.len()` bytes.
    unsafe {
        core::ptr::copy_nonoverlapping(image.as_ptr(), view, image.len());
        UnmapViewOfFile(view);
    }
    section
}

/// One step of a walk: check `dwSize`, map the snapshot, copy the entry and move the position.
///
/// # Safety
/// `entry` is null or the caller's `PROCESSENTRY32W`.
unsafe fn walk(snapshot: Handle, entry: *mut u8, which: Step) -> i32 {
    let layout = EntryLayout::NATIVE;
    if entry.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: the caller's PROCESSENTRY32W starts with dwSize.
    if unsafe { entry.cast::<u32>().read_unaligned() } as usize != layout.size {
        return fail(ERROR_BAD_LENGTH, FALSE);
    }
    if snapshot == 0 || snapshot == INVALID_HANDLE_VALUE {
        return fail(ERROR_INVALID_HANDLE, FALSE);
    }
    // SAFETY: a view of the whole section (0 bytes: to its end).
    let view = unsafe { MapViewOfFile(snapshot, FILE_MAP_WRITE, 0, 0, 0) };
    if view.is_null() {
        return fail(ERROR_INVALID_HANDLE, FALSE);
    }
    // The section's size: the header says how many records follow.
    let mut size = HEADER_SIZE;
    if let Some(info) = query_memory(view as usize) {
        size = size.max(info.region_size);
    }
    // SAFETY: the view is at least `size` bytes (a mapped region of that size).
    let section = unsafe { core::slice::from_raw_parts_mut(view, size) };
    let r = match toolhelp::step(section, which) {
        Ok((at, next)) => {
            // SAFETY: the caller's entry of `layout.size` bytes (checked through dwSize).
            let out = unsafe { core::slice::from_raw_parts_mut(entry, layout.size) };
            layout.fill(out, &section[at..at + toolhelp::RECORD_SIZE]);
            section[4..8].copy_from_slice(&next.to_le_bytes());
            TRUE
        }
        Err(e) => fail(e, FALSE),
    };
    // SAFETY: the view mapped above.
    unsafe { UnmapViewOfFile(view) };
    r
}

/// `Process32FirstW(hSnapshot, lppe)`: the first entry, and the walk starts over there.
/// `ERROR_BAD_LENGTH` for a `dwSize` that is not the structure's, `ERROR_INVALID_HANDLE` for a
/// handle that is no snapshot, `ERROR_NO_MORE_FILES` for a snapshot without processes.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Process32FirstW(snapshot: Handle, entry: *mut u8) -> i32 {
    // SAFETY: the caller's entry.
    unsafe { walk(snapshot, entry, Step::First) }
}

/// `Process32NextW(hSnapshot, lppe)`: the entry after the last one given (the first on a fresh
/// snapshot), `ERROR_NO_MORE_FILES` past the end.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Process32NextW(snapshot: Handle, entry: *mut u8) -> i32 {
    // SAFETY: the caller's entry.
    unsafe { walk(snapshot, entry, Step::Next) }
}

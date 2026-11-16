//! The tool help library's process snapshot (`tlhelp32.h`, Learn: "CreateToolhelp32Snapshot",
//! "Process32FirstW", "Process32NextW"). A program that wants to know whether another copy of
//! itself runs walks it; Poly's snapshot holds the calling process only (the processes of an
//! NT session are not listed to each other), so a program sees itself, with its image name, its
//! thread count (1: the threads are not counted) and its parent. The snapshot is a handle
//! (an event object) that `CloseHandle` closes; the position of its walk is kept in a table of
//! this module.
//!
//! `PROCESSENTRY32W` is 568 bytes in a 64-bit process and 556 in a 32-bit one (the heap
//! identifier is pointer sized): `dwSize`, `cntUsage`, `th32ProcessID`, `th32DefaultHeapID`,
//! `th32ModuleID`, `cntThreads`, `th32ParentProcessID`, `pcPriClassBase`, `dwFlags`, `szExeFile[260]`.

use crate::base::*;
use crate::nt::Handle;
use crate::process::{GetCurrentProcessId, GetModuleFileNameW};
use crate::sync::CreateEventW;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use poly_kernelbase::error::*;

/// `TH32CS_SNAPPROCESS` (`tlhelp32.h`).
const TH32CS_SNAPPROCESS: u32 = 0x2;
/// `TH32CS_SNAPALL`: the process part is all that Poly lists.
const TH32CS_ALL_KINDS: u32 = 0x1 | 0x2 | 0x4 | 0x8;
/// `TH32CS_INHERIT`.
const TH32CS_INHERIT: u32 = 0x8000_0000;

const POINTER: usize = core::mem::size_of::<usize>();
/// `sizeof(PROCESSENTRY32W)`.
const ENTRY_SIZE: usize = 4 * 3 + POINTER + 4 * 5 + 520 + if POINTER == 8 { 4 } else { 0 };
const OFF_ID: usize = 8;
const OFF_THREADS: usize = 12 + POINTER + 4;
const OFF_PARENT: usize = OFF_THREADS + 4;
const OFF_PRIORITY: usize = OFF_PARENT + 4;
const OFF_FLAGS: usize = OFF_PRIORITY + 4;
const OFF_NAME: usize = OFF_FLAGS + 4;

struct Walks {
    locked: AtomicBool,
    /// The snapshots: handle, and whether `Process32FirstW` has been answered.
    list: core::cell::UnsafeCell<Vec<(Handle, bool)>>,
}

// SAFETY: access to `list` is serialised by `locked`.
unsafe impl Sync for Walks {}

static WALKS: Walks = Walks { locked: AtomicBool::new(false), list: core::cell::UnsafeCell::new(Vec::new()) };

fn with_walks<R>(f: impl FnOnce(&mut Vec<(Handle, bool)>) -> R) -> R {
    while WALKS.locked.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
    // SAFETY: the lock is held.
    let r = f(unsafe { &mut *WALKS.list.get() });
    WALKS.locked.store(false, Ordering::Release);
    r
}

/// `CreateToolhelp32Snapshot(dwFlags, th32ProcessID)`: a snapshot handle of the processes, or `INVALID_HANDLE_VALUE`
/// for flags that name nothing (`ERROR_INVALID_PARAMETER`). Only the process list exists; the other kinds are
/// accepted and hold nothing.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateToolhelp32Snapshot(flags: u32, _process: u32) -> Handle {
    if flags & !(TH32CS_ALL_KINDS | TH32CS_INHERIT) != 0 || flags & TH32CS_ALL_KINDS == 0 {
        return fail(ERROR_INVALID_PARAMETER, -1);
    }
    // SAFETY: an unnamed manual-reset event is the snapshot's handle.
    let h = unsafe { CreateEventW(core::ptr::null(), 1, 0, core::ptr::null()) };
    if h == 0 {
        return -1;
    }
    let walks_processes = flags & TH32CS_SNAPPROCESS != 0;
    with_walks(|w| w.push((h, !walks_processes)));
    h
}

/// The entry of the calling process, in `entry` (of `ENTRY_SIZE` bytes).
unsafe fn fill(entry: *mut u8) {
    let mut path = [0u16; 520];
    // SAFETY: a buffer of 520 units; the entry holds `ENTRY_SIZE` bytes.
    unsafe {
        let n = GetModuleFileNameW(core::ptr::null_mut(), path.as_mut_ptr(), 520) as usize;
        let name_start = path[..n.min(520)].iter().rposition(|&c| c == u16::from(b'\\')).map_or(0, |i| i + 1);
        let name = &path[name_start..n.min(520)];
        core::ptr::write_bytes(entry.add(4), 0, ENTRY_SIZE - 4);
        entry.add(OFF_ID).cast::<u32>().write_unaligned(GetCurrentProcessId());
        entry.add(OFF_THREADS).cast::<u32>().write_unaligned(1);
        // The base priority of the normal class.
        entry.add(OFF_PRIORITY).cast::<i32>().write_unaligned(8);
        for (i, &c) in name.iter().take(259).enumerate() {
            entry.add(OFF_NAME + 2 * i).cast::<u16>().write_unaligned(c);
        }
    }
}

/// `Process32FirstW(hSnapshot, lppe)`: the first entry, `FALSE` with `ERROR_NO_MORE_FILES` when the snapshot holds none;
/// `ERROR_BAD_LENGTH` for an entry whose `dwSize` is not the structure's, `ERROR_INVALID_HANDLE` for a handle that
/// is no snapshot.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Process32FirstW(snapshot: Handle, entry: *mut u8) -> i32 {
    if entry.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: the caller's PROCESSENTRY32W.
    if unsafe { entry.cast::<u32>().read_unaligned() } as usize != ENTRY_SIZE {
        return fail(ERROR_BAD_LENGTH, FALSE);
    }
    // 1: a snapshot to walk; 2: a snapshot of other kinds, which holds no process; 0: no such snapshot.
    let state = with_walks(|w| match w.iter_mut().find(|(h, _)| *h == snapshot) {
        Some((_, skip)) => {
            let walkable = !*skip;
            *skip = true;
            if walkable { 1 } else { 2 }
        }
        None => 0,
    });
    match state {
        0 => fail(ERROR_INVALID_HANDLE, FALSE),
        1 => {
            // SAFETY: the entry holds ENTRY_SIZE bytes (checked through `dwSize`).
            unsafe { fill(entry) };
            TRUE
        }
        _ => fail(ERROR_NO_MORE_FILES, FALSE),
    }
}

/// `Process32NextW(hSnapshot, lppe)`: nothing is left after the one entry: `FALSE` with `ERROR_NO_MORE_FILES`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Process32NextW(snapshot: Handle, entry: *mut u8) -> i32 {
    if entry.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: the caller's PROCESSENTRY32W.
    if unsafe { entry.cast::<u32>().read_unaligned() } as usize != ENTRY_SIZE {
        return fail(ERROR_BAD_LENGTH, FALSE);
    }
    if with_walks(|w| w.iter().any(|(h, _)| *h == snapshot)) {
        fail(ERROR_NO_MORE_FILES, FALSE)
    } else {
        fail(ERROR_INVALID_HANDLE, FALSE)
    }
}

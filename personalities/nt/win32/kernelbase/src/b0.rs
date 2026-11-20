//! What Google's Chrome installers and their updater need from `kernel32` that Poly lacked (batch
//! B0 of `docs/windows/chrome-installer-plan.md`): `GetDriveTypeW`, the working set functions,
//! `K32QueryWorkingSetEx`, `WTSGetActiveConsoleSessionId`, `GetTempPath2W` and
//! `GetVolumeInformationByHandleW`. The decisions are `poly_kernelbase::{drivetype, workingset,
//! wtsenum}`'s; the sheets are under `specs/windows/kernel32.dll/`.

use crate::base::*;
use crate::nt::{self, Handle, IoStatusBlock};
use alloc::vec::Vec;
use poly_kernelbase::drivetype::{self, DriveQuery};
use poly_kernelbase::workingset::{self, WorkingSet};
use poly_ntobj::NtStatus;

/// Poly's own process information class for the working set limits (`poly_kernelbase::workingset`:
/// set takes a request of four `u64`s, query answers three), the same for 32- and 64-bit callers.
const PROCESS_POLY_WORKING_SET: u32 = 0x1002;
/// Poly's own `NtQueryVirtualMemory` class for `K32QueryWorkingSetEx`: an array of 16-byte entries
/// (`u64` address in, `u64` attribute word out) whatever the caller's pointer size.
const MEMORY_POLY_WORKING_SET_EX: u32 = 0x1004;
/// `ERROR_NOACCESS`, `ERROR_BAD_LENGTH` (`winerror.h`).
const ERROR_NOACCESS: u32 = 998;
const INVALID_FILE_ATTRIBUTES: u32 = u32::MAX;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;

// ----- GetDriveTypeW --------------------------------------------------------------------------

/// The type of drive `letter`: `DRIVE_FIXED` for a drive Poly has (its volumes are local disks),
/// else `DRIVE_NO_ROOT_DIR`.
fn letter_type(letter: u8) -> u32 {
    let bit = 1u32 << (letter - b'A');
    if crate::system::GetLogicalDrives() & bit != 0 { drivetype::DRIVE_FIXED } else { drivetype::DRIVE_NO_ROOT_DIR }
}

/// `GetDriveTypeW(lpRootPathName)`: the kind of drive whose root the name gives
/// (`specs/windows/kernel32.dll/GetDriveTypeW.yaml`): 3 for every spelling of a root of one of
/// Poly's drives (and for a directory named with a trailing backslash that exists), 1 for
/// everything else. NULL is the current directory's root. The last error is not part of the answer.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetDriveTypeW(root: *const u16) -> u32 {
    let cwd = current_dir();
    // SAFETY: the caller's NUL-terminated name (or NULL).
    let name = unsafe { wide_arg(root) };
    let error = last_error();
    let kind = match drivetype::drive_query(name, &cwd) {
        DriveQuery::Root(d) => letter_type(d),
        DriveQuery::Directory(mut p) => {
            p.push(0);
            // SAFETY: a NUL-terminated name.
            let a = unsafe { crate::file::GetFileAttributesW(p.as_ptr()) };
            if a != INVALID_FILE_ATTRIBUTES && a & FILE_ATTRIBUTE_DIRECTORY != 0 {
                letter_type(p[0] as u8)
            } else {
                drivetype::DRIVE_NO_ROOT_DIR
            }
        }
        DriveQuery::NoRoot => drivetype::DRIVE_NO_ROOT_DIR,
    };
    set_last_error(error);
    kind
}

// ----- sessions -------------------------------------------------------------------------------

/// `WTSGetActiveConsoleSessionId`: the session of the console (`poly_kernelbase::wtsenum`).
#[unsafe(no_mangle)]
pub extern "system" fn WTSGetActiveConsoleSessionId() -> u32 {
    poly_kernelbase::wtsenum::CONSOLE_SESSION
}

// ----- GetTempPath2W --------------------------------------------------------------------------

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    fn NtOpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> NtStatus;
    fn NtQueryInformationToken(token: Handle, class: u32, info: *mut u8, length: u32, ret: *mut u32) -> NtStatus;
    fn NtClose(handle: Handle) -> NtStatus;
}

/// Whether the process runs as the local system (its token's user is `S-1-5-18`).
fn runs_as_system() -> bool {
    const TOKEN_QUERY: u32 = 8;
    const TOKEN_USER: u32 = 1;
    let mut token: Handle = 0;
    // SAFETY: the current process; a local out-handle.
    if !unsafe { NtOpenProcessToken(nt::CURRENT_PROCESS, TOKEN_QUERY, &mut token) }.is_success() {
        return false;
    }
    let mut buf = [0usize; 16];
    let mut ret = 0u32;
    // SAFETY: a buffer of 16 pointers; the token opened above, closed below.
    let ok = unsafe {
        let st = NtQueryInformationToken(token, TOKEN_USER, buf.as_mut_ptr().cast(), 128, &mut ret);
        NtClose(token);
        st.is_success()
    };
    let start = buf.as_ptr() as usize;
    let sid = buf[0];
    if !ok || sid < start || sid + 12 > start + 128 {
        return false;
    }
    // SAFETY: the SID lies in `buf` (checked above).
    let bytes = unsafe { core::slice::from_raw_parts(sid as *const u8, 12) };
    bytes == [1, 1, 0, 0, 0, 0, 0, 5, 18, 0, 0, 0]
}

/// `GetTempPath2W(BufferLength, Buffer)`: `C:\Windows\SystemTemp\` for a process of the local
/// system, else what `GetTempPathW` gives (`specs/windows/kernel32.dll/SystemTemp.yaml`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetTempPath2W(size: u32, buf: *mut u16) -> u32 {
    if runs_as_system() {
        let p: Vec<u16> = "C:\\Windows\\SystemTemp\\".encode_utf16().collect();
        // SAFETY: the caller's buffer.
        return unsafe { return_sized(&p, buf, size) };
    }
    // SAFETY: the caller's buffer.
    unsafe { crate::file::GetTempPathW(size, buf) }
}

// ----- working sets ---------------------------------------------------------------------------

/// `SetProcessWorkingSetSizeEx(hProcess, dwMinimumWorkingSetSize, dwMaximumWorkingSetSize,
/// Flags)`: the flags, then the sizes are judged here (`ERROR_INVALID_PARAMETER`), then the kernel
/// checks the handle (`PROCESS_SET_QUOTA`) and that the sizes fit the machine
/// (`ERROR_NO_SYSTEM_RESOURCES`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetProcessWorkingSetSizeEx(process: Handle, min: usize, max: usize, flags: u32) -> i32 {
    if let Err(e) = workingset::check_flags(flags) {
        return fail(e, FALSE);
    }
    let request = match workingset::check_sizes(min as u64, max as u64, usize::MAX as u64) {
        Ok(r) => r,
        Err(e) => return fail(e, FALSE),
    };
    let bytes = workingset::request_bytes(request, flags);
    // SAFETY: a buffer of the class's size.
    let st = unsafe { nt::NtSetInformationProcess(process, PROCESS_POLY_WORKING_SET, bytes.as_ptr(), 32) };
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    TRUE
}

/// `SetProcessWorkingSetSize`: [`SetProcessWorkingSetSizeEx`] with no flags.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetProcessWorkingSetSize(process: Handle, min: usize, max: usize) -> i32 {
    // SAFETY: as the Ex function.
    unsafe { SetProcessWorkingSetSizeEx(process, min, max, 0) }
}

/// The working set of the process `process` names (a handle with a query right).
fn query_working_set(process: Handle) -> Result<WorkingSet, NtStatus> {
    let mut b = [0u8; 24];
    // SAFETY: a buffer of the class's size.
    let st = unsafe {
        nt::NtQueryInformationProcess(process, PROCESS_POLY_WORKING_SET, b.as_mut_ptr(), 24, core::ptr::null_mut())
    };
    if st.is_success() { Ok(WorkingSet::from_bytes(&b)) } else { Err(st) }
}

/// `GetProcessWorkingSetSizeEx(hProcess, lpMinimumWorkingSetSize, lpMaximumWorkingSetSize,
/// Flags)`: the limits and the hard-limit flags (a handle with `PROCESS_QUERY_INFORMATION` or
/// `PROCESS_QUERY_LIMITED_INFORMATION`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetProcessWorkingSetSizeEx(
    process: Handle,
    min: *mut usize,
    max: *mut usize,
    flags: *mut u32,
) -> i32 {
    if min.is_null() || max.is_null() || flags.is_null() {
        return fail(ERROR_NOACCESS, FALSE);
    }
    match query_working_set(process) {
        Ok(ws) => {
            // SAFETY: the caller's out-pointers.
            unsafe {
                min.write_unaligned(ws.minimum as usize);
                max.write_unaligned(ws.maximum as usize);
                flags.write_unaligned(ws.flags);
            }
            TRUE
        }
        Err(st) => fail_status(st, FALSE),
    }
}

/// `GetProcessWorkingSetSize(hProcess, lpMinimumWorkingSetSize, lpMaximumWorkingSetSize)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetProcessWorkingSetSize(process: Handle, min: *mut usize, max: *mut usize) -> i32 {
    let mut flags = 0u32;
    // SAFETY: as the Ex function.
    unsafe { GetProcessWorkingSetSizeEx(process, min, max, &mut flags) }
}

/// `K32QueryWorkingSetEx(hProcess, pv, cb)`: for each `PSAPI_WORKING_SET_EX_INFORMATION` entry
/// (an address, then the attribute word it fills) whether that page is in the working set and how
/// it is mapped. Less than one entry is `ERROR_BAD_LENGTH`; whole entries are answered, the
/// addresses left alone (`specs/windows/kernel32.dll/K32QueryWorkingSetEx.yaml`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn K32QueryWorkingSetEx(process: Handle, pv: *mut usize, cb: u32) -> i32 {
    let entry = 2 * core::mem::size_of::<usize>();
    if (cb as usize) < entry {
        return fail(workingset::ERROR_BAD_LENGTH, FALSE);
    }
    if pv.is_null() {
        return fail(ERROR_NOACCESS, FALSE);
    }
    let n = cb as usize / entry;
    let mut buf: Vec<u64> = Vec::with_capacity(2 * n);
    for i in 0..n {
        // SAFETY: the caller's array of `n` entries.
        buf.push(unsafe { pv.add(2 * i).read_unaligned() } as u64);
        buf.push(0);
    }
    // SAFETY: `buf` holds `n` entries of 16 bytes.
    let st = unsafe {
        nt::NtQueryVirtualMemory(
            process,
            core::ptr::null(),
            MEMORY_POLY_WORKING_SET_EX,
            buf.as_mut_ptr().cast(),
            16 * n,
            core::ptr::null_mut(),
        )
    };
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    // This process's locked pages (`VirtualLock`) are marked locked when they are valid.
    let own = process as isize == -1 || crate::spawn::GetProcessId(process) == crate::process::GetCurrentProcessId();
    for i in 0..n {
        let mut a = buf[2 * i + 1];
        if own && a & 1 != 0 && with_locked(|l| l.is_locked(buf[2 * i])) {
            a |= workingset::WS_EX_LOCKED;
        }
        // SAFETY: the caller's array of `n` entries.
        unsafe { pv.add(2 * i + 1).write_unaligned(a as usize) };
    }
    TRUE
}

// ----- VirtualLock, VirtualUnlock -------------------------------------------------------------

/// This process's locked pages, under [`LOCKED_LOCK`].
struct LockedCell(core::cell::UnsafeCell<workingset::LockedPages>);
// SAFETY: every access is under `LOCKED_LOCK`.
unsafe impl Sync for LockedCell {}
static LOCKED: LockedCell = LockedCell(core::cell::UnsafeCell::new(workingset::LockedPages::new()));
/// An `SRWLOCK` (one pointer, zero when free).
static LOCKED_LOCK: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

fn with_locked<R>(f: impl FnOnce(&mut workingset::LockedPages) -> R) -> R {
    // SAFETY: a valid SRW lock word.
    unsafe { nt::RtlAcquireSRWLockExclusive(LOCKED_LOCK.as_ptr()) };
    // SAFETY: the lock is held, so this is the only reference.
    let r = f(unsafe { &mut *LOCKED.0.get() });
    // SAFETY: held by this thread.
    unsafe { nt::RtlReleaseSRWLockExclusive(LOCKED_LOCK.as_ptr()) };
    r
}

/// The released or decommitted range `[address, address + size)` (`VirtualFree`) is locked no more.
pub fn forget_locked(address: usize, size: usize) {
    with_locked(|l| l.forget(workingset::pages_of(address as u64, size as u64)));
}

/// `MEMORY_BASIC_INFORMATION` (`winnt.h`), as `VirtualQuery` fills it for this width.
#[repr(C)]
struct BasicInfo {
    base: usize,
    allocation_base: usize,
    allocation_protect: u32,
    region_size: usize,
    state: u32,
    protect: u32,
    kind: u32,
}

/// `MEM_COMMIT`, `PAGE_NOACCESS`, `PAGE_GUARD` (`winnt.h`); `ERROR_NOT_LOCKED` (`winerror.h`).
const MEM_COMMIT: u32 = 0x1000;
const PAGE_NOACCESS: u32 = 0x01;
const PAGE_GUARD: u32 = 0x100;
const ERROR_NOT_LOCKED: u32 = 158;

/// Whether every page of `pages` is committed and accessible (Learn, "VirtualLock": "All pages in
/// the specified region must be committed. Memory protected with PAGE_NOACCESS cannot be locked").
fn lockable(pages: core::ops::Range<u64>) -> bool {
    let mut at = pages.start * 4096;
    let end = pages.end * 4096;
    while at < end {
        let mut info = BasicInfo {
            base: 0,
            allocation_base: 0,
            allocation_protect: 0,
            region_size: 0,
            state: 0,
            protect: 0,
            kind: 0,
        };
        // SAFETY: a buffer of the structure's size.
        let n = unsafe {
            crate::memory::VirtualQuery(
                at as usize as *const u8,
                (&mut info as *mut BasicInfo).cast(),
                core::mem::size_of::<BasicInfo>(),
            )
        };
        if n == 0
            || info.state != MEM_COMMIT
            || info.protect & (PAGE_NOACCESS | PAGE_GUARD) != 0
            || info.region_size == 0
        {
            return false;
        }
        at = info.base as u64 + info.region_size as u64;
    }
    true
}

/// `VirtualLock(lpAddress, dwSize)`: the pages of the range are brought into the working set and
/// stay there (Learn). Poly keeps every page of a process in memory; what locking adds is the
/// `locked` attribute `K32QueryWorkingSetEx` reports
/// (`specs/windows/kernel32.dll/K32QueryWorkingSetEx.yaml`, rule `lock`: VirtualLock succeeds; the
/// page is valid, its protection unchanged, locked 1). A range with a page that is not committed
/// or not accessible is `ERROR_NOACCESS` (Poly's choice: no sheet records the error).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn VirtualLock(address: *mut u8, size: usize) -> i32 {
    let pages = workingset::pages_of(address as u64, size as u64);
    if pages.is_empty() || !lockable(pages.clone()) {
        return fail(ERROR_NOACCESS, FALSE);
    }
    for p in pages.clone() {
        // SAFETY: a committed, accessible page: reading one byte brings it in.
        let _ = unsafe { core::ptr::read_volatile((p * 4096) as usize as *const u8) };
    }
    with_locked(|l| l.lock(pages));
    TRUE
}

/// `VirtualUnlock(lpAddress, dwSize)`: the pages are locked no more; a page of the range that was
/// not locked makes the call fail with `ERROR_NOT_LOCKED` (Learn, "VirtualUnlock").
#[unsafe(no_mangle)]
pub unsafe extern "system" fn VirtualUnlock(address: *mut u8, size: usize) -> i32 {
    let pages = workingset::pages_of(address as u64, size as u64);
    if pages.is_empty() || !lockable(pages.clone()) {
        return fail(ERROR_NOACCESS, FALSE);
    }
    if with_locked(|l| l.unlock(pages)) { TRUE } else { fail(ERROR_NOT_LOCKED, FALSE) }
}

// ----- GetVolumeInformationByHandleW ----------------------------------------------------------

/// `FileFsVolumeInformation` and `FileFsAttributeInformation` (`FS_INFORMATION_CLASS`).
const FILE_FS_VOLUME_INFORMATION: u32 = 1;
const FILE_FS_ATTRIBUTE_INFORMATION: u32 = 5;

/// Copy `s` and its NUL into a caller's buffer of `size` characters (a NULL buffer is skipped);
/// `false` when it does not fit.
///
/// # Safety
/// `buf` is NULL or holds `size` characters.
unsafe fn put_name(s: &[u16], buf: *mut u16, size: u32) -> bool {
    if buf.is_null() {
        return true;
    }
    if s.len() + 1 > size as usize {
        return false;
    }
    // SAFETY: the caller's buffer holds `size` characters.
    unsafe {
        core::ptr::copy_nonoverlapping(s.as_ptr(), buf, s.len());
        buf.add(s.len()).write(0);
    }
    true
}

/// `GetVolumeInformationByHandleW(hFile, lpVolumeNameBuffer, nVolumeNameSize,
/// lpVolumeSerialNumber, lpMaximumComponentLength, lpFileSystemFlags, lpFileSystemNameBuffer,
/// nFileSystemNameSize)`: the volume facts of the file system that holds the handle's file,
/// from `NtQueryVolumeInformationFile`; every output is optional. A handle whose volume does not
/// answer (the volume device itself, a pipe) fails with the status's error.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "system" fn GetVolumeInformationByHandleW(
    file: Handle,
    name: *mut u16,
    name_size: u32,
    serial: *mut u32,
    max_component: *mut u32,
    fs_flags: *mut u32,
    fs_name: *mut u16,
    fs_name_size: u32,
) -> i32 {
    let mut vol = [0u8; 18 + 2 * 64];
    let mut attr = [0u8; 12 + 2 * 64];
    let mut iosb = IoStatusBlock::default();
    // SAFETY: local buffers of the sizes passed.
    let st = unsafe {
        nt::NtQueryVolumeInformationFile(
            file,
            &mut iosb,
            vol.as_mut_ptr(),
            vol.len() as u32,
            FILE_FS_VOLUME_INFORMATION,
        )
    };
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    // SAFETY: as above.
    let st = unsafe {
        nt::NtQueryVolumeInformationFile(
            file,
            &mut iosb,
            attr.as_mut_ptr(),
            attr.len() as u32,
            FILE_FS_ATTRIBUTE_INFORMATION,
        )
    };
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    let u32_at = |b: &[u8], at: usize| u32::from_le_bytes(b[at..at + 4].try_into().expect("4 bytes"));
    let text = |b: &[u8], at: usize, bytes: usize| -> Vec<u16> {
        let end = (at + bytes).min(b.len());
        b[at..end].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect()
    };
    let label = text(&vol, 18, u32_at(&vol, 12) as usize);
    let fs = text(&attr, 12, u32_at(&attr, 8) as usize);
    // SAFETY: the caller's buffers and out-pointers (each may be NULL).
    unsafe {
        if !put_name(&label, name, name_size) || !put_name(&fs, fs_name, fs_name_size) {
            return fail(workingset::ERROR_BAD_LENGTH, FALSE);
        }
        if !serial.is_null() {
            serial.write_unaligned(u32_at(&vol, 8));
        }
        if !max_component.is_null() {
            max_component.write_unaligned(u32_at(&attr, 4));
        }
        if !fs_flags.is_null() {
            fs_flags.write_unaligned(u32_at(&attr, 0));
        }
    }
    TRUE
}

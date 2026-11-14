//! What Chrome's browser asks about processes beyond their times (ADR 0155, step 9):
//! `QueryProcessCycleTime` (a failure is fatal for the browser), `GetProcessInformation` and
//! `SetProcessInformation`, the shutdown parameters and `K32GetMappedFileNameW` (a failure ends
//! a child). The decisions are `poly_kernelbase::procinfo`'s, from Learn; none of them was
//! observed on Windows.

use crate::base::*;
use crate::nt::{self, Handle};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};
use poly_kernelbase::error::*;
use poly_kernelbase::procinfo::{self, AppMemory, ProcessState};

/// `ProcessTimes` (`ntddk.h`'s `PROCESSINFOCLASS`): `KERNEL_USER_TIMES`.
const PROCESS_TIMES: u32 = 4;

/// `QueryProcessCycleTime(ProcessHandle, CycleTime*)`: the CPU time the kernel counted for the
/// process (`NtQueryInformationProcess(ProcessTimes)`), as cycles at Poly's nominal rate
/// (`procinfo::cycle_time`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn QueryProcessCycleTime(process: Handle, cycles: *mut u64) -> i32 {
    if cycles.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    let mut times = [0u8; 32];
    // SAFETY: a buffer of the class's size.
    let st =
        unsafe { nt::NtQueryInformationProcess(process, PROCESS_TIMES, times.as_mut_ptr(), 32, core::ptr::null_mut()) };
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    let q = |at: usize| u64::from_le_bytes(times[at..at + 8].try_into().expect("8 bytes"));
    // SAFETY: the caller's out-pointer (unaligned is fine for a write through write_unaligned).
    unsafe { cycles.write_unaligned(procinfo::cycle_time(q(16), q(24))) };
    TRUE
}

/// What this process set with `SetProcessInformation`: memory priority, the two power
/// throttling masks, the leap-second flags.
static MEMORY_PRIORITY: AtomicU32 = AtomicU32::new(procinfo::MEMORY_PRIORITY_NORMAL);
static THROTTLE_CONTROL: AtomicU32 = AtomicU32::new(0);
static THROTTLE_STATE: AtomicU32 = AtomicU32::new(0);
static LEAP_FLAGS: AtomicU32 = AtomicU32::new(0);

/// Whether `process` is a handle to this process (a bad handle sets the last error).
fn is_self(process: Handle) -> Option<bool> {
    if process == nt::CURRENT_PROCESS {
        return Some(true);
    }
    let pid = crate::spawn::GetProcessId(process);
    (pid != 0).then(|| pid == crate::process::GetCurrentProcessId())
}

/// The state `GetProcessInformation` reports for `process`: this process's own, or a new
/// process's for another (Poly keeps the state in the process that set it).
fn state_of(own: bool) -> ProcessState {
    if !own {
        return ProcessState::NEW;
    }
    ProcessState {
        memory_priority: MEMORY_PRIORITY.load(Ordering::Relaxed),
        throttle_control: THROTTLE_CONTROL.load(Ordering::Relaxed),
        throttle_state: THROTTLE_STATE.load(Ordering::Relaxed),
        leap_flags: LEAP_FLAGS.load(Ordering::Relaxed),
    }
}

/// `GetProcessInformation(hProcess, ProcessInformationClass, ProcessInformation,
/// ProcessInformationSize)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetProcessInformation(process: Handle, class: u32, info: *mut u8, size: u32) -> i32 {
    if info.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    let Some(own) = is_self(process) else { return FALSE };
    let mut memory = AppMemory::default();
    if class == procinfo::PROCESS_APP_MEMORY_INFO {
        // MEMORYSTATUSEX: dwLength, then ullAvailPageFile at offset 32 (sysinfoapi.h).
        let mut status = [0u8; 64];
        status[0] = 64;
        // SAFETY: a MEMORYSTATUSEX with its length set.
        if unsafe { crate::system::GlobalMemoryStatusEx(status.as_mut_ptr()) } != 0 {
            memory.available_commit = u64::from_le_bytes(status[32..40].try_into().expect("8 bytes"));
        }
    }
    // The sheet's rule `version_first`: the caller's buffer says Version 1 before the call.
    // SAFETY: the caller's buffer of `size` bytes (at least the field when the size is right).
    if class == procinfo::PROCESS_POWER_THROTTLING
        && size as usize == 12
        && !procinfo::power_throttling_version_ok(unsafe { info.cast::<u32>().read_unaligned() })
    {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    match procinfo::get(&state_of(own), &memory, class, size as usize) {
        Ok(bytes) => {
            // SAFETY: the caller's buffer holds `size` bytes, which is `bytes.len()`.
            unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), info, bytes.len()) };
            TRUE
        }
        Err(e) => fail(e, FALSE),
    }
}

/// `SetProcessInformation`: kept for this process and read back by `GetProcessInformation`;
/// for another process the request is checked and accepted without effect (nothing in Poly
/// acts on memory priority or power throttling yet).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetProcessInformation(process: Handle, class: u32, info: *const u8, size: u32) -> i32 {
    if info.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    let Some(own) = is_self(process) else { return FALSE };
    // The buffer is read only when its size is the class's structure's; otherwise `set` sees
    // an empty one and refuses it (a wrong length, or a class it does not know).
    let len = procinfo::class_size(class).filter(|&n| n == size as usize).unwrap_or(0);
    // SAFETY: the caller's buffer of `size` bytes; `len` is `size` or 0.
    let data: Vec<u8> = unsafe { core::slice::from_raw_parts(info, len) }.to_vec();
    let mut state = state_of(own);
    match procinfo::set(&mut state, class, &data) {
        Ok(()) => {
            if own {
                MEMORY_PRIORITY.store(state.memory_priority, Ordering::Relaxed);
                THROTTLE_CONTROL.store(state.throttle_control, Ordering::Relaxed);
                THROTTLE_STATE.store(state.throttle_state, Ordering::Relaxed);
                LEAP_FLAGS.store(state.leap_flags, Ordering::Relaxed);
            }
            TRUE
        }
        Err(e) => fail(e, FALSE),
    }
}

/// This process's shutdown level and flags (`SetProcessShutdownParameters`). Poly has no
/// session shutdown that asks processes in order yet: the values are kept and read back.
static SHUTDOWN_LEVEL: AtomicU32 = AtomicU32::new(procinfo::SHUTDOWN_LEVEL_DEFAULT);
static SHUTDOWN_FLAGS: AtomicU32 = AtomicU32::new(0);

/// `SetProcessShutdownParameters(dwLevel, dwFlags)`.
#[unsafe(no_mangle)]
pub extern "system" fn SetProcessShutdownParameters(level: u32, flags: u32) -> i32 {
    match procinfo::shutdown_parameters(level, flags) {
        Ok((level, flags)) => {
            SHUTDOWN_LEVEL.store(level, Ordering::Relaxed);
            SHUTDOWN_FLAGS.store(flags, Ordering::Relaxed);
            TRUE
        }
        Err(e) => fail(e, FALSE),
    }
}

/// `GetProcessShutdownParameters(lpdwLevel, lpdwFlags)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetProcessShutdownParameters(level: *mut u32, flags: *mut u32) -> i32 {
    if level.is_null() || flags.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: the caller's out-pointers.
    unsafe {
        level.write_unaligned(SHUTDOWN_LEVEL.load(Ordering::Relaxed));
        flags.write_unaligned(SHUTDOWN_FLAGS.load(Ordering::Relaxed));
    }
    TRUE
}

/// `ProcessIdToSessionId(dwProcessId, pSessionId)`: "Retrieves the Remote Desktop Services
/// session associated with a specified process" (Learn). Poly has one session: every process
/// is in the caller's (`PEB.SessionId`, PEB + 0x2C0, `winternl.h`). Doc-derived.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ProcessIdToSessionId(_process_id: u32, session: *mut u32) -> i32 {
    if session.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: the PEB is mapped; the caller's out-pointer.
    unsafe { session.write_unaligned(((peb() + 0x2C0) as *const u32).read()) };
    TRUE
}

/// `ERROR_FILE_INVALID` (`winerror.h`): the address is not in a mapped file.
const ERROR_FILE_INVALID: u32 = 1006;
/// `ObjectNameInformation` (`OBJECT_INFORMATION_CLASS`, `ntifs.h`).
const OBJECT_NAME_INFORMATION: u32 = 1;

/// The loader's modules as `(base, size, full path)` (`PEB.Ldr`'s memory-order list;
/// `SizeOfImage` at + 0x40 of the entry, `winternl.h` / `poly_ldr::ldrdata`).
fn module_spans() -> Vec<(u64, u64, Vec<u16>)> {
    let mut out = Vec::new();
    // SAFETY: ntdll keeps the loader data in process memory (ADR 0071); the list is circular
    // through its head.
    unsafe {
        let ldr = ((peb() + 0x18) as *const u64).read();
        if ldr == 0 {
            return out;
        }
        let head = ldr + 0x20;
        let mut link = (head as *const u64).read();
        let mut guard = 0;
        while link != head && link != 0 && guard < 4096 {
            let entry = link - 0x10;
            let base = ((entry + 0x30) as *const u64).read();
            let size = u64::from(((entry + 0x40) as *const u32).read());
            out.push((base, size, unicode_at(entry + 0x48).to_vec()));
            link = (link as *const u64).read();
            guard += 1;
        }
    }
    out
}

/// The object manager's name of the file at `path` (`\Device\...`), by opening it and asking
/// `NtQueryObject(ObjectNameInformation)`.
fn device_name(path: &[u16]) -> Option<Vec<u16>> {
    let mut z = path.to_vec();
    z.push(0);
    // FILE_READ_ATTRIBUTES (0x80), every share mode, OPEN_EXISTING (3).
    // SAFETY: a NUL-terminated path.
    let h = unsafe { crate::file::CreateFileW(z.as_ptr(), 0x80, 7, core::ptr::null(), 3, 0, 0) };
    if h == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut buf = alloc::vec![0u8; 16 + 2 * 1024];
    let mut ret = 0u32;
    // SAFETY: a buffer of the stated length.
    let st = unsafe {
        crate::handles::NtQueryObject(h, OBJECT_NAME_INFORMATION, buf.as_mut_ptr(), buf.len() as u32, &mut ret)
    };
    // SAFETY: the handle opened above.
    unsafe { nt::NtClose(h) };
    if !st.is_success() {
        return None;
    }
    // A UNICODE_STRING (Length at 0) followed by its characters at offset 16.
    let len = usize::from(u16::from_le_bytes([buf[0], buf[1]])).min(buf.len() - 16);
    Some(buf[16..16 + len].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect())
}

/// The last user-mode address (`MM_HIGHEST_USER_ADDRESS` of x64 Windows).
const USER_ADDRESS_LIMIT: u64 = 0x7FFF_FFFE_FFFF;
/// `ERROR_UNEXP_NET_ERR` (`winerror.h`): what an address with no file mapped at it gives
/// (the sheet: "as observed").
const ERROR_UNEXP_NET_ERR: u32 = 59;
/// `MemoryMappedFilenameInformation`, `STATUS_FILE_INVALID`.
const MEMORY_MAPPED_FILENAME_INFORMATION: u32 = 2;
const STATUS_FILE_INVALID: u32 = 0xC000_0098;

/// The object name of the file a view or image at `address` of this process maps, from the
/// kernel (`NtQueryVirtualMemory(MemoryMappedFilenameInformation)`), or the error
/// `K32GetMappedFileNameW` reports: 1006 for a view of a section without a file, 59 for an
/// address nothing is mapped at (private memory, freed or unmapped memory).
fn mapped_view_name(process: Handle, address: u64) -> Result<Vec<u16>, u32> {
    let mut buf = alloc::vec![0u8; 16 + 2 * 1024];
    let mut ret = 0usize;
    // SAFETY: a buffer of the stated length.
    let st = unsafe {
        nt::NtQueryVirtualMemory(
            process,
            address as *const u8,
            MEMORY_MAPPED_FILENAME_INFORMATION,
            buf.as_mut_ptr(),
            buf.len(),
            &mut ret,
        )
    };
    if st.0 == STATUS_FILE_INVALID {
        return Err(ERROR_FILE_INVALID);
    }
    if !st.is_success() {
        return Err(ERROR_UNEXP_NET_ERR);
    }
    // A UNICODE_STRING (Length at 0) followed by its characters at offset 16.
    let len = usize::from(u16::from_le_bytes([buf[0], buf[1]])).min(buf.len() - 16);
    Ok(buf[16..16 + len].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect())
}

/// `K32GetMappedFileNameW(hProcess, lpv, lpFilename, nSize)` (`GetMappedFileNameW` of PSAPI
/// version 2): the device-form name of the file mapped at `lpv`. Poly answers for the images
/// the loader mapped into this process (what Chrome asks about); an address in no image, and
/// any address of another process, is `ERROR_FILE_INVALID`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn K32GetMappedFileNameW(process: Handle, address: u64, name: *mut u16, size: u32) -> u32 {
    if name.is_null() {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    match is_self(process) {
        None => return 0,
        Some(false) => return fail(ERROR_FILE_INVALID, 0),
        Some(true) => {}
    }
    // The sheet: an address above the user address space is ERROR_INVALID_PARAMETER.
    if address > USER_ADDRESS_LIMIT {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    let modules = module_spans();
    // One of the loader's images: the file of its path. Anything else (and an image whose file
    // cannot be opened by that path): ask the kernel what file the address is in.
    let device = match procinfo::module_at(&modules, address).and_then(device_name) {
        Some(d) => d,
        None => match mapped_view_name(process, address) {
            Ok(d) => d,
            Err(e) => return fail(e, 0),
        },
    };
    let (n, nul, ret, error) = procinfo::mapped_name_copy(&device, size);
    // SAFETY: the caller's buffer holds `size` characters; `n` and the NUL fit in it.
    unsafe {
        core::ptr::copy_nonoverlapping(device.as_ptr(), name, n);
        if nul {
            name.add(n).write(0);
        }
    }
    // A successful call sets the last error to 0 (the sheet).
    set_last_error(error.unwrap_or(0));
    ret
}

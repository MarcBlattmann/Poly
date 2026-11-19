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

/// The leap-second flags this process set with `SetProcessInformation` (Poly keeps them here;
/// the memory priority and the power-throttling masks are the kernel's, so another process can
/// set and read them: `NtSetInformationProcess` classes 39 and 77).
static LEAP_FLAGS: AtomicU32 = AtomicU32::new(0);

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    fn NtSetInformationProcess(process: Handle, class: u32, info: *const u8, length: u32) -> poly_ntobj::NtStatus;
}

/// `ProcessMemoryPriority` and `ProcessPowerThrottlingState` of `NtSetInformationProcess` /
/// `NtQueryInformationProcess` (`ntddk.h`): the structures are `SetProcessInformation`'s.
const NT_PROCESS_MEMORY_PRIORITY: u32 = 39;
const NT_PROCESS_POWER_THROTTLING_STATE: u32 = 77;

/// The kernel's class for a `PROCESS_INFORMATION_CLASS` it keeps per process.
fn kernel_class(class: u32) -> Option<u32> {
    match class {
        procinfo::PROCESS_MEMORY_PRIORITY => Some(NT_PROCESS_MEMORY_PRIORITY),
        procinfo::PROCESS_POWER_THROTTLING => Some(NT_PROCESS_POWER_THROTTLING_STATE),
        _ => None,
    }
}

/// Whether `process` is a handle to this process (a bad handle sets the last error).
fn is_self(process: Handle) -> Option<bool> {
    if process == nt::CURRENT_PROCESS {
        return Some(true);
    }
    let pid = crate::spawn::GetProcessId(process);
    (pid != 0).then(|| pid == crate::process::GetCurrentProcessId())
}

/// `GetProcessInformation(hProcess, ProcessInformationClass, ProcessInformation,
/// ProcessInformationSize)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetProcessInformation(process: Handle, class: u32, info: *mut u8, size: u32) -> i32 {
    if info.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // The sheet's rule `version_first`: the caller's buffer says Version 1 before the call.
    // SAFETY: the caller's buffer of `size` bytes (at least the field when the size is right).
    if class == procinfo::PROCESS_POWER_THROTTLING
        && size as usize == 12
        && !procinfo::power_throttling_version_ok(unsafe { info.cast::<u32>().read_unaligned() })
    {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // The class and the size first (the sheet's `ERROR_INVALID_PARAMETER` / `ERROR_BAD_LENGTH`).
    if let Err(e) = procinfo::get(&ProcessState::NEW, &AppMemory::default(), class, size as usize) {
        return fail(e, FALSE);
    }
    // What the kernel keeps per process: memory priority and power throttling, of any process the
    // handle can query.
    if let Some(kclass) = kernel_class(class) {
        // SAFETY: the caller's buffer of `size` bytes.
        let st = unsafe { nt::NtQueryInformationProcess(process, kclass, info, size, core::ptr::null_mut()) };
        return if st.is_success() { TRUE } else { fail_status(st, FALSE) };
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
    let state =
        ProcessState { leap_flags: if own { LEAP_FLAGS.load(Ordering::Relaxed) } else { 0 }, ..ProcessState::NEW };
    match procinfo::get(&state, &memory, class, size as usize) {
        Ok(bytes) => {
            // SAFETY: the caller's buffer holds `size` bytes, which is `bytes.len()`.
            unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), info, bytes.len()) };
            TRUE
        }
        Err(e) => fail(e, FALSE),
    }
}

/// `SetProcessInformation`: the class, the size and the values are judged by
/// `poly_kernelbase::procinfo::set` (87 / 24); memory priority and power throttling go to the
/// kernel (`NtSetInformationProcess`, needs `PROCESS_SET_INFORMATION`: 5 and 6 come from there)
/// and are read back by any process that can query this one; the leap-second flags are this
/// process's own.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetProcessInformation(process: Handle, class: u32, info: *const u8, size: u32) -> i32 {
    if info.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // The buffer is read only when its size is the class's structure's; otherwise `set` sees
    // an empty one and refuses it (a wrong length, or a class it does not know).
    let len = procinfo::set_class_size(class).filter(|&n| n == size as usize).unwrap_or(0);
    // SAFETY: the caller's buffer of `size` bytes; `len` is `size` or 0.
    let data: Vec<u8> = unsafe { core::slice::from_raw_parts(info, len) }.to_vec();
    let mut state = ProcessState::NEW;
    if let Err(e) = procinfo::set(&mut state, class, &data) {
        return fail(e, FALSE);
    }
    if let Some(kclass) = kernel_class(class) {
        // SAFETY: the validated structure.
        let st = unsafe { NtSetInformationProcess(process, kclass, data.as_ptr(), data.len() as u32) };
        return if st.is_success() { TRUE } else { fail_status(st, FALSE) };
    }
    // Leap-second flags: kept for this process; another process's are accepted without effect.
    let Some(own) = is_self(process) else { return FALSE };
    if own {
        LEAP_FLAGS.store(state.leap_flags, Ordering::Relaxed);
    }
    TRUE
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
        let l = crate::base::LDR;
        let ldr = crate::base::ptr_at(peb() + poly_rtl::layout::NATIVE.peb_ldr);
        if ldr == 0 {
            return out;
        }
        let head = ldr + l.ldr_in_memory_order;
        let mut link = crate::base::ptr_at(head);
        let mut guard = 0;
        while link != head && link != 0 && guard < 4096 {
            let entry = link - l.entry_in_memory_order;
            let base = crate::base::ptr_at(entry + l.entry_dll_base);
            let size = u64::from(((entry + l.entry_size_of_image) as *const u32).read());
            out.push((base, size, unicode_at(entry + l.entry_full_name).to_vec()));
            link = crate::base::ptr_at(link);
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
    let mut buf = alloc::vec![0u8; 2 * core::mem::size_of::<usize>() + 2 * 1024];
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
    // A UNICODE_STRING (Length at 0) followed by its characters: at offset 16, 8 in a 32-bit process.
    const HEADER: usize = 2 * core::mem::size_of::<usize>();
    let len = usize::from(u16::from_le_bytes([buf[0], buf[1]])).min(buf.len() - HEADER);
    Some(buf[HEADER..HEADER + len].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect())
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
    let mut buf = alloc::vec![0u8; 2 * core::mem::size_of::<usize>() + 2 * 1024];
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
    // A UNICODE_STRING (Length at 0) followed by its characters: at offset 16 in a 64-bit process,
    // 8 in a 32-bit one (the kernel lays the answer out in the caller's width).
    const HEADER: usize = 2 * core::mem::size_of::<usize>();
    let len = usize::from(u16::from_le_bytes([buf[0], buf[1]])).min(buf.len() - HEADER);
    Ok(buf[HEADER..HEADER + len].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect())
}

/// `K32GetMappedFileNameW(hProcess, lpv, lpFilename, nSize)` (`GetMappedFileNameW` of PSAPI
/// version 2): the device-form name of the file mapped at `lpv` in this process or, with a
/// handle that has `PROCESS_QUERY_LIMITED_INFORMATION`, in another one.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn K32GetMappedFileNameW(process: Handle, address: usize, name: *mut u16, size: u32) -> u32 {
    let address = address as u64;
    if name.is_null() {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    let Some(own) = is_self(process) else { return 0 };
    // The sheet: an address above the user address space is ERROR_INVALID_PARAMETER.
    if address > USER_ADDRESS_LIMIT {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    // One of this process's loader images: the file of its path. Anything else (another
    // process's views, and an image whose file cannot be opened by that path): ask the kernel
    // what file the address is in.
    let from_loader = if own { procinfo::module_at(&module_spans(), address).and_then(device_name) } else { None };
    let device = match from_loader {
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

/// `ERROR_PARTIAL_COPY` (`winerror.h`): another process's headers cannot be read here.
const ERROR_PARTIAL_COPY: u32 = 299;

/// `K32GetModuleInformation(hProcess, hModule, lpmodinfo, cb)` (`GetModuleInformation` of PSAPI
/// version 2, Learn): the base, size and entry point of one of this process's loaded modules,
/// from the loader's list and the module's own PE headers (`hModule` NULL: the program). A
/// module the loader does not know is `ERROR_INVALID_HANDLE`; another process's modules are not
/// readable from here yet (`ERROR_PARTIAL_COPY`).
///
/// # Safety
/// `info` holds `cb` writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn K32GetModuleInformation(process: Handle, module: usize, info: *mut u8, cb: u32) -> i32 {
    let Some(own) = is_self(process) else { return FALSE };
    if !own {
        return fail(ERROR_PARTIAL_COPY, FALSE);
    }
    let spans = module_spans();
    let found = if module == 0 { spans.first() } else { spans.iter().find(|s| s.0 == module as u64) };
    let Some(&(base, size, _)) = found else { return fail(ERROR_INVALID_HANDLE, FALSE) };
    // SAFETY: a loaded module's headers are mapped readable for its whole first page.
    let header = unsafe { core::slice::from_raw_parts(base as usize as *const u8, 0x1000) };
    let entry = match procinfo::entry_point_rva(header) {
        Some(0) | None => 0,
        Some(rva) => base + u64::from(rva),
    };
    match procinfo::module_info(base, size as u32, entry, cb, core::mem::size_of::<usize>()) {
        Ok(b) => {
            if info.is_null() {
                return fail(ERROR_INVALID_PARAMETER, FALSE);
            }
            // SAFETY: `b` is at most `cb` bytes, the caller's structure.
            unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), info, b.len()) };
            TRUE
        }
        Err(e) => fail(e, FALSE),
    }
}

/// `ProcessImageFileName` (`ntddk.h`'s `PROCESSINFOCLASS`): the image's device-form name as a
/// `UNICODE_STRING` and its characters.
const PROCESS_IMAGE_FILE_NAME: u32 = 27;

/// The device-form name of `process`'s image (`\Device\HarddiskVolumeN\...`), from the kernel;
/// it checks the handle and its rights (a query right).
fn image_device_name(process: Handle) -> Result<Vec<u16>, poly_ntobj::NtStatus> {
    // A UNICODE_STRING (Length at 0) and its characters: 16 bytes in a 64-bit process, 8 in a
    // 32-bit one (the kernel lays the answer out in the caller's width).
    const HEADER: usize = 2 * core::mem::size_of::<usize>();
    let mut buf = alloc::vec![0u8; HEADER + 2 * 32768];
    let mut ret = 0u32;
    // SAFETY: a buffer of the stated length.
    let st = unsafe {
        nt::NtQueryInformationProcess(process, PROCESS_IMAGE_FILE_NAME, buf.as_mut_ptr(), buf.len() as u32, &mut ret)
    };
    if !st.is_success() {
        return Err(st);
    }
    let len = usize::from(u16::from_le_bytes([buf[0], buf[1]])).min(buf.len() - HEADER);
    Ok(buf[HEADER..HEADER + len].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect())
}

/// The Win32 form (`C:\...`) of a device-form path: the file opened by that name and asked for
/// its final path with the drive letter.
fn win32_name_of_device_path(device: &[u16]) -> Result<Vec<u16>, u32> {
    let us = nt::UnicodeString::new(device);
    let oa = nt::ObjectAttributes::new(Some(&us), nt::OBJ_CASE_INSENSITIVE);
    let mut h: Handle = 0;
    let mut iosb = nt::IoStatusBlock::default();
    // FILE_READ_ATTRIBUTES | SYNCHRONIZE, every share mode, FILE_SYNCHRONOUS_IO_NONALERT.
    // SAFETY: valid pointers.
    let st = unsafe { nt::NtOpenFile(&mut h, 0x80 | 0x0010_0000, &oa, &mut iosb, 7, 0x20) };
    if !st.is_success() {
        return Err(from_status(st));
    }
    let mut path = alloc::vec![0u16; 32768];
    // VOLUME_NAME_DOS | FILE_NAME_NORMALIZED (0).
    // SAFETY: a buffer of the stated length; the handle opened above.
    let n = unsafe { crate::fileex::GetFinalPathNameByHandleW(h, path.as_mut_ptr(), path.len() as u32, 0) };
    let e = last_error();
    close(h);
    if n == 0 || n as usize >= path.len() {
        return Err(if n == 0 { e } else { ERROR_INSUFFICIENT_BUFFER });
    }
    Ok(procinfo::without_long_prefix(&path[..n as usize]))
}

/// `QueryFullProcessImageNameW(hProcess, dwFlags, lpExeName, lpdwSize)`: the path of `hProcess`'s
/// executable, the Win32 form (flags 0; for this process the text of `GetModuleFileNameW(NULL)`)
/// or the device form (`PROCESS_NAME_NATIVE`). `*lpdwSize` is the buffer's size in characters on
/// the way in and the name's length without the NUL on the way out; a buffer without room for
/// the NUL is `ERROR_INSUFFICIENT_BUFFER` and leaves it as it was
/// (`specs/windows/kernel32.dll/QueryFullProcessImageNameW.yaml`).
///
/// # Safety
/// `size` points to a `DWORD`; `name` holds `*size` writable characters.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn QueryFullProcessImageNameW(
    process: Handle,
    flags: u32,
    name: *mut u16,
    size: *mut u32,
) -> i32 {
    let native = match procinfo::image_name_native(flags) {
        Ok(n) => n,
        Err(e) => return fail(e, FALSE),
    };
    if size.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // The kernel's answer checks the handle (NULL, a thread: ERROR_INVALID_HANDLE) and its rights
    // (SYNCHRONIZE alone: ERROR_ACCESS_DENIED) for both forms.
    let device = match image_device_name(process) {
        Ok(d) => d,
        Err(st) => return fail_status(st, FALSE),
    };
    let text = if native {
        device
    } else if is_self(process) == Some(true) {
        let mut own = alloc::vec![0u16; 32768];
        // SAFETY: a buffer of the stated length; NULL is the executable.
        let n = unsafe { crate::process::GetModuleFileNameW(core::ptr::null_mut(), own.as_mut_ptr(), 32768) };
        own.truncate(n as usize);
        own
    } else {
        match win32_name_of_device_path(&device) {
            Ok(t) => t,
            Err(e) => return fail(e, FALSE),
        }
    };
    // SAFETY: the caller's DWORD.
    let capacity = unsafe { size.read_unaligned() };
    if let Err(e) = procinfo::image_name_fits(text.len(), capacity) {
        return fail(e, FALSE);
    }
    if name.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: the caller's buffer holds `capacity` > `text.len()` characters; its DWORD.
    unsafe {
        core::ptr::copy_nonoverlapping(text.as_ptr(), name, text.len());
        name.add(text.len()).write(0);
        size.write_unaligned(text.len() as u32);
    }
    TRUE
}

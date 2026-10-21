//! What 7-Zip's command line and MSVC's start-up code import from
//! `kernel32` beyond the first two slices (ADR 0087): threads and
//! processes (`ResumeThread`, `SuspendThread`, `GetProcessTimes`,
//! `GetStartupInfoW`, the affinity masks), the processor
//! (`IsProcessorFeaturePresent`), memory (`GlobalMemoryStatusEx`,
//! `GetLargePageMinimum`), events (`OpenEventW`), modules
//! (`GetModuleHandleA`), drives and disk space, and `DeviceIoControl`.
//! The decisions are `poly_kernelbase::{cpu, system}`'s.

use crate::base::*;
use crate::nt::{self, Handle, IoStatusBlock, ObjectAttributes, UnicodeString};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use poly_kernelbase::cpu::CpuId;
use poly_kernelbase::error::*;
use poly_kernelbase::file::{FILE_DIRECTORY_FILE, FILE_READ_ATTRIBUTES, FILE_SYNCHRONOUS_IO_NONALERT, SYNCHRONIZE};
use poly_kernelbase::system::{self, DiskSpace};
use poly_ntobj::NtStatus;

// ----- threads and processes ----------------------------------------------------------

/// `ResumeThread`: the previous suspend count, `(DWORD)-1` on failure.
#[unsafe(no_mangle)]
pub extern "system" fn ResumeThread(thread: Handle) -> u32 {
    let mut previous = 0u32;
    // SAFETY: a valid out-pointer.
    let st = unsafe { nt::NtResumeThread(thread, &mut previous) };
    if st.is_success() { previous } else { fail_status(st, u32::MAX) }
}

/// `SuspendThread`: as `ResumeThread`.
#[unsafe(no_mangle)]
pub extern "system" fn SuspendThread(thread: Handle) -> u32 {
    let mut previous = 0u32;
    // SAFETY: a valid out-pointer.
    let st = unsafe { nt::NtSuspendThread(thread, &mut previous) };
    if st.is_success() { previous } else { fail_status(st, u32::MAX) }
}

/// `ProcessTimes` (`ntddk.h`'s `PROCESSINFOCLASS`): `KERNEL_USER_TIMES`,
/// four `LARGE_INTEGER`s: creation, exit, kernel and user time.
const PROCESS_TIMES: u32 = 4;

/// `GetProcessTimes`: the four `FILETIME`s of `NtQueryInformationProcess
/// (ProcessTimes)` (ADR 0087).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetProcessTimes(
    process: Handle,
    creation: *mut u8,
    exit: *mut u8,
    kernel: *mut u8,
    user: *mut u8,
) -> i32 {
    let mut times = [0u8; 32];
    // SAFETY: a valid buffer of the class's size.
    let st =
        unsafe { nt::NtQueryInformationProcess(process, PROCESS_TIMES, times.as_mut_ptr(), 32, core::ptr::null_mut()) };
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    for (i, out) in [creation, exit, kernel, user].into_iter().enumerate() {
        if out.is_null() {
            return fail(ERROR_NOACCESS, FALSE);
        }
        // SAFETY: the caller's FILETIME (unaligned).
        unsafe { core::ptr::copy_nonoverlapping(times[i * 8..].as_ptr(), out, 8) };
    }
    TRUE
}

/// `ERROR_NOACCESS` (`winerror.h`).
const ERROR_NOACCESS: u32 = 998;

/// `sizeof(STARTUPINFOW)` on x64 (`processthreadsapi.h`).
const STARTUPINFOW_SIZE: usize = 104;

/// `GetStartupInfoW`: Learn: "the STARTUPINFO structure that was specified
/// when the calling process was created". Poly's `CreateProcessW` does not
/// pass one on yet (ADR 0079), so every process gets `cb` and zeros: no
/// `STARTF_*` flags, so the standard handles come from `GetStdHandle`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetStartupInfoW(info: *mut u8) {
    let mut b = [0u8; STARTUPINFOW_SIZE];
    b[0..4].copy_from_slice(&(STARTUPINFOW_SIZE as u32).to_le_bytes());
    // SAFETY: the caller's STARTUPINFOW.
    unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), info, STARTUPINFOW_SIZE) };
}

/// This process's affinity mask as `SetProcessAffinityMask` set it (0:
/// the system's). Poly's scheduler does not pin threads yet, so the masks
/// are recorded and reported, not enforced (ADR 0087).
static PROCESS_AFFINITY: AtomicU64 = AtomicU64::new(0);

/// Whether `process` is a handle to this process.
fn is_current_process(process: Handle) -> bool {
    let last = last_error();
    let same = crate::spawn::GetProcessId(process) == crate::process::GetCurrentProcessId();
    set_last_error(last);
    same
}

fn system_affinity() -> u64 {
    system::system_affinity(crate::misc::processors())
}

fn process_affinity() -> u64 {
    match PROCESS_AFFINITY.load(Ordering::Relaxed) {
        0 => system_affinity(),
        m => m,
    }
}

/// `GetProcessAffinityMask`: the process's and the system's masks.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetProcessAffinityMask(process: Handle, mask: *mut u64, system_mask: *mut u64) -> i32 {
    if mask.is_null() || system_mask.is_null() {
        return fail(ERROR_NOACCESS, FALSE);
    }
    let own = process == nt::CURRENT_PROCESS || is_current_process(process);
    // SAFETY: the caller's out-pointers.
    unsafe {
        mask.write_unaligned(if own { process_affinity() } else { system_affinity() });
        system_mask.write_unaligned(system_affinity());
    }
    TRUE
}

/// `SetProcessAffinityMask`: Learn: the mask must be a subset of the
/// system's (`ERROR_INVALID_PARAMETER`).
#[unsafe(no_mangle)]
pub extern "system" fn SetProcessAffinityMask(process: Handle, mask: u64) -> i32 {
    if !system::valid_affinity(mask, system_affinity()) {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    if process == nt::CURRENT_PROCESS || is_current_process(process) {
        PROCESS_AFFINITY.store(mask, Ordering::Relaxed);
    }
    TRUE
}

/// `SetThreadAffinityMask`: the thread's previous mask (Poly: the
/// process's), 0 with `ERROR_INVALID_PARAMETER` for a mask outside the
/// process's.
#[unsafe(no_mangle)]
pub extern "system" fn SetThreadAffinityMask(_thread: Handle, mask: u64) -> u64 {
    let process = process_affinity();
    if !system::valid_affinity(mask, process) {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    process
}

// ----- the processor ----------------------------------------------------------------

/// The processor's `CPUID` leaves and `XCR0` (`poly_kernelbase::cpu`).
fn cpuid() -> CpuId {
    use core::arch::x86_64::__cpuid_count;
    // SAFETY: CPUID exists on every x64 processor; leaves beyond the
    // maximum are not read.
    unsafe {
        let max = __cpuid_count(0, 0).eax;
        let ext_max = __cpuid_count(0x8000_0000, 0).eax;
        let l1 = __cpuid_count(1, 0);
        let l7 = if max >= 7 { __cpuid_count(7, 0) } else { core::mem::zeroed() };
        let e1 = if ext_max >= 0x8000_0001 { __cpuid_count(0x8000_0001, 0) } else { core::mem::zeroed() };
        let xcr0 = if l1.ecx >> 27 & 1 != 0 {
            let (lo, hi): (u32, u32);
            // XGETBV(0) is allowed once the OS set CR4.OSXSAVE (OSXSAVE).
            core::arch::asm!("xgetbv", in("ecx") 0u32, out("eax") lo, out("edx") hi, options(nomem, nostack));
            u64::from(hi) << 32 | u64::from(lo)
        } else {
            0
        };
        CpuId {
            leaf1_ecx: l1.ecx,
            leaf1_edx: l1.edx,
            leaf7_ebx: l7.ebx,
            leaf7_ecx: l7.ecx,
            ext1_ecx: e1.ecx,
            ext1_edx: e1.edx,
            xcr0,
        }
    }
}

/// `IsProcessorFeaturePresent`.
#[unsafe(no_mangle)]
pub extern "system" fn IsProcessorFeaturePresent(feature: u32) -> i32 {
    i32::from(cpuid().has(feature))
}

// ----- memory ----------------------------------------------------------------------

/// `GetLargePageMinimum`: Learn: zero when large pages are not supported;
/// Poly does not give programs large pages.
#[unsafe(no_mangle)]
pub extern "system" fn GetLargePageMinimum() -> usize {
    0
}

/// Read a small file whole (an NT path), or `None`.
fn read_file(nt_path: &str) -> Option<Vec<u8>> {
    let units: Vec<u16> = nt_path.encode_utf16().collect();
    let name = UnicodeString::new(&units);
    let oa = ObjectAttributes::new(Some(&name), nt::OBJ_CASE_INSENSITIVE);
    let mut h: Handle = 0;
    let mut iosb = IoStatusBlock::default();
    // FILE_READ_DATA (`winnt.h`).
    let access = 1 | SYNCHRONIZE;
    // SAFETY: valid pointers.
    let st = unsafe { nt::NtOpenFile(&mut h, access, &oa, &mut iosb, 7, FILE_SYNCHRONOUS_IO_NONALERT) };
    if !st.is_success() {
        return None;
    }
    let mut out = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        let mut iosb = IoStatusBlock::default();
        // SAFETY: valid pointers; reads at the current position.
        let st = unsafe {
            nt::NtReadFile(
                h,
                0,
                core::ptr::null(),
                core::ptr::null(),
                &mut iosb,
                buf.as_mut_ptr(),
                buf.len() as u32,
                core::ptr::null(),
                core::ptr::null(),
            )
        };
        let n = (iosb.information as usize).min(buf.len());
        if !st.is_success() || n == 0 || out.len() > 64 * 1024 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    close(h);
    Some(out)
}

/// `GlobalMemoryStatusEx`: physical memory from Poly's `/proc/meminfo`
/// (`Z:` is the whole VFS, ADR 0067), the rest as
/// `poly_kernelbase::system::memory_status` says. `dwLength` must be set
/// (Learn); a wrong one is `ERROR_INVALID_PARAMETER` (Poly's reading).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GlobalMemoryStatusEx(status: *mut u8) -> i32 {
    // SAFETY: the caller's MEMORYSTATUSEX.
    if status.is_null() || unsafe { status.cast::<u32>().read_unaligned() } as usize != system::MEMORYSTATUSEX_SIZE {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    let phys = read_file("\\??\\Z:\\proc\\meminfo")
        .and_then(|b| system::parse_meminfo(core::str::from_utf8(&b).ok()?))
        .ok_or(ERROR_NOT_SUPPORTED);
    match phys {
        Ok(p) => {
            let b = system::memory_status(p);
            // SAFETY: as above.
            unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), status, b.len()) };
            TRUE
        }
        Err(e) => fail(e, FALSE),
    }
}

// ----- events and modules -------------------------------------------------------------

/// `OpenEventW`: Learn: the name is required; a missing one is
/// `ERROR_FILE_NOT_FOUND` (the object manager's not-found status).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn OpenEventW(access: u32, inherit: i32, name: *const u16) -> Handle {
    // SAFETY: the caller's string.
    let Some(full) = (unsafe { wide_arg(name) }).and_then(poly_kernelbase::sync::object_name) else {
        return fail(ERROR_INVALID_PARAMETER, 0);
    };
    let string = UnicodeString::new(&full);
    let attrs = if inherit != 0 { nt::OBJ_INHERIT } else { 0 };
    let oa = ObjectAttributes::new(Some(&string), attrs);
    let mut h: Handle = 0;
    // SAFETY: valid pointers.
    let st = unsafe { nt::NtOpenEvent(&mut h, access, &oa) };
    if st.is_success() { h } else { fail_status(st, 0) }
}

/// `GetModuleHandleA`: the name through the ANSI code page, then
/// `GetModuleHandleW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetModuleHandleA(name: *const u8) -> *mut u8 {
    // SAFETY: the caller's string.
    match unsafe { ansi_arg(name) } {
        None => {
            // SAFETY: NULL asks for the executable.
            unsafe { crate::process::GetModuleHandleW(core::ptr::null()) }
        }
        Some(w) => {
            let w = with_nul(&w);
            // SAFETY: a NUL-terminated string.
            unsafe { crate::process::GetModuleHandleW(w.as_ptr()) }
        }
    }
}

// ----- drives and disk space ----------------------------------------------------------------

/// `GetLogicalDrives`: a bit per drive letter whose root exists
/// (`\??\X:\`; Poly has `C:` and `Z:`, ADR 0067, 0073).
#[unsafe(no_mangle)]
pub extern "system" fn GetLogicalDrives() -> u32 {
    let mut mask = 0;
    for i in 0..26u8 {
        let path = [b'\\', b'?', b'?', b'\\', b'A' + i, b':', b'\\'].map(u16::from);
        let name = UnicodeString::new(&path);
        let oa = ObjectAttributes::new(Some(&name), nt::OBJ_CASE_INSENSITIVE);
        let mut info = [0u8; 40];
        // SAFETY: valid pointers; FILE_BASIC_INFORMATION is 40 bytes.
        if unsafe { nt::NtQueryAttributesFile(&oa, info.as_mut_ptr()) }.is_success() {
            mask |= 1 << i;
        }
    }
    set_last_error(ERROR_SUCCESS);
    mask
}

/// `GetLogicalDriveStringsW`: Learn: the length copied without the final
/// NUL, or the size needed when the buffer is too small.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetLogicalDriveStringsW(size: u32, buf: *mut u16) -> u32 {
    let list = system::drive_strings(GetLogicalDrives());
    if (size as usize) < list.len() || buf.is_null() {
        return list.len() as u32;
    }
    // SAFETY: the caller's buffer holds `size` >= `list.len()` units.
    unsafe { core::ptr::copy_nonoverlapping(list.as_ptr(), buf, list.len()) };
    list.len() as u32 - 1
}

/// `FileFsFullSizeInformation` (`ntifs.h`).
const FILE_FS_FULL_SIZE_INFORMATION: u32 = 7;

/// The volume of `dir` (NULL: the current directory's), or the Win32
/// error.
///
/// # Safety
/// `dir` is null or a NUL-terminated string.
unsafe fn disk_space(dir: *const u16) -> Result<DiskSpace, u32> {
    // SAFETY: the caller's string.
    let path = match unsafe { wide_arg(dir) } {
        Some(p) => p.to_vec(),
        None => current_dir(),
    };
    let name = nt_name(&path)?;
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
        return Err(from_status(st));
    }
    let mut info = [0u8; 32];
    // SAFETY: valid pointers; the class is 32 bytes.
    let st: NtStatus =
        unsafe { nt::NtQueryVolumeInformationFile(h, &mut iosb, info.as_mut_ptr(), 32, FILE_FS_FULL_SIZE_INFORMATION) };
    close(h);
    if st.is_success() { Ok(DiskSpace::from_full_size(&info)) } else { Err(from_status(st)) }
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

/// `GetDiskFreeSpaceW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetDiskFreeSpaceW(
    root: *const u16,
    sectors_per_cluster: *mut u32,
    bytes_per_sector: *mut u32,
    free_clusters: *mut u32,
    total_clusters: *mut u32,
) -> i32 {
    // SAFETY: the caller's string.
    match unsafe { disk_space(root) } {
        Ok(d) => {
            let (s, b, f, t) = d.clusters();
            // SAFETY: the caller's out-pointers (each may be null).
            unsafe {
                put(sectors_per_cluster, s);
                put(bytes_per_sector, b);
                put(free_clusters, f);
                put(total_clusters, t);
            }
            TRUE
        }
        Err(e) => fail(e, FALSE),
    }
}

/// `GetDiskFreeSpaceExW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetDiskFreeSpaceExW(
    dir: *const u16,
    free_to_caller: *mut u64,
    total: *mut u64,
    total_free: *mut u64,
) -> i32 {
    // SAFETY: the caller's string.
    match unsafe { disk_space(dir) } {
        Ok(d) => {
            let (c, t, f) = d.bytes();
            // SAFETY: the caller's out-pointers (each may be null).
            unsafe {
                put(free_to_caller, c);
                put(total, t);
                put(total_free, f);
            }
            TRUE
        }
        Err(e) => fail(e, FALSE),
    }
}

// ----- DeviceIoControl -------------------------------------------------------------------

/// `ERROR_IO_PENDING` (`winerror.h`).
const ERROR_IO_PENDING: u32 = 997;

/// `DeviceIoControl` over `NtDeviceIoControlFile` (Poly has no separate
/// `NtFsControlFile` yet: file-system control codes go the same way and
/// fail with `ERROR_INVALID_FUNCTION` where no device takes them). With an
/// `OVERLAPPED` its `Internal`/`InternalHigh` are the I/O status block and
/// its `hEvent` (at 24) is signalled; a pending request is
/// `ERROR_IO_PENDING`. Without one a pending request is waited for.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DeviceIoControl(
    device: Handle,
    code: u32,
    input: *const u8,
    input_len: u32,
    output: *mut u8,
    output_len: u32,
    returned: *mut u32,
    overlapped: *mut u8,
) -> i32 {
    let mut local = IoStatusBlock::default();
    let (iosb, event) = if overlapped.is_null() {
        (&mut local as *mut IoStatusBlock, 0)
    } else {
        // SAFETY: the caller's OVERLAPPED: Internal at 0, hEvent at 24.
        (overlapped.cast::<IoStatusBlock>(), unsafe { overlapped.add(24).cast::<Handle>().read_unaligned() })
    };
    // SAFETY: the caller's buffers; the kernel checks their sizes.
    let mut st = unsafe {
        nt::NtDeviceIoControlFile(
            device,
            event,
            core::ptr::null(),
            core::ptr::null(),
            iosb,
            code,
            input,
            input_len,
            output,
            output_len,
        )
    };
    if st == nt::STATUS_PENDING {
        if !overlapped.is_null() {
            return fail(ERROR_IO_PENDING, FALSE);
        }
        // SAFETY: a handle value; no timeout.
        unsafe { nt::NtWaitForSingleObject(device, 0, core::ptr::null()) };
        // SAFETY: the kernel completed the status block.
        st = NtStatus(unsafe { (*iosb).status } as u32);
    }
    // SAFETY: as above.
    let info = unsafe { (*iosb).information } as u32;
    // SAFETY: the caller's out-pointer, which may be null with an OVERLAPPED.
    unsafe { put(returned, info) };
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}

/// Recorded for `AreFileApisANSI` (`SetFileApisToOEM` / `ToANSI`): 1 while
/// the OEM code page is chosen. Poly's few ANSI file functions still
/// convert through the ANSI code page (ADR 0087).
static FILE_APIS_OEM: AtomicU32 = AtomicU32::new(0);

/// `SetFileApisToOEM`.
#[unsafe(no_mangle)]
pub extern "system" fn SetFileApisToOEM() {
    FILE_APIS_OEM.store(1, Ordering::Relaxed);
}

/// `SetFileApisToANSI`.
#[unsafe(no_mangle)]
pub extern "system" fn SetFileApisToANSI() {
    FILE_APIS_OEM.store(0, Ordering::Relaxed);
}

/// `AreFileApisANSI`.
#[unsafe(no_mangle)]
pub extern "system" fn AreFileApisANSI() -> i32 {
    i32::from(FILE_APIS_OEM.load(Ordering::Relaxed) == 0)
}

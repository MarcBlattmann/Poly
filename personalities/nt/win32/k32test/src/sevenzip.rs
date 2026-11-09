//! What 7-Zip's command line and MSVC's start-up code import (ADR 0087).

use super::*;

// ----- what 7-Zip's command line and MSVC's start-up code import (ADR 0087) ----------

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn IsProcessorFeaturePresent(feature: u32) -> i32;
    fn InitializeSListHead(head: *mut u64);
    fn InterlockedPushEntrySList(head: *mut u64, entry: *mut u64) -> *mut u64;
    fn InterlockedPopEntrySList(head: *mut u64) -> *mut u64;
    fn InterlockedFlushSList(head: *mut u64) -> *mut u64;
    fn QueryDepthSList(head: *mut u64) -> u16;
    fn RtlCaptureContext(context: *mut u8);
    fn RaiseException(code: u32, flags: u32, count: u32, args: *const usize);
    fn SetUnhandledExceptionFilter(filter: usize) -> usize;
    fn UnhandledExceptionFilter(pointers: *mut u8) -> i32;
    fn FileTimeToDosDateTime(ft: *const u64, date: *mut u16, time: *mut u16) -> i32;
    fn DosDateTimeToFileTime(date: u16, time: u16, ft: *mut u64) -> i32;
    fn GetProcessTimes(p: Handle, c: *mut u64, e: *mut u64, k: *mut u64, u: *mut u64) -> i32;
    fn GlobalMemoryStatusEx(status: *mut u8) -> i32;
    fn GetLogicalDrives() -> u32;
    fn GetLogicalDriveStringsW(size: u32, buf: *mut u16) -> u32;
    fn GetDiskFreeSpaceExW(dir: *const u16, caller: *mut u64, total: *mut u64, free: *mut u64) -> i32;
    fn GetDiskFreeSpaceW(root: *const u16, spc: *mut u32, bps: *mut u32, free: *mut u32, total: *mut u32) -> i32;
    fn OpenEventW(access: u32, inherit: i32, name: *const u16) -> Handle;
    pub(super) fn ResumeThread(h: Handle) -> u32;
    fn GetModuleHandleA(name: *const u8) -> *mut u8;
    pub(super) fn GetStartupInfoW(info: *mut u8);
    fn GetProcessAffinityMask(p: Handle, mask: *mut u64, system: *mut u64) -> i32;
    fn SetProcessAffinityMask(p: Handle, mask: u64) -> i32;
    fn SetThreadAffinityMask(t: Handle, mask: u64) -> u64;
    fn GetLargePageMinimum() -> usize;
    fn AreFileApisANSI() -> i32;
    fn SetFileApisToOEM();
    fn SetFileApisToANSI();
    fn MoveFileW(from: *const u16, to: *const u16) -> i32;
    fn MoveFileWithProgressW(from: *const u16, to: *const u16, p: *const u8, d: *mut u8, flags: u32) -> i32;
    fn FindFirstStreamW(name: *const u16, level: u32, data: *mut u8, flags: u32) -> Handle;
    fn FindNextStreamW(find: Handle, data: *mut u8) -> i32;
    fn CreateHardLinkW(new: *const u16, existing: *const u16, sa: *const u8) -> i32;
}

const ERROR_HANDLE_EOF: u32 = 38;
/// `EXCEPTION_CONTINUE_EXECUTION`, `EXCEPTION_CONTINUE_SEARCH`,
/// `EXCEPTION_EXECUTE_HANDLER` (`excpt.h`).
const CONTINUE_EXECUTION: i32 = -1;
const CONTINUE_SEARCH: i32 = 0;
const EXECUTE_HANDLER: i32 = 1;
/// The code `RaiseException` raises here: an application code (bit 29).
const TEST_CODE: u32 = 0xE000_0087;

static RAISED: AtomicU32 = AtomicU32::new(0);

/// A vectored handler (`PVECTORED_EXCEPTION_HANDLER`): takes the test's
/// exception, noting its first parameter and that it has an address.
pub(super) extern "system" fn veh(pointers: *mut u8) -> i32 {
    // SAFETY: EXCEPTION_POINTERS: the record, then the context; the
    // record's code at 0, address at 16, count at 24, parameters at 32.
    unsafe {
        let record = pointers.cast::<*const u8>().read();
        if record.cast::<u32>().read() != TEST_CODE {
            return CONTINUE_SEARCH;
        }
        let address = record.add(16).cast::<u64>().read();
        let count = record.add(24).cast::<u32>().read();
        let first = record.add(32).cast::<u64>().read();
        let ok = address != 0 && count == 2 && first == 0x7A;
        RAISED.store(if ok { 1 } else { 2 }, Ordering::SeqCst);
    }
    CONTINUE_EXECUTION
}

pub(super) extern "system" fn top_filter(_pointers: *mut u8) -> i32 {
    CONTINUE_EXECUTION
}

static RESUMED: AtomicU32 = AtomicU32::new(0);

pub(super) extern "system" fn resumed_worker(_p: *mut u8) -> u32 {
    RESUMED.store(1, Ordering::SeqCst);
    7
}

#[repr(C, align(16))]
struct Aligned<const N: usize>([u8; N]);

pub(super) unsafe fn seven_zip_imports() {
    // SAFETY: valid arguments throughout.
    unsafe {
        // The processor: SSE2 and PAE on every x64 processor.
        check(IsProcessorFeaturePresent(10) == 1 && IsProcessorFeaturePresent(9) == 1);
        check(IsProcessorFeaturePresent(1000) == 0);

        // An interlocked list: last in, first out, with its depth.
        let mut head = Aligned([0u8; 16]);
        let mut entries = [Aligned([0u8; 16]), Aligned([0u8; 16])];
        let h = head.0.as_mut_ptr().cast::<u64>();
        InitializeSListHead(h);
        let (a, b) = (entries[0].0.as_mut_ptr().cast::<u64>(), entries[1].0.as_mut_ptr().cast::<u64>());
        check(InterlockedPushEntrySList(h, a).is_null());
        check(InterlockedPushEntrySList(h, b) == a && QueryDepthSList(h) == 2);
        check(InterlockedPopEntrySList(h) == b && QueryDepthSList(h) == 1);
        check(InterlockedFlushSList(h) == a && QueryDepthSList(h) == 0);
        check(InterlockedPopEntrySList(h).is_null());

        // RtlCaptureContext through kernel32's forwarder to ntdll: the
        // caller's stack and code, every group of registers.
        let mut ctx = Aligned([0u8; 0x4D0]);
        let local = 0u64;
        RtlCaptureContext(ctx.0.as_mut_ptr());
        let q = |o: usize| u64::from_le_bytes(ctx.0[o..o + 8].try_into().unwrap());
        let flags = u32::from_le_bytes(ctx.0[0x30..0x34].try_into().unwrap());
        let rsp = q(0x98);
        let here = &local as *const u64 as u64;
        check(flags == 0x10_000F && q(0xF8) != 0 && rsp.abs_diff(here) < 0x1_0000);
        check(u16::from_le_bytes([ctx.0[0x38], ctx.0[0x39]]) != 0);

        // RaiseException: a vectored handler (ntdll's, looked up by name)
        // sees the record and continues execution, so the call returns.
        let ntdll = GetModuleHandleW(w!("ntdll.dll"));
        let add = GetProcAddress(ntdll, c"RtlAddVectoredExceptionHandler".as_ptr().cast());
        check(!add.is_null());
        type AddVeh = unsafe extern "system" fn(u32, extern "system" fn(*mut u8) -> i32) -> *mut u8;
        let cookie = core::mem::transmute::<*mut u8, AddVeh>(add)(1, veh);
        check(!cookie.is_null());
        let args = [0x7Ausize, 0x7B];
        RaiseException(TEST_CODE, 0, 2, args.as_ptr());
        check(RAISED.load(Ordering::SeqCst) == 1);
        let remove = GetProcAddress(ntdll, c"RtlRemoveVectoredExceptionHandler".as_ptr().cast());
        type RemoveVeh = unsafe extern "system" fn(*mut u8) -> u32;
        check(!remove.is_null() && core::mem::transmute::<*mut u8, RemoveVeh>(remove)(cookie) != 0);

        // The top-level filter: installed, called by UnhandledExceptionFilter.
        check(SetUnhandledExceptionFilter(top_filter as *const () as usize) == 0);
        let mut pointers = [0u64; 2];
        check(UnhandledExceptionFilter(pointers.as_mut_ptr().cast()) == CONTINUE_EXECUTION);
        check(SetUnhandledExceptionFilter(0) == top_filter as *const () as usize);
        check(UnhandledExceptionFilter(pointers.as_mut_ptr().cast()) == EXECUTE_HANDLER);

        // MS-DOS dates: 2023-10-21 13:45:30 and back.
        let st: [u16; 8] = [2023, 10, 0, 21, 13, 45, 31, 0];
        let mut ft = 0u64;
        check(SystemTimeToFileTime(st.as_ptr(), &mut ft) != 0);
        let (mut date, mut time) = (0u16, 0u16);
        check(FileTimeToDosDateTime(&ft, &mut date, &mut time) != 0);
        check(date == (43 << 9) | (10 << 5) | 21 && time == (13 << 11) | (45 << 5) | 15);
        let mut back = 0u64;
        check(DosDateTimeToFileTime(date, time, &mut back) != 0 && ft - back == 10_000_000);
        check(DosDateTimeToFileTime(0, 0, &mut back) == 0);
        check_error(ERROR_INVALID_PARAMETER);

        // Process times: created before now.
        let (mut c, mut e, mut k, mut u) = (0u64, 0u64, 0u64, 0u64);
        check(GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) != 0);
        let mut now = 0u64;
        GetSystemTimeAsFileTime(&mut now);
        check(c != 0 && c <= now && e == 0);

        // Memory: dwLength must be set.
        let mut ms = [0u8; 64];
        ms[0] = 64;
        check(GlobalMemoryStatusEx(ms.as_mut_ptr()) != 0);
        let mq = |o: usize| u64::from_le_bytes(ms[o..o + 8].try_into().unwrap());
        check(mq(8) > 0 && mq(16) <= mq(8) && mq(40) > mq(8));
        ms[0] = 0;
        check(GlobalMemoryStatusEx(ms.as_mut_ptr()) == 0);
        check_error(ERROR_INVALID_PARAMETER);
        check(GetLargePageMinimum() == 0);

        // Drives: C: and Z: (ADR 0067, 0073), and the space on C:.
        let drives = GetLogicalDrives();
        check(drives & (1 << 2) != 0 && drives & (1 << 25) != 0);
        let n = drives.count_ones();
        check(GetLogicalDriveStringsW(0, core::ptr::null_mut()) == 4 * n + 1);
        let mut list = [0u16; 128];
        check(GetLogicalDriveStringsW(128, list.as_mut_ptr()) == 4 * n);
        check(list[..4] == [u16::from(b'C'), u16::from(b':'), u16::from(b'\\'), 0] || drives & 3 != 0);
        let (mut caller, mut total, mut free) = (0u64, 0u64, 0u64);
        check(GetDiskFreeSpaceExW(w!("C:\\"), &mut caller, &mut total, &mut free) != 0);
        check(total > 0 && caller <= total && free <= total);
        let (mut spc, mut bps, mut fc, mut tc) = (0u32, 0u32, 0u32, 0u32);
        check(GetDiskFreeSpaceW(core::ptr::null(), &mut spc, &mut bps, &mut fc, &mut tc) != 0);
        check(spc * bps > 0 && tc > 0);
        check(GetDiskFreeSpaceExW(w!("C:\\no such dir"), &mut caller, &mut total, &mut free) == 0);

        // OpenEventW: a named event, and a name that does not exist.
        let ev = CreateEventW(core::ptr::null(), 1, 0, w!("k32test-7zip"));
        check(ev != 0);
        let opened = OpenEventW(0x1F_0003, 0, w!("k32test-7zip"));
        check(opened != 0 && SetEvent(opened) != 0 && WaitForSingleObject(ev, 0) == WAIT_OBJECT_0);
        check(CloseHandle(opened) != 0 && CloseHandle(ev) != 0);
        check(OpenEventW(0x1F_0003, 0, w!("k32test-no-such-event")) == 0);
        check_error(ERROR_FILE_NOT_FOUND);

        // A suspended thread runs once resumed (its suspend count was 1).
        let t = CreateThread(core::ptr::null(), 0, resumed_worker, core::ptr::null_mut(), 4, core::ptr::null_mut());
        check(t != 0);
        Sleep(10);
        check(RESUMED.load(Ordering::SeqCst) == 0);
        check(ResumeThread(t) == 1);
        check(WaitForSingleObject(t, 10_000) == WAIT_OBJECT_0 && RESUMED.load(Ordering::SeqCst) == 1);
        let mut code = 0u32;
        check(GetExitCodeThread(t, &mut code) != 0 && code == 7 && CloseHandle(t) != 0);

        // Modules, start-up information, affinity.
        check(GetModuleHandleA(c"KERNEL32.DLL".as_ptr().cast()) == GetModuleHandleW(w!("kernel32.dll")));
        check(GetModuleHandleA(core::ptr::null()) == GetModuleHandleW(core::ptr::null()));
        let mut si = [0xFFu8; 104];
        GetStartupInfoW(si.as_mut_ptr());
        check(u32::from_le_bytes(si[0..4].try_into().unwrap()) == 104 && si[60..64] == [0; 4]);
        let (mut pm, mut sm) = (0u64, 0u64);
        check(GetProcessAffinityMask(GetCurrentProcess(), &mut pm, &mut sm) != 0);
        check(pm != 0 && pm & !sm == 0);
        check(SetThreadAffinityMask(GetCurrentThread(), sm) == pm);
        check(SetProcessAffinityMask(GetCurrentProcess(), 0) == 0);
        check_error(ERROR_INVALID_PARAMETER);

        // The file API code page switch.
        check(AreFileApisANSI() != 0);
        SetFileApisToOEM();
        check(AreFileApisANSI() == 0);
        SetFileApisToANSI();
        check(AreFileApisANSI() != 0);

        // The older moves; streams (the default data stream only) and hard
        // links (ADR 0091).
        let f = CreateFileW(w!("C:\\k32move1.txt"), GENERIC_WRITE, 0, core::ptr::null(), CREATE_ALWAYS, 0, 0);
        let mut n = 0u32;
        check(f != INVALID_HANDLE_VALUE && WriteFile(f, b"five!".as_ptr(), 5, &mut n, core::ptr::null_mut()) != 0);
        check(CloseHandle(f) != 0);
        check(MoveFileW(w!("C:\\k32move1.txt"), w!("C:\\k32move2.txt")) != 0);
        check(GetFileAttributesW(w!("C:\\k32move1.txt")) == INVALID_FILE_ATTRIBUTES);
        check(
            MoveFileWithProgressW(w!("C:\\k32move2.txt"), w!("C:\\k32move3.txt"), np(), core::ptr::null_mut(), 0) != 0,
        );
        let mut data = [0u8; 600];
        let fs = FindFirstStreamW(w!("C:\\k32move3.txt"), 0, data.as_mut_ptr(), 0);
        check(fs != INVALID_HANDLE_VALUE);
        check(u64::from_le_bytes(data[0..8].try_into().unwrap()) == 5);
        let stream: [u16; 8] =
            [b':' as u16, b':' as u16, b'$' as u16, b'D' as u16, b'A' as u16, b'T' as u16, b'A' as u16, 0];
        check(data[8..24].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).eq(stream));
        check(FindNextStreamW(fs, data.as_mut_ptr()) == 0);
        check_error(ERROR_HANDLE_EOF);
        check(FindClose(fs) != 0);
        // A directory has no data stream.
        check(FindFirstStreamW(w!("C:\\"), 0, data.as_mut_ptr(), 0) == INVALID_HANDLE_VALUE);
        check_error(ERROR_HANDLE_EOF);
        check(CreateHardLinkW(w!("C:\\k32link.txt"), w!("C:\\k32move3.txt"), core::ptr::null()) != 0);
        let mut info = [0u8; 52];
        let l = CreateFileW(w!("C:\\k32link.txt"), GENERIC_READ, 7, core::ptr::null(), OPEN_EXISTING, 0, 0);
        check(l != INVALID_HANDLE_VALUE && GetFileInformationByHandle(l, info.as_mut_ptr()) != 0);
        // nNumberOfLinks, and the same bytes under the second name.
        check(u32::from_le_bytes(info[40..44].try_into().unwrap()) == 2);
        let mut five = [0u8; 5];
        check(ReadFile(l, five.as_mut_ptr(), 5, &mut n, core::ptr::null_mut()) != 0 && &five == b"five!");
        check(CloseHandle(l) != 0);
        check(CreateHardLinkW(w!("C:\\k32link.txt"), w!("C:\\k32move3.txt"), core::ptr::null()) == 0);
        check_error(ERROR_ALREADY_EXISTS);
        check(DeleteFileW(w!("C:\\k32move3.txt")) != 0);
        check(DeleteFileW(w!("C:\\k32link.txt")) != 0);

        // oleaut32 by ordinal, user32 and advapi32's RtlGenRandom, loaded.
        let ole = LoadLibraryW(w!("oleaut32.dll"));
        check(!ole.is_null());
        type AllocString = unsafe extern "system" fn(*const u16) -> *mut u16;
        type StringLen = unsafe extern "system" fn(*mut u16) -> u32;
        type FreeString = unsafe extern "system" fn(*mut u16);
        type VariantFn = unsafe extern "system" fn(*mut u8) -> u32;
        type VariantCopy = unsafe extern "system" fn(*mut u8, *const u8) -> u32;
        let by = |o: usize| GetProcAddress(ole, o as *const u8);
        check(!by(2).is_null() && !by(6).is_null() && !by(7).is_null() && !by(9).is_null() && !by(10).is_null());
        let alloc_s = core::mem::transmute::<*mut u8, AllocString>(by(2));
        let len_s = core::mem::transmute::<*mut u8, StringLen>(by(7));
        let free_s = core::mem::transmute::<*mut u8, FreeString>(by(6));
        let clear = core::mem::transmute::<*mut u8, VariantFn>(by(9));
        let copy = core::mem::transmute::<*mut u8, VariantCopy>(by(10));
        let s = alloc_s(w!("7-Zip"));
        check(!s.is_null() && len_s(s) == 5 && wslice(s) == wslice(w!("7-Zip")));
        check(alloc_s(core::ptr::null()).is_null() && len_s(core::ptr::null_mut()) == 0);
        // A VT_BSTR variant copied (the string duplicated) and cleared.
        let mut v1 = Aligned([0u8; 24]);
        let mut v2 = Aligned([0u8; 24]);
        v1.0[0] = 8;
        v1.0[8..16].copy_from_slice(&(s as u64).to_le_bytes());
        check(copy(v2.0.as_mut_ptr(), v1.0.as_ptr()) == 0 && v2.0[0] == 8);
        let dup = u64::from_le_bytes(v2.0[8..16].try_into().unwrap()) as *mut u16;
        check(dup != s && len_s(dup) == 5);
        check(clear(v2.0.as_mut_ptr()) == 0 && v2.0[0] == 0);
        check(clear(v1.0.as_mut_ptr()) == 0);
        v1.0[0] = 15;
        check(clear(v1.0.as_mut_ptr()) == 0x8002_0008);
        let _ = free_s;
        let user = LoadLibraryW(w!("user32.dll"));
        let upper = GetProcAddress(user, c"CharUpperW".as_ptr().cast());
        check(!upper.is_null());
        type CharUpper = unsafe extern "system" fn(*mut u16) -> *mut u16;
        let upper = core::mem::transmute::<*mut u8, CharUpper>(upper);
        let mut text = [u16::from(b'z'), u16::from(b'i'), u16::from(b'p'), 0];
        check(upper(text.as_mut_ptr()) == text.as_mut_ptr() && text[..3] == [0x5A, 0x49, 0x50]);
        check(upper(0x61 as *mut u16) as usize == 0x41);
        let advapi = LoadLibraryW(w!("advapi32.dll"));
        let random = GetProcAddress(advapi, c"SystemFunction036".as_ptr().cast());
        check(!random.is_null());
        type GenRandom = unsafe extern "system" fn(*mut u8, u32) -> u8;
        let mut bytes = [0u8; 32];
        check(core::mem::transmute::<*mut u8, GenRandom>(random)(bytes.as_mut_ptr(), 32) != 0);
        check(bytes.iter().any(|&b| b != 0));
    }
}

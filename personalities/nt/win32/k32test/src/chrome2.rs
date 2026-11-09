//! The checks of Chrome's batch 2 (ADR 0155): files, loader, memory,
//! waits, locale, time zones and system information, as Learn's pages for
//! each function document them. Run from `k32test_main` after the earlier
//! groups. Everything is imported from `kernel32`, except the three memory
//! functions Learn lists for `kernelbase` only, which are looked up with
//! `GetProcAddress` the way Chrome does.

use super::{Handle, INVALID_HANDLE_VALUE, ascii_eq, check, check_error, np, push_ascii, wslice};
use core::sync::atomic::{AtomicU32, Ordering};

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn GetTempPathW(size: u32, buf: *mut u16) -> u32;
    fn CreateFileW(n: *const u16, a: u32, s: u32, sa: *const u8, d: u32, f: u32, t: Handle) -> Handle;
    fn WriteFile(h: Handle, buf: *const u8, len: u32, written: *mut u32, ov: *mut u8) -> i32;
    fn CloseHandle(h: Handle) -> i32;
    fn DeleteFileW(n: *const u16) -> i32;
    fn CreateDirectoryW(n: *const u16, sa: *const u8) -> i32;
    fn RemoveDirectoryW(n: *const u16) -> i32;
    fn FindFirstFileExW(p: *const u16, level: u32, data: *mut u8, op: u32, filter: *const u8, flags: u32) -> Handle;
    fn FindNextFileW(h: Handle, data: *mut u8) -> i32;
    fn FindClose(h: Handle) -> i32;
    fn GetFileAttributesExW(n: *const u16, level: u32, data: *mut u8) -> i32;
    fn GetFileInformationByHandleEx(h: Handle, class: u32, buf: *mut u8, size: u32) -> i32;
    fn SetFileInformationByHandle(h: Handle, class: u32, buf: *const u8, size: u32) -> i32;
    fn LockFileEx(h: Handle, flags: u32, reserved: u32, lo: u32, hi: u32, ov: *mut u8) -> i32;
    fn UnlockFileEx(h: Handle, reserved: u32, lo: u32, hi: u32, ov: *mut u8) -> i32;
    fn ReplaceFileW(a: *const u16, b: *const u16, c: *const u16, flags: u32, x: *mut u8, y: *mut u8) -> i32;
    fn GetFinalPathNameByHandleW(h: Handle, buf: *mut u16, size: u32, flags: u32) -> u32;
    fn GetLongPathNameW(s: *const u16, l: *mut u16, size: u32) -> u32;
    fn GetSystemDirectoryW(buf: *mut u16, size: u32) -> u32;
    fn GetWindowsDirectoryW(buf: *mut u16, size: u32) -> u32;
    fn GetModuleHandleW(n: *const u16) -> *mut u8;
    fn GetModuleHandleExW(flags: u32, n: *const u16, out: *mut *mut u8) -> i32;
    fn GetModuleHandleExA(flags: u32, n: *const u8, out: *mut *mut u8) -> i32;
    fn GetProcAddress(m: *mut u8, name: *const u8) -> *mut u8;
    fn GetCurrentThread() -> Handle;
    fn SetThreadDescription(t: Handle, d: *const u16) -> i32;
    fn GetThreadDescription(t: Handle, d: *mut *mut u16) -> i32;
    fn LocalFree(m: *mut u8) -> *mut u8;
    fn WerRegisterRuntimeExceptionModule(p: *const u16, c: *mut u8) -> i32;
    fn VirtualAlloc(a: *mut u8, size: usize, kind: u32, protect: u32) -> *mut u8;
    fn VirtualFree(a: *mut u8, size: usize, kind: u32) -> i32;
    fn DiscardVirtualMemory(a: *mut u8, size: usize) -> u32;
    fn PrefetchVirtualMemory(p: Handle, n: usize, e: *const u8, flags: u32) -> i32;
    fn HeapSetInformation(h: *mut u8, class: u32, info: *mut u8, len: usize) -> i32;
    fn GetProcessHeap() -> *mut u8;
    fn CreateFileMappingW(f: Handle, sa: *const u8, prot: u32, hi: u32, lo: u32, n: *const u16) -> Handle;
    fn CreateEventW(sa: *const u8, manual: i32, initial: i32, name: *const u16) -> Handle;
    fn SetEvent(h: Handle) -> i32;
    fn WaitForSingleObject(h: Handle, ms: u32) -> u32;
    fn RegisterWaitForSingleObject(
        out: *mut Handle,
        object: Handle,
        callback: unsafe extern "system" fn(*mut u8, u8),
        ctx: *mut u8,
        ms: u32,
        flags: u32,
    ) -> i32;
    fn UnregisterWaitEx(wait: Handle, completion: Handle) -> i32;
    fn UnregisterWait(wait: Handle) -> i32;
    fn SwitchToThread() -> i32;
    fn GetLocaleInfoEx(name: *const u16, lctype: u32, buf: *mut u16, cch: i32) -> i32;
    fn IsValidLocaleName(name: *const u16) -> i32;
    fn GetUserGeoID(class: u32) -> i32;
    fn GetGeoInfoW(loc: i32, ty: u32, data: *mut u16, cch: i32, lang: u16) -> i32;
    fn GetTimeZoneInformation(tz: *mut u8) -> u32;
    fn GetDynamicTimeZoneInformation(tz: *mut u8) -> u32;
    fn SystemTimeToTzSpecificLocalTime(tz: *const u8, utc: *const u8, local: *mut u8) -> i32;
    fn TzSpecificLocalTimeToSystemTime(tz: *const u8, local: *const u8, utc: *mut u8) -> i32;
    fn GetLogicalProcessorInformation(buf: *mut u8, len: *mut u32) -> i32;
    fn GetLogicalProcessorInformationEx(rel: u32, buf: *mut u8, len: *mut u32) -> i32;
    fn GetComputerNameExW(kind: u32, buf: *mut u16, size: *mut u32) -> i32;
    fn GetComputerNameW(buf: *mut u16, size: *mut u32) -> i32;
    fn IsWow64Process2(p: Handle, a: *mut u16, b: *mut u16) -> i32;
    fn GetSystemPowerStatus(s: *mut u8) -> i32;
    fn GetCurrentProcess() -> Handle;
    fn lstrcmpiA(a: *const u8, b: *const u8) -> i32;
}

const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const DELETE: u32 = 0x1_0000;
const SHARE_ALL: u32 = 7;
const CREATE_ALWAYS: u32 = 2;
const OPEN_EXISTING: u32 = 3;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const MEM_COMMIT_RESERVE: u32 = 0x3000;
const MEM_RELEASE: u32 = 0x8000;
const PAGE_READWRITE: u32 = 4;
const INFINITE: u32 = u32::MAX;

/// `[a, b]` joined as `a` + `b`, NUL-terminated.
fn join(a: &[u16], b: &[u8]) -> [u16; 300] {
    let mut p = [0u16; 300];
    p[..a.len()].copy_from_slice(a);
    let mut at = a.len();
    push_ascii(&mut p, &mut at, b);
    p
}

fn len_of(p: &[u16]) -> usize {
    p.iter().position(|&c| c == 0).unwrap_or(p.len())
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

fn ends_with_ascii(w: &[u16], tail: &[u8]) -> bool {
    w.len() >= tail.len() && ascii_eq(&w[w.len() - tail.len()..], tail)
}

pub unsafe fn run() {
    // SAFETY: every call is a kernel32 export with valid arguments.
    unsafe {
        files();
        loader_and_threads();
        memory();
        waits();
        locale_time_and_system();
    }
}

unsafe fn files() {
    // SAFETY: valid arguments throughout.
    unsafe {
        let mut temp = [0u16; 260];
        let n = GetTempPathW(260, temp.as_mut_ptr()) as usize;
        check(n > 3);
        let dir = join(&temp[..n], b"k32c2");
        DeleteFileW(join(&dir[..len_of(&dir)], b"\\a.dat").as_ptr());
        RemoveDirectoryW(dir.as_ptr());
        check(CreateDirectoryW(dir.as_ptr(), np()) != 0);
        let d = &dir[..len_of(&dir)];
        let file = join(d, b"\\a.dat");

        // A file of five bytes.
        let h = CreateFileW(
            file.as_ptr(),
            GENERIC_READ | GENERIC_WRITE | DELETE,
            SHARE_ALL,
            np(),
            CREATE_ALWAYS,
            FILE_ATTRIBUTE_NORMAL,
            0,
        );
        check(h != INVALID_HANDLE_VALUE);
        let mut done = 0u32;
        check(WriteFile(h, b"hello".as_ptr(), 5, &mut done, core::ptr::null_mut()) != 0 && done == 5);

        // GetFileAttributesExW: WIN32_FILE_ATTRIBUTE_DATA.
        let mut data = [0u8; 36];
        check(GetFileAttributesExW(file.as_ptr(), 0, data.as_mut_ptr()) != 0);
        check(u32_at(&data, 0) & 0x10 == 0 && u32_at(&data, 28) == 0 && u32_at(&data, 32) == 5);
        check(u64_at(&data, 4) != 0);
        check(GetFileAttributesExW(file.as_ptr(), 1, data.as_mut_ptr()) == 0);
        check_error(87);
        check(GetFileAttributesExW(join(d, b"\\nope").as_ptr(), 0, data.as_mut_ptr()) == 0);
        check_error(2);
        check(GetFileAttributesExW(join(d, b"\\nope\\x").as_ptr(), 0, data.as_mut_ptr()) == 0);
        check_error(3);

        // FindFirstFileExW: the basic level has no short name; a filter
        // with the name-match operation is invalid; devices are not supported.
        let mut find = [0u8; 592];
        let pattern = join(d, b"\\*.dat");
        let f = FindFirstFileExW(pattern.as_ptr(), 1, find.as_mut_ptr(), 0, np(), 2);
        check(f != INVALID_HANDLE_VALUE);
        let name = core::slice::from_raw_parts(find.as_ptr().add(44) as *const u16, 5);
        check(ascii_eq(name, b"a.dat"));
        check(find[564] == 0 && find[565] == 0);
        check(u32_at(&find, 32) == 5);
        check(FindNextFileW(f, find.as_mut_ptr()) == 0);
        check_error(18);
        check(FindClose(f) != 0);
        check(FindFirstFileExW(pattern.as_ptr(), 2, find.as_mut_ptr(), 0, np(), 0) == INVALID_HANDLE_VALUE);
        check_error(87);
        check(FindFirstFileExW(pattern.as_ptr(), 0, find.as_mut_ptr(), 0, find.as_ptr(), 0) == INVALID_HANDLE_VALUE);
        check_error(87);
        check(FindFirstFileExW(pattern.as_ptr(), 0, find.as_mut_ptr(), 2, np(), 0) == INVALID_HANDLE_VALUE);
        check_error(50);
        check(FindFirstFileExW(join(d, b"\\*.zzz").as_ptr(), 0, find.as_mut_ptr(), 0, np(), 0) == INVALID_HANDLE_VALUE);
        check_error(2);

        // GetFileInformationByHandleEx.
        let mut b = [0u8; 64];
        check(GetFileInformationByHandleEx(h, 0, b.as_mut_ptr(), 40) != 0); // FileBasicInfo
        check(u32_at(&b, 32) & 0x10 == 0);
        check(GetFileInformationByHandleEx(h, 1, b.as_mut_ptr(), 24) != 0); // FileStandardInfo
        check(u64_at(&b, 8) == 5 && u32_at(&b, 16) >= 1 && b[20] == 0 && b[21] == 0);
        check(GetFileInformationByHandleEx(h, 8, b.as_mut_ptr(), 16) != 0); // FileCompressionInfo
        check(u64_at(&b, 0) == 5 && b[8] == 0 && b[9] == 0);
        check(GetFileInformationByHandleEx(h, 9, b.as_mut_ptr(), 8) != 0); // FileAttributeTagInfo
        check(u32_at(&b, 4) == 0);
        check(GetFileInformationByHandleEx(h, 18, b.as_mut_ptr(), 24) != 0); // FileIdInfo
        check(GetFileInformationByHandleEx(h, 18, b.as_mut_ptr(), 8) == 0);
        check_error(24);
        check(GetFileInformationByHandleEx(h, 12, b.as_mut_ptr(), 4) != 0 && u32_at(&b, 0) == 2); // normal priority
        let mut nm = [0u8; 600];
        check(GetFileInformationByHandleEx(h, 2, nm.as_mut_ptr(), 600) != 0); // FileNameInfo
        let nl = u32_at(&nm, 0) as usize / 2;
        let nw = core::slice::from_raw_parts(nm.as_ptr().add(4) as *const u16, nl);
        check(ends_with_ascii(nw, b"\\k32c2\\a.dat"));
        check(GetFileInformationByHandleEx(h, 3, b.as_mut_ptr(), 40) == 0); // rename is set-only
        check_error(87);
        check(GetFileInformationByHandleEx(h, 99, b.as_mut_ptr(), 40) == 0);
        check_error(87);

        // GetFinalPathNameByHandleW: \\?\C:\...; the size when too small.
        let mut fin = [0u16; 400];
        let fl = GetFinalPathNameByHandleW(h, fin.as_mut_ptr(), 400, 0) as usize;
        check(fl > 8 && ascii_eq(&fin[..4], b"\\\\?\\") && fin[5] == u16::from(b':'));
        check(ends_with_ascii(&fin[..fl], b"\\k32c2\\a.dat"));
        check(GetFinalPathNameByHandleW(h, fin.as_mut_ptr(), 3, 0) as usize == fl + 1);
        let nl2 = GetFinalPathNameByHandleW(h, fin.as_mut_ptr(), 400, 4) as usize; // VOLUME_NAME_NONE
        check(nl2 > 0 && fin[0] == u16::from(b'\\') && fin[1] != u16::from(b'\\'));
        check(GetFinalPathNameByHandleW(h, fin.as_mut_ptr(), 400, 3) == 0);
        check_error(87);

        // SetFileInformationByHandle: end of file, then the I/O priority.
        let mut eof = [0u8; 8];
        eof[0] = 2;
        check(SetFileInformationByHandle(h, 6, eof.as_ptr(), 8) != 0);
        check(GetFileInformationByHandleEx(h, 1, b.as_mut_ptr(), 24) != 0 && u64_at(&b, 8) == 2);
        check(SetFileInformationByHandle(h, 6, eof.as_ptr(), 4) == 0);
        check_error(24);
        check(SetFileInformationByHandle(h, 1, eof.as_ptr(), 8) == 0);
        check_error(87);

        // LockFileEx / UnlockFileEx: a second handle conflicts.
        let h2 = CreateFileW(
            file.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            SHARE_ALL,
            np(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            0,
        );
        check(h2 != INVALID_HANDLE_VALUE);
        let mut ov = [0u8; 32];
        ov[16..20].copy_from_slice(&0u32.to_le_bytes());
        check(LockFileEx(h, 3, 0, 100, 0, ov.as_mut_ptr()) != 0); // exclusive, fail immediately
        check(LockFileEx(h2, 3, 0, 100, 0, ov.as_mut_ptr()) == 0);
        check_error(33); // ERROR_LOCK_VIOLATION
        check(LockFileEx(h2, 3, 0, 100, 0, core::ptr::null_mut()) == 0);
        check_error(87); // the OVERLAPPED is required
        check(UnlockFileEx(h, 0, 100, 0, ov.as_mut_ptr()) != 0);
        check(UnlockFileEx(h, 0, 100, 0, ov.as_mut_ptr()) == 0);
        check_error(158); // ERROR_NOT_LOCKED
        check(LockFileEx(h2, 3, 0, 100, 0, ov.as_mut_ptr()) != 0);
        check(UnlockFileEx(h2, 0, 100, 0, ov.as_mut_ptr()) != 0);
        check(CloseHandle(h2) != 0);

        // GetLongPathNameW: exact names come back with the directory's case.
        let upper = join(d, b"\\A.DAT");
        let mut long = [0u16; 400];
        let ll = GetLongPathNameW(upper.as_ptr(), long.as_mut_ptr(), 400) as usize;
        check(ll == len_of(&upper));
        check(ends_with_ascii(&long[..ll], b"\\a.dat"));
        check(GetLongPathNameW(upper.as_ptr(), long.as_mut_ptr(), 3) as usize == ll + 1);
        check(GetLongPathNameW(join(d, b"\\nope").as_ptr(), long.as_mut_ptr(), 400) == 0);
        check_error(2);
        check(GetLongPathNameW(join(d, b"\\nope\\x").as_ptr(), long.as_mut_ptr(), 400) == 0);
        check_error(3);

        // ReplaceFileW: new content takes the name; the old goes to the backup.
        check(CloseHandle(h) != 0);
        let new = join(d, b"\\b.dat");
        let bak = join(d, b"\\a.bak");
        let hb = CreateFileW(new.as_ptr(), GENERIC_WRITE, SHARE_ALL, np(), CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, 0);
        check(hb != INVALID_HANDLE_VALUE);
        check(WriteFile(hb, b"newer data".as_ptr(), 10, &mut done, core::ptr::null_mut()) != 0);
        check(CloseHandle(hb) != 0);
        check(
            ReplaceFileW(file.as_ptr(), new.as_ptr(), bak.as_ptr(), 0, np::<u8>() as *mut u8, np::<u8>() as *mut u8)
                != 0,
        );
        check(GetFileAttributesExW(file.as_ptr(), 0, data.as_mut_ptr()) != 0 && u32_at(&data, 32) == 10);
        check(GetFileAttributesExW(bak.as_ptr(), 0, data.as_mut_ptr()) != 0 && u32_at(&data, 32) == 2);
        check(GetFileAttributesExW(new.as_ptr(), 0, data.as_mut_ptr()) == 0);
        // Without a backup; a missing replaced file is ERROR_FILE_NOT_FOUND.
        let third = join(d, b"\\c.dat");
        let hc = CreateFileW(third.as_ptr(), GENERIC_WRITE, SHARE_ALL, np(), CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, 0);
        check(hc != INVALID_HANDLE_VALUE && CloseHandle(hc) != 0);
        check(ReplaceFileW(file.as_ptr(), third.as_ptr(), np(), 0, np::<u8>() as *mut u8, np::<u8>() as *mut u8) != 0);
        check(GetFileAttributesExW(file.as_ptr(), 0, data.as_mut_ptr()) != 0 && u32_at(&data, 32) == 0);
        check(
            ReplaceFileW(
                join(d, b"\\none").as_ptr(),
                file.as_ptr(),
                np(),
                0,
                np::<u8>() as *mut u8,
                np::<u8>() as *mut u8,
            ) == 0,
        );
        check_error(2);
        check(ReplaceFileW(file.as_ptr(), file.as_ptr(), np(), 8, np::<u8>() as *mut u8, np::<u8>() as *mut u8) == 0);
        check_error(87);

        // Directories by handle: FileIdBothDirectoryInfo lists the entries.
        let hd = CreateFileW(dir.as_ptr(), 1, SHARE_ALL, np(), OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, 0);
        check(hd != INVALID_HANDLE_VALUE);
        let mut list = [0u8; 2048];
        check(GetFileInformationByHandleEx(hd, 11, list.as_mut_ptr(), 2048) != 0);
        let mut at = 0usize;
        let mut names = 0;
        loop {
            let nlen = u32_at(&list, at + 60) as usize / 2;
            let nm = core::slice::from_raw_parts(list.as_ptr().add(at + 104) as *const u16, nlen);
            if ascii_eq(nm, b"a.dat") || ascii_eq(nm, b"a.bak") {
                names += 1;
            }
            let next = u32_at(&list, at) as usize;
            if next == 0 {
                break;
            }
            at += next;
        }
        check(names == 2);
        check(CloseHandle(hd) != 0);

        // GetSystemDirectoryW / GetWindowsDirectoryW.
        let mut sys = [0u16; 260];
        let sl = GetSystemDirectoryW(sys.as_mut_ptr(), 260) as usize;
        check(sl > 10 && ends_with_ascii(&sys[..sl], b"\\System32"));
        check(GetSystemDirectoryW(sys.as_mut_ptr(), 4) as usize == sl + 1);
        let wl = GetWindowsDirectoryW(sys.as_mut_ptr(), 260) as usize;
        check(wl > 2 && wl + 9 == sl);

        for name in [&b"\\a.dat"[..], b"\\a.bak"] {
            check(DeleteFileW(join(d, name).as_ptr()) != 0);
        }
        check(RemoveDirectoryW(dir.as_ptr()) != 0);
    }
}

unsafe fn loader_and_threads() {
    // SAFETY: valid arguments throughout.
    unsafe {
        // GetModuleHandleExW: by name, unchanged count, from an address.
        let k32 = GetModuleHandleW(w!("kernel32.dll"));
        check(!k32.is_null());
        let mut m: *mut u8 = core::ptr::null_mut();
        check(GetModuleHandleExW(2, w!("KERNEL32.DLL"), &mut m) != 0 && m == k32);
        m = core::ptr::null_mut();
        check(GetModuleHandleExW(0, w!("kernel32"), &mut m) != 0 && m == k32);
        m = core::ptr::null_mut();
        check(GetModuleHandleExW(1, w!("kernel32.dll"), &mut m) != 0 && m == k32); // pinned
        let own = GetModuleHandleW(core::ptr::null());
        m = core::ptr::null_mut();
        check(GetModuleHandleExW(6, run as *const () as *const u16, &mut m) != 0 && m == own);
        m = core::ptr::dangling_mut::<u8>();
        check(GetModuleHandleExW(0, w!("no-such-module.dll"), &mut m) == 0 && m.is_null());
        check_error(126);
        check(GetModuleHandleExW(3, w!("kernel32.dll"), &mut m) == 0); // PIN | UNCHANGED_REFCOUNT
        check_error(87);
        check(GetModuleHandleExW(8, w!("kernel32.dll"), &mut m) == 0);
        check_error(87);
        check(GetModuleHandleExW(0, w!("kernel32.dll"), core::ptr::null_mut()) == 0);
        check_error(87);
        m = core::ptr::null_mut();
        check(GetModuleHandleExA(2, c"kernel32.dll".as_ptr().cast(), &mut m) != 0 && m == k32);

        // The crash-reporting registration succeeds for a path.
        check(WerRegisterRuntimeExceptionModule(w!("C:\\chrome_wer.dll"), core::ptr::null_mut()) == 0);
        check(WerRegisterRuntimeExceptionModule(core::ptr::null(), core::ptr::null_mut()) != 0);

        // SetThreadDescription / GetThreadDescription round trip.
        let me = GetCurrentThread();
        let mut got: *mut u16 = core::ptr::null_mut();
        check(GetThreadDescription(me, &mut got) == 0 && !got.is_null() && wslice(got).is_empty());
        check(LocalFree(got.cast()).is_null());
        check(SetThreadDescription(me, w!("ChromeIOThread")) == 0);
        check(GetThreadDescription(me, &mut got) == 0 && ascii_eq(wslice(got), b"ChromeIOThread"));
        check(LocalFree(got.cast()).is_null());
        check(SetThreadDescription(me, core::ptr::null()) != 0);
        check(SetThreadDescription(me, w!("")) == 0);
        check(GetThreadDescription(me, &mut got) == 0 && wslice(got).is_empty());
        check(LocalFree(got.cast()).is_null());

        // SwitchToThread returns (whether or not another thread ran).
        let _ = SwitchToThread();
        let (mut a, mut b) = (7u16, 7u16);
        check(IsWow64Process2(GetCurrentProcess(), &mut a, &mut b) != 0 && a == 0 && b == 0x8664);
        check(IsWow64Process2(GetCurrentProcess(), core::ptr::null_mut(), &mut b) == 0);
        check_error(87);
    }
}

unsafe fn memory() {
    // SAFETY: valid arguments throughout.
    unsafe {
        // HeapSetInformation.
        check(HeapSetInformation(core::ptr::null_mut(), 1, core::ptr::null_mut(), 0) != 0);
        let mut lfh = 2u32.to_le_bytes();
        check(HeapSetInformation(GetProcessHeap(), 0, lfh.as_mut_ptr(), 4) != 0);
        check(HeapSetInformation(GetProcessHeap(), 1, lfh.as_mut_ptr(), 4) == 0);
        check_error(87);
        check(HeapSetInformation(GetProcessHeap(), 77, core::ptr::null_mut(), 0) == 0);
        check_error(87);

        // DiscardVirtualMemory succeeds on committed pages.
        let p = VirtualAlloc(core::ptr::null_mut(), 0x4000, MEM_COMMIT_RESERVE, PAGE_READWRITE);
        check(!p.is_null());
        *p = 0x5A;
        *p.add(0x1000) = 0x5A;
        check(DiscardVirtualMemory(p, 0x2000) == 0);
        check(PrefetchVirtualMemory(GetCurrentProcess(), 1, [p as usize, 0x1000usize].as_ptr().cast(), 0) != 0);
        check(PrefetchVirtualMemory(GetCurrentProcess(), 1, [p as usize, 0x1000usize].as_ptr().cast(), 1) == 0);
        check_error(87);
        check(VirtualFree(p, 0, MEM_RELEASE) != 0);

        // VirtualAlloc2 / MapViewOfFile3 / UnmapViewOfFile2: kernelbase's
        // exports, found the way Chrome finds them.
        let kb = GetModuleHandleW(w!("kernelbase.dll"));
        check(!kb.is_null());
        type VirtualAlloc2 = unsafe extern "system" fn(Handle, *mut u8, usize, u32, u32, *mut u8, u32) -> *mut u8;
        type MapViewOfFile3 =
            unsafe extern "system" fn(Handle, Handle, *mut u8, u64, usize, u32, u32, *mut u8, u32) -> *mut u8;
        type UnmapViewOfFile2 = unsafe extern "system" fn(Handle, *const u8, u32) -> i32;
        let va2 = GetProcAddress(kb, c"VirtualAlloc2".as_ptr().cast());
        let mv3 = GetProcAddress(kb, c"MapViewOfFile3".as_ptr().cast());
        let uv2 = GetProcAddress(kb, c"UnmapViewOfFile2".as_ptr().cast());
        check(!va2.is_null() && !mv3.is_null() && !uv2.is_null());
        let va2: VirtualAlloc2 = core::mem::transmute(va2);
        let mv3: MapViewOfFile3 = core::mem::transmute(mv3);
        let uv2: UnmapViewOfFile2 = core::mem::transmute(uv2);

        // No parameters: VirtualAlloc in this process.
        let q = va2(0, core::ptr::null_mut(), 0x2000, MEM_COMMIT_RESERVE, PAGE_READWRITE, core::ptr::null_mut(), 0);
        check(!q.is_null());
        *q = 1;
        check(VirtualFree(q, 0, MEM_RELEASE) != 0);
        // An address requirement: 4 MiB alignment, below 2^46.
        let mut req = [0u8; 24];
        req[8..16].copy_from_slice(&((1u64 << 46) - 1).to_le_bytes());
        req[16..24].copy_from_slice(&0x40_0000u64.to_le_bytes());
        let mut ext = [0u8; 16];
        ext[0] = 1;
        ext[8..16].copy_from_slice(&(req.as_ptr() as u64).to_le_bytes());
        let r = va2(0, core::ptr::null_mut(), 0x10_0000, MEM_COMMIT_RESERVE, PAGE_READWRITE, ext.as_mut_ptr(), 1);
        check(!r.is_null() && (r as usize).is_multiple_of(0x40_0000) && (r as usize) < (1 << 46));
        *r = 2;
        check(VirtualFree(r, 0, MEM_RELEASE) != 0);
        // Not supported / invalid parameters.
        let mut bad = [0u8; 16];
        bad[0] = 3; // MemExtendedParameterPartitionHandle
        check(va2(0, core::ptr::null_mut(), 0x1000, MEM_COMMIT_RESERVE, PAGE_READWRITE, bad.as_mut_ptr(), 1).is_null());
        check_error(50);
        bad[0] = 0;
        check(va2(0, core::ptr::null_mut(), 0x1000, MEM_COMMIT_RESERVE, PAGE_READWRITE, bad.as_mut_ptr(), 1).is_null());
        check_error(87);

        // A view of a section, mapped with an alignment requirement and
        // unmapped.
        let sec = CreateFileMappingW(INVALID_HANDLE_VALUE, np(), PAGE_READWRITE, 0, 0x1_0000, core::ptr::null());
        check(sec != 0);
        let v = mv3(sec, 0, core::ptr::null_mut(), 0, 0x1_0000, 0, PAGE_READWRITE, ext.as_mut_ptr(), 1);
        check(!v.is_null() && (v as usize).is_multiple_of(0x40_0000));
        *v = 9;
        check(uv2(0, v, 0) != 0);
        check(uv2(0, v, 0) == 0);
        check(uv2(0, v, 4) == 0);
        check_error(87);
        let v = mv3(sec, 0, core::ptr::null_mut(), 0, 0x1_0000, 0, PAGE_READWRITE, core::ptr::null_mut(), 0);
        check(!v.is_null());
        // The view stays a reservation when unmapped with MEM_PRESERVE_PLACEHOLDER.
        check(uv2(0, v, 2) != 0);
        check(VirtualFree(v, 0, MEM_RELEASE) != 0);
        check(CloseHandle(sec) != 0);
    }
}

static FIRED: AtomicU32 = AtomicU32::new(0);
static FIRED_TIMEOUT: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn on_wait(context: *mut u8, timed_out: u8) {
    FIRED.fetch_add(1, Ordering::SeqCst);
    if timed_out != 0 {
        FIRED_TIMEOUT.fetch_add(1, Ordering::SeqCst);
    }
    // SAFETY: `context` is the event the test passed.
    unsafe { SetEvent(context as Handle) };
}

unsafe fn waits() {
    // SAFETY: valid arguments throughout.
    unsafe {
        let trigger = CreateEventW(np(), 0, 0, np());
        let done = CreateEventW(np(), 0, 0, np());
        check(trigger != 0 && done != 0);
        let mut wait: Handle = 0;
        // An object signaled: the callback runs with TimerOrWaitFired FALSE.
        check(RegisterWaitForSingleObject(&mut wait, trigger, on_wait, done as *mut u8, INFINITE, 0) != 0 && wait != 0);
        check(WaitForSingleObject(done, 100) == 0x102);
        check(SetEvent(trigger) != 0);
        check(WaitForSingleObject(done, 10_000) == 0);
        check(FIRED.load(Ordering::SeqCst) == 1 && FIRED_TIMEOUT.load(Ordering::SeqCst) == 0);
        // Not once-only: signaled again, it fires again.
        check(SetEvent(trigger) != 0);
        check(WaitForSingleObject(done, 10_000) == 0);
        check(FIRED.load(Ordering::SeqCst) == 2);
        // Unregistered, blocking until the callbacks are done.
        check(UnregisterWaitEx(wait, INVALID_HANDLE_VALUE) != 0);
        check(SetEvent(trigger) != 0);
        check(WaitForSingleObject(done, 300) == 0x102);
        check(FIRED.load(Ordering::SeqCst) == 2);

        // A timeout: the callback runs with TimerOrWaitFired TRUE, once
        // (WT_EXECUTEONLYONCE).
        let never = CreateEventW(np(), 1, 0, np());
        check(RegisterWaitForSingleObject(&mut wait, never, on_wait, done as *mut u8, 50, 8) != 0);
        check(WaitForSingleObject(done, 10_000) == 0);
        check(FIRED_TIMEOUT.load(Ordering::SeqCst) == 1);
        check(WaitForSingleObject(done, 300) == 0x102);
        check(UnregisterWait(wait) != 0);

        // Arguments.
        check(RegisterWaitForSingleObject(&mut wait, never, on_wait, np::<u8>() as *mut u8, 50, 0x400) == 0);
        check_error(87);
        check(RegisterWaitForSingleObject(core::ptr::null_mut(), never, on_wait, np::<u8>() as *mut u8, 50, 0) == 0);
        check_error(87);
        check(UnregisterWaitEx(0, 0) == 0);
        check_error(6);
        let _ = GetLastError();
        check(CloseHandle(trigger) != 0 && CloseHandle(done) != 0 && CloseHandle(never) != 0);
    }
}

unsafe fn locale_time_and_system() {
    // SAFETY: valid arguments throughout.
    unsafe {
        // Locale names.
        let mut b = [0u16; 64];
        check(IsValidLocaleName(w!("en-US")) != 0 && IsValidLocaleName(w!("EN-us")) != 0);
        check(IsValidLocaleName(w!("xx-YY")) == 0);
        check(GetLocaleInfoEx(w!("en-US"), 0x2, b.as_mut_ptr(), 64) > 0); // LOCALE_SLANGUAGE
        check(ascii_eq(wslice(b.as_ptr()), b"English (United States)"));
        check(GetLocaleInfoEx(core::ptr::null(), 0x5, b.as_mut_ptr(), 64) > 0); // LOCALE_ICOUNTRY, user default
        check(GetLocaleInfoEx(w!("tlh-XX"), 0x2, b.as_mut_ptr(), 64) == 0);
        check_error(87);
        check(GetLocaleInfoEx(w!("en-US"), 0x2, b.as_mut_ptr(), 4) == 0);
        check_error(122);

        // Geographic location.
        check(GetUserGeoID(16) == 244);
        check(GetUserGeoID(99) == -1);
        check_error(87);
        check(GetGeoInfoW(244, 4, b.as_mut_ptr(), 64, 0) == 3 && ascii_eq(wslice(b.as_ptr()), b"US"));
        check(GetGeoInfoW(244, 4, core::ptr::null_mut(), 0, 0) == 3);

        // Time zones: Poly's is UTC.
        let mut tz = [0xFFu8; 172];
        check(GetTimeZoneInformation(tz.as_mut_ptr()) == 0);
        check(u32_at(&tz, 0) == 0 && u32_at(&tz, 84) == 0 && u32_at(&tz, 168) == 0);
        let mut dtz = [0xFFu8; 432];
        check(GetDynamicTimeZoneInformation(dtz.as_mut_ptr()) == 0);
        check(dtz[172] == b'U' && dtz[428] == 0);
        // SYSTEMTIME: 2024-07-04 12:00:00 (a Thursday).
        let mut utc = [0u8; 16];
        utc[0..2].copy_from_slice(&2024u16.to_le_bytes());
        utc[2..4].copy_from_slice(&7u16.to_le_bytes());
        utc[4..6].copy_from_slice(&4u16.to_le_bytes());
        utc[6..8].copy_from_slice(&4u16.to_le_bytes());
        utc[8..10].copy_from_slice(&12u16.to_le_bytes());
        let mut local = [0u8; 16];
        check(SystemTimeToTzSpecificLocalTime(core::ptr::null(), utc.as_ptr(), local.as_mut_ptr()) != 0);
        check(local[0..12] == utc[0..12]);
        // A zone with an offset: UTC-8, no daylight saving time.
        let mut pst = [0u8; 172];
        pst[0..4].copy_from_slice(&480i32.to_le_bytes());
        check(SystemTimeToTzSpecificLocalTime(pst.as_ptr(), utc.as_ptr(), local.as_mut_ptr()) != 0);
        check(u16::from_le_bytes([local[8], local[9]]) == 4 && u16::from_le_bytes([local[6], local[7]]) == 4);
        let mut back = [0u8; 16];
        check(TzSpecificLocalTimeToSystemTime(pst.as_ptr(), local.as_ptr(), back.as_mut_ptr()) != 0);
        check(back[0..12] == utc[0..12]);
        utc[2..4].copy_from_slice(&13u16.to_le_bytes());
        check(SystemTimeToTzSpecificLocalTime(pst.as_ptr(), utc.as_ptr(), local.as_mut_ptr()) == 0);
        check_error(87);

        // Processors.
        let mut len = 0u32;
        check(GetLogicalProcessorInformation(core::ptr::null_mut(), &mut len) == 0);
        check_error(122);
        check(len >= 3 * 32 && len.is_multiple_of(32));
        let mut buf = [0u8; 4096];
        let mut have = buf.len() as u32;
        check(GetLogicalProcessorInformation(buf.as_mut_ptr(), &mut have) != 0 && have == len);
        check(u32_at(&buf, 8) == 0 && u64_at(&buf, 0) == 1);
        for rel in [0u32, 0xFFFF, 1, 3, 4] {
            let mut need = 0u32;
            check(GetLogicalProcessorInformationEx(rel, core::ptr::null_mut(), &mut need) == 0);
            check_error(122);
            check(need >= 48 && need as usize <= buf.len());
            let mut have = buf.len() as u32;
            check(GetLogicalProcessorInformationEx(rel, buf.as_mut_ptr(), &mut have) != 0 && have == need);
            check(u32_at(&buf, 4) >= 48);
        }
        check(GetLogicalProcessorInformationEx(77, buf.as_mut_ptr(), &mut len) == 0);
        check_error(87);

        // The computer's names.
        let mut size = 64u32;
        check(GetComputerNameExW(0, b.as_mut_ptr(), &mut size) != 0);
        check(size as usize == wslice(b.as_ptr()).len() && size > 0 && size <= 15);
        let nb = size;
        size = 1;
        check(GetComputerNameExW(1, b.as_mut_ptr(), &mut size) == 0);
        check_error(234);
        check(size > 1);
        size = 64;
        check(GetComputerNameExW(2, b.as_mut_ptr(), &mut size) != 0 && size == 0 && b[0] == 0);
        check(GetComputerNameExW(8, b.as_mut_ptr(), &mut size) == 0);
        check_error(87);
        size = 2;
        check(GetComputerNameW(b.as_mut_ptr(), &mut size) == 0);
        check_error(111);
        check(size == nb + 1);
        size = 64;
        check(GetComputerNameW(b.as_mut_ptr(), &mut size) != 0 && size == nb);

        // Power status: on AC power, no battery.
        let mut ps = [0u8; 12];
        check(GetSystemPowerStatus(ps.as_mut_ptr()) != 0);
        check(ps[0] == 1 && ps[1] == 128 && ps[2] == 255);

        // lstrcmpiA ignores case.
        check(lstrcmpiA(c"Chrome".as_ptr().cast(), c"cHROME".as_ptr().cast()) == 0);
        check(lstrcmpiA(c"a".as_ptr().cast(), c"B".as_ptr().cast()) < 0);
        check(lstrcmpiA(c"b".as_ptr().cast(), c"A".as_ptr().cast()) > 0);
    }
}

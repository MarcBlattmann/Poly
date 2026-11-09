//! The second slice's checks (ADR 0079): processes, TLS and FLS, code pages, file mappings, conditions, file information.

use super::*;

// ----- the second slice (ADR 0079) -------------------------------------------------------------

const CREATE_UNICODE_ENVIRONMENT: u32 = 0x400;
pub(super) const STILL_ACTIVE: u32 = 259;
const PAGE_READWRITE: u32 = 0x04;
const FILE_MAP_WRITE: u32 = 0x2;
const FILE_MAP_READ: u32 = 0x4;
const FILE_MAP_ALL_ACCESS: u32 = 0xF_001F;
const FILE_BEGIN: u32 = 0;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const ERROR_NO_UNICODE_TRANSLATION: u32 = 1113;
const ERROR_MAPPED_ALIGNMENT: u32 = 1132;
const ERROR_TIMEOUT: u32 = 1460;

/// A null pointer of any type.
pub(super) fn np<T>() -> *const T {
    core::ptr::null()
}

/// Whether `w` contains the ASCII text `a`.
pub(super) fn contains(w: &[u16], a: &[u8]) -> bool {
    w.windows(a.len()).any(|win| ascii_eq(win, a))
}

/// `s` (ASCII, may hold NULs) and a NUL into `dst`.
fn fill(dst: &mut [u16], s: &[u8]) {
    for (d, &c) in dst.iter_mut().zip(s) {
        *d = u16::from(c);
    }
    dst[s.len()] = 0;
}

/// The child `processes` starts: `--child1` checks what its creator passed
/// explicitly, `--child2` what it inherited; `--child3` and `--child4` are
/// `inheritance_and_startup`'s.
pub(super) unsafe fn child(cmd: &[u16]) -> ! {
    if contains(cmd, b"--child3") {
        // SAFETY: as below.
        unsafe { child3(cmd) }
    }
    if contains(cmd, b"--child4") {
        // SAFETY: as below.
        unsafe { child4() }
    }
    // SAFETY: valid arguments throughout.
    unsafe {
        let mut v = [0u16; 64];
        let mut cwd = [0u16; 260];
        let n = GetCurrentDirectoryW(260, cwd.as_mut_ptr()) as usize;
        if contains(cmd, b"--child1") {
            check(ascii_eq(cmd, b"\"C:\\Program Files\\PolyK32Test\\k32test.exe\" --child1 \"two words\""));
            let k = GetEnvironmentVariableW(w!("POLY_CHILD"), v.as_mut_ptr(), 64) as usize;
            check(ascii_eq(&v[..k], b"explicit"));
            // Only the block it was given.
            check(GetEnvironmentVariableW(w!("POLY_K32_INHERITED"), v.as_mut_ptr(), 64) == 0);
            check(ascii_eq(&cwd[..n], b"C:\\Windows"));
            ExitProcess(0xC0DE_0001)
        }
        check(ascii_eq(cmd, b"k32test.exe --child2"));
        let k = GetEnvironmentVariableW(w!("POLY_K32_INHERITED"), v.as_mut_ptr(), 64) as usize;
        check(ascii_eq(&v[..k], b"yes"));
        check(ascii_eq(&cwd[..n], b"C:\\Program Files\\PolyK32Test"));
        // Alive long enough for the creator to see it running.
        Sleep(300);
        ExitProcess(0xC0DE_0002)
    }
}

pub(super) unsafe fn processes() {
    // SAFETY: valid arguments throughout.
    unsafe {
        let mut si = [0u8; 104];
        si[..4].copy_from_slice(&104u32.to_le_bytes());
        let mut pi = [0u8; 24];
        let mut cmd = [0u16; 128];
        let handles = |pi: &[u8; 24]| {
            (
                isize::from_le_bytes(pi[0..8].try_into().unwrap()),
                isize::from_le_bytes(pi[8..16].try_into().unwrap()),
                u32::from_le_bytes(pi[16..20].try_into().unwrap()),
                u32::from_le_bytes(pi[20..24].try_into().unwrap()),
            )
        };
        check(SetEnvironmentVariableW(w!("POLY_K32_INHERITED"), w!("yes")) != 0);
        // An explicit environment, current directory and quoted program.
        fill(&mut cmd, b"\"C:\\Program Files\\PolyK32Test\\k32test.exe\" --child1 \"two words\"");
        let mut env = [0u16; 64];
        fill(&mut env, b"POLY_CHILD=explicit\0Path=C:\\Windows\\System32\0");
        let flags = CREATE_UNICODE_ENVIRONMENT;
        let ok = CreateProcessW(
            core::ptr::null(),
            cmd.as_mut_ptr(),
            core::ptr::null(),
            core::ptr::null(),
            0,
            flags,
            env.as_ptr().cast(),
            w!("C:\\Windows"),
            si.as_ptr(),
            pi.as_mut_ptr(),
        );
        check(ok != 0);
        let (ph, th, pid, tid) = handles(&pi);
        check(ph != 0 && th != 0 && pid != 0 && pid != GetCurrentProcessId() && tid != 0);
        check(GetProcessId(ph) == pid);
        check(WaitForSingleObject(ph, 30_000) == WAIT_OBJECT_0);
        let mut code = 0u32;
        check(GetExitCodeProcess(ph, &mut code) != 0 && code == 0xC0DE_0001);
        check(WaitForSingleObject(th, 0) == WAIT_OBJECT_0);
        check(CloseHandle(th) != 0 && CloseHandle(ph) != 0);

        // A name searched for (the application's directory first), the
        // creator's environment and current directory inherited.
        check(SetCurrentDirectoryW(w!("C:\\Program Files\\PolyK32Test")) != 0);
        fill(&mut cmd, b"k32test.exe --child2");
        let ok = CreateProcessW(
            core::ptr::null(),
            cmd.as_mut_ptr(),
            core::ptr::null(),
            core::ptr::null(),
            0,
            0,
            core::ptr::null(),
            core::ptr::null(),
            si.as_ptr(),
            pi.as_mut_ptr(),
        );
        check(ok != 0);
        let (ph, th, _, _) = handles(&pi);
        check(GetExitCodeProcess(ph, &mut code) != 0 && code == STILL_ACTIVE);
        check(WaitForSingleObject(ph, 0) == WAIT_TIMEOUT);
        check(WaitForSingleObject(ph, 30_000) == WAIT_OBJECT_0);
        check(GetExitCodeProcess(ph, &mut code) != 0 && code == 0xC0DE_0002);
        check(CloseHandle(th) != 0 && CloseHandle(ph) != 0);

        // Failures.
        fill(&mut cmd, b"no-such-program --x");
        let r = CreateProcessW(np(), cmd.as_mut_ptr(), np(), np(), 0, 0, np(), np(), si.as_ptr(), pi.as_mut_ptr());
        check(r == 0);
        check_error(ERROR_FILE_NOT_FOUND);
        let r = CreateProcessW(
            w!("C:\\no-such.exe"),
            core::ptr::null_mut(),
            np(),
            np(),
            0,
            0,
            np(),
            np(),
            si.as_ptr(),
            pi.as_mut_ptr(),
        );
        check(r == 0);
        check_error(ERROR_FILE_NOT_FOUND);
        let r = CreateProcessW(np(), core::ptr::null_mut(), np(), np(), 0, 0, np(), np(), si.as_ptr(), pi.as_mut_ptr());
        check(r == 0);
        check_error(ERROR_INVALID_PARAMETER);
    }
}

static FLS_SEEN: AtomicU32 = AtomicU32::new(0);

pub(super) extern "system" fn fls_callback(v: usize) {
    FLS_SEEN.fetch_add(v as u32, Ordering::SeqCst);
}

pub(super) extern "system" fn tls_worker(p: *mut u8) -> u32 {
    let (idx, fls) = (p as u32 & 0xFFFF, p as u32 >> 16);
    // SAFETY: kernel32 calls.
    unsafe {
        if TlsGetValue(idx) != 0 {
            return 1;
        }
        if TlsSetValue(idx, 0x99) == 0 || TlsGetValue(idx) != 0x99 {
            return 2;
        }
        if FlsGetValue(fls) != 0 || FlsSetValue(fls, 0x66) == 0 {
            return 3;
        }
    }
    0x55
}

pub(super) unsafe fn tls_and_fls() {
    // SAFETY: valid arguments throughout.
    unsafe {
        let idx = TlsAlloc();
        check(idx != u32::MAX);
        SetLastError(5);
        check(TlsGetValue(idx) == 0);
        check_error(0);
        check(TlsSetValue(idx, 0x1234) != 0 && TlsGetValue(idx) == 0x1234);
        // Past the TEB's 64 slots: the expansion slots.
        let mut held = [0u32; 80];
        let mut n = 0;
        let mut big = u32::MAX;
        while n < held.len() {
            let i = TlsAlloc();
            check(i != u32::MAX);
            held[n] = i;
            n += 1;
            if i >= 64 {
                big = i;
                break;
            }
        }
        check(big != u32::MAX);
        check(TlsGetValue(big) == 0 && TlsSetValue(big, 0xABCD) != 0 && TlsGetValue(big) == 0xABCD);
        // A thread has values of its own; its FLS callback runs when it ends.
        let fls = FlsAlloc(Some(fls_callback));
        check(fls != u32::MAX && fls < 0x1_0000);
        let arg = ((fls << 16) | idx) as usize as *mut u8;
        let t = CreateThread(core::ptr::null(), 0, tls_worker, arg, 0, core::ptr::null_mut());
        check(t != 0);
        check(WaitForSingleObject(t, 10_000) == WAIT_OBJECT_0);
        let mut code = 0u32;
        check(GetExitCodeThread(t, &mut code) != 0 && code == 0x55);
        check(CloseHandle(t) != 0);
        check(TlsGetValue(idx) == 0x1234);
        check(FLS_SEEN.load(Ordering::SeqCst) == 0x66);
        check(FlsSetValue(fls, 0x11) != 0 && FlsGetValue(fls) == 0x11);
        // Freeing an FLS index hands each value to the callback.
        check(FlsFree(fls) != 0);
        check(FLS_SEEN.load(Ordering::SeqCst) == 0x77);
        check(FlsGetValue(fls) == 0);
        check_error(ERROR_INVALID_PARAMETER);
        check(FlsFree(fls) == 0);
        // Freed indices are reused, their slots cleared.
        for &i in &held[..n] {
            check(TlsFree(i) != 0);
        }
        check(TlsFree(big) == 0);
        check_error(ERROR_INVALID_PARAMETER);
        let mut again = [0u32; 80];
        let mut found = false;
        for slot in again.iter_mut().take(n) {
            *slot = TlsAlloc();
            if *slot == big {
                found = true;
            }
        }
        check(found && TlsGetValue(big) == 0);
        for &i in &again[..n] {
            check(TlsFree(i) != 0);
        }
        check(TlsGetValue(5000) == 0);
        check_error(ERROR_INVALID_PARAMETER);
        check(TlsFree(idx) != 0);
    }
}

pub(super) unsafe fn code_pages_and_strings() {
    // SAFETY: valid arguments throughout.
    unsafe {
        check(GetACP() == 1252 && GetOEMCP() == 437);
        check(IsValidCodePage(65001) != 0 && IsValidCodePage(437) != 0 && IsValidCodePage(12345) == 0);
        let mut info = [0u8; 20];
        check(GetCPInfo(65001, info.as_mut_ptr()) != 0 && info[0] == 4 && info[4] == b'?');
        check(GetCPInfo(0, info.as_mut_ptr()) != 0 && info[0] == 1);
        let utf8 = b"h\xC3\xA9\xE2\x82\xAC\0";
        let null16 = core::ptr::null_mut();
        check(MultiByteToWideChar(65001, 0, utf8.as_ptr(), -1, null16, 0) == 4);
        let mut w = [0u16; 8];
        check(MultiByteToWideChar(65001, 0, utf8.as_ptr(), -1, w.as_mut_ptr(), 8) == 4);
        check(w[..4] == [0x68, 0xE9, 0x20AC, 0]);
        check(MultiByteToWideChar(65001, 0, utf8.as_ptr(), -1, w.as_mut_ptr(), 2) == 0);
        check_error(ERROR_INSUFFICIENT_BUFFER);
        check(MultiByteToWideChar(65001, 8, b"\xC0\x80".as_ptr(), 2, w.as_mut_ptr(), 8) == 0);
        check_error(ERROR_NO_UNICODE_TRANSLATION);
        check(MultiByteToWideChar(65001, 0, b"\xC0\x80".as_ptr(), 2, w.as_mut_ptr(), 8) == 2 && w[0] == 0xFFFD);
        let src = [0x68u16, 0xE9, 0x20AC, 0];
        let mut b = [0u8; 16];
        let none = core::ptr::null();
        let no_used = core::ptr::null_mut();
        check(WideCharToMultiByte(65001, 0, src.as_ptr(), -1, core::ptr::null_mut(), 0, none, no_used) == 7);
        check(WideCharToMultiByte(65001, 0, src.as_ptr(), -1, b.as_mut_ptr(), 16, none, no_used) == 7);
        check(&b[..7] == utf8);
        let mut used = 0;
        let cjk = [0x20ACu16, 0x4E2D];
        check(WideCharToMultiByte(1252, 0, cjk.as_ptr(), 2, b.as_mut_ptr(), 16, none, &mut used) == 2);
        check(b[0] == 0x80 && b[1] == b'?' && used == 1);
        check(MultiByteToWideChar(437, 0, [0x82u8].as_ptr(), 1, w.as_mut_ptr(), 8) == 1 && w[0] == 0xE9);
        check(CompareStringOrdinal(w!("abc"), -1, w!("ABC"), -1, 1) == 2);
        check(CompareStringOrdinal(w!("abc"), -1, w!("ABC"), -1, 0) == 3);
        check(CompareStringOrdinal(w!("abc"), 2, w!("abc"), 3, 0) == 1);
        check(lstrlenW(w!("hello")) == 5 && lstrlenW(core::ptr::null()) == 0);
        check(lstrcmpiW(w!("Hello"), w!("hELLO")) == 0 && lstrcmpW(w!("a"), w!("b")) < 0);
    }
}

pub(super) unsafe fn file_mappings() {
    // SAFETY: valid arguments throughout; views are used inside their size.
    unsafe {
        let rw = GENERIC_READ | GENERIC_WRITE;
        let h = CreateFileW(w!("C:\\k32map.bin"), rw, 0, core::ptr::null(), CREATE_ALWAYS, 0, 0);
        check(h != INVALID_HANDLE_VALUE);
        let mut n = 0u32;
        check(WriteFile(h, b"mapped!".as_ptr(), 7, &mut n, core::ptr::null_mut()) != 0);
        // A writable mapping larger than the file grows it.
        let m = CreateFileMappingW(h, core::ptr::null(), PAGE_READWRITE, 0, 0x2000, core::ptr::null());
        check(m != 0);
        check(GetFileSize(h, core::ptr::null_mut()) == 0x2000);
        let p = MapViewOfFile(m, FILE_MAP_ALL_ACCESS, 0, 0, 0);
        check(!p.is_null());
        check(core::slice::from_raw_parts(p, 7) == b"mapped!");
        p.write(b'M');
        p.add(0x1000).write(b'Z');
        check(FlushViewOfFile(p, 0) != 0);
        // A second view shows the same pages.
        let q = MapViewOfFile(m, FILE_MAP_READ, 0, 0, 0x1000);
        check(!q.is_null() && q != p && *q == b'M');
        check(UnmapViewOfFile(q) != 0 && UnmapViewOfFile(p) != 0);
        check(UnmapViewOfFile(p) == 0);
        // The file holds what the view wrote.
        check(SetFilePointer(h, 0x1000, core::ptr::null_mut(), FILE_BEGIN) == 0x1000);
        let mut c = [0u8; 1];
        check(ReadFile(h, c.as_mut_ptr(), 1, &mut n, core::ptr::null_mut()) != 0 && n == 1 && c[0] == b'Z');
        check(MapViewOfFile(m, FILE_MAP_READ, 0, 0x1000, 0).is_null());
        check_error(ERROR_MAPPED_ALIGNMENT);
        check(CloseHandle(m) != 0);
        // A named mapping of the paging file, opened again by name.
        let a = CreateFileMappingW(
            INVALID_HANDLE_VALUE,
            core::ptr::null(),
            PAGE_READWRITE,
            0,
            0x1000,
            w!("Local\\PolyK32Map"),
        );
        check(a != 0);
        check_error(0);
        let b = OpenFileMappingW(FILE_MAP_ALL_ACCESS, 0, w!("PolyK32Map"));
        check(b != 0);
        let again =
            CreateFileMappingW(INVALID_HANDLE_VALUE, core::ptr::null(), PAGE_READWRITE, 0, 0x1000, w!("PolyK32Map"));
        check(again != 0);
        check_error(ERROR_ALREADY_EXISTS);
        let pa = MapViewOfFile(a, FILE_MAP_WRITE, 0, 0, 0);
        let pb = MapViewOfFileEx(b, FILE_MAP_READ, 0, 0, 0, core::ptr::null_mut());
        check(!pa.is_null() && !pb.is_null() && pa != pb);
        pa.add(10).write(42);
        check(*pb.add(10) == 42);
        check(UnmapViewOfFile(pa) != 0 && UnmapViewOfFile(pb) != 0);
        check(CloseHandle(a) != 0 && CloseHandle(b) != 0 && CloseHandle(again) != 0);
        check(
            CreateFileMappingW(INVALID_HANDLE_VALUE, core::ptr::null(), PAGE_READWRITE, 0, 0, core::ptr::null()) == 0,
        );
        check_error(ERROR_INVALID_PARAMETER);
        check(CloseHandle(h) != 0);
        check(DeleteFileW(w!("C:\\k32map.bin")) != 0);
    }
}

/// Shared with the condition-variable worker.
struct Cond {
    lock: usize,
    cv: usize,
    ready: u32,
    cs: [u8; 40],
    cv2: usize,
    ready2: u32,
}

static mut COND: Cond = Cond { lock: 0, cv: 0, ready: 0, cs: [0; 40], cv2: 0, ready2: 0 };

pub(super) extern "system" fn cond_worker(p: *mut u8) -> u32 {
    let c = p as *mut Cond;
    // SAFETY: the static, guarded by its SRW lock and critical section.
    unsafe {
        Sleep(20);
        AcquireSRWLockExclusive(&raw mut (*c).lock);
        (*c).ready = 1;
        WakeAllConditionVariable(&raw mut (*c).cv);
        ReleaseSRWLockExclusive(&raw mut (*c).lock);
        EnterCriticalSection((&raw mut (*c).cs).cast());
        (*c).ready2 = 1;
        WakeConditionVariable(&raw mut (*c).cv2);
        LeaveCriticalSection((&raw mut (*c).cs).cast());
    }
    0
}

static INIT_RUNS: AtomicU32 = AtomicU32::new(0);

pub(super) extern "system" fn init_fn(_once: *mut usize, param: *mut u8, ctx: *mut *mut u8) -> i32 {
    INIT_RUNS.fetch_add(1, Ordering::SeqCst);
    if param as usize == 1 {
        return 0;
    }
    // SAFETY: the out-pointer InitOnceExecuteOnce passes.
    unsafe { ctx.write(0x1000 as *mut u8) };
    1
}

pub(super) unsafe fn conditions_and_init_once() {
    // SAFETY: valid arguments throughout.
    unsafe {
        let c = &raw mut COND;
        InitializeConditionVariable(&raw mut (*c).cv);
        InitializeConditionVariable(&raw mut (*c).cv2);
        InitializeSRWLock(&raw mut (*c).lock);
        InitializeCriticalSection((&raw mut (*c).cs).cast());
        AcquireSRWLockExclusive(&raw mut (*c).lock);
        // Nobody wakes it: a timeout, the lock held again.
        check(SleepConditionVariableSRW(&raw mut (*c).cv, &raw mut (*c).lock, 20, 0) == 0);
        check_error(ERROR_TIMEOUT);
        check(TryAcquireSRWLockExclusive(&raw mut (*c).lock) == 0);
        let t = CreateThread(core::ptr::null(), 0, cond_worker, c.cast(), 0, core::ptr::null_mut());
        check(t != 0);
        while (*c).ready == 0 {
            check(SleepConditionVariableSRW(&raw mut (*c).cv, &raw mut (*c).lock, 10_000, 0) != 0);
        }
        ReleaseSRWLockExclusive(&raw mut (*c).lock);
        EnterCriticalSection((&raw mut (*c).cs).cast());
        while (*c).ready2 == 0 {
            check(SleepConditionVariableCS(&raw mut (*c).cv2, (&raw mut (*c).cs).cast(), 10_000) != 0);
        }
        LeaveCriticalSection((&raw mut (*c).cs).cast());
        check(WaitForSingleObject(t, 10_000) == WAIT_OBJECT_0 && CloseHandle(t) != 0);
        DeleteCriticalSection((&raw mut (*c).cs).cast());

        // One-time initialisation: a failed run lets the next one try.
        static mut ONCE: usize = 0;
        let mut ctx: *mut u8 = core::ptr::null_mut();
        check(InitOnceExecuteOnce(&raw mut ONCE, init_fn, core::ptr::without_provenance_mut(1), &mut ctx) == 0);
        check(InitOnceExecuteOnce(&raw mut ONCE, init_fn, core::ptr::null_mut(), &mut ctx) != 0);
        check(ctx as usize == 0x1000);
        ctx = core::ptr::null_mut();
        check(InitOnceExecuteOnce(&raw mut ONCE, init_fn, core::ptr::null_mut(), &mut ctx) != 0);
        check(ctx as usize == 0x1000 && INIT_RUNS.load(Ordering::SeqCst) == 2);
    }
}

pub(super) unsafe fn file_information_and_times() {
    // SAFETY: valid arguments throughout.
    unsafe {
        let rw = GENERIC_READ | GENERIC_WRITE;
        let h = CreateFileW(w!("C:\\k32info.txt"), rw, 0, core::ptr::null(), CREATE_ALWAYS, 0, 0);
        check(h != INVALID_HANDLE_VALUE);
        let mut n = 0u32;
        check(WriteFile(h, b"12345".as_ptr(), 5, &mut n, core::ptr::null_mut()) != 0);
        let mut info = [0u8; 52];
        check(GetFileInformationByHandle(h, info.as_mut_ptr()) != 0);
        let d = |o: usize| u32::from_le_bytes(info[o..o + 4].try_into().unwrap());
        check(d(0) & FILE_ATTRIBUTE_DIRECTORY == 0);
        check(d(32) == 0 && d(36) == 5 && d(40) >= 1);
        check(d(44) | d(48) != 0);
        // 2001-02-03 04:05:06, a Saturday.
        let st: [u16; 8] = [2001, 2, 0, 3, 4, 5, 6, 0];
        let mut ft = 0u64;
        check(SystemTimeToFileTime(st.as_ptr(), &mut ft) != 0);
        check(SetFileTime(h, core::ptr::null(), core::ptr::null(), &ft) != 0);
        let (mut c, mut a, mut wr) = (0u64, 0u64, 0u64);
        check(GetFileTime(h, &mut c, &mut a, &mut wr) != 0);
        check(wr == ft && c != 0 && a != 0);
        let mut back = [0u16; 8];
        check(FileTimeToSystemTime(&wr, back.as_mut_ptr()) != 0);
        check(back == [2001, 2, 6, 3, 4, 5, 6, 0]);
        let later = ft + 1;
        check(CompareFileTime(&ft, &wr) == 0 && CompareFileTime(&ft, &later) == -1);
        let bad: [u16; 8] = [2001, 2, 0, 30, 0, 0, 0, 0];
        check(SystemTimeToFileTime(bad.as_ptr(), &mut ft) == 0);
        check_error(ERROR_INVALID_PARAMETER);
        let mut now = [0u16; 8];
        GetSystemTime(now.as_mut_ptr());
        check(now[0] >= 1601 && (1..=12).contains(&now[1]) && now[2] < 7);
        check(CloseHandle(h) != 0);
        check(DeleteFileW(w!("C:\\k32info.txt")) != 0);
    }
}

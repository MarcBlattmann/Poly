//! The first slice's checks: errors, environment, files, memory, threads and synchronisation, system and modules.

use super::*;

// ----- the thread ----------------------------------------------------------------------

/// Shared with the worker thread.
pub(super) struct Shared {
    go: Handle,
    done: Handle,
    cs: [u8; 40],
    srw: usize,
    counter: u32,
    seen_tid: u32,
}

pub(super) static mut SHARED: Shared = Shared { go: 0, done: 0, cs: [0; 40], srw: 0, counter: 0, seen_tid: 0 };

pub(super) extern "system" fn worker(p: *mut u8) -> u32 {
    let s = p as *mut Shared;
    // SAFETY: `s` is the static; the critical section and the SRW lock
    // serialise the counter.
    unsafe {
        if WaitForSingleObject((*s).go, 10_000) != WAIT_OBJECT_0 {
            return 1;
        }
        (*s).seen_tid = GetCurrentThreadId();
        for _ in 0..1000 {
            EnterCriticalSection((&raw mut (*s).cs).cast());
            (*s).counter += 1;
            LeaveCriticalSection((&raw mut (*s).cs).cast());
        }
        AcquireSRWLockExclusive(&raw mut (*s).srw);
        (*s).counter += 1;
        ReleaseSRWLockExclusive(&raw mut (*s).srw);
        SetEvent((*s).done);
    }
    0x77
}

pub(super) unsafe fn errors_and_environment() {
    // SAFETY: valid arguments throughout.
    unsafe {
        SetLastError(1234);
        check(GetLastError() == 1234);
        // A failing call sets the documented error.
        let h = CreateFileW(w!("C:\\no-such-file.k32"), GENERIC_READ, 0, core::ptr::null(), OPEN_EXISTING, 0, 0);
        check(h == INVALID_HANDLE_VALUE);
        check_error(ERROR_FILE_NOT_FOUND);
        let h = CreateFileW(w!("C:\\no-such-dir\\f.k32"), GENERIC_READ, 0, core::ptr::null(), OPEN_EXISTING, 0, 0);
        check(h == INVALID_HANDLE_VALUE);
        check_error(ERROR_PATH_NOT_FOUND);
        let h = CreateFileW(w!("C:\\x.k32"), GENERIC_READ, 0, core::ptr::null(), 9, 0, 0);
        check(h == INVALID_HANDLE_VALUE);
        check_error(ERROR_INVALID_PARAMETER);

        // The environment the kernel gave the process (ADR 0073).
        let mut buf = [0u16; 260];
        let n = GetEnvironmentVariableW(w!("temp"), buf.as_mut_ptr(), 260);
        check(n > 3 && (n as usize) < 260);
        check(GetEnvironmentVariableW(w!("TEMP"), buf.as_mut_ptr(), 2) == n + 1);
        check(GetEnvironmentVariableW(w!("POLY_K32"), buf.as_mut_ptr(), 260) == 0);
        check_error(ERROR_ENVVAR_NOT_FOUND);
        check(SetEnvironmentVariableW(w!("POLY_K32"), w!("yes")) != 0);
        check(GetEnvironmentVariableW(w!("poly_k32"), buf.as_mut_ptr(), 260) == 3);
        check(ascii_eq(&buf[..3], b"yes"));
        // The block holds it too.
        let block = GetEnvironmentStringsW();
        check(!block.is_null());
        let mut p = block as *const u16;
        let mut found = false;
        while *p != 0 {
            let e = wslice(p);
            found |= ascii_eq(e, b"POLY_K32=yes");
            p = p.add(e.len() + 1);
        }
        check(found);
        check(FreeEnvironmentStringsW(block) != 0);
        check(SetEnvironmentVariableW(w!("POLY_K32"), core::ptr::null()) != 0);
        check(GetEnvironmentVariableW(w!("POLY_K32"), buf.as_mut_ptr(), 260) == 0);
        check(SetEnvironmentVariableW(w!("A=B"), w!("x")) == 0);
        check_error(ERROR_INVALID_PARAMETER);

        // The command line: the program's path, then the argument.
        let cl = wslice(GetCommandLineW());
        check(ends_with_ignore_case(cl, b" --poly"));
        let cla = GetCommandLineA();
        check(*cla != 0);
    }
}

pub(super) unsafe fn files() {
    // SAFETY: valid arguments throughout.
    unsafe {
        // Work in the temporary directory.
        let mut temp = [0u16; 260];
        let n = GetTempPathW(260, temp.as_mut_ptr()) as usize;
        check(n > 3 && temp[n - 1] == u16::from(b'\\'));
        check(SetCurrentDirectoryW(temp.as_ptr()) != 0);
        let mut cur = [0u16; 260];
        let c = GetCurrentDirectoryW(260, cur.as_mut_ptr()) as usize;
        check(c == n - 1 && cur[..c] == temp[..c]);
        check(GetCurrentDirectoryW(1, cur.as_mut_ptr()) as usize == c + 1);
        check(SetCurrentDirectoryW(w!("C:\\no-such-dir")) == 0);

        // A directory, twice.
        let _ = DeleteFileW(w!("k32dir\\a.txt"));
        let _ = DeleteFileW(w!("k32dir\\b.txt"));
        let _ = RemoveDirectoryW(w!("k32dir"));
        check(CreateDirectoryW(w!("k32dir"), core::ptr::null()) != 0);
        check(CreateDirectoryW(w!("K32DIR"), core::ptr::null()) == 0);
        check_error(ERROR_ALREADY_EXISTS);
        check(GetFileAttributesW(w!("k32dir")) & FILE_ATTRIBUTE_DIRECTORY != 0);

        // A file: create, write, size, seek, read, EOF.
        let h =
            CreateFileW(w!("k32dir\\a.txt"), GENERIC_READ | GENERIC_WRITE, 0, core::ptr::null(), CREATE_NEW, 0x80, 0);
        check(h != INVALID_HANDLE_VALUE);
        check_error(0);
        let text = b"hello, kernel32";
        let mut done = 0u32;
        check(WriteFile(h, text.as_ptr(), text.len() as u32, &mut done, core::ptr::null_mut()) != 0);
        check(done == text.len() as u32);
        let mut high = 7u32;
        check(GetFileSize(h, &mut high) == text.len() as u32 && high == 0);
        let mut size = 0i64;
        check(GetFileSizeEx(h, &mut size) != 0 && size == text.len() as i64);
        // A success other than INVALID_SET_FILE_POINTER leaves the last
        // error alone (ADR 0114).
        SetLastError(87);
        check(SetFilePointer(h, 0, core::ptr::null_mut(), 0) == 0);
        check_error(87);
        let mut back = [0u8; 32];
        check(ReadFile(h, back.as_mut_ptr(), 32, &mut done, core::ptr::null_mut()) != 0);
        check(done == text.len() as u32 && &back[..text.len()] == text);
        // At the end: success with no bytes.
        check(ReadFile(h, back.as_mut_ptr(), 32, &mut done, core::ptr::null_mut()) != 0 && done == 0);
        let mut pos = 0i64;
        check(SetFilePointerEx(h, -5, &mut pos, 1) != 0 && pos == 10);
        check(SetFilePointer(h, -100, core::ptr::null_mut(), 1) == u32::MAX);
        check_error(ERROR_NEGATIVE_SEEK);
        check(CloseHandle(h) != 0);
        check(CloseHandle(h) == 0);

        // CREATE_NEW on an existing file; CREATE_ALWAYS reports it existed.
        let h = CreateFileW(w!("k32dir\\a.txt"), GENERIC_WRITE, 0, core::ptr::null(), CREATE_NEW, 0, 0);
        check(h == INVALID_HANDLE_VALUE);
        check_error(ERROR_FILE_EXISTS);
        let h = CreateFileA(c"k32dir\\A.TXT".as_ptr().cast(), GENERIC_READ, 1, core::ptr::null(), OPEN_EXISTING, 0, 0);
        check(h != INVALID_HANDLE_VALUE);
        check(GetFileSize(h, core::ptr::null_mut()) == text.len() as u32);
        check(CloseHandle(h) != 0);
        let h = CreateFileW(w!("k32dir\\b.txt"), GENERIC_WRITE, 0, core::ptr::null(), CREATE_ALWAYS, 0, 0);
        check(h != INVALID_HANDLE_VALUE);
        check_error(0);
        check(CloseHandle(h) != 0);
        let h = CreateFileW(w!("k32dir\\b.txt"), GENERIC_WRITE, 0, core::ptr::null(), CREATE_ALWAYS, 0, 0);
        check(h != INVALID_HANDLE_VALUE);
        check_error(ERROR_ALREADY_EXISTS);
        check(CloseHandle(h) != 0);

        // Finding files.
        let mut data = [0u8; 592];
        SetLastError(87);
        let f = FindFirstFileW(w!("k32dir\\a.*"), data.as_mut_ptr());
        check(f != INVALID_HANDLE_VALUE);
        check_error(87);
        let name = wslice(data.as_ptr().add(44).cast());
        check(ascii_eq(name, b"a.txt"));
        check(u32::from_le_bytes(data[32..36].try_into().unwrap()) == text.len() as u32);
        check(FindNextFileW(f, data.as_mut_ptr()) == 0);
        check_error(ERROR_NO_MORE_FILES);
        check(FindClose(f) != 0);
        let f = FindFirstFileW(w!("k32dir\\*.*"), data.as_mut_ptr());
        check(f != INVALID_HANDLE_VALUE);
        let mut names = 1;
        while FindNextFileW(f, data.as_mut_ptr()) != 0 {
            names += 1;
        }
        // a.txt and b.txt (and "." and ".." where the volume reports them).
        check((2..=4).contains(&names));
        check(FindClose(f) != 0);
        check(FindFirstFileW(w!("k32dir\\*.none"), data.as_mut_ptr()) == INVALID_HANDLE_VALUE);
        check_error(ERROR_FILE_NOT_FOUND);
        // "k32dir\." is the directory itself, by its own name (ADR 0114).
        let f = FindFirstFileW(w!("k32dir\\."), data.as_mut_ptr());
        check(f != INVALID_HANDLE_VALUE);
        check(ascii_eq(wslice(data.as_ptr().add(44).cast()), b"k32dir"));
        check(FindClose(f) != 0);

        // Full paths.
        let mut full = [0u16; 260];
        let mut part: *mut u16 = core::ptr::null_mut();
        let len = GetFullPathNameW(w!("k32dir\\..\\k32dir\\.\\a.txt"), 260, full.as_mut_ptr(), &mut part) as usize;
        check(len == c + "\\k32dir\\a.txt".len());
        check(full[..c] == cur[..c] && ascii_eq(&full[c..len], b"\\k32dir\\a.txt"));
        check(!part.is_null() && ascii_eq(wslice(part), b"a.txt"));

        // Rename, delete, remove.
        check(MoveFileExW(w!("k32dir\\a.txt"), w!("k32dir\\b.txt"), 0) == 0);
        check_error(ERROR_ALREADY_EXISTS);
        check(MoveFileExW(w!("k32dir\\a.txt"), w!("k32dir\\b.txt"), 1) != 0);
        check(GetFileAttributesW(w!("k32dir\\a.txt")) == INVALID_FILE_ATTRIBUTES);
        check_error(ERROR_FILE_NOT_FOUND);
        check(GetFileAttributesW(w!("k32dir\\b.txt")) != INVALID_FILE_ATTRIBUTES);
        check(RemoveDirectoryW(w!("k32dir")) == 0);
        check_error(ERROR_DIR_NOT_EMPTY);
        check(DeleteFileW(w!("k32dir")) == 0);
        check(DeleteFileW(w!("k32dir\\b.txt")) != 0);
        check(DeleteFileW(w!("k32dir\\b.txt")) == 0);
        check_error(ERROR_FILE_NOT_FOUND);
        check(RemoveDirectoryW(w!("k32dir")) != 0);
        check(GetFileAttributesW(w!("k32dir")) == INVALID_FILE_ATTRIBUTES);
    }
}

pub(super) unsafe fn memory() {
    // SAFETY: valid arguments throughout.
    unsafe {
        let heap = GetProcessHeap();
        check(!heap.is_null());
        let p = HeapAlloc(heap, 0x8, 100);
        check(!p.is_null() && (p as usize).is_multiple_of(16));
        check((0..100).all(|i| *p.add(i) == 0));
        check(HeapSize(heap, 0, p) == 100);
        p.write_bytes(0xAB, 100);
        let q = HeapReAlloc(heap, 0, p, 300);
        check(!q.is_null() && *q.add(99) == 0xAB && HeapSize(heap, 0, q) == 300);
        check(HeapFree(heap, 0, q) != 0);

        let private = HeapCreate(0, 0, 0);
        check(!private.is_null());
        let r = HeapAlloc(private, 0, 5000);
        check(!r.is_null());
        check(HeapFree(private, 0, r) != 0);
        check(HeapDestroy(private) != 0);

        let l = LocalAlloc(0x40, 64);
        check(!l.is_null() && *l.add(63) == 0);
        check(LocalFree(l).is_null());
        check(LocalAlloc(0x1, 8).is_null());
        check_error(ERROR_INVALID_PARAMETER);
        let g = GlobalAlloc(0x42, 32);
        check(!g.is_null() && *g == 0);
        check(GlobalFree(g).is_null());

        // Virtual memory: commit, write, query, protect, release.
        let v = VirtualAlloc(core::ptr::null_mut(), 0x10000, 0x3000, 0x04);
        check(!v.is_null() && (v as usize).is_multiple_of(0x10000));
        v.add(0x1000).write(42);
        let mut mbi = [0u8; 48];
        check(VirtualQuery(v, mbi.as_mut_ptr(), 48) == 48);
        // MEMORY_BASIC_INFORMATION: State at 32 (MEM_COMMIT), Protect at 36.
        check(u32::from_le_bytes(mbi[32..36].try_into().unwrap()) == 0x1000);
        check(u32::from_le_bytes(mbi[36..40].try_into().unwrap()) == 0x04);
        let mut old = 0u32;
        check(VirtualProtect(v, 0x1000, 0x02, &mut old) != 0 && old == 0x04);
        check(VirtualFree(v, 0x10000, 0x8000) == 0);
        check_error(ERROR_INVALID_PARAMETER);
        check(VirtualFree(v, 0, 0x8000) != 0);
    }
}

pub(super) unsafe fn threads_and_sync() {
    // SAFETY: valid arguments throughout; the shared state is a static the
    // worker only touches under the locks or after the events.
    unsafe {
        let s = &raw mut SHARED;
        (*s).go = CreateEventW(core::ptr::null(), 0, 0, core::ptr::null());
        (*s).done = CreateEventW(core::ptr::null(), 1, 0, core::ptr::null());
        check((*s).go != 0 && (*s).done != 0);
        InitializeCriticalSection((&raw mut (*s).cs).cast());
        InitializeSRWLock(&raw mut (*s).srw);
        let mut tid = 0u32;
        let t = CreateThread(core::ptr::null(), 0, worker, s.cast(), 0, &mut tid);
        check(t != 0 && tid != 0 && tid != GetCurrentThreadId());
        let mut code = 0u32;
        check(GetExitCodeThread(t, &mut code) != 0 && code == 259);
        // The worker waits for `go`.
        check(WaitForSingleObject((*s).done, 20) == WAIT_TIMEOUT);
        for _ in 0..500 {
            EnterCriticalSection((&raw mut (*s).cs).cast());
            (*s).counter += 1;
            LeaveCriticalSection((&raw mut (*s).cs).cast());
        }
        check(SetEvent((*s).go) != 0);
        check(WaitForSingleObject((*s).done, 10_000) == WAIT_OBJECT_0);
        check(WaitForSingleObject(t, 10_000) == WAIT_OBJECT_0);
        check(GetExitCodeThread(t, &mut code) != 0 && code == 0x77);
        check((*s).seen_tid == tid);
        check((*s).counter == 1501);
        // `go` was auto-reset by the worker's wait; `done` stays set.
        check(WaitForSingleObject((*s).go, 0) == WAIT_TIMEOUT);
        let both = [(*s).go, (*s).done];
        check(WaitForMultipleObjects(2, both.as_ptr(), 0, 0) == 1);
        check(WaitForMultipleObjects(2, both.as_ptr(), 1, 0) == WAIT_TIMEOUT);
        check(ResetEvent((*s).done) != 0);
        check(WaitForSingleObject((*s).done, 0) == WAIT_TIMEOUT);
        check(WaitForMultipleObjects(0, both.as_ptr(), 0, 0) == u32::MAX);
        check_error(ERROR_INVALID_PARAMETER);
        DeleteCriticalSection((&raw mut (*s).cs).cast());
        check(TryAcquireSRWLockExclusive(&raw mut (*s).srw) != 0);
        ReleaseSRWLockExclusive(&raw mut (*s).srw);
        AcquireSRWLockShared(&raw mut (*s).srw);
        check(TryAcquireSRWLockExclusive(&raw mut (*s).srw) == 0);
        ReleaseSRWLockShared(&raw mut (*s).srw);
        for h in [t, (*s).go, (*s).done] {
            check(CloseHandle(h) != 0);
        }

        // Mutexes: owned at creation, released once.
        let m = CreateMutexW(core::ptr::null(), 1, core::ptr::null());
        check(m != 0);
        check(ReleaseMutex(m) != 0);
        check(ReleaseMutex(m) == 0);
        check_error(ERROR_NOT_OWNER);
        check(WaitForSingleObject(m, 0) == WAIT_OBJECT_0);
        check(ReleaseMutex(m) != 0);
        check(CloseHandle(m) != 0);

        // Semaphores.
        let sem = CreateSemaphoreW(core::ptr::null(), 1, 2, core::ptr::null());
        check(sem != 0);
        check(WaitForSingleObject(sem, 0) == WAIT_OBJECT_0);
        check(WaitForSingleObject(sem, 0) == WAIT_TIMEOUT);
        let mut prev = -1;
        check(ReleaseSemaphore(sem, 2, &mut prev) != 0 && prev == 0);
        check(ReleaseSemaphore(sem, 1, core::ptr::null_mut()) == 0);
        check_error(ERROR_TOO_MANY_POSTS);
        check(CloseHandle(sem) != 0);
        check(CreateSemaphoreW(core::ptr::null(), 3, 2, core::ptr::null()) == 0);
        check_error(ERROR_INVALID_PARAMETER);

        // A named event: the second create opens the first.
        let a = CreateEventW(core::ptr::null(), 1, 0, w!("Local\\PolyK32Event"));
        check(a != 0);
        check_error(0);
        let b = CreateEventW(core::ptr::null(), 1, 0, w!("PolyK32Event"));
        check(b != 0);
        check_error(ERROR_ALREADY_EXISTS);
        check(SetEvent(a) != 0);
        check(WaitForSingleObject(b, 0) == WAIT_OBJECT_0);
        check(CloseHandle(a) != 0 && CloseHandle(b) != 0);
    }
}

pub(super) unsafe fn system_and_modules() {
    // SAFETY: valid arguments throughout.
    unsafe {
        check(GetCurrentProcess() == -1 && GetCurrentThread() == -2);
        check(GetCurrentProcessId() != 0 && GetCurrentThreadId() != 0);

        // Time.
        let mut freq = 0i64;
        check(QueryPerformanceFrequency(&mut freq) != 0 && freq == 10_000_000);
        let (mut c1, mut c2) = (0i64, 0i64);
        check(QueryPerformanceCounter(&mut c1) != 0);
        let t1 = GetTickCount64();
        Sleep(30);
        check(QueryPerformanceCounter(&mut c2) != 0);
        let t2 = GetTickCount64();
        check(t2 >= t1 + 20 && c2 - c1 >= 200_000);
        check(GetTickCount().wrapping_sub(t2 as u32) < 1000);
        let mut ft = 0u64;
        GetSystemTimeAsFileTime(&mut ft);
        check(ft != 0);

        // The version: this program has no manifest, so it is told
        // Windows 8 (6.2), as Learn documents for unmanifested programs.
        let v = GetVersion();
        check(v & 0xFF == 6 && (v >> 8) & 0xFF == 2 && v >> 16 == 9200);
        let mut info = [0u8; 284];
        info[..4].copy_from_slice(&284u32.to_le_bytes());
        check(GetVersionExW(info.as_mut_ptr()) != 0);
        check(u32::from_le_bytes(info[4..8].try_into().unwrap()) == 6);
        check(u32::from_le_bytes(info[16..20].try_into().unwrap()) == 2);
        info[..4].copy_from_slice(&100u32.to_le_bytes());
        check(GetVersionExW(info.as_mut_ptr()) == 0);
        let mut si = [0u8; 48];
        GetSystemInfo(si.as_mut_ptr());
        check(u32::from_le_bytes(si[4..8].try_into().unwrap()) == 4096);
        check(u32::from_le_bytes(si[32..36].try_into().unwrap()) >= 1);
        let cpus = u32::from_le_bytes(si[32..36].try_into().unwrap());
        check(GetActiveProcessorGroupCount() == 1);
        check(GetActiveProcessorCount(0xFFFF) == cpus && GetActiveProcessorCount(0) == cpus);
        check(GetActiveProcessorCount(1) == 0);
        check_error(ERROR_INVALID_PARAMETER);
        check(SetDefaultDllDirectories(0x1000) != 0);
        check(SetDefaultDllDirectories(0) == 0);
        check_error(ERROR_INVALID_PARAMETER);
        check(IsDebuggerPresent() == 0);

        // Messages.
        let mut msg = [0u16; 128];
        let n = FormatMessageW(0x1200, core::ptr::null(), 2, 0, msg.as_mut_ptr(), 128, core::ptr::null()) as usize;
        check(ascii_eq(&msg[..n], b"The system cannot find the file specified.\r\n"));
        check(FormatMessageW(0x1200, core::ptr::null(), 2, 0, msg.as_mut_ptr(), 8, core::ptr::null()) == 0);
        check_error(122);
        messages();

        // Modules: this program, kernel32 (with its forwarders) and
        // kernelbase.
        let me = GetModuleHandleW(core::ptr::null());
        check(!me.is_null() && *me == b'M');
        let mut path = [0u16; 260];
        let n = GetModuleFileNameW(core::ptr::null_mut(), path.as_mut_ptr(), 260) as usize;
        check(ends_with_ignore_case(&path[..n], b"\\PolyK32Test\\k32test.exe"));
        check(GetModuleFileNameW(core::ptr::null_mut(), path.as_mut_ptr(), 4) == 4 && path[3] == 0);
        let k32 = GetModuleHandleW(w!("KERNEL32.DLL"));
        check(!k32.is_null());
        let kb = LoadLibraryW(w!("kernelbase"));
        check(!kb.is_null() && kb != k32);
        // A forwarded export resolves to kernelbase's code.
        let via_k32 = GetProcAddress(k32, c"GetTickCount".as_ptr().cast());
        let direct = GetProcAddress(kb, c"GetTickCount".as_ptr().cast());
        check(!via_k32.is_null() && via_k32 == direct);
        // kernel32's own export stays in kernel32.
        check(!GetProcAddress(k32, c"GlobalAlloc".as_ptr().cast()).is_null());
        check(GetProcAddress(k32, c"NoSuchFunction".as_ptr().cast()).is_null());
        check_error(ERROR_PROC_NOT_FOUND);
        check(FreeLibrary(kb) != 0);
        check(LoadLibraryW(w!("no-such-module.dll")).is_null());
        check_error(ERROR_MOD_NOT_FOUND);
        check(GetModuleHandleW(w!("no-such-module.dll")).is_null());
        check_error(ERROR_MOD_NOT_FOUND);
    }
}

//! `k32test.exe --vm-stress` (ADR 0262): memory calls from many threads of many processes at once,
//! and calls on other processes' memory while those processes make their own. Each process's own
//! memory calls run under its own lock; this checks that they stay right when they really run side
//! by side, and that the calls on another process (`VirtualAllocEx`, `WriteProcessMemory`, ...)
//! take its lock safely. The kernel test `nt_memory_calls_of_many_threads_and_processes_at_once`
//! runs it.
//!
//! The parent starts [`CHILDREN`] copies of itself with `--vm-stress-child`. Every process runs
//! [`WORKERS`] threads, each [`ROUNDS`] times: commit, write, protect, query, read back, decommit
//! a page, release; and map a pagefile section twice, write through one view and read through the
//! other, unmap. The parent meanwhile has one thread per child allocating, writing, reading,
//! protecting, querying and freeing memory in that child. The children wait for the parent's
//! event before they end, so they are alive for all of it.
//!
//! Every call is on Microsoft Learn (`VirtualAlloc(Ex)`, `VirtualFree(Ex)`, `VirtualProtect(Ex)`,
//! `VirtualQuery(Ex)`, `WriteProcessMemory`, `ReadProcessMemory`, `CreateFileMappingW`,
//! `MapViewOfFile`, `UnmapViewOfFile`, `CreateThread`, `CreateProcessW`, `CreateEventW`); the
//! constants are `winnt.h`'s, `MEMORY_BASIC_INFORMATION` is `winnt.h`'s.

use super::slice2::{STILL_ACTIVE, contains, np};
use super::*;

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn VirtualAllocEx(p: Handle, addr: *mut u8, size: usize, kind: u32, protect: u32) -> *mut u8;
    fn VirtualFreeEx(p: Handle, addr: *mut u8, size: usize, kind: u32) -> i32;
    fn VirtualProtectEx(p: Handle, addr: *mut u8, size: usize, protect: u32, old: *mut u32) -> i32;
    fn VirtualQueryEx(p: Handle, addr: *const u8, info: *mut u8, len: usize) -> usize;
    fn WriteProcessMemory(p: Handle, addr: *mut u8, buf: *const u8, n: usize, done: *mut usize) -> i32;
    fn ReadProcessMemory(p: Handle, addr: *const u8, buf: *mut u8, n: usize, done: *mut usize) -> i32;
}

const MEM_COMMIT: u32 = 0x1000;
const MEM_RESERVE: u32 = 0x2000;
const MEM_DECOMMIT: u32 = 0x4000;
const MEM_RELEASE: u32 = 0x8000;
const PAGE_READONLY: u32 = 0x02;
const PAGE_READWRITE: u32 = 0x04;
const FILE_MAP_ALL_ACCESS: u32 = 0x000F_001F;
const PAGE: usize = 0x1000;

/// How many copies of itself the parent starts.
const CHILDREN: usize = 3;
/// How many threads of each process make their own memory calls.
const WORKERS: usize = 4;
/// How many times each of them goes round.
const ROUNDS: u32 = 400;
/// `--vm-stress-child`'s exit code when all its checks passed.
const CHILD_CODE: u32 = 0x0005_7A55;

/// `MEMORY_BASIC_INFORMATION`'s `State` and `Protect` (`winnt.h`: 48 bytes in a 64-bit process,
/// 28 in a 32-bit one).
fn state_and_protect(mbi: &[u8; 48]) -> (u32, u32) {
    let at = |o: usize| u32::from_le_bytes(mbi[o..o + 4].try_into().unwrap());
    if cfg!(target_pointer_width = "64") { (at(32), at(36)) } else { (at(16), at(20)) }
}

/// A pattern for round `round` of thread `who`.
fn pattern(who: u32, round: u32) -> u64 {
    (u64::from(who) << 40) ^ (u64::from(round) << 8) ^ 0x5A5A_0000_0000_00A5
}

/// One round on the process's own memory.
unsafe fn own_round(who: u32, round: u32) {
    // SAFETY: the memory is this round's own, the calls get valid arguments.
    unsafe {
        let size = PAGE * (4 + (round as usize * 7 + who as usize) % 60);
        let p = VirtualAlloc(core::ptr::null_mut(), size, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE);
        check(!p.is_null());
        let v = pattern(who, round);
        (p as *mut u64).write_volatile(v);
        (p.add(size - 8) as *mut u64).write_volatile(!v);
        let mut old = 0;
        check(VirtualProtect(p, PAGE, PAGE_READONLY, &mut old) != 0 && old == PAGE_READWRITE);
        let mut mbi = [0u8; 48];
        check(VirtualQuery(p, mbi.as_mut_ptr(), mbi.len()) != 0);
        check(state_and_protect(&mbi) == (MEM_COMMIT, PAGE_READONLY));
        check((p as *const u64).read_volatile() == v && (p.add(size - 8) as *const u64).read_volatile() == !v);
        check(VirtualProtect(p, PAGE, PAGE_READWRITE, &mut old) != 0 && old == PAGE_READONLY);
        // Decommit the second page: reserved, the rest still there.
        check(VirtualFree(p.add(PAGE), PAGE, MEM_DECOMMIT) != 0);
        check(VirtualQuery(p.add(PAGE), mbi.as_mut_ptr(), mbi.len()) != 0);
        check(state_and_protect(&mbi).0 == MEM_RESERVE);
        check((p.add(size - 8) as *const u64).read_volatile() == !v);
        check(VirtualFree(p, 0, MEM_RELEASE) != 0);
        // A pagefile section seen through two views.
        let m = CreateFileMappingW(INVALID_HANDLE_VALUE, np(), PAGE_READWRITE, 0, 0x1_0000, np());
        check(m != 0);
        let a = MapViewOfFile(m, FILE_MAP_ALL_ACCESS, 0, 0, 0);
        let b = MapViewOfFile(m, FILE_MAP_ALL_ACCESS, 0, 0, 0);
        check(!a.is_null() && !b.is_null() && a != b);
        (a.add(0x8000) as *mut u64).write_volatile(v);
        check((b.add(0x8000) as *const u64).read_volatile() == v);
        check(UnmapViewOfFile(a) != 0 && UnmapViewOfFile(b) != 0);
        CloseHandle(m);
    }
}

extern "system" fn own_worker(who: *mut u8) -> u32 {
    for round in 0..ROUNDS {
        // SAFETY: see `own_round`.
        unsafe { own_round(who as u32, round) };
    }
    0
}

/// One thread's round on `child`'s memory, from the parent.
unsafe fn remote_round(child: Handle, round: u32) {
    // SAFETY: the memory is in the child, the calls get valid arguments.
    unsafe {
        let size = PAGE * (2 + round as usize % 16);
        let p = VirtualAllocEx(child, core::ptr::null_mut(), size, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE);
        check(!p.is_null());
        let v = pattern(0xFF, round).to_le_bytes();
        let mut done = 0;
        check(WriteProcessMemory(child, p.add(size - 8), v.as_ptr(), 8, &mut done) != 0 && done == 8);
        let mut back = [0u8; 8];
        check(ReadProcessMemory(child, p.add(size - 8), back.as_mut_ptr(), 8, &mut done) != 0 && back == v);
        let mut old = 0;
        check(VirtualProtectEx(child, p, PAGE, PAGE_READONLY, &mut old) != 0 && old == PAGE_READWRITE);
        let mut mbi = [0u8; 48];
        check(VirtualQueryEx(child, p, mbi.as_mut_ptr(), mbi.len()) != 0);
        check(state_and_protect(&mbi) == (MEM_COMMIT, PAGE_READONLY));
        check(VirtualFreeEx(child, p, 0, MEM_RELEASE) != 0);
    }
}

extern "system" fn remote_worker(child: *mut u8) -> u32 {
    for round in 0..ROUNDS {
        // SAFETY: see `remote_round`.
        unsafe { remote_round(child as Handle, round) };
    }
    0
}

/// Start `n` threads of `f`, the i-th with argument `arg(i)`, into `out`.
unsafe fn start(out: &mut [Handle], f: extern "system" fn(*mut u8) -> u32, arg: impl Fn(usize) -> usize) {
    for (i, slot) in out.iter_mut().enumerate() {
        let mut id = 0;
        // SAFETY: a thread of a function that takes its argument by value.
        *slot = unsafe { CreateThread(np(), 0, f, arg(i) as *mut u8, 0, &mut id) };
        check(*slot != 0);
    }
}

/// Wait for `threads` to end with 0, and close them.
unsafe fn join(threads: &[Handle]) {
    // SAFETY: thread handles this process made.
    unsafe {
        check(WaitForMultipleObjects(threads.len() as u32, threads.as_ptr(), 1, 300_000) < WAIT_OBJECT_0 + 64);
        for &t in threads {
            let mut code = STILL_ACTIVE;
            check(GetExitCodeThread(t, &mut code) != 0 && code == 0);
            CloseHandle(t);
        }
    }
}

/// `--vm-stress` and `--vm-stress-child`.
pub(super) unsafe fn run(cmd: &[u16]) -> ! {
    // SAFETY: valid arguments throughout.
    unsafe {
        let done = CreateEventW(np(), 1, 0, w!("PolyVmStressDone"));
        check(done != 0);
        let child = contains(cmd, b"--vm-stress-child");
        let base = if child { GetCurrentProcessId() << 8 } else { 0 };
        let mut own = [0 as Handle; WORKERS];
        if child {
            start(&mut own, own_worker, |i| base as usize + i);
            join(&own);
            // Alive until the parent is done with this process's memory.
            check(WaitForSingleObject(done, 300_000) == WAIT_OBJECT_0);
            ExitProcess(CHILD_CODE)
        }
        out(b"k32test: vm-stress: memory calls of many threads and processes at once\n");
        let started = GetTickCount64();
        let mut si = [0u8; 104];
        si[..4].copy_from_slice(&104u32.to_le_bytes());
        let mut children = [0 as Handle; CHILDREN];
        let hsize = core::mem::size_of::<usize>();
        for slot in children.iter_mut() {
            let mut line = [0u16; 64];
            for (d, &c) in line.iter_mut().zip(b"k32test.exe --vm-stress-child") {
                *d = u16::from(c);
            }
            let mut pi = [0u8; 24];
            let ok =
                CreateProcessW(np(), line.as_mut_ptr(), np(), np(), 0, 0, np(), np(), si.as_ptr(), pi.as_mut_ptr());
            check(ok != 0);
            let read = |at: usize| {
                let mut b = [0u8; 8];
                b[..hsize].copy_from_slice(&pi[at..at + hsize]);
                i64::from_le_bytes(b) as Handle
            };
            *slot = read(0);
            CloseHandle(read(hsize));
        }
        start(&mut own, own_worker, |i| i);
        let mut remote = [0 as Handle; CHILDREN];
        start(&mut remote, remote_worker, |i| children[i] as usize);
        join(&own);
        join(&remote);
        check(SetEvent(done) != 0);
        for &h in &children {
            check(WaitForSingleObject(h, 120_000) == WAIT_OBJECT_0);
            let mut code = STILL_ACTIVE;
            check(GetExitCodeProcess(h, &mut code) != 0);
            if code != CHILD_CODE {
                out(b"k32test: vm-stress: a child ended with ");
                out_num(code);
                out(b"\n");
                check(false);
            }
            CloseHandle(h);
        }
        out(b"k32test: vm-stress: took ");
        out_num((GetTickCount64() - started) as u32);
        out(b" ms\n");
        out(b"k32test: vm-stress: all checks passed\n");
        ExitProcess(PASS)
    }
}

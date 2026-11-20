//! `k32test.exe --oom` (ADR 0250): committing memory until the system says no fails cleanly with
//! the commit limit, gives everything back when freed, and works again; four processes doing it at
//! once all see the same. The kernel test `nt_commit_until_refused_then_again` runs it.
//!
//! `k32test.exe --oom-job` (ADR 0250 addendum, the victim's score): a parent, as a browser is,
//! holds [`PARENT_FILL`] and starts a copy of itself with `--oom-eat` in a job the parent made, as
//! a browser starts its renderers. The child takes memory until none is left. The out-of-memory
//! policy must end the child (`STATUS_NO_MEMORY`), not the larger parent, whose pages are all
//! still there afterwards. The kernel test `nt_oom_ends_the_child_in_a_job_not_its_parent` runs it
//! with user memory stopping 32 MiB above what is free (`frames::set_test_floor`), so the child
//! gets less than the parent, on any machine and quickly.
//!
//! The pages are committed, not touched: what is checked is the system's commit limit, which
//! refuses a commit before the frames for it could run out. (Touching every page as well would
//! reach the kernel's frame reserve, which the kernel's own test covers.)

use super::slice2::{STILL_ACTIVE, contains, np};
use super::*;

const MEM_COMMIT: u32 = 0x1000;
const MEM_RESERVE: u32 = 0x2000;
const MEM_RELEASE: u32 = 0x8000;
const PAGE_READWRITE: u32 = 0x04;
/// `ERROR_NOT_ENOUGH_MEMORY`, `ERROR_COMMITMENT_LIMIT` (`winerror.h`).
const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
const ERROR_COMMITMENT_LIMIT: u32 = 1455;
/// What one commit asks for.
const CHUNK: usize = 16 << 20;
/// The most chunks kept (4 GiB): far more than any test machine holds.
const MAX_CHUNKS: usize = 256;
/// `--oom-child`'s exit code when it saw the limit.
const CHILD_PASS: u32 = 0x00C0_FFEE;

/// Commit chunks until refused. Returns the chunks and the error of the refusal.
unsafe fn commit_until_refused(chunks: &mut [*mut u8; MAX_CHUNKS]) -> (usize, u32) {
    let mut n = 0;
    while n < MAX_CHUNKS {
        // SAFETY: a plain allocation.
        let p = unsafe { VirtualAlloc(core::ptr::null_mut(), CHUNK, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE) };
        if p.is_null() {
            // SAFETY: a plain call.
            return (n, unsafe { GetLastError() });
        }
        chunks[n] = p;
        n += 1;
    }
    (n, 0)
}

unsafe fn release(chunks: &[*mut u8]) {
    for &p in chunks {
        // SAFETY: chunks VirtualAlloc returned.
        check(unsafe { VirtualFree(p, 0, MEM_RELEASE) } != 0);
    }
}

fn refused_for_memory(error: u32) -> bool {
    error == ERROR_COMMITMENT_LIMIT || error == ERROR_NOT_ENOUGH_MEMORY
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateJobObjectW(sa: *const u8, name: *const u16) -> Handle;
    fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
    fn ResumeThread(thread: Handle) -> u32;
    fn TerminateProcess(process: Handle, code: u32) -> i32;
}

/// `CREATE_SUSPENDED`, `FILE_MAP_WRITE`, `INVALID_HANDLE_VALUE` (`winbase.h`, `memoryapi.h`).
const CREATE_SUSPENDED: u32 = 4;
const FILE_MAP_WRITE: u32 = 2;
const INVALID_HANDLE_VALUE: Handle = -1;
/// `STATUS_NO_MEMORY` (`ntstatus.h`): the exit code of a process the policy ended.
const STATUS_NO_MEMORY: u32 = 0xC000_0017;
/// What `--oom-job`'s parent holds: more than its child can take under the kernel test's floor.
const PARENT_FILL: usize = 16 << 20;

/// Commit and write `CHUNK` bytes, each page holding its own number and `tag`; null when refused.
unsafe fn fill_chunk(tag: u32) -> *mut u8 {
    // SAFETY: a plain allocation, then writes inside it.
    unsafe {
        let p = VirtualAlloc(core::ptr::null_mut(), CHUNK, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
        if !p.is_null() {
            for i in (0..CHUNK).step_by(4096) {
                (p.add(i) as *mut u32).write_volatile(tag ^ i as u32);
            }
        }
        p
    }
}

/// `--oom-eat`: take memory until the policy ends this process: private commits while the
/// commit limit allows, then views of pagefile-backed sections (which it does not charge), every
/// page written; when both are refused, wait for the policy. Ends with 2 if nobody ends it.
unsafe fn eat() -> ! {
    // SAFETY: plain allocations and writes inside them.
    unsafe {
        let mut idle = 0;
        while idle < 6000 {
            let mut p = fill_chunk(0x5EED);
            if p.is_null() {
                let m = CreateFileMappingW(INVALID_HANDLE_VALUE, np(), PAGE_READWRITE, 0, CHUNK as u32, np());
                if m != 0 {
                    p = MapViewOfFile(m, FILE_MAP_WRITE, 0, 0, CHUNK);
                    if !p.is_null() {
                        for i in (0..CHUNK).step_by(4096) {
                            p.add(i).write_volatile(1);
                        }
                    }
                }
            }
            if p.is_null() {
                idle += 1;
                Sleep(10);
            }
        }
        ExitProcess(2)
    }
}

/// `--oom-job`: the parent; see the module's documentation.
unsafe fn job_parent() -> ! {
    let mut chunks = [core::ptr::null_mut::<u8>(); MAX_CHUNKS];
    // SAFETY: valid arguments throughout.
    unsafe {
        out(b"k32test: oom-job: the parent fills memory\n");
        let mut n = 0;
        while n < MAX_CHUNKS && n * CHUNK < PARENT_FILL {
            let p = fill_chunk(0xB0B);
            if p.is_null() {
                break;
            }
            chunks[n] = p;
            n += 1;
        }
        check(n > 0);
        out(b"k32test: oom-job: the parent holds ");
        out_num((n * (CHUNK >> 20)) as u32);
        out(b" MiB; the child in its job eats the rest\n");
        let job = CreateJobObjectW(np(), np());
        check(job != 0);
        let mut si = [0u8; 104];
        si[..4].copy_from_slice(&104u32.to_le_bytes());
        let mut line = [0u16; 64];
        for (d, &c) in line.iter_mut().zip(b"k32test.exe --oom-eat") {
            *d = u16::from(c);
        }
        let mut pi = [0u8; 24];
        let ok = CreateProcessW(
            np(),
            line.as_mut_ptr(),
            np(),
            np(),
            0,
            CREATE_SUSPENDED,
            np(),
            np(),
            si.as_ptr(),
            pi.as_mut_ptr(),
        );
        check(ok != 0);
        let process = isize::from_le_bytes(pi[0..8].try_into().unwrap());
        let thread = isize::from_le_bytes(pi[8..16].try_into().unwrap());
        check(AssignProcessToJobObject(job, process) != 0);
        check(ResumeThread(thread) != u32::MAX);
        CloseHandle(thread);
        let waited = WaitForSingleObject(process, 120_000);
        if waited != WAIT_OBJECT_0 {
            TerminateProcess(process, 1);
        }
        check(waited == WAIT_OBJECT_0);
        let mut code = STILL_ACTIVE;
        check(GetExitCodeProcess(process, &mut code) != 0);
        out(b"k32test: oom-job: the child ended with ");
        out_num(code);
        out(b"\n");
        check(code == STATUS_NO_MEMORY);
        CloseHandle(process);
        CloseHandle(job);
        // Every page of the parent is as it wrote it.
        let mut intact = true;
        for &p in &chunks[..n] {
            for i in (0..CHUNK).step_by(4096) {
                intact &= (p.add(i) as *const u32).read_volatile() == 0xB0B ^ i as u32;
            }
        }
        check(intact);
        release(&chunks[..n]);
        out(b"k32test: oom-job: all checks passed\n");
        ExitProcess(PASS)
    }
}

/// `--oom`, `--oom-child`, `--oom-job` and `--oom-eat`.
pub(super) unsafe fn run(cmd: &[u16]) -> ! {
    let mut chunks = [core::ptr::null_mut::<u8>(); MAX_CHUNKS];
    // SAFETY: valid arguments throughout.
    unsafe {
        if contains(cmd, b"--oom-eat") {
            eat()
        }
        if contains(cmd, b"--oom-job") {
            job_parent()
        }
        if contains(cmd, b"--oom-child") {
            let (n, error) = commit_until_refused(&mut chunks);
            // Hold it a moment, so the others hit the limit while this holds part of it.
            Sleep(500);
            release(&chunks[..n]);
            ExitProcess(if n < MAX_CHUNKS && refused_for_memory(error) { CHILD_PASS } else { 1 })
        }
        out(b"k32test: oom: commit until refused\n");
        let (n, error) = commit_until_refused(&mut chunks);
        check(n > 0 && n < MAX_CHUNKS);
        check(refused_for_memory(error));
        out(b"k32test: oom: refused after ");
        out_num((n * (CHUNK >> 20)) as u32);
        out(b" MiB with error ");
        out_num(error);
        out(b"\n");
        release(&chunks[..n]);
        // All of it can be had again, and written.
        let (again, _) = commit_until_refused(&mut chunks);
        check(again + 1 >= n);
        release(&chunks[..again]);
        let p = VirtualAlloc(core::ptr::null_mut(), CHUNK, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
        check(!p.is_null());
        for i in (0..CHUNK).step_by(4096) {
            p.add(i).write_volatile(i as u8);
        }
        release(&[p]);

        // Four processes at once.
        out(b"k32test: oom: four processes commit until refused\n");
        let mut si = [0u8; 104];
        si[..4].copy_from_slice(&104u32.to_le_bytes());
        let mut procs = [0isize; 4];
        for slot in procs.iter_mut() {
            let mut line = [0u16; 64];
            for (d, &c) in line.iter_mut().zip(b"k32test.exe --oom-child") {
                *d = u16::from(c);
            }
            let mut pi = [0u8; 24];
            let ok =
                CreateProcessW(np(), line.as_mut_ptr(), np(), np(), 0, 0, np(), np(), si.as_ptr(), pi.as_mut_ptr());
            check(ok != 0);
            *slot = isize::from_le_bytes(pi[0..8].try_into().unwrap());
            CloseHandle(isize::from_le_bytes(pi[8..16].try_into().unwrap()));
        }
        for &h in &procs {
            check(WaitForSingleObject(h, 120_000) == WAIT_OBJECT_0);
            let mut code = STILL_ACTIVE;
            check(GetExitCodeProcess(h, &mut code) != 0 && code == CHILD_PASS);
            CloseHandle(h);
        }
        // And memory is there again afterwards.
        let p = VirtualAlloc(core::ptr::null_mut(), CHUNK, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
        check(!p.is_null());
        release(&[p]);
        out(b"k32test: oom: all checks passed\n");
        ExitProcess(PASS)
    }
}

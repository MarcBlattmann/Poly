//! `k32test.exe --oom` (ADR 0250): committing memory until the system says no fails cleanly with
//! the commit limit, gives everything back when freed, and works again; four processes doing it at
//! once all see the same. The kernel test `nt_commit_until_refused_then_again` runs it.
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

/// `--oom` and `--oom-child`.
pub(super) unsafe fn run(cmd: &[u16]) -> ! {
    let mut chunks = [core::ptr::null_mut::<u8>(); MAX_CHUNKS];
    // SAFETY: valid arguments throughout.
    unsafe {
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

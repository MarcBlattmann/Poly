//! `k32test.exe --resume-race`: a child created suspended whose first thread runs to its end
//! before `ResumeThread` has returned in the parent still ends as a process: the wait on it is
//! satisfied at once and its exit code is the one it gave. The kernel test
//! `nt_suspended_child_that_ends_during_resume_is_signalled` runs it with the kernel's resume
//! waiting, between starting the thread and letting go of the creation's hold, until the child's
//! threads have left: the order a loaded machine can give (the parent preempted there, the child
//! quick).
//!
//! Every call is on Microsoft Learn (`CreateProcessW`, `ResumeThread`, `WaitForSingleObject`,
//! `GetExitCodeProcess`, `GetTickCount64`, `CloseHandle`, `ExitProcess`); `CREATE_SUSPENDED` is
//! `winbase.h`'s, `STARTUPINFOW` and `PROCESS_INFORMATION` are `processthreadsapi.h`'s.

use super::slice2::{STILL_ACTIVE, contains, np};
use super::*;

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn ResumeThread(t: Handle) -> u32;
}

const CREATE_SUSPENDED: u32 = 0x4;
/// `--resume-race-child`'s exit code.
const CHILD_CODE: u32 = 0x5EED;
/// How many children are started, one after another.
const CHILDREN: u32 = 4;
/// The longest a child may take from `ResumeThread`'s return to its wait being satisfied (the
/// kernel test's wait is spent inside `ResumeThread`, so this is the time the end takes).
const MAX_END_MS: u64 = 3000;

/// `--resume-race` and `--resume-race-child`.
pub(super) unsafe fn run(cmd: &[u16]) -> ! {
    // SAFETY: valid arguments throughout.
    unsafe {
        if contains(cmd, b"--resume-race-child") {
            ExitProcess(CHILD_CODE)
        }
        out(b"k32test: resume-race: children created suspended that end while they are resumed\n");
        let mut si = [0u8; 104];
        si[..4].copy_from_slice(&104u32.to_le_bytes());
        let mut slowest = 0u64;
        for _ in 0..CHILDREN {
            let mut line = [0u16; 64];
            for (d, &c) in line.iter_mut().zip(b"k32test.exe --resume-race-child") {
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
            check(ResumeThread(thread) == 1);
            let resumed = GetTickCount64();
            check(WaitForSingleObject(process, 10_000) == WAIT_OBJECT_0);
            slowest = slowest.max(GetTickCount64() - resumed);
            let mut code = STILL_ACTIVE;
            check(GetExitCodeProcess(process, &mut code) != 0 && code == CHILD_CODE);
            CloseHandle(thread);
            CloseHandle(process);
        }
        out(b"k32test: resume-race: the slowest end took ");
        out_num(slowest as u32);
        out(b" ms after ResumeThread\n");
        check(slowest <= MAX_END_MS);
        out(b"k32test: resume-race: all checks passed\n");
        ExitProcess(PASS)
    }
}

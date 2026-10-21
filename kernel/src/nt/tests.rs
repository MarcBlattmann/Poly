//! In-kernel tests: a PE executable Poly builds itself ([`test_pe`]) runs
//! from ring 3 and drives the NT object, memory and section services
//! through real `syscall` instructions.

use super::{test_pe, test_seh, test_threads};
use crate::kernel_test;
use crate::process::Process;
use crate::thread;
use crate::user::memory::Layout;
use crate::user::{self, Memory};
use alloc::sync::Arc;
use poly_abi::Personality;
use poly_binfmt::{Arch, Bits, Format};

const FORMAT: Format = Format::Pe { bits: Bits::B64, arch: Arch::X86_64 };

kernel_test! {
    /// Build the NT test PE, start it as an NT process, and let it drive the
    /// object manager, virtual memory and sections from ring 3 to its end.
    fn nt_test_program_runs_from_ring3() -> TestResult {
        let file = test_pe::image();
        let memory = Arc::new(Memory::new(Layout::default()).map_err(|_| "no memory map")?);
        let pid = crate::process::alloc_pid();
        let process = Arc::new(Process::new_nt(pid, FORMAT, false));
        assert_eq!(process.personality, Personality::Nt);

        let start = super::start(&process, &memory, &file, pid).map_err(|_| "the NT loader rejected the test PE")?;
        let tid = user::spawn_nt(Arc::clone(&process), Arc::clone(&memory), start.entry, start.teb, user::DEFAULT_HOOK)
            .map_err(|_| "could not start the NT thread")?;
        let code = thread::join(tid).map_err(|_| "join failed")?;
        if code != test_pe::PASS_CODE as usize {
            crate::kprintln!("nt: the test program ended with {code:#x}, not {:#x}", test_pe::PASS_CODE);
            return Err("the NT test program did not pass every check");
        }

        // Its process left the table and its memory is the only reference.
        if !thread::wait_until(10, || Arc::strong_count(&memory) == 1) {
            return Err("the NT process kept its memory after it exited");
        }
        if super::is_nt_process(pid) {
            return Err("the NT process state was not cleaned up");
        }
        Ok(())
    }
}

kernel_test! {
    /// Build the NT threads/synchronisation test PE and run it from ring 3:
    /// it creates threads, blocks on and is woken through events, a mutant it
    /// abandons, a semaphore in a wait-all, a user APC, a periodic timer, and
    /// reads a suspended thread's context. See [`test_threads`].
    fn nt_threads_and_sync_from_ring3() -> TestResult {
        let file = test_threads::image();
        let memory = Arc::new(Memory::new(Layout::default()).map_err(|_| "no memory map")?);
        let pid = crate::process::alloc_pid();
        let process = Arc::new(Process::new_nt(pid, FORMAT, false));

        let start = super::start(&process, &memory, &file, pid).map_err(|_| "the NT loader rejected the test PE")?;
        let tid = user::spawn_nt(Arc::clone(&process), Arc::clone(&memory), start.entry, start.teb, user::DEFAULT_HOOK)
            .map_err(|_| "could not start the NT thread")?;
        let code = thread::join(tid).map_err(|_| "join failed")?;
        if code != test_threads::PASS_CODE as usize {
            crate::kprintln!("nt: the threads test ended with {code:#x}, not {:#x}", test_threads::PASS_CODE);
            return Err("the NT threads/sync test did not pass every check");
        }
        if !thread::wait_until(10, || Arc::strong_count(&memory) == 1) {
            return Err("the NT process kept its memory after it exited");
        }
        if super::is_nt_process(pid) {
            return Err("the NT process state was not cleaned up");
        }
        Ok(())
    }
}


kernel_test! {
    /// Kernel-to-user exception dispatch (ADR 0068): a PE registers a
    /// user-mode dispatcher, faults with an access violation and a
    /// divide-by-zero, and the dispatcher inspects each `EXCEPTION_RECORD`,
    /// fixes the `CONTEXT` up and resumes with `NtContinue`. See
    /// [`test_seh`].
    fn nt_seh_dispatch_and_continue_from_ring3() -> TestResult {
        let file = test_seh::handled_image();
        let memory = Arc::new(Memory::new(Layout::default()).map_err(|_| "no memory map")?);
        let pid = crate::process::alloc_pid();
        let process = Arc::new(Process::new_nt(pid, FORMAT, false));

        let start = super::start(&process, &memory, &file, pid).map_err(|_| "the NT loader rejected the test PE")?;
        let tid = user::spawn_nt(Arc::clone(&process), Arc::clone(&memory), start.entry, start.teb, user::DEFAULT_HOOK)
            .map_err(|_| "could not start the NT thread")?;
        let code = thread::join(tid).map_err(|_| "join failed")?;
        if code != test_seh::PASS_CODE as usize {
            crate::kprintln!("nt: the SEH test ended with {code:#x}, not {:#x}", test_seh::PASS_CODE);
            return Err("the SEH dispatch/continue test did not pass every check");
        }
        if !thread::wait_until(10, || Arc::strong_count(&memory) == 1) {
            return Err("the NT process kept its memory after it exited");
        }
        Ok(())
    }
}

kernel_test! {
    /// An NT exception nobody handles ends the process with the exception
    /// code as its exit status (ADR 0068). The PE registers no dispatcher
    /// and writes to a null pointer.
    fn nt_seh_unhandled_terminates_with_the_exception_code() -> TestResult {
        let file = test_seh::unhandled_image();
        let memory = Arc::new(Memory::new(Layout::default()).map_err(|_| "no memory map")?);
        let pid = crate::process::alloc_pid();
        let process = Arc::new(Process::new_nt(pid, FORMAT, false));

        let start = super::start(&process, &memory, &file, pid).map_err(|_| "the NT loader rejected the test PE")?;
        let tid = user::spawn_nt(Arc::clone(&process), Arc::clone(&memory), start.entry, start.teb, user::DEFAULT_HOOK)
            .map_err(|_| "could not start the NT thread")?;
        let code = thread::join(tid).map_err(|_| "join failed")?;
        // thread::join reports the full exception code the process ended with.
        if code != test_seh::STATUS_ACCESS_VIOLATION as usize {
            crate::kprintln!("nt: the unhandled SEH test ended with {code:#x}");
            return Err("an unhandled NT exception did not terminate with STATUS_ACCESS_VIOLATION");
        }
        let last = super::last_unhandled().ok_or("no unhandled exception was recorded")?;
        if last.pid != pid || last.code != test_seh::STATUS_ACCESS_VIOLATION {
            return Err("the recorded unhandled exception did not match");
        }
        if !thread::wait_until(10, || Arc::strong_count(&memory) == 1) {
            return Err("the NT process kept its memory after it exited");
        }
        Ok(())
    }
}

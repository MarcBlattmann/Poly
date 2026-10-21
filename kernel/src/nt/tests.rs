//! In-kernel tests: a PE executable Poly builds itself ([`test_pe`]) runs
//! from ring 3 and drives the NT object, memory and section services
//! through real `syscall` instructions.

use super::test_pe;
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

//! System call routing: one entry point, three call tables.

use crate::process::Process;
use poly_abi::{Outcome, Personality, SyscallDispatch, SyscallRegs};
use poly_personality_linux::Linux;
use poly_personality_nt::Nt;
use poly_personality_xnu::Xnu;

/// Route a system call to the calling process's personality.
pub fn dispatch(process: &Process, regs: &mut SyscallRegs) -> Outcome {
    let outcome = match process.personality {
        Personality::Linux => Linux::dispatch(regs),
        Personality::Nt => Nt::dispatch(regs),
        Personality::Xnu => Xnu::dispatch(regs),
    };
    if let Outcome::Unimplemented { table, number } = outcome {
        // This line is the seed of the API tracer: every unimplemented call
        // an app makes is a work item.
        crate::kprintln!(
            "syscall: pid {} {}/{} #{} not implemented",
            process.pid,
            process.personality.name(),
            table,
            number
        );
    }
    outcome
}

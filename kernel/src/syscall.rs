//! System call routing: one entry point, three call tables.

use crate::process::Process;
use poly_abi::{Outcome, Personality, SyscallDispatch, SyscallRegs};
use poly_personality_linux::Linux;
use poly_personality_nt::Nt;
use poly_personality_xnu::Xnu;

/// Route a system call to the calling process's personality. A Linux
/// process with personality state gets the kernel's services
/// (`crate::linux`); a bare record only the personality's answers without
/// them (`-ENOSYS`).
pub fn dispatch(process: &Process, regs: &mut SyscallRegs) -> Outcome {
    let outcome = match process.personality {
        Personality::Linux if process.linux.is_some() => crate::linux::syscall(process, regs),
        Personality::Linux => Linux::dispatch(regs),
        Personality::Nt => Nt::dispatch(regs),
        Personality::Xnu => Xnu::dispatch(regs),
    };
    if let Outcome::Unimplemented { table, number } = outcome {
        // This line is the seed of the API tracer: every unimplemented call
        // an app makes is a work item.
        let name = match process.personality {
            Personality::Linux => poly_personality_linux::table::name(number),
            _ => None,
        };
        crate::kprintln!(
            "syscall: pid {} {}/{} #{}{}{} not implemented",
            process.pid,
            process.personality.name(),
            table,
            number,
            if name.is_some() { " " } else { "" },
            name.unwrap_or_default()
        );
    }
    outcome
}

/// [`dispatch`] as a `user::SyscallHook`: what user threads use unless
/// they are given another hook.
pub fn dispatch_hook(process: &Process, regs: &mut SyscallRegs) {
    dispatch(process, regs);
}

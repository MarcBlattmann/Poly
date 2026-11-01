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
        Personality::Nt if crate::nt::is_nt_process(process.pid) => crate::nt::syscall(process, regs),
        Personality::Nt => Nt::dispatch(regs),
        Personality::Xnu => Xnu::dispatch(regs),
    };
    if let Outcome::Unimplemented { table, number } = outcome
        && first_time(process.personality, number)
    {
        // This line is the seed of the API tracer: every unimplemented call
        // an app makes is a work item. Each is logged once per boot, so a
        // call every program of a kind makes does not flood the console of
        // an interactive shell; `poly.strace` shows every call.
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

/// Whether this is the first time call `number` of `personality` is
/// reported unimplemented in this boot (numbers past 1023 always are).
fn first_time(personality: Personality, number: u64) -> bool {
    use core::sync::atomic::{AtomicU64, Ordering};
    const WORDS: usize = 16;
    static SEEN: [[AtomicU64; WORDS]; 3] = [const { [const { AtomicU64::new(0) }; WORDS] }; 3];
    let table = match personality {
        Personality::Linux => 0,
        Personality::Nt => 1,
        Personality::Xnu => 2,
    };
    let Some(word) = SEEN[table].get((number / 64) as usize) else { return true };
    let bit = 1 << (number % 64);
    word.fetch_or(bit, Ordering::Relaxed) & bit == 0
}

/// [`dispatch`] as a `user::SyscallHook`: what user threads use unless
/// they are given another hook.
pub fn dispatch_hook(process: &Process, regs: &mut SyscallRegs) {
    dispatch(process, regs);
}

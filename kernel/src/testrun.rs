//! The test-run protocol of ADR 0027: the records the kernel writes on the
//! serial port about the first program (`poly.init=<path>`), which the
//! diff runner (`tools/pipeline/src/poly.rs`) turns into a trace.
//!
//! ```text
//! POLY-INIT: exec /test-binary                     the program starts
//! POLY-INIT: unsupported <reason>                  it cannot run
//! POLY-SYSCALL: pid=2 tid=2 nr=1 args=0x1,0x401000,0xd,0x0,0x0,0x0 ret=13
//! POLY-ENTER: pid=2 tid=2 nr=56 args=...           clone/fork/vfork/clone3 entered (before the child runs)
//! POLY-OUT: fd=1 hex=68656c6c6f0a                  bytes that reached its stdout (1) or stderr (2)
//! POLY-SIGNAL: pid=2 tid=3 signal=11               a signal taken for delivery (not SIGKILL)
//! POLY-EXIT: pid=2 code=0                          it ended (or signal=N); an NT program adds ntstatus=0x...
//! ```
//!
//! With `poly.test.capture` the program runs as the child of the kernel's
//! init, pid 1 (`testinit.rs`, ADR 0064), so it is pid 2; as the user,
//! groups and umask `poly.test.uid`, `poly.test.gid`, `poly.test.groups`
//! and `poly.test.umask` give (the diff runner's reference's, ADR 0081). Output is
//! captured by open file: fds 1 and 2 start as two console files whose
//! bytes are reported (`tty::log_console_files`), wherever they are
//! duplicated to; `poly.test.stdout=<path>` / `poly.test.stderr=<path>`
//! also report a file's contents when the program has ended.
//!
//! `POLY-INIT` and `POLY-EXIT` are written whenever `poly.init` runs a
//! program; the others only for processes marked traced, which the first
//! program is with `poly.test.capture`. Each record is one line of its own
//! on the serial port only (not the screen); the program's own console
//! output goes through the kernel log as usual, so a record may follow
//! program text on the same line, which the parser allows. Syscall records
//! carry no `data<i>` fields and no times yet: pointer arguments then match
//! anything in a comparison.

use crate::log;
use crate::process::ExitStatus;
use alloc::format;
use alloc::string::String;
use core::fmt::Write;

/// Kernel argument: the first program to run.
pub const INIT_ARG: &str = "poly.init";
/// Kernel argument, once per argument: the first program's arguments after
/// its path, in order (`poly.init.arg=a poly.init.arg=t.7z`; no spaces in
/// one; ADR 0087).
pub const INIT_ARGV: &str = "poly.init.arg";
/// Kernel flag: trace the first program (`POLY-SYSCALL`, `POLY-OUT`,
/// `POLY-SIGNAL` records).
pub const CAPTURE_FLAG: &str = "poly.test.capture";

/// Most bytes of program output per `POLY-OUT` record.
const OUT_CHUNK: usize = 256;

fn record(line: &str) {
    log::write_console(line);
}

/// `POLY-INIT: exec <path>`.
pub fn init(path: &str) {
    record(&format!("POLY-INIT: exec {path}\n"));
}

/// `POLY-INIT: unsupported <reason>`.
pub fn unsupported(reason: &str) {
    record(&format!("POLY-INIT: unsupported {reason}\n"));
}

/// `POLY-SYSCALL`: thread `tid` of process `pid` made call `nr` with
/// `args`, which returned `ret` (`None`: it did not return, like
/// `exit_group`). The `tid=` field lets the diff runner tell a process's
/// threads apart (a tracer sees each thread as a task of its own).
pub fn syscall_of(pid: u32, tid: u32, nr: u64, args: &[u64; 6], ret: Option<u64>) {
    let mut line = String::with_capacity(136);
    let _ = write!(line, "POLY-SYSCALL: pid={pid} tid={tid} nr={nr} args=");
    push_args(&mut line, args);
    match ret {
        Some(r) => {
            let _ = writeln!(line, " ret={}", r as i64);
        }
        None => line.push_str(" ret=?\n"),
    }
    record(&line);
}

/// `POLY-ENTER`: thread `tid` of process `pid` has entered call `nr`, whose
/// `POLY-SYSCALL` record comes when it returns. Written for the calls that
/// start another task (`clone`, `fork`, `vfork`, `clone3`) before the task
/// exists, so the trace has the call where a tracer sees it: at its entry,
/// before anything the new task does.
pub fn enter(pid: u32, tid: u32, nr: u64, args: &[u64; 6]) {
    let mut line = String::with_capacity(128);
    let _ = write!(line, "POLY-ENTER: pid={pid} tid={tid} nr={nr} args=");
    push_args(&mut line, args);
    line.push('\n');
    record(&line);
}

/// Whether call `nr` starts another task, so its [`enter`] record goes out
/// first: `clone`, `fork`, `vfork`, `clone3`.
pub fn starts_task(nr: u64) -> bool {
    use poly_personality_linux::nr;
    matches!(nr, nr::CLONE | nr::FORK | nr::VFORK | nr::CLONE3)
}

fn push_args(line: &mut String, args: &[u64; 6]) {
    for (i, a) in args.iter().enumerate() {
        let _ = write!(line, "{}{a:#x}", if i == 0 { "" } else { "," });
    }
}

/// `POLY-SIGNAL`: thread `tid` of process `pid` took `signal` to act on it
/// (a handler, a stop, a continue, the end of the process, or dropping it
/// as ignored), as a ptrace(2) tracer sees a signal-delivery-stop: never
/// for `SIGKILL`, nor for a signal rt_sigtimedwait(2) or a signal file
/// takes. A system call a signal interrupted is reported with the result
/// a tracer sees (`ret=-512` and the like for one awaiting its restart
/// decision; `Linux::traced_result`).
pub fn signal_of(pid: u32, tid: u32, signal: u32) {
    record(&format!("POLY-SIGNAL: pid={pid} tid={tid} signal={signal}\n"));
}

/// `POLY-OUT` records for `bytes` written to `fd` (1 or 2).
pub fn output(fd: i32, bytes: &[u8]) {
    for chunk in bytes.chunks(OUT_CHUNK) {
        let mut line = String::with_capacity(32 + 2 * chunk.len());
        let _ = write!(line, "POLY-OUT: fd={fd} hex=");
        for b in chunk {
            let _ = write!(line, "{b:02x}");
        }
        line.push('\n');
        record(&line);
    }
}

/// `POLY-EXIT`: process `pid` ended with `status`; an NT process also
/// with its full `NTSTATUS` (`ntstatus=0x...`, ADR 0075), of which `code`
/// is the low 8 bits.
pub fn exit(pid: u32, status: ExitStatus, ntstatus: Option<u32>) {
    let mut how = match status {
        ExitStatus::Exited(code) => format!("code={code}"),
        ExitStatus::Signaled { signal, .. } => format!("signal={signal}"),
        // Only bare processes fault without a signal; report it as the
        // SIGSEGV it would be.
        ExitStatus::Faulted(_) => format!("signal={}", crate::linux::SIGSEGV),
    };
    if let Some(s) = ntstatus {
        let _ = write!(how, " ntstatus={s:#x}");
    }
    record(&format!("POLY-EXIT: pid={pid} {how}\n"));
}

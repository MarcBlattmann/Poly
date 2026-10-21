//! The test-run protocol of ADR 0027: the records the kernel writes on the
//! serial port about the first program (`poly.init=<path>`), which the
//! diff runner (`tools/pipeline/src/poly.rs`) turns into a trace.
//!
//! ```text
//! POLY-INIT: exec /test-binary                     the program starts
//! POLY-INIT: unsupported <reason>                  it cannot run
//! POLY-SYSCALL: pid=1 nr=1 args=0x1,0x401000,0xd,0x0,0x0,0x0 ret=13
//! POLY-OUT: fd=1 hex=68656c6c6f0a                  bytes it wrote to fd 1 or 2
//! POLY-SIGNAL: pid=1 signal=11                     a signal taken for delivery (not SIGKILL)
//! POLY-EXIT: pid=1 code=0                          it ended (or signal=N)
//! ```
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

/// `POLY-SYSCALL`: call `nr` with `args` returned `ret` (`None`: it did
/// not return, like `exit_group`).
pub fn syscall(pid: u32, nr: u64, args: &[u64; 6], ret: Option<u64>) {
    let mut line = String::with_capacity(128);
    let _ = write!(line, "POLY-SYSCALL: pid={pid} nr={nr} args=");
    for (i, a) in args.iter().enumerate() {
        let _ = write!(line, "{}{a:#x}", if i == 0 { "" } else { "," });
    }
    match ret {
        Some(r) => {
            let _ = writeln!(line, " ret={}", r as i64);
        }
        None => line.push_str(" ret=?\n"),
    }
    record(&line);
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

/// `POLY-SIGNAL`: a thread of process `pid` took `signal` to act on it
/// (a handler, a stop, a continue, the end of the process, or dropping it
/// as ignored), as a ptrace(2) tracer sees a signal-delivery-stop: never
/// for `SIGKILL`, nor for a signal rt_sigtimedwait(2) or a signal file
/// takes. A system call a signal interrupted is reported with the result
/// a tracer sees (`ret=-512` and the like for one awaiting its restart
/// decision; `Linux::traced_result`).
pub fn signal(pid: u32, signal: u32) {
    record(&format!("POLY-SIGNAL: pid={pid} signal={signal}\n"));
}

/// `POLY-EXIT`: process `pid` ended with `status`.
pub fn exit(pid: u32, status: ExitStatus) {
    let how = match status {
        ExitStatus::Exited(code) => format!("code={code}"),
        ExitStatus::Signaled { signal, .. } => format!("signal={signal}"),
        // Only bare processes fault without a signal; report it as the
        // SIGSEGV it would be.
        ExitStatus::Faulted(_) => format!("signal={}", crate::linux::SIGSEGV),
    };
    record(&format!("POLY-EXIT: pid={pid} {how}\n"));
}

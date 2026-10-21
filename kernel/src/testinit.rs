//! The kernel's init for traced test runs (ADR 0064): with
//! `poly.test.capture`, the first program does not run as pid 1 but as the
//! child of an init the kernel provides, as a program under a tracer on
//! Linux is a child of the tracer, so it gets a parent (`getppid` is not
//! 0), a session it does not lead and a process group of its own.
//!
//! The init is pid 1 in the process tree with no threads of its own: the
//! kernel thread that started the program does its work. It waits for the
//! program and reaps every child it has, the program and the orphans the
//! tree gives it (an init reaps its children, as wait(2) describes for
//! orphans). It is not a process record, so `kill(1, ...)` finds nothing.
//!
//! At the end the kernel also reports the output a test run asked for by
//! file (`poly.test.stdout=<path>`, `poly.test.stderr=<path>`): the app
//! tests' launcher sends the app's output into regular files, and what
//! those files hold is the run's output, as on Linux.

use crate::boot_info::BootInfo;
use crate::linux::task;
use crate::process::{self, ExitStatus, Process};
use crate::{kprintln, testrun};
use poly_personality_linux::abi::signal::SIGCHLD;
use poly_personality_linux::task::tree::{Clones, Poll, Selector, WaitFilter};

/// The init's pid.
pub const INIT_PID: u32 = 1;

/// Kernel arguments: files whose contents are the run's standard output
/// and standard error, reported when the program has ended.
pub const STDOUT_FILE_ARG: &str = "poly.test.stdout";
pub const STDERR_FILE_ARG: &str = "poly.test.stderr";

/// The most bytes reported from each of those files.
const OUTPUT_FILE_LIMIT: usize = 16 << 20;

/// Make the init: pid 1 in the process tree, the root of its own session
/// and process group. Returns false (no init) if pid 1 was taken already.
pub fn start() -> bool {
    let pid = process::alloc_pid();
    if pid != INIT_PID {
        kprintln!("init: pid {pid}, not {INIT_PID}: the program runs without the kernel's init");
        return false;
    }
    task::with_tree(|t| t.insert_root(INIT_PID as i32, 0));
    true
}

/// `program`, not started yet, becomes the init's child, as fork(2) would
/// make it (exit signal `SIGCHLD`, the init's session), in a process group
/// of its own (setpgid(2) by the init before the program runs).
pub fn adopt(program: &Process) {
    let pid = program.pid as i32;
    let init = INIT_PID as i32;
    task::with_tree(|t| {
        t.remove(pid);
        t.insert_child(init, init, pid, SIGCHLD, 0);
        if let Err(e) = t.setpgid(init, pid, 0) {
            kprintln!("init: setpgid({pid}) failed: {e:?}");
        }
    });
}

/// Wait for `program` to end, reaping the init's children meanwhile and
/// once more after it ended; returns how it ended.
pub fn wait(program: &Process) -> ExitStatus {
    loop {
        let seen = task::generation();
        reap_children();
        if program.has_ended() {
            break;
        }
        process::wait_event(|| task::generation() != seen || program.has_ended());
    }
    let status = program.wait();
    reap_children();
    status
}

/// Reap every child of the init that has ended.
fn reap_children() {
    let any = WaitFilter {
        selector: Selector::Any,
        ended: true,
        stopped: false,
        continued: false,
        consume: true,
        clones: Clones::Include,
    };
    while let Poll::Ready(_) = task::with_tree(|t| t.poll(INIT_PID as i32, &any)) {}
}

/// `POLY-OUT` records for the files `poly.test.stdout` and
/// `poly.test.stderr` name, if given.
pub fn report_output_files(boot: &BootInfo) {
    for (arg, stream) in [(STDOUT_FILE_ARG, 1), (STDERR_FILE_ARG, 2)] {
        let Some(path) = boot.arg(arg) else { continue };
        match crate::fs::Files::new().read_to_end(path.as_bytes(), OUTPUT_FILE_LIMIT) {
            Ok((bytes, more)) => {
                testrun::output(stream, &bytes);
                if more {
                    kprintln!("init: {path}: only the first {OUTPUT_FILE_LIMIT} bytes reported");
                }
            }
            Err(e) => kprintln!("init: {path}: {}", e.name()),
        }
    }
}

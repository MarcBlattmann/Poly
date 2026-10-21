//! The kernel's init for traced test runs (ADR 0064): with
//! `poly.test.capture`, the first program does not run as pid 1 but as the
//! child of an init the kernel provides, as a program under a tracer on
//! Linux is a child of the tracer, so it gets a parent (`getppid` is not
//! 0), a session it does not lead and a process group of its own.
//!
//! The init is a Linux process record, pid 1, listed in the process table
//! and the root of the process tree, with no user thread of its own: the
//! kernel thread that started the program does its work. So lookups by
//! pid find it (`kill(1, 0)`, `getpgid(1)`, `getsid(1)`, `/proc/1`,
//! `SCM_CREDENTIALS` naming pid 1), and signals sent to it follow init's
//! rule (kill(2): only those it has a handler for are delivered; it has
//! none, so they are dropped). It waits for the program and reaps every
//! child it has, the program and the orphans the tree gives it (an init
//! reaps its children, as wait(2) describes for orphans).
//!
//! At the end the kernel also reports the output a test run asked for by
//! file (`poly.test.stdout=<path>`, `poly.test.stderr=<path>`): the app
//! tests' launcher sends the app's output into regular files, and what
//! those files hold is the run's output, as on Linux.

use crate::boot_info::BootInfo;
use crate::linux::task;
use crate::process::{self, ExitStatus, Process};
use crate::sync::SpinLock;
use crate::{kprintln, testrun};
use alloc::sync::Arc;
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

/// The init's process record, kept alive for the run (the process table
/// holds it weakly).
static INIT: SpinLock<Option<Arc<Process>>> = SpinLock::new(None);

/// Reserve pid 1 for the init. Returns false (no init) if pid 1 was taken
/// already.
pub fn start() -> bool {
    let pid = process::alloc_pid();
    if pid != INIT_PID {
        kprintln!("init: pid {pid}, not {INIT_PID}: the program runs without the kernel's init");
        return false;
    }
    true
}

/// Make the init (a Linux process of `program`'s format, root of its own
/// session and process group, named `init`, with the console on fds 0-2),
/// and make `program`, not started yet, its child, as fork(2) would make it
/// (exit signal `SIGCHLD`, the init's session), in a process group of its
/// own (setpgid(2) by the init before the program runs).
pub fn adopt(program: &Process) {
    // `LinuxState::new` makes pid 1 a root of the process tree.
    let record = Arc::new(Process::new_linux(INIT_PID, 0, program.format, false));
    if let Some(linux) = &record.linux {
        let comm = poly_personality_linux::sys::prctl::comm_from_path(b"init");
        linux.with(|p| p.thread_mut(INIT_PID as i32).comm = comm);
    }
    process::register_threadless(&record);
    *INIT.lock() = Some(record);
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

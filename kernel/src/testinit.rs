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
//!
//! The program runs as the identity the diff runner's reference ran it as
//! (ADR 0078): `poly.test.uid=R[,E,S]`, `poly.test.gid=R[,E,S]`,
//! `poly.test.groups=G1,G2,...` and `poly.test.umask=OCTAL` give its
//! credentials (what root makes of itself with setgroups, setresgid and
//! setresuid: `Creds::login`, so a user other than root has no
//! capabilities) and its umask before its first instruction runs. The
//! init gets the same credentials: on Linux the program's parent is the
//! tracer, which runs as the same user. Without them the program runs as
//! root with umask 022.

use crate::boot_info::BootInfo;
use crate::linux::task;
use crate::process::{self, ExitStatus, Process};
use crate::sync::SpinLock;
use crate::{kprintln, testrun};
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_personality_linux::abi::signal::SIGCHLD;
use poly_personality_linux::cred::Creds;
use poly_personality_linux::task::tree::{Clones, Poll, Selector, WaitFilter};

/// Kernel arguments: the program's user ids, group ids, supplementary
/// groups and umask (see the module docs).
pub const UID_ARG: &str = "poly.test.uid";
pub const GID_ARG: &str = "poly.test.gid";
pub const GROUPS_ARG: &str = "poly.test.groups";
pub const UMASK_ARG: &str = "poly.test.umask";

/// Who the first program runs as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub creds: Creds,
    /// `None`: the default, 022.
    pub umask: Option<u16>,
}

/// `R` or `R,E,S`.
fn id_triple(s: &str) -> Option<[u32; 3]> {
    let ids: Vec<u32> = s.split(',').map(|v| v.parse().ok()).collect::<Option<_>>()?;
    match ids.as_slice() {
        [id] => Some([*id; 3]),
        [r, e, s] => Some([*r, *e, *s]),
        _ => None,
    }
}

/// The identity the kernel arguments give the first program (`uid`,
/// `gid`, `groups`, `umask`: the values of [`UID_ARG`] ...), `None` if
/// they give none: an absent id is 0, absent groups none.
pub fn identity_of(
    uid: Option<&str>,
    gid: Option<&str>,
    groups: Option<&str>,
    umask: Option<&str>,
) -> Result<Option<Identity>, String> {
    if uid.is_none() && gid.is_none() && groups.is_none() && umask.is_none() {
        return Ok(None);
    }
    let bad = |arg: &str, v: &str| format!("bad {arg}={v}");
    let uids = uid.map_or(Some([0; 3]), id_triple).ok_or_else(|| bad(UID_ARG, uid.unwrap_or("")))?;
    let gids = gid.map_or(Some([0; 3]), id_triple).ok_or_else(|| bad(GID_ARG, gid.unwrap_or("")))?;
    let list: Vec<u32> = match groups {
        None | Some("") => Vec::new(),
        Some(g) => g.split(',').map(|v| v.parse().ok()).collect::<Option<_>>().ok_or_else(|| bad(GROUPS_ARG, g))?,
    };
    let umask = match umask {
        None => None,
        Some(m) => Some(u16::from_str_radix(m, 8).ok().filter(|m| *m <= 0o777).ok_or_else(|| bad(UMASK_ARG, m))?),
    };
    let creds = Creds::login(uids, gids, &list).map_err(|e| format!("credentials refused: {e:?}"))?;
    Ok(Some(Identity { creds, umask }))
}

/// [`identity_of`] the boot's command line.
pub fn identity(boot: &BootInfo) -> Result<Option<Identity>, String> {
    identity_of(boot.arg(UID_ARG), boot.arg(GID_ARG), boot.arg(GROUPS_ARG), boot.arg(UMASK_ARG))
}

/// Give `program` (prepared, not started) and the init, if there is one,
/// `id`'s credentials, and `program` its umask.
pub fn apply(program: &Process, id: &Identity) {
    let set = |p: &Process, tid: u32| {
        if let Some(linux) = &p.linux {
            linux.with(|s| s.thread_mut(tid as i32).creds = id.creds.clone());
        }
    };
    set(program, program.pid);
    let init = INIT.lock().clone();
    if let Some(init) = init {
        set(&init, INIT_PID);
    }
    if let Some(mask) = id.umask {
        program.files.umask(mask);
    }
}

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

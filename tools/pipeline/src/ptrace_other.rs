//! The tracer on systems other than Linux. The real one (`ptrace.rs`) is
//! ptrace(2) with Linux's options, mount and user namespaces; none of that
//! exists on macOS, where the pipeline runs for the macOS oracle (ADR 0182).
//! The tool still builds there: tracing, the diff runner's reference runs
//! and app tests report that they need Linux.

use crate::trace::Trace;
use crate::util::Result;
use std::path::{Path, PathBuf};
use std::time::Duration;

const NEEDS_LINUX: &str = "the system call tracer needs Linux (ptrace and namespaces); this host is not Linux";

/// The running system, e.g. `darwin 27.0.0`.
pub fn host_system() -> String {
    let release = std::process::Command::new("uname")
        .arg("-r")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    format!("{} {release}", std::env::consts::OS)
}

/// Mark every descriptor from 3 up close-on-exec (see `ptrace.rs`): one by
/// one, up to the descriptor limit. Only async-signal-safe calls.
pub(crate) fn cloexec_from_3() {
    // SAFETY: `lim` is a valid rlimit for getrlimit to fill.
    let mut lim: libc::rlimit = unsafe { std::mem::zeroed() };
    // SAFETY: as above.
    let max = if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) } == 0 {
        lim.rlim_cur.min(1 << 16) as libc::c_int
    } else {
        1024
    };
    for fd in 3..max {
        // SAFETY: F_SETFD on a descriptor that may not be open (EBADF,
        // ignored) only sets its flags.
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
    }
}

pub fn trace(_program: &Path, _test: &str, _timeout: Duration) -> Result<Trace> {
    Err(NEEDS_LINUX.into())
}

pub fn trace_in(
    _root: &Path,
    _overlays: &[(PathBuf, String)],
    _program: &str,
    _test: &str,
    _timeout: Duration,
) -> Result<Trace> {
    Err(NEEDS_LINUX.into())
}

//! System information, time and randomness: uname(2), clock_gettime(2),
//! getrandom(2).

use crate::abi::{Timespec, clock as id, random, utsname_bytes};
use crate::admin::SystemName;
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::kernel::{Clock, LinuxKernel, RandomMode};

/// `uname`'s `sysname`. Programs compare it with "Linux" to pick their
/// Linux code paths, which is exactly what this personality is for.
pub const UNAME_SYSNAME: &str = "Linux";

/// `uname`'s `release`.
///
/// It must look like a Linux release: libraries parse it as
/// `major.minor.patch` and some refuse to run below a minimum (static glibc
/// binaries carry "for GNU/Linux 3.2.0" in their ABI note). 6.1 is a
/// long-term-support release that every current C library and Rust's std
/// support. Programs must still probe each call (and fall back on
/// `ENOSYS`), which they do because real kernels can be configured without
/// many calls. The `-poly` suffix, like a distribution suffix, lets people
/// and bug reports see that this is Poly.
pub const UNAME_RELEASE: &str = "6.1.0-poly";

/// `uname`'s `version` (free-form on Linux: build number and options).
pub const UNAME_VERSION: &str = "#1 Poly";

/// `uname`'s `machine`.
pub const UNAME_MACHINE: &str = "x86_64";

/// uname(buf): fills `struct new_utsname`; `EFAULT` if `buf` cannot be
/// written ("EFAULT buf is not valid").
pub fn uname(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    // The names sethostname(2) and setdomainname(2) set, where the kernel
    // keeps them (ADR 0051).
    let names = k.admin().map(|s| (s.system_name(SystemName::Host), s.system_name(SystemName::Domain)));
    let (host, domain) = match &names {
        Some((h, d)) => (h.as_slice(), d.as_slice()),
        None => (k.hostname(), k.domainname()),
    };
    let bytes = utsname_bytes([
        UNAME_SYSNAME.as_bytes(),
        host,
        UNAME_RELEASE.as_bytes(),
        UNAME_VERSION.as_bytes(),
        UNAME_MACHINE.as_bytes(),
        domain,
    ]);
    k.copy_to_user(a.ptr(0), &bytes)?;
    Ok(0)
}

/// The kernel clock behind a clock id, or `None` for ids that are not
/// supported.
///
/// - `CLOCK_REALTIME`, `CLOCK_REALTIME_COARSE`: the wall clock (the coarse
///   variant may be less precise; returning the precise value is allowed).
/// - `CLOCK_MONOTONIC`, `CLOCK_MONOTONIC_COARSE`, `CLOCK_MONOTONIC_RAW`,
///   `CLOCK_BOOTTIME`: the monotonic clock. Poly never slews it (so it is
///   also "raw") and never suspends (so boot time equals monotonic time).
/// - `CLOCK_PROCESS_CPUTIME_ID`, `CLOCK_THREAD_CPUTIME_ID`: CPU time, as
///   far as the kernel tracks it.
///
/// Everything else gets `EINVAL`: the alarm clocks and `CLOCK_TAI` for now,
/// and negative ids (other processes' CPU clocks and dynamic clocks), which
/// clock_gettime(2) says fail when "the dynamic clock ID does not refer to
/// a valid instance".
pub const fn clock_for(clockid: i32) -> Option<Clock> {
    match clockid {
        id::CLOCK_REALTIME | id::CLOCK_REALTIME_COARSE => Some(Clock::Realtime),
        id::CLOCK_MONOTONIC | id::CLOCK_MONOTONIC_COARSE | id::CLOCK_MONOTONIC_RAW | id::CLOCK_BOOTTIME => {
            Some(Clock::Monotonic)
        }
        id::CLOCK_PROCESS_CPUTIME_ID => Some(Clock::ProcessCpuTime),
        id::CLOCK_THREAD_CPUTIME_ID => Some(Clock::ThreadCpuTime),
        _ => None,
    }
}

/// clock_gettime(clockid, tp): `EINVAL` for unsupported clocks, `EFAULT`
/// if `tp` cannot be written.
pub fn clock_gettime(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let clock = clock_for(a.int(0)).ok_or(Errno::EINVAL)?;
    let now: Timespec = k.clock(clock)?;
    k.copy_to_user(a.ptr(1), &now.to_bytes())?;
    Ok(0)
}

/// The most bytes one getrandom call returns: getrandom(2), "a maximum of
/// 32Mi-1 bytes is returned by a single call ... where int has a size of
/// 32 bits".
pub const GETRANDOM_MAX: u64 = 32 * 1024 * 1024 - 1;

/// getrandom(buf, buflen, flags)
///
/// getrandom(2): `EINVAL` for flags other than `GRND_NONBLOCK`,
/// `GRND_RANDOM` and `GRND_INSECURE`; `EAGAIN` with `GRND_NONBLOCK` before
/// the source is ready; `EFAULT` for an unwritable buffer. Returns the
/// bytes copied, at most [`GETRANDOM_MAX`]. `GRND_RANDOM` has no effect:
/// `linux/random.h` documents it as "No effect" (both sources are the same
/// generator on current Linux).
///
/// Bytes are produced 256 at a time (getrandom(2): requests of up to 256
/// bytes always complete), so a failure after the first block returns the
/// count so far, as the man page allows for large requests.
pub fn getrandom(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let buf = a.ptr(0);
    let len = a.ulong(1).min(GETRANDOM_MAX);
    let flags = a.uint(2);
    let known = random::GRND_NONBLOCK | random::GRND_RANDOM | random::GRND_INSECURE;
    if flags & !known != 0 {
        return Err(Errno::EINVAL);
    }
    let mode = if flags & random::GRND_INSECURE != 0 {
        RandomMode::Insecure
    } else if flags & random::GRND_NONBLOCK != 0 {
        RandomMode::NonBlock
    } else {
        RandomMode::Block
    };
    let mut block = [0u8; 256];
    let mut done: u64 = 0;
    while done < len {
        let want = (len - done).min(block.len() as u64) as usize;
        let step = k.random_bytes(&mut block[..want], mode).and_then(|n| {
            let n = n.min(want);
            let at = buf.offset(done).ok_or(Errno::EFAULT)?;
            k.copy_to_user(at, &block[..n])?;
            Ok(n)
        });
        match step {
            Ok(0) => break,
            Ok(n) => done += n as u64,
            Err(e) if done == 0 => return Err(e),
            Err(_) => break,
        }
    }
    Ok(done)
}

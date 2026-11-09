//! BSD process accounting, acct(2) (ADR 0135): a record of each process
//! that ends, appended to the file the pid namespace's `acct` named.
//!
//! Accounting belongs to a pid namespace. A process that ends is
//! recorded in the file of its own namespace and of every namespace
//! above it that has one, with the process ids as that namespace sees
//! them (`ac_ppid` is 0 when the parent is outside it). The file is
//! written through a [`Sink`] the kernel opens for appending.

use crate::kernel::{Clock, CpuTimesOf, LinuxKernel};
use crate::ns::lock::Mutex;
use crate::ns::pid::{self, PidNsRef};
use crate::signal::deliver::with_own;
use crate::task::tree::Termination;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;

/// Where records go: the file accounting was turned on for.
pub trait Sink: Send + Sync {
    /// Append one record. A failing write (a full disk) loses the record.
    fn append(&self, record: &[u8]);
}

/// `sizeof(struct acct_v3)` (`linux/acct.h`).
pub const RECORD_SIZE: usize = 64;
/// `ac_version` of `struct acct_v3`.
pub const VERSION: u8 = 3;

/// `ac_flag` bits (`linux/acct.h`).
pub mod flag {
    /// Forked but did not exec.
    pub const AFORK: u8 = 0x01;
    /// Dumped core.
    pub const ACORE: u8 = 0x08;
    /// Killed by a signal.
    pub const AXSIG: u8 = 0x10;
}

/// The clock ticks per second the record's times count in (`USER_HZ`).
const TICKS_PER_SEC: u64 = 100;

/// One accounting record (`struct acct_v3`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Record {
    pub flag: u8,
    pub tty: u16,
    /// The wait status of the end (0 in a record made by acct(2) itself).
    pub exit_code: u32,
    pub uid: u32,
    pub gid: u32,
    pub pid: u32,
    pub ppid: u32,
    /// Start, seconds since the epoch.
    pub btime: u32,
    /// Elapsed time in clock ticks.
    pub etime: f32,
    pub utime_ticks: u64,
    pub stime_ticks: u64,
    pub minflt: u64,
    pub comm: [u8; 16],
}

/// A `comp_t`: a 13-bit mantissa and a 3-bit base-8 exponent.
pub fn comp_t(mut value: u64) -> u16 {
    let mut exp = 0u16;
    let mut round = false;
    while value > 0x1fff && exp < 7 {
        round = value & 4 != 0;
        value >>= 3;
        exp += 1;
    }
    if round {
        value += 1;
        if value > 0x1fff && exp < 7 {
            value >>= 3;
            exp += 1;
        }
    }
    (exp << 13) | value.min(0x1fff) as u16
}

impl Record {
    /// The 64 bytes of the file.
    pub fn to_bytes(&self) -> [u8; RECORD_SIZE] {
        let mut b = [0u8; RECORD_SIZE];
        b[0] = self.flag;
        b[1] = VERSION;
        b[2..4].copy_from_slice(&self.tty.to_le_bytes());
        b[4..8].copy_from_slice(&self.exit_code.to_le_bytes());
        b[8..12].copy_from_slice(&self.uid.to_le_bytes());
        b[12..16].copy_from_slice(&self.gid.to_le_bytes());
        b[16..20].copy_from_slice(&self.pid.to_le_bytes());
        b[20..24].copy_from_slice(&self.ppid.to_le_bytes());
        b[24..28].copy_from_slice(&self.btime.to_le_bytes());
        b[28..32].copy_from_slice(&self.etime.to_bits().to_le_bytes());
        b[32..34].copy_from_slice(&comp_t(self.utime_ticks).to_le_bytes());
        b[34..36].copy_from_slice(&comp_t(self.stime_ticks).to_le_bytes());
        // ac_mem, ac_io, ac_rw: not accounted.
        b[42..44].copy_from_slice(&comp_t(self.minflt).to_le_bytes());
        // ac_majflt, ac_swaps: none.
        b[48..64].copy_from_slice(&self.comm);
        b
    }
}

/// The wait status and flags of how a process ended.
pub fn termination(how: Termination) -> (u32, u8) {
    match how {
        Termination::Exited(code) => (u32::from(code) << 8, 0),
        Termination::Killed { signal, core } => {
            (signal | if core { 0x80 } else { 0 }, flag::AXSIG | if core { flag::ACORE } else { 0 })
        }
    }
}

/// A namespace's accounting files, by namespace (0: the initial one).
static FILES: Mutex<BTreeMap<u32, Arc<dyn Sink>>> = Mutex::new(BTreeMap::new());

/// The key of a pid namespace.
pub fn key(ns: &PidNsRef) -> u32 {
    ns.as_ref().map_or(0, |n| n.inum())
}

/// Accounting on in namespace `ns`: the file records go to.
pub fn current(ns: &PidNsRef) -> Option<Arc<dyn Sink>> {
    FILES.lock().get(&key(ns)).cloned()
}

/// Turn accounting on (or switch it) for `ns`: the file it had before.
pub fn set(ns: &PidNsRef, sink: Arc<dyn Sink>) -> Option<Arc<dyn Sink>> {
    FILES.lock().insert(key(ns), sink)
}

/// Turn accounting off for `ns`: the file it had.
pub fn clear(ns: &PidNsRef) -> Option<Arc<dyn Sink>> {
    FILES.lock().remove(&key(ns))
}

/// A process is ending: append its record to the accounting file of every
/// namespace it is in that has one.
pub fn process_exited(k: &mut dyn LinuxKernel, how: Termination) {
    let mut ns = k.pid_ns();
    loop {
        if let Some(sink) = current(&ns) {
            sink.append(&build(k, &ns, Some(how)).to_bytes());
        }
        match ns {
            None => break,
            Some(n) => ns = n.parent().clone(),
        }
    }
}

/// The namespace's init ended: accounting stops, the file is closed.
pub fn forget(ns: &PidNsRef) {
    clear(ns);
}

/// The record of the calling process as `ns` sees it, `how` it ended
/// (`None`: acct(2) writes it when it switches or turns accounting off,
/// with status 0 and no flags).
fn build(k: &mut dyn LinuxKernel, ns: &PidNsRef, how: Option<Termination>) -> Record {
    let pid = k.pid();
    let ppid = crate::task::with_tree(k, |t| t.parent(pid)).ok().flatten().unwrap_or(0);
    let (exit_code, mut flags) = how.map_or((0, 0), termination);
    let (started, comm, forked, creds) =
        with_own(k, |p| (p.started_ns, p.last_comm(), p.forked, p.process_creds(pid).cloned()));
    if how.is_some() && forked {
        flags |= flag::AFORK;
    }
    let now = k.clock(Clock::Monotonic).map_or(0, |t| (t.sec as u64) * 1_000_000_000 + t.nsec as u64);
    let real = k.clock(Clock::Realtime).map_or(0, |t| t.sec as u64);
    let elapsed_ns = now.saturating_sub(started);
    let times = k.cpu_times(CpuTimesOf::Process).unwrap_or_default();
    let faults = k.memory_counts(CpuTimesOf::Process).minor_faults;
    let (uid, gid) = creds.map_or((0, 0), |c| (c.uid, c.gid));
    Record {
        flag: flags,
        tty: 0,
        exit_code,
        uid,
        gid,
        pid: pid::local_or_zero(ns, pid) as u32,
        ppid: pid::local_or_zero(ns, ppid) as u32,
        btime: real.saturating_sub(elapsed_ns / 1_000_000_000) as u32,
        etime: (elapsed_ns as f64 / (1_000_000_000.0 / TICKS_PER_SEC as f64)) as f32,
        utime_ticks: times.user_ns / (1_000_000_000 / TICKS_PER_SEC),
        stime_ticks: times.system_ns / (1_000_000_000 / TICKS_PER_SEC),
        minflt: faults,
        comm,
    }
}

/// acct(2)'s own record for the caller, when it switches or turns
/// accounting off: written to the file being left.
pub fn caller_record(k: &mut dyn LinuxKernel, ns: &PidNsRef) -> Record {
    build(k, ns, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comp_t_encoding() {
        assert_eq!(comp_t(0), 0);
        assert_eq!(comp_t(100), 100);
        assert_eq!(comp_t(0x1fff), 0x1fff);
        // 8192 needs the next exponent: mantissa 1024, exponent 1.
        assert_eq!(comp_t(8192), (1 << 13) | 1024);
        assert_eq!(comp_t(u64::MAX) >> 13, 7);
    }

    #[test]
    fn record_layout() {
        let mut comm = [0u8; 16];
        comm[..4].copy_from_slice(b"true");
        let r = Record {
            flag: flag::AFORK | flag::AXSIG,
            tty: 5,
            exit_code: 7 << 8,
            uid: 1000,
            gid: 1001,
            pid: 2,
            ppid: 1,
            btime: 1_700_000_000,
            etime: 12.5,
            utime_ticks: 3,
            stime_ticks: 4,
            minflt: 9,
            comm,
        };
        let b = r.to_bytes();
        assert_eq!(b.len(), 64);
        assert_eq!((b[0], b[1]), (0x11, 3));
        assert_eq!(u16::from_le_bytes([b[2], b[3]]), 5);
        assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), 7 << 8);
        assert_eq!(u32::from_le_bytes(b[16..20].try_into().unwrap()), 2);
        assert_eq!(u32::from_le_bytes(b[20..24].try_into().unwrap()), 1);
        assert_eq!(f32::from_bits(u32::from_le_bytes(b[28..32].try_into().unwrap())), 12.5);
        assert_eq!(u16::from_le_bytes([b[32], b[33]]), 3);
        assert_eq!(&b[48..52], b"true");
    }

    #[test]
    fn how_a_process_ended() {
        assert_eq!(termination(Termination::Exited(7)), (7 << 8, 0));
        assert_eq!(termination(Termination::Killed { signal: 11, core: false }), (11, flag::AXSIG));
        assert_eq!(termination(Termination::Killed { signal: 11, core: true }), (11 | 0x80, flag::AXSIG | flag::ACORE));
    }
}

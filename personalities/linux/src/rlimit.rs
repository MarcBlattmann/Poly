//! Resource limits (getrlimit(2)): what each process may use, kept in its
//! [`LinuxProcess`](crate::LinuxProcess), inherited by fork(2) and kept
//! across execve(2).
//!
//! Only `RLIMIT_NOFILE` is enforced so far (the kernel's descriptor table
//! refuses descriptors at or above the soft limit with `EMFILE`); the
//! others are stored and reported, and `RLIMIT_SIGPENDING` mirrors the
//! signal queue limit.

use crate::fs::rlimit::*;

/// One limit: soft (`rlim_cur`) and hard (`rlim_max`), in the resource's
/// unit; [`RLIM_INFINITY`] for none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rlimit {
    pub cur: u64,
    pub max: u64,
}

impl Rlimit {
    pub const UNLIMITED: Rlimit = Rlimit { cur: RLIM_INFINITY, max: RLIM_INFINITY };

    /// `struct rlimit64` / x86-64 `struct rlimit`: two `u64`s.
    pub const SIZE: usize = 16;

    pub fn to_bytes(self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[..8].copy_from_slice(&self.cur.to_le_bytes());
        b[8..].copy_from_slice(&self.max.to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8; Self::SIZE]) -> Rlimit {
        let word = |i: usize| u64::from_le_bytes(b[i..i + 8].try_into().expect("8 bytes"));
        Rlimit { cur: word(0), max: word(8) }
    }
}

/// The soft and hard `RLIMIT_NOFILE` a process starts with: the soft one
/// is the kernel descriptor table's default size; the hard one bounds how
/// far a program may raise it without privileges, kept small because some
/// programs close every descriptor up to it.
pub const DEFAULT_NOFILE: Rlimit = Rlimit { cur: 1024, max: 4096 };

/// The default stack limit: 8 MiB soft, no hard limit.
pub const DEFAULT_STACK: Rlimit = Rlimit { cur: 8 << 20, max: RLIM_INFINITY };

/// Every limit of one process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceLimits {
    limits: [Rlimit; RLIM_NLIMITS as usize],
}

impl Default for ResourceLimits {
    /// What the first process starts with. These are Poly's choices (the
    /// manual pages give no defaults): no limit on CPU time, file size,
    /// data, address space, resident set, locks and real-time CPU time;
    /// [`DEFAULT_STACK`]; no core dumps by default (soft 0); 8 MiB of
    /// locked memory; [`DEFAULT_NOFILE`]; 4096 processes and queued
    /// signals (the signal queue's limit); mq_overview(7)'s 819200 bytes of
    /// message queues; nice and real-time priority ceilings 0.
    fn default() -> Self {
        let mut limits = [Rlimit::UNLIMITED; RLIM_NLIMITS as usize];
        limits[RLIMIT_STACK as usize] = DEFAULT_STACK;
        limits[RLIMIT_CORE as usize] = Rlimit { cur: 0, max: RLIM_INFINITY };
        limits[RLIMIT_NPROC as usize] = Rlimit { cur: 4096, max: 4096 };
        limits[RLIMIT_NOFILE as usize] = DEFAULT_NOFILE;
        limits[RLIMIT_MEMLOCK as usize] = Rlimit { cur: 8 << 20, max: 8 << 20 };
        let queue = crate::signal::state::DEFAULT_QUEUE_LIMIT as u64;
        limits[RLIMIT_SIGPENDING as usize] = Rlimit { cur: queue, max: queue };
        limits[RLIMIT_MSGQUEUE as usize] = Rlimit { cur: 819_200, max: 819_200 };
        limits[RLIMIT_NICE as usize] = Rlimit { cur: 0, max: 0 };
        limits[RLIMIT_RTPRIO as usize] = Rlimit { cur: 0, max: 0 };
        ResourceLimits { limits }
    }
}

impl ResourceLimits {
    pub fn new() -> Self {
        Self::default()
    }

    /// The limit of `resource` (`None` beyond `RLIM_NLIMITS`).
    pub fn get(&self, resource: u32) -> Option<Rlimit> {
        self.limits.get(resource as usize).copied()
    }

    /// Replace a limit (the caller has checked it).
    pub fn set(&mut self, resource: u32, limit: Rlimit) {
        if let Some(slot) = self.limits.get_mut(resource as usize) {
            *slot = limit;
        }
    }
}

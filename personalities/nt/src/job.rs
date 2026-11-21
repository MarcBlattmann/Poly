//! Job objects (ADR 0169): `NtCreateJobObject`, `NtOpenJobObject`,
//! `NtAssignProcessToJobObject`, `NtTerminateJobObject`,
//! `NtQueryInformationJobObject`, `NtSetInformationJobObject` and
//! `NtIsProcessInJob` (Learn: "Job Objects", "JOBOBJECT_BASIC_LIMIT_INFORMATION",
//! "JOBOBJECT_ASSOCIATE_COMPLETION_PORT"; the sheets of `CreateJobObjectW`,
//! `AssignProcessToJobObject`, `QueryInformationJobObject`,
//! `SetInformationJobObject`, `IsProcessInJob`).
//!
//! A job is a set of processes. A process joins by `NtAssignProcessToJobObject`
//! or by being created inside one (the job list of the process attributes,
//! or its creator's jobs, unless it breaks away); it can be in several jobs
//! (a hierarchy is flattened: each job counts it). The members that exited
//! are remembered for `NtIsProcessInJob` until their process object goes.
//! Limits are recorded and read back; only the active-process limit, the
//! breakaway flags and kill-on-job-close are enforced. A job with a
//! completion port posts the `JOB_OBJECT_MSG_*` packets there: the key of
//! the association, the process id as the context, the message as the byte
//! count.

use crate::abi;
use crate::io::iocp::{PacketPost, PortAssoc};
use crate::kernel::NtKernel;
use crate::objects::return_handle;
use crate::threads::{process_of, process_of_any};
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use poly_abi::SyscallRegs;
use poly_ntobj::access::*;
use poly_ntobj::status::*;
use poly_ntobj::{Handle, NewObject, NoParse, NtStatus, ObjectId, ObjectManager, ObjectType, ProcessId};

pub type Result<T> = core::result::Result<T, NtStatus>;

/// `JOB_OBJECT_LIMIT_*` (`winnt.h`).
pub const LIMIT_ACTIVE_PROCESS: u32 = 0x8;
pub const LIMIT_BREAKAWAY_OK: u32 = 0x800;
pub const LIMIT_SILENT_BREAKAWAY_OK: u32 = 0x1000;
pub const LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x2000;
/// `JOB_OBJECT_MSG_*` (`winnt.h`).
pub const MSG_ACTIVE_PROCESS_LIMIT: u64 = 3;
pub const MSG_ACTIVE_PROCESS_ZERO: u64 = 4;
pub const MSG_NEW_PROCESS: u64 = 6;
pub const MSG_EXIT_PROCESS: u64 = 7;
pub const MSG_ABNORMAL_EXIT_PROCESS: u64 = 8;
/// `STATUS_QUOTA_EXCEEDED` (`ntstatus.h`): `ERROR_NOT_ENOUGH_QUOTA`.
const STATUS_QUOTA_EXCEEDED: NtStatus = NtStatus(0xC000_0044);
/// `STATUS_PROCESS_IS_TERMINATING`'s neighbour the access check uses.
const PROCESS_SET_QUOTA: u32 = 0x0100;
const PROCESS_TERMINATE: u32 = 0x0001;
const PROCESS_QUERY_INFORMATION: u32 = 0x0400;

/// One job.
#[derive(Debug)]
pub struct Job {
    /// The job object (`None` only for the stand-in a `NULL` handle gets in a
    /// process that is in no job).
    pub object: Option<ObjectId>,
    /// The job this one was put below when a process that was in it was
    /// assigned to this one.
    pub parent: Option<u64>,
    /// The members still running (and those of the jobs below this one).
    pub active: BTreeSet<ProcessId>,
    pub total: u32,
    /// `TotalTerminatedProcesses`: stays 0, also after a terminate (the
    /// sheets: an ended member is not counted there).
    pub terminated: u32,
    /// `JOBOBJECT_BASIC_LIMIT_INFORMATION` as set (64 bytes).
    pub basic: [u8; 64],
    /// The rest of `JOBOBJECT_EXTENDED_LIMIT_INFORMATION` as set (80 bytes).
    pub extended_tail: [u8; 80],
    pub ui: u32,
    pub end_of_job: u32,
    /// The completion port and key (it holds a reference on the port object).
    pub port: Option<PortAssoc>,
    /// The process that made the job (`NtCreateJobObject`): a parent that puts its children
    /// in jobs of its own manages them (the out-of-memory policy's score, `crate::oomadj`).
    pub creator: Option<ProcessId>,
}

/// What a successful query answers.
#[derive(Debug, PartialEq, Eq)]
pub struct Reply {
    /// What is written to the caller's buffer.
    pub bytes: Vec<u8>,
    /// `STATUS_SUCCESS`, or `STATUS_BUFFER_OVERFLOW` for a list that did not fit.
    pub status: NtStatus,
    /// What `*ReturnLength` gets.
    pub returned: u32,
}

/// A refused query: the status, and the `*ReturnLength` it still sets.
#[derive(Debug, PartialEq, Eq)]
pub struct Refused {
    pub status: NtStatus,
    pub returned: Option<u32>,
}

/// `JOB_OBJECT_LIMIT_*` flags the basic structure accepts (`winnt.h`), and those of the
/// extended one.
const BASIC_FLAGS: u32 = 0xFF;
const EXTENDED_FLAGS: u32 = 0x3FFF;
const LIMIT_WORKINGSET: u32 = 0x1;
const LIMIT_PROCESS_TIME: u32 = 0x2;
const LIMIT_JOB_TIME: u32 = 0x4;
const LIMIT_AFFINITY: u32 = 0x10;
const LIMIT_PRIORITY_CLASS: u32 = 0x20;
const LIMIT_PRESERVE_JOB_TIME: u32 = 0x40;
const LIMIT_SCHEDULING_CLASS: u32 = 0x80;
const LIMIT_PROCESS_MEMORY: u32 = 0x100;
const LIMIT_JOB_MEMORY: u32 = 0x200;
const LIMIT_SUBSET_AFFINITY: u32 = 0x4000;
/// A job that has no scheduling class reads 5 (the sheets: `defaults`).
const DEFAULT_SCHEDULING_CLASS: u32 = 5;
/// `JOB_OBJECT_UILIMIT_*`: the bits `HANDLES` to `IME` (`winnt.h`).
const UI_LIMITS: u32 = 0x1FF;

/// The lengths of the query classes that answer all zero bytes for a fresh job.
const ZERO_CLASSES: &[(u32, &[usize])] = &[
    (10, &[4]),
    (12, &[48, 56]),
    (13, &[80, 88]),
    (15, &[8]),
    (16, &[4]),
    (17, &[8]),
    (18, &[16]),
    (20, &[40, 64]),
    (21, &[1]),
    (26, &[8]),
    (28, &[16, 40]),
    (29, &[8]),
    (30, &[16, 36]),
    (32, &[16]),
    (33, &[72]),
    (34, &[104]),
    (43, &[1]),
    (44, &[16]),
];

impl Job {
    pub fn new(object: Option<ObjectId>) -> Job {
        let mut basic = [0u8; 64];
        basic[60..64].copy_from_slice(&DEFAULT_SCHEDULING_CLASS.to_le_bytes());
        Job {
            object,
            parent: None,
            active: BTreeSet::new(),
            total: 0,
            terminated: 0,
            basic,
            extended_tail: [0; 80],
            ui: 0,
            end_of_job: 0,
            port: None,
            creator: None,
        }
    }

    pub fn limit_flags(&self) -> u32 {
        u32::from_le_bytes(self.basic[16..20].try_into().expect("4 bytes"))
    }

    pub fn active_limit(&self) -> u32 {
        u32::from_le_bytes(self.basic[40..44].try_into().expect("4 bytes"))
    }

    /// Answer `NtQueryInformationJobObject(class)` for a buffer of `len` bytes
    /// (specs/windows/ntdll.dll/NtQueryInformationJobObject.yaml: `exact_lengths`,
    /// `odd_lengths`, `process_list`, `defaults`).
    pub fn query(&self, class: u32, len: usize) -> core::result::Result<Reply, Refused> {
        let refuse = |status: NtStatus| Refused { status, returned: None };
        let mismatch = || refuse(STATUS_INFO_LENGTH_MISMATCH);
        let ok = |bytes: Vec<u8>| {
            let returned = bytes.len() as u32;
            Reply { bytes, status: STATUS_SUCCESS, returned }
        };
        let exact = |lens: &[usize]| if lens.contains(&len) { Ok(()) } else { Err(mismatch()) };
        match class {
            0 => Err(refuse(STATUS_INVALID_INFO_CLASS)),
            BASIC_ACCOUNTING | BASIC_AND_IO => {
                exact(&[if class == BASIC_ACCOUNTING { 48 } else { 96 }])?;
                let mut b = alloc::vec![0u8; len];
                b[36..40].copy_from_slice(&self.total.to_le_bytes());
                b[40..44].copy_from_slice(&(self.active.len() as u32).to_le_bytes());
                b[44..48].copy_from_slice(&self.terminated.to_le_bytes());
                Ok(ok(b))
            }
            BASIC_LIMIT => {
                exact(&[64])?;
                let mut b = self.basic;
                // The four bytes between ActiveProcessLimit and Affinity read zero.
                b[44..48].fill(0);
                Ok(ok(b.to_vec()))
            }
            EXTENDED_LIMIT => {
                exact(&[144, 152])?;
                let mut b = self.basic.to_vec();
                b[44..48].fill(0);
                b.extend_from_slice(&self.extended_tail);
                b.resize(len, 0);
                Ok(ok(b))
            }
            UI_RESTRICTIONS => {
                exact(&[4])?;
                Ok(ok(self.ui.to_le_bytes().to_vec()))
            }
            END_OF_JOB_TIME => {
                exact(&[4])?;
                Ok(ok(self.end_of_job.to_le_bytes().to_vec()))
            }
            PROCESS_ID_LIST => {
                if len < 16 {
                    return Err(mismatch());
                }
                let room = (len - 8) / 8;
                let ids: Vec<u64> = self.active.iter().take(room).map(|p| u64::from(p.0)).collect();
                let mut bytes = (self.active.len() as u32).to_le_bytes().to_vec();
                bytes.extend_from_slice(&(ids.len() as u32).to_le_bytes());
                for id in &ids {
                    bytes.extend_from_slice(&id.to_le_bytes());
                }
                let full = 8 + 8 * self.active.len() as u32;
                let status = if ids.len() < self.active.len() { STATUS_BUFFER_OVERFLOW } else { STATUS_SUCCESS };
                Ok(Reply { bytes, status, returned: full })
            }
            SECURITY_LIMIT => {
                if len < 40 {
                    return Err(mismatch());
                }
                Ok(ok(alloc::vec![0u8; len]))
            }
            ASSOCIATE_PORT => Err(if len == 16 { refuse(STATUS_INVALID_INFO_CLASS) } else { mismatch() }),
            GROUP_INFORMATION => {
                if len < 2 {
                    Err(mismatch())
                } else if !len.is_multiple_of(2) {
                    Err(refuse(STATUS_INVALID_PARAMETER))
                } else {
                    Ok(ok(Vec::new()))
                }
            }
            GROUP_INFORMATION_EX => match len {
                0 => Err(Refused { status: STATUS_BUFFER_TOO_SMALL, returned: Some(16) }),
                33.. => Err(mismatch()),
                16 | 32 => Ok(ok(alloc::vec![0u8; 16])),
                _ => Err(refuse(STATUS_INVALID_PARAMETER)),
            },
            31 => Err(if len >= 144 { refuse(STATUS_NOT_IMPLEMENTED) } else { mismatch() }),
            c => match ZERO_CLASSES.iter().find(|(z, _)| *z == c) {
                Some((_, lens)) => {
                    exact(lens)?;
                    Ok(ok(alloc::vec![0u8; len]))
                }
                None if c <= MAX_CLASS => Err(mismatch()),
                None => Err(refuse(STATUS_INVALID_INFO_CLASS)),
            },
        }
    }
}

/// What a set operation changes.
#[derive(Debug, PartialEq, Eq)]
pub enum SetOp {
    /// The basic (and, from the extended structure, the rest) limits.
    Limits {
        basic: [u8; 64],
        tail: Option<[u8; 80]>,
    },
    Ui(u32),
    EndOfJob(u32),
    /// `JOBOBJECT_ASSOCIATE_COMPLETION_PORT`: key and port handle.
    Port {
        key: u64,
        port: u64,
    },
    /// A class that is accepted and has no effect here.
    Accepted,
}

/// What `NtSetInformationJobObject` does with a class at one of its lengths.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SetRule {
    /// The structure is read and used (or ignored).
    Take,
    /// The structure is read and a zero one is `STATUS_INVALID_PARAMETER`.
    Invalid,
    /// A query-only class: `STATUS_INVALID_INFO_CLASS`.
    NoSet,
    /// `STATUS_NOT_SUPPORTED`.
    NotSupported,
}

/// The classes 1 to 35 that are not accepted at every length: their lengths and what happens at
/// them (every other length is `STATUS_INFO_LENGTH_MISMATCH`), as the visible test of the sheet
/// scanned them with a zeroed structure. Class 11 and class 14 take several lengths
/// ([`set_length`]); class 19 none.
const SET_CLASSES: &[(u32, &[usize], SetRule)] = &[
    (1, &[48], SetRule::NoSet),
    (2, &[64], SetRule::Take),
    (3, &[16], SetRule::NoSet),
    (4, &[4], SetRule::Take),
    (5, &[40], SetRule::NotSupported),
    (6, &[4], SetRule::Take),
    (7, &[16], SetRule::Take),
    (8, &[96], SetRule::NoSet),
    (9, &[144, 152], SetRule::Take),
    (10, &[4], SetRule::NoSet),
    (12, &[48, 56], SetRule::Take),
    (13, &[0], SetRule::NoSet),
    (15, &[8], SetRule::Invalid),
    (16, &[4], SetRule::Take),
    (17, &[8], SetRule::NoSet),
    (18, &[16], SetRule::Invalid),
    (20, &[0], SetRule::NoSet),
    (21, &[1], SetRule::Invalid),
    (22, &[1], SetRule::Invalid),
    (23, &[1], SetRule::Invalid),
    (24, &[8], SetRule::Invalid),
    (25, &[0], SetRule::Take),
    (26, &[8], SetRule::NoSet),
    (27, &[0], SetRule::Take),
    (28, &[16], SetRule::NoSet),
    (29, &[8], SetRule::NoSet),
    (30, &[0], SetRule::NoSet),
    (31, &[48, 96, 144], SetRule::Take),
    (32, &[16], SetRule::Invalid),
    (33, &[72], SetRule::Take),
    (34, &[104], SetRule::NoSet),
    (35, &[0], SetRule::Invalid),
];

/// The rule of `class` at `len`, or the status of a class or length it refuses
/// (specs/windows/ntdll.dll/NtSetInformationJobObject.yaml: `lengths`, `class_lengths`, and the
/// scan of the visible test `ntsijo_misc`).
fn set_rule(class: u32, len: usize) -> Result<SetRule> {
    match class {
        0 => Err(STATUS_INVALID_INFO_CLASS),
        // Group information: 2 bytes are taken, the other even lengths are invalid.
        GROUP_INFORMATION => match len {
            2 => Ok(SetRule::Take),
            4.. if len.is_multiple_of(2) => Ok(SetRule::Invalid),
            _ => Err(STATUS_INFO_LENGTH_MISMATCH),
        },
        GROUP_INFORMATION_EX if len.is_multiple_of(16) => Ok(SetRule::Take),
        GROUP_INFORMATION_EX => Err(STATUS_INFO_LENGTH_MISMATCH),
        // Silos and containers were not probed: not a class this set knows.
        c if c > 35 => Err(STATUS_INVALID_INFO_CLASS),
        c => match SET_CLASSES.iter().find(|(k, _, _)| *k == c) {
            Some((_, lens, rule)) if lens.contains(&len) => Ok(*rule),
            _ => Err(STATUS_INFO_LENGTH_MISMATCH),
        },
    }
}

/// The number of bytes `NtSetInformationJobObject` reads for `class` and `len`, or the status of a
/// class or length it refuses.
pub fn set_length(class: u32, len: usize) -> Result<usize> {
    match set_rule(class, len)? {
        SetRule::Take | SetRule::Invalid => Ok(len),
        SetRule::NoSet => Err(STATUS_INVALID_INFO_CLASS),
        SetRule::NotSupported => Err(STATUS_NOT_SUPPORTED),
    }
}

/// Check `data` (the bytes [`set_length`] asked for) and say what it changes.
pub fn parse_set(class: u32, data: &[u8]) -> Result<SetOp> {
    let u32_at = |o: usize| u32::from_le_bytes(data[o..o + 4].try_into().expect("4 bytes"));
    let u64_at = |o: usize| u64::from_le_bytes(data[o..o + 8].try_into().expect("8 bytes"));
    match class {
        BASIC_LIMIT | EXTENDED_LIMIT => parse_limits(class == EXTENDED_LIMIT, data),
        UI_RESTRICTIONS => {
            let v = u32_at(0);
            if v & !UI_LIMITS != 0 { Err(STATUS_INVALID_PARAMETER) } else { Ok(SetOp::Ui(v)) }
        }
        END_OF_JOB_TIME => {
            let v = u32_at(0);
            if v > 1 { Err(STATUS_INVALID_PARAMETER) } else { Ok(SetOp::EndOfJob(v)) }
        }
        ASSOCIATE_PORT => Ok(SetOp::Port { key: u64_at(0), port: u64_at(8) }),
        // Rate control and the other classes that refuse a zero structure.
        c if set_rule(c, data.len()) == Ok(SetRule::Invalid) && data.iter().all(|&b| b == 0) => {
            Err(STATUS_INVALID_PARAMETER)
        }
        _ => Ok(SetOp::Accepted),
    }
}

/// `JOBOBJECT_BASIC_LIMIT_INFORMATION` and `..._EXTENDED_...` checked and reduced to what
/// is stored: only the fields their flags select (the others read back 0; a fresh job's
/// scheduling class 5 stays when `SCHEDULING_CLASS` is not set).
fn parse_limits(extended: bool, data: &[u8]) -> Result<SetOp> {
    let u32_at = |o: usize| u32::from_le_bytes(data[o..o + 4].try_into().expect("4 bytes"));
    let u64_at = |o: usize| u64::from_le_bytes(data[o..o + 8].try_into().expect("8 bytes"));
    let flags = u32_at(16);
    let valid = if extended {
        EXTENDED_FLAGS | if flags & LIMIT_AFFINITY != 0 { LIMIT_SUBSET_AFFINITY } else { 0 }
    } else {
        BASIC_FLAGS
    };
    if flags & !valid != 0 {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let (per_process, per_job) = (u64_at(0), u64_at(8));
    let (min_ws, max_ws, active, affinity) = (u64_at(24), u64_at(32), u32_at(40), u64_at(48));
    let (priority, scheduling) = (u32_at(56), u32_at(60));
    if flags & LIMIT_PROCESS_TIME != 0 && per_process == 0 {
        return Err(STATUS_INVALID_PARAMETER);
    }
    if flags & LIMIT_WORKINGSET != 0 {
        let page = |v: u64| v.is_multiple_of(4096);
        if max_ws == 0 || min_ws > max_ws || !page(min_ws) || !page(max_ws) {
            return Err(STATUS_INVALID_PARAMETER);
        }
    }
    if flags & LIMIT_AFFINITY != 0 && (affinity == 0 || affinity >> 32 != 0) {
        return Err(STATUS_INVALID_PARAMETER);
    }
    if flags & LIMIT_PRIORITY_CLASS != 0 && priority > 6 {
        return Err(STATUS_INVALID_PARAMETER);
    }
    if flags & LIMIT_SCHEDULING_CLASS != 0 && scheduling > 9 {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let (process_memory, job_memory) = if extended { (u64_at(112), u64_at(120)) } else { (0, 0) };
    if flags & LIMIT_PROCESS_MEMORY != 0 && process_memory == 0 {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let mut basic = [0u8; 64];
    let put64 = |b: &mut [u8], o: usize, v: u64| b[o..o + 8].copy_from_slice(&v.to_le_bytes());
    let put32 = |b: &mut [u8], o: usize, v: u32| b[o..o + 4].copy_from_slice(&v.to_le_bytes());
    if flags & LIMIT_PROCESS_TIME != 0 {
        put64(&mut basic, 0, per_process);
    }
    if flags & LIMIT_JOB_TIME != 0 {
        put64(&mut basic, 8, per_job);
    }
    put32(&mut basic, 16, flags & !LIMIT_PRESERVE_JOB_TIME);
    if flags & LIMIT_WORKINGSET != 0 {
        put64(&mut basic, 24, min_ws);
        put64(&mut basic, 32, max_ws);
    }
    if flags & LIMIT_ACTIVE_PROCESS != 0 {
        put32(&mut basic, 40, active);
    }
    if flags & LIMIT_AFFINITY != 0 {
        put64(&mut basic, 48, affinity);
    }
    if flags & LIMIT_PRIORITY_CLASS != 0 {
        put32(&mut basic, 56, priority);
    }
    put32(&mut basic, 60, if flags & LIMIT_SCHEDULING_CLASS != 0 { scheduling } else { DEFAULT_SCHEDULING_CLASS });
    let tail = extended.then(|| {
        let mut t = [0u8; 80];
        if flags & LIMIT_PROCESS_MEMORY != 0 {
            put64(&mut t, 48, process_memory);
        }
        if flags & LIMIT_JOB_MEMORY != 0 {
            put64(&mut t, 56, job_memory);
        }
        t
    });
    Ok(SetOp::Limits { basic, tail })
}

/// The machine's jobs (in [`crate::NtSystem`]).
#[derive(Debug, Default)]
pub struct Jobs {
    jobs: BTreeMap<u64, Job>,
    /// The jobs each process belongs to, outermost first; kept after the
    /// process ended until its object goes.
    member_of: BTreeMap<ProcessId, Vec<u64>>,
    /// Processes to terminate (kill on close, `NtTerminateJobObject`), for
    /// the kernel to carry out after the call.
    pub kills: Vec<(ProcessId, NtStatus)>,
    /// Port objects whose reference a deleted job held.
    released: Vec<ObjectId>,
    /// Processes whose first thread has not run yet (created suspended and
    /// not resumed): their end posts no exit message
    /// (specs/windows/ntdll.dll/NtSetInformationJobObject.yaml: `completion_port`).
    unstarted: BTreeSet<ProcessId>,
    /// Each process's commit limit from its jobs ([`Jobs::commit_limit_of`]), kept up to date for
    /// its own memory calls, which read it under the process's lock alone (ADR 0262).
    commit_cells: BTreeMap<ProcessId, Arc<AtomicU64>>,
}

impl Jobs {
    /// A new job object made by process `creator`.
    pub fn create(&mut self, cookie: u64, object: ObjectId, creator: Option<ProcessId>) {
        let mut job = Job::new(Some(object));
        job.creator = creator;
        self.jobs.insert(cookie, job);
    }

    /// Whether `pid` is in a job that another process made and that process is still there
    /// (`alive`): a child its parent manages, as a browser does its renderers.
    pub fn managed_by_another(&self, pid: ProcessId, alive: impl Fn(ProcessId) -> bool) -> bool {
        self.jobs_of(pid)
            .iter()
            .filter_map(|j| self.jobs.get(j))
            .any(|j| j.creator.is_some_and(|c| c != pid && alive(c)))
    }

    pub fn job(&self, cookie: u64) -> Option<&Job> {
        self.jobs.get(&cookie)
    }

    /// Process `pid` was just made and its first thread has not run.
    pub fn mark_unstarted(&mut self, pid: ProcessId) {
        self.unstarted.insert(pid);
    }

    /// The first thread of `pid` started.
    pub fn started(&mut self, pid: ProcessId) {
        self.unstarted.remove(&pid);
    }

    /// The jobs `pid` is in (empty if none).
    pub fn jobs_of(&self, pid: ProcessId) -> &[u64] {
        self.member_of.get(&pid).map_or(&[], Vec::as_slice)
    }

    pub fn in_job(&self, pid: ProcessId, job: Option<u64>) -> bool {
        let m = self.jobs_of(pid);
        match job {
            Some(j) => m.contains(&j),
            None => !m.is_empty(),
        }
    }

    fn post(&self, io: &mut crate::io::IoState, om: &mut ObjectManager, job: u64, message: u64, pid: u64) {
        let Some(port) = self.jobs.get(&job).and_then(|j| j.port) else { return };
        io.port_post_for(om, &PacketPost { port, context: pid, status: STATUS_SUCCESS, information: message });
    }

    /// Put `pid` into `job` (`NtAssignProcessToJobObject`, and the join of a
    /// new process). A job that is full refuses and says so on its port.
    ///
    /// Jobs nest: a process that is in no job joins `job` and the jobs above
    /// it; one that is in jobs joins `job` and the jobs between its
    /// innermost one and `job` when `job` is below that one, or puts `job`
    /// below its innermost one when `job` is empty and has no parent; any
    /// other job refuses it with `STATUS_ACCESS_DENIED`. A job the process is
    /// already in (or an outer one) changes nothing
    /// (specs/windows/ntdll.dll/NtAssignProcessToJobObject.yaml: `hierarchy`, `conflicts`).
    pub fn assign(
        &mut self,
        io: &mut crate::io::IoState,
        om: &mut ObjectManager,
        job: u64,
        pid: ProcessId,
    ) -> Result<()> {
        if !self.jobs.contains_key(&job) {
            return Err(STATUS_INVALID_HANDLE);
        }
        let chain = self.jobs_of(pid).to_vec();
        if chain.contains(&job) {
            return Ok(());
        }
        // The jobs the process joins, outermost first, and the parent `job` gets.
        let mut nest = None;
        let join = match chain.last() {
            None => {
                let mut up = self.ancestors(job);
                up.reverse();
                up.push(job);
                up
            }
            Some(&inner) => match self.path_below(job, inner) {
                Some(path) => path,
                None => {
                    let j = &self.jobs[&job];
                    if j.parent.is_some() || j.total != 0 {
                        return Err(STATUS_ACCESS_DENIED);
                    }
                    nest = Some(inner);
                    alloc::vec![job]
                }
            },
        };
        // A full job on the way refuses and says so on its port.
        for &c in &join {
            let j = &self.jobs[&c];
            if j.limit_flags() & LIMIT_ACTIVE_PROCESS != 0 && j.active.len() as u32 >= j.active_limit() {
                self.post(io, om, c, MSG_ACTIVE_PROCESS_LIMIT, 0);
                return Err(STATUS_QUOTA_EXCEEDED);
            }
        }
        if let Some(inner) = nest {
            self.jobs.get_mut(&job).expect("checked").parent = Some(inner);
        }
        for &c in &join {
            let j = self.jobs.get_mut(&c).expect("checked");
            j.active.insert(pid);
            j.total += 1;
            self.member_of.entry(pid).or_default().push(c);
        }
        for &c in &join {
            self.post(io, om, c, MSG_NEW_PROCESS, u64::from(pid.0));
        }
        self.refresh_commit_cells([pid]);
        Ok(())
    }

    /// The jobs above `job`, nearest first.
    fn ancestors(&self, job: u64) -> Vec<u64> {
        let mut out = Vec::new();
        let mut at = self.jobs.get(&job).and_then(|j| j.parent);
        while let Some(p) = at {
            if !self.jobs.contains_key(&p) || out.contains(&p) {
                break;
            }
            out.push(p);
            at = self.jobs[&p].parent;
        }
        out
    }

    /// The jobs from just below `above` down to `job` (outermost first), or
    /// `None` when `job` is not below `above`.
    fn path_below(&self, job: u64, above: u64) -> Option<Vec<u64>> {
        let mut path = alloc::vec![job];
        path.extend(self.ancestors(job));
        let at = path.iter().position(|&j| j == above)?;
        path.truncate(at);
        path.reverse();
        Some(path)
    }

    /// A process was created by `parent`: it joins the jobs of its creator,
    /// unless it breaks away (`CREATE_BREAKAWAY_FROM_JOB`, which needs
    /// `JOB_OBJECT_LIMIT_BREAKAWAY_OK`) or the job says its children do
    /// (`SILENT_BREAKAWAY_OK`). `STATUS_ACCESS_DENIED` for a refused
    /// breakaway.
    pub fn check_create(&self, parent: ProcessId, breakaway: bool) -> Result<bool> {
        let parent_jobs = self.jobs_of(parent);
        if parent_jobs.is_empty() {
            return Ok(true);
        }
        let any = |flag: u32| parent_jobs.iter().any(|j| self.jobs.get(j).is_some_and(|j| j.limit_flags() & flag != 0));
        if breakaway {
            return if any(LIMIT_BREAKAWAY_OK) { Ok(false) } else { Err(STATUS_ACCESS_DENIED) };
        }
        Ok(!any(LIMIT_SILENT_BREAKAWAY_OK))
    }

    /// `pid` was created: it joins its creator's jobs (`join`) and `extra`.
    pub fn process_created(
        &mut self,
        io: &mut crate::io::IoState,
        om: &mut ObjectManager,
        parent: ProcessId,
        pid: ProcessId,
        join: bool,
        extra: &[u64],
    ) -> Result<()> {
        let mut jobs: Vec<u64> = if join { self.jobs_of(parent).to_vec() } else { Vec::new() };
        for &e in extra {
            if !jobs.contains(&e) {
                jobs.push(e);
            }
        }
        for j in jobs {
            self.assign(io, om, j, pid)?;
        }
        Ok(())
    }

    /// `pid` ended with `status`.
    pub fn process_exited(
        &mut self,
        io: &mut crate::io::IoState,
        om: &mut ObjectManager,
        pid: ProcessId,
        status: NtStatus,
    ) {
        // A process whose first thread never ran leaves without an exit message.
        let ran = !self.unstarted.remove(&pid);
        for job in self.jobs_of(pid).to_vec() {
            let Some(j) = self.jobs.get_mut(&job) else { continue };
            if !j.active.remove(&pid) {
                continue;
            }
            let empty = j.active.is_empty();
            let abnormal = status.0 & 0xF000_0000 == 0xC000_0000;
            let message = if abnormal { MSG_ABNORMAL_EXIT_PROCESS } else { MSG_EXIT_PROCESS };
            if ran {
                self.post(io, om, job, message, u64::from(pid.0));
            }
            if empty {
                self.post(io, om, job, MSG_ACTIVE_PROCESS_ZERO, 0);
            }
        }
    }

    /// The process object of `pid` was deleted: forget its membership.
    pub fn process_forgotten(&mut self, pid: ProcessId) {
        self.member_of.remove(&pid);
        self.commit_cells.remove(&pid);
    }

    /// The most private memory `pid` may have committed by its jobs' limits
    /// (`JOBOBJECT_EXTENDED_LIMIT_INFORMATION`'s `ProcessMemoryLimit` and `JobMemoryLimit`, which
    /// count the committed private bytes; a job's limit counts this process alone here), or
    /// `u64::MAX` if no job limits it.
    pub fn commit_limit_of(&self, pid: ProcessId) -> u64 {
        let mut limit = u64::MAX;
        for job in self.jobs_of(pid).iter().filter_map(|c| self.jobs.get(c)) {
            let at = |from: usize| u64::from_le_bytes(job.extended_tail[from..from + 8].try_into().expect("8 bytes"));
            let flags = job.limit_flags();
            if flags & LIMIT_PROCESS_MEMORY != 0 {
                limit = limit.min(at(48));
            }
            if flags & LIMIT_JOB_MEMORY != 0 {
                limit = limit.min(at(56));
            }
        }
        limit
    }

    /// Keep `cell` at `pid`'s [`commit_limit_of`](Self::commit_limit_of) from now on.
    pub fn register_commit_cell(&mut self, pid: ProcessId, cell: Arc<AtomicU64>) {
        cell.store(self.commit_limit_of(pid), Ordering::Relaxed);
        self.commit_cells.insert(pid, cell);
    }

    /// Bring the commit cells of `pids` up to date (they joined a job, or its limits changed).
    fn refresh_commit_cells(&self, pids: impl IntoIterator<Item = ProcessId>) {
        for pid in pids {
            if let Some(cell) = self.commit_cells.get(&pid) {
                cell.store(self.commit_limit_of(pid), Ordering::Relaxed);
            }
        }
    }

    /// `NtTerminateJobObject`: every member still running is to end, and the
    /// job becomes signalled (a wait on it ends; the sheet of `OpenJobObjectW`).
    /// `TotalTerminatedProcesses` stays 0.
    pub fn terminate(&mut self, om: &mut ObjectManager, job: u64, status: NtStatus) {
        if let Some(j) = self.jobs.get(&job) {
            let members: Vec<ProcessId> = j.active.iter().copied().collect();
            self.kills.extend(members.into_iter().map(|p| (p, status)));
            if let Some(object) = j.object {
                let _ = om.set_signaled(object, true);
            }
        }
    }

    /// The job object went (its last handle closed): kill-on-close ends its
    /// members with status 0.
    pub fn deleted(&mut self, cookie: u64) {
        let Some(j) = self.jobs.remove(&cookie) else { return };
        if j.limit_flags() & LIMIT_KILL_ON_JOB_CLOSE != 0 {
            self.kills.extend(j.active.iter().map(|&p| (p, STATUS_SUCCESS)));
        }
        if let Some(port) = j.port {
            self.released.push(port.object);
        }
        for list in self.member_of.values_mut() {
            list.retain(|&c| c != cookie);
        }
    }

    /// References the deleted jobs held on ports.
    pub fn take_released(&mut self) -> Vec<ObjectId> {
        core::mem::take(&mut self.released)
    }
}

// ----- the services

/// The specific access rights of a job object (`winnt.h`: `JOB_OBJECT_ALL_ACCESS`'s low bits).
const JOB_SPECIFIC_RIGHTS: u32 = 0x3F;

/// The job behind handle `h`, which must have `access`.
fn job_of(sys: &mut crate::NtSystem, caller: &poly_ntobj::Caller, h: u64, access: u32) -> Result<u64> {
    let obj = sys.objects.object_by_handle(caller, Handle(h), access, Some(ObjectType::Job))?;
    sys.objects.cookie(obj).ok_or(STATUS_INVALID_HANDLE)
}

/// `NtCreateJobObject(JobHandle*, DesiredAccess, ObjectAttributes*)`: a name
/// that is taken is `STATUS_OBJECT_NAME_COLLISION`, or with `OBJ_OPENIF` opens the
/// job (`STATUS_OBJECT_NAME_EXISTS`). Specific rights a job does not have are
/// dropped from the access asked for (granted 0 for 0x8000; the sheet's `access_masks`).
pub fn create_job_object<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [out, access, attrs] = abi::args::<K, 3>(k, regs)?;
    let attrs = abi::read_attributes(k, attrs)?;
    let caller = k.caller();
    let created = k.with_state(|sys, _, b| {
        let cookie = sys.io.cookie();
        let new = NewObject::Opaque { kind: ObjectType::Job, cookie, signaled: Some(false), parse: false };
        let oa = attrs.as_object_attributes();
        let access = access as u32 & !(0xFFFF & !JOB_SPECIFIC_RIGHTS);
        let r = sys.objects.create_object(&caller, &oa, access, new, &mut NoParse);
        if let Ok(c) = &r
            && sys.objects.cookie(c.object) == Some(cookie)
        {
            sys.jobs.create(cookie, c.object, Some(caller.process));
        }
        sys.reap(b);
        r
    })?;
    return_handle(k, out, created.handle)?;
    Ok(created.status)
}

/// `NtOpenJobObject(JobHandle*, DesiredAccess, ObjectAttributes*)`.
pub fn open_job_object<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [out, access, attrs] = abi::args::<K, 3>(k, regs)?;
    // No `ObjectAttributes` at all: `STATUS_INVALID_PARAMETER`.
    if attrs == 0 {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let attrs = abi::read_attributes(k, attrs)?;
    let caller = k.caller();
    let h = k.with_state(|sys, _, _| {
        sys.objects.open_object(
            &caller,
            &attrs.as_object_attributes(),
            access as u32,
            Some(ObjectType::Job),
            &mut NoParse,
        )
    })?;
    return_handle(k, out, h)?;
    Ok(STATUS_SUCCESS)
}

/// `NtAssignProcessToJobObject(JobHandle, ProcessHandle)`: the process needs
/// `PROCESS_SET_QUOTA` and `PROCESS_TERMINATE`, the job
/// `JOB_OBJECT_ASSIGN_PROCESS`; a process that has ended is refused with
/// `STATUS_ACCESS_DENIED`.
pub fn assign_process_to_job_object<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [job, process] = abi::args::<K, 2>(k, regs)?;
    let caller = k.caller();
    let pid = process_of(k, Handle(process), PROCESS_SET_QUOTA | PROCESS_TERMINATE)?;
    k.with_state(|sys, _, b| -> Result<()> {
        let cookie = job_of(sys, &caller, job, JOB_OBJECT_ASSIGN_PROCESS)?;
        if sys.process_exits.contains_key(&pid) {
            return Err(STATUS_ACCESS_DENIED);
        }
        let r = sys.jobs.assign(&mut sys.io, &mut sys.objects, cookie, pid);
        sys.reap(b);
        r
    })?;
    Ok(STATUS_SUCCESS)
}

/// `NtTerminateJobObject(JobHandle, ExitStatus)`.
pub fn terminate_job_object<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [job, status] = abi::args::<K, 2>(k, regs)?;
    let caller = k.caller();
    k.with_state(|sys, _, _| -> Result<()> {
        let cookie = job_of(sys, &caller, job, JOB_OBJECT_TERMINATE)?;
        sys.jobs.terminate(&mut sys.objects, cookie, NtStatus(status as u32));
        Ok(())
    })?;
    Ok(STATUS_SUCCESS)
}

/// `NtIsProcessInJob(ProcessHandle, JobHandle)` (a null job: any job):
/// `STATUS_PROCESS_NOT_IN_JOB` (0x123) or `STATUS_PROCESS_IN_JOB` (0x124).
pub fn is_process_in_job<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [process, job] = abi::args::<K, 2>(k, regs)?;
    let caller = k.caller();
    let pid = process_of_any(k, Handle(process), &[PROCESS_QUERY_INFORMATION, 0x1000])?;
    let inside = k.with_state(|sys, _, _| -> Result<bool> {
        let cookie = if job == 0 { None } else { Some(job_of(sys, &caller, job, JOB_OBJECT_QUERY)?) };
        Ok(sys.jobs.in_job(pid, cookie))
    })?;
    Ok(NtStatus(if inside { 0x124 } else { 0x123 }))
}

/// The last class of `JOBOBJECTINFOCLASS` the query answers with a length
/// error rather than a bad class.
const MAX_CLASS: u32 = 45;
const BASIC_ACCOUNTING: u32 = 1;
const BASIC_LIMIT: u32 = 2;
const PROCESS_ID_LIST: u32 = 3;
const UI_RESTRICTIONS: u32 = 4;
const SECURITY_LIMIT: u32 = 5;
const END_OF_JOB_TIME: u32 = 6;
const ASSOCIATE_PORT: u32 = 7;
const BASIC_AND_IO: u32 = 8;
const EXTENDED_LIMIT: u32 = 9;
const GROUP_INFORMATION: u32 = 11;
const GROUP_INFORMATION_EX: u32 = 14;

/// `NtQueryInformationJobObject(JobHandle, JobInformationClass,
/// JobInformation, JobInformationLength, ReturnLength*)`. A `NULL` handle
/// asks for the calling process's own job; a process that is in none gets
/// the answers of a job nothing was set on.
pub fn query_information_job_object<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, class, buf, len, ret] = abi::args::<K, 5>(k, regs)?;
    let (class, len) = (class as u32, len as u32 as usize);
    let caller = k.caller();
    let answer = k.with_state(|sys, _, _| -> Result<core::result::Result<Reply, Refused>> {
        if h == 0 {
            return Ok(match sys.jobs.jobs_of(caller.process).last().and_then(|c| sys.jobs.job(*c)) {
                Some(j) => j.query(class, len),
                None => Job::new(None).query(class, len),
            });
        }
        let cookie = job_of(sys, &caller, h, JOB_OBJECT_QUERY)?;
        let j = sys.jobs.job(cookie).ok_or(STATUS_INVALID_HANDLE)?;
        Ok(j.query(class, len))
    })?;
    match answer {
        Err(refused) => {
            if let Some(n) = refused.returned
                && ret != 0
            {
                abi::write_u32(k, ret, n)?;
            }
            Err(refused.status)
        }
        Ok(reply) => {
            if !reply.bytes.is_empty() {
                abi::write(k, buf, &reply.bytes)?;
            }
            if ret != 0 {
                abi::write_u32(k, ret, reply.returned)?;
            }
            Ok(reply.status)
        }
    }
}

/// `NtSetInformationJobObject(JobHandle, JobInformationClass,
/// JobInformation, JobInformationLength)`: the basic (2) and extended (9)
/// limits, the UI restrictions (4), the end-of-job action (6) and the
/// completion port (7, once); other classes are checked as the sheet
/// observed and change nothing ([`set_length`], [`parse_set`]).
pub fn set_information_job_object<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, class, buf, len] = abi::args::<K, 4>(k, regs)?;
    let (class, len) = (class as u32, len as u32 as usize);
    let caller = k.caller();
    // The rights come before the class (an unusable handle first).
    k.with_state(|sys, _, _| job_of(sys, &caller, h, JOB_OBJECT_SET_ATTRIBUTES).map(|_| ()))?;
    let size = set_length(class, len)?;
    let mut data = alloc::vec![0u8; size];
    // A `NULL` buffer with a length is an access violation.
    if size > 0 {
        abi::read(k, buf, &mut data)?;
    }
    let op = parse_set(class, &data)?;
    k.with_state(|sys, _, b| -> Result<()> {
        let cookie = job_of(sys, &caller, h, JOB_OBJECT_SET_ATTRIBUTES)?;
        let r = match op {
            SetOp::Limits { basic, tail } => {
                let j = sys.jobs.jobs.get_mut(&cookie).ok_or(STATUS_INVALID_HANDLE)?;
                j.basic = basic;
                if let Some(t) = tail {
                    j.extended_tail = t;
                }
                // Every process's: a job's limits bind the processes of the jobs nested in it too.
                let all: Vec<ProcessId> = sys.jobs.commit_cells.keys().copied().collect();
                sys.jobs.refresh_commit_cells(all);
                Ok(())
            }
            SetOp::Ui(v) => {
                sys.jobs.jobs.get_mut(&cookie).ok_or(STATUS_INVALID_HANDLE)?.ui = v;
                Ok(())
            }
            SetOp::EndOfJob(v) => {
                sys.jobs.jobs.get_mut(&cookie).ok_or(STATUS_INVALID_HANDLE)?.end_of_job = v;
                Ok(())
            }
            SetOp::Port { key, port } => {
                // A second association is refused; a `NULL` port associates nothing.
                if sys.jobs.job(cookie).is_some_and(|j| j.port.is_some()) {
                    return Err(STATUS_INVALID_PARAMETER);
                }
                if port == 0 {
                    return Ok(());
                }
                let object = sys.objects.reference_by_handle(
                    &caller,
                    Handle(port),
                    IO_COMPLETION_MODIFY_STATE,
                    Some(ObjectType::IoCompletion),
                )?;
                let pcookie = sys.objects.cookie(object).ok_or(STATUS_INVALID_HANDLE)?;
                sys.jobs.jobs.get_mut(&cookie).ok_or(STATUS_INVALID_HANDLE)?.port =
                    Some(PortAssoc { object, cookie: pcookie, key });
                Ok(())
            }
            SetOp::Accepted => Ok(()),
        };
        sys.reap(b);
        r
    })?;
    Ok(STATUS_SUCCESS)
}

/// The processes a job-bearing create should join: resolve `handles` (job
/// handles of the creating process) to job cookies, each needing
/// `JOB_OBJECT_ASSIGN_PROCESS`.
pub fn resolve_jobs(sys: &mut crate::NtSystem, caller: &poly_ntobj::Caller, handles: &[u64]) -> Result<Vec<u64>> {
    handles.iter().map(|&h| job_of(sys, caller, h, JOB_OBJECT_ASSIGN_PROCESS)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use poly_ntobj::ThreadRef;

    fn world() -> (Jobs, crate::io::IoState, ObjectManager) {
        (Jobs::default(), crate::io::IoState::new(), ObjectManager::new())
    }

    fn new_job(jobs: &mut Jobs, om: &mut ObjectManager, cookie: u64) {
        let new = NewObject::Opaque { kind: ObjectType::Job, cookie, signaled: Some(false), parse: false };
        let object = om.new_object(new, ThreadRef(1)).expect("object");
        jobs.create(cookie, object, None);
    }

    fn chain(jobs: &Jobs, pid: u32) -> Vec<u64> {
        jobs.jobs_of(ProcessId(pid)).to_vec()
    }

    #[test]
    fn a_process_assigned_to_empty_jobs_nests_them_below_its_innermost() {
        let (mut jobs, mut io, mut om) = world();
        for c in 1..=4 {
            new_job(&mut jobs, &mut om, c);
        }
        let p = ProcessId(10);
        assert_eq!(jobs.assign(&mut io, &mut om, 1, p), Ok(()));
        // Again: nothing changes.
        assert_eq!(jobs.assign(&mut io, &mut om, 1, p), Ok(()));
        assert_eq!(jobs.job(1).unwrap().total, 1);
        // The empty j2, then the empty and unrelated j3, go below the innermost job.
        assert_eq!(jobs.assign(&mut io, &mut om, 2, p), Ok(()));
        assert_eq!(jobs.assign(&mut io, &mut om, 3, p), Ok(()));
        assert_eq!(chain(&jobs, 10), [1, 2, 3]);
        // An outer job again: no change.
        assert_eq!(jobs.assign(&mut io, &mut om, 1, p), Ok(()));
        assert_eq!(chain(&jobs, 10), [1, 2, 3]);
        // A new process assigned straight to the innermost job joins all of them.
        let q = ProcessId(11);
        assert_eq!(jobs.assign(&mut io, &mut om, 3, q), Ok(()));
        assert_eq!(chain(&jobs, 11), [1, 2, 3]);
        assert_eq!((jobs.job(1).unwrap().active.len(), jobs.job(3).unwrap().active.len()), (2, 2));
        // A job with a member of its own refuses a process that is in other jobs, and the
        // other way round.
        let r = ProcessId(12);
        assert_eq!(jobs.assign(&mut io, &mut om, 4, r), Ok(()));
        assert_eq!(jobs.assign(&mut io, &mut om, 4, p), Err(STATUS_ACCESS_DENIED));
        assert_eq!(chain(&jobs, 10), [1, 2, 3]);
        assert_eq!(jobs.job(4).unwrap().active.len(), 1);
        assert_eq!(jobs.assign(&mut io, &mut om, 1, r), Err(STATUS_ACCESS_DENIED));
    }

    #[test]
    fn nested_accounting_counts_a_process_in_every_job_and_terminated_stays_zero() {
        let (mut jobs, mut io, mut om) = world();
        new_job(&mut jobs, &mut om, 1);
        new_job(&mut jobs, &mut om, 2);
        let (p_out, p_both, p_in) = (ProcessId(1), ProcessId(2), ProcessId(3));
        jobs.assign(&mut io, &mut om, 1, p_out).unwrap();
        jobs.assign(&mut io, &mut om, 1, p_both).unwrap();
        jobs.assign(&mut io, &mut om, 2, p_both).unwrap();
        jobs.assign(&mut io, &mut om, 2, p_in).unwrap();
        assert_eq!((jobs.job(1).unwrap().total, jobs.job(1).unwrap().active.len()), (3, 3));
        assert_eq!((jobs.job(2).unwrap().total, jobs.job(2).unwrap().active.len()), (2, 2));
        jobs.terminate(&mut om, 2, NtStatus(0x31));
        let killed: Vec<u32> = jobs.kills.iter().map(|(p, _)| p.0).collect();
        assert_eq!(killed, [2, 3]);
        for (pid, status) in core::mem::take(&mut jobs.kills) {
            jobs.process_exited(&mut io, &mut om, pid, status);
        }
        assert_eq!((jobs.job(1).unwrap().total, jobs.job(1).unwrap().active.len()), (3, 1));
        assert_eq!((jobs.job(2).unwrap().total, jobs.job(2).unwrap().active.len()), (2, 0));
        assert_eq!((jobs.job(1).unwrap().terminated, jobs.job(2).unwrap().terminated), (0, 0));
        // The job is signalled: a wait on it ends.
        assert!(om.is_ready_for(jobs.job(2).unwrap().object.unwrap(), ThreadRef(1)));
    }

    #[test]
    fn query_defaults_and_lengths() {
        let j = Job::new(None);
        // The basic and extended limits are zero except SchedulingClass 5 at offset 60.
        let b = j.query(BASIC_LIMIT, 64).unwrap();
        assert_eq!(b.returned, 64);
        assert!(b.bytes[..60].iter().all(|&x| x == 0));
        assert_eq!(&b.bytes[60..], 5u32.to_le_bytes());
        assert_eq!(j.query(BASIC_LIMIT, 63).unwrap_err().status, STATUS_INFO_LENGTH_MISMATCH);
        assert_eq!(j.query(EXTENDED_LIMIT, 152).unwrap().bytes.len(), 152);
        assert_eq!(j.query(EXTENDED_LIMIT, 145).unwrap_err().status, STATUS_INFO_LENGTH_MISMATCH);
        // Lengths of the other classes, from the sheet.
        for (class, len) in [(10, 4), (12, 48), (12, 56), (13, 80), (15, 8), (16, 4), (17, 8), (18, 16), (33, 72)] {
            assert_eq!(j.query(class, len).unwrap().bytes, alloc::vec![0u8; len], "{class}");
            assert_eq!(j.query(class, len + 1).unwrap_err().status, STATUS_INFO_LENGTH_MISMATCH, "{class}");
        }
        assert_eq!(j.query(SECURITY_LIMIT, 39).unwrap_err().status, STATUS_INFO_LENGTH_MISMATCH);
        assert_eq!(j.query(SECURITY_LIMIT, 300).unwrap().returned, 300);
        // Class 7 and class 0 and the unknown classes.
        assert_eq!(j.query(ASSOCIATE_PORT, 16).unwrap_err().status, STATUS_INVALID_INFO_CLASS);
        assert_eq!(j.query(ASSOCIATE_PORT, 8).unwrap_err().status, STATUS_INFO_LENGTH_MISMATCH);
        assert_eq!(j.query(0, 64).unwrap_err().status, STATUS_INVALID_INFO_CLASS);
        assert_eq!(j.query(4096, 4).unwrap_err().status, STATUS_INVALID_INFO_CLASS);
        assert_eq!(j.query(19, 8).unwrap_err().status, STATUS_INFO_LENGTH_MISMATCH);
        assert_eq!(j.query(31, 144).unwrap_err().status, STATUS_NOT_IMPLEMENTED);
    }

    #[test]
    fn query_group_information_odd_and_even_lengths() {
        let j = Job::new(None);
        // Class 11: even lengths from 2 give no bytes, odd ones are invalid, 0 and 1 mismatch.
        assert_eq!(j.query(GROUP_INFORMATION, 0).unwrap_err().status, STATUS_INFO_LENGTH_MISMATCH);
        assert_eq!(j.query(GROUP_INFORMATION, 1).unwrap_err().status, STATUS_INFO_LENGTH_MISMATCH);
        assert_eq!(j.query(GROUP_INFORMATION, 2).unwrap().returned, 0);
        assert_eq!(j.query(GROUP_INFORMATION, 3).unwrap_err().status, STATUS_INVALID_PARAMETER);
        assert_eq!(j.query(GROUP_INFORMATION, 1024).unwrap().returned, 0);
        // Class 14: too small at 0 (16 returned), invalid between, 16 and 32 succeed, more mismatch.
        let r = j.query(GROUP_INFORMATION_EX, 0).unwrap_err();
        assert_eq!((r.status, r.returned), (STATUS_BUFFER_TOO_SMALL, Some(16)));
        for len in (1..16).chain(17..32) {
            assert_eq!(j.query(GROUP_INFORMATION_EX, len).unwrap_err().status, STATUS_INVALID_PARAMETER, "{len}");
        }
        assert_eq!(j.query(GROUP_INFORMATION_EX, 16).unwrap().returned, 16);
        assert_eq!(j.query(GROUP_INFORMATION_EX, 32).unwrap().returned, 16);
        assert_eq!(j.query(GROUP_INFORMATION_EX, 33).unwrap_err().status, STATUS_INFO_LENGTH_MISMATCH);
    }

    #[test]
    fn query_process_list_overflows_with_the_full_size() {
        let mut j = Job::new(None);
        j.active.insert(ProcessId(4));
        j.active.insert(ProcessId(8));
        for len in 0..16 {
            assert_eq!(j.query(PROCESS_ID_LIST, len).unwrap_err().status, STATUS_INFO_LENGTH_MISMATCH);
        }
        for len in [16, 17] {
            let r = j.query(PROCESS_ID_LIST, len).unwrap();
            assert_eq!((r.status, r.returned, r.bytes.len()), (STATUS_BUFFER_OVERFLOW, 24, 16), "{len}");
            assert_eq!(&r.bytes[..8], [2, 0, 0, 0, 1, 0, 0, 0]);
        }
        let r = j.query(PROCESS_ID_LIST, 24).unwrap();
        assert_eq!((r.status, r.returned, r.bytes.len()), (STATUS_SUCCESS, 24, 24));
        // An empty job returns the header.
        let r = Job::new(None).query(PROCESS_ID_LIST, 64).unwrap();
        assert_eq!((r.status, r.returned), (STATUS_SUCCESS, 8));
    }

    type Structure = [u8; 152];

    fn structure(flags: u32, fill: &dyn Fn(&mut Structure)) -> Structure {
        let mut d = [0u8; 152];
        d[16..20].copy_from_slice(&flags.to_le_bytes());
        fill(&mut d);
        d
    }

    fn put32(d: &mut Structure, o: usize, v: u32) {
        d[o..o + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn put64(d: &mut Structure, o: usize, v: u64) {
        d[o..o + 8].copy_from_slice(&v.to_le_bytes());
    }

    fn at32(b: &[u8], o: usize) -> u32 {
        u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
    }

    fn basic_set(flags: u32, fill: &dyn Fn(&mut Structure)) -> Result<SetOp> {
        parse_set(BASIC_LIMIT, &structure(flags, fill)[..64])
    }

    fn extended_set(flags: u32, fill: &dyn Fn(&mut Structure)) -> Result<SetOp> {
        parse_set(EXTENDED_LIMIT, &structure(flags, fill)[..144])
    }

    #[test]
    fn set_limits_store_only_what_the_flags_select() {
        // Active limit 9 only: the priority and scheduling values of the structure are not kept.
        let r = basic_set(0x8, &|d| {
            put32(d, 40, 9);
            put32(d, 56, 3);
            put32(d, 60, 4);
        });
        let Ok(SetOp::Limits { basic: b, tail: None }) = r else { panic!("{r:?}") };
        assert_eq!((at32(&b, 40), at32(&b, 56), at32(&b, 60)), (9, 0, 5));
        // Flags 0xa8: active 3, priority 3, scheduling 4.
        let r = basic_set(0xa8, &|d| {
            put32(d, 40, 3);
            put32(d, 56, 3);
            put32(d, 60, 4);
        });
        let Ok(SetOp::Limits { basic: b, .. }) = r else { panic!("{r:?}") };
        assert_eq!((at32(&b, 16), at32(&b, 40), at32(&b, 56), at32(&b, 60)), (0xa8, 3, 3, 4));
        // The preserve bit is not stored.
        let Ok(SetOp::Limits { basic: b, .. }) = basic_set(0x4 | 0x40, &|_| {}) else { panic!() };
        assert_eq!(at32(&b, 16), 0x4);
        // Memory values without their flags are accepted and not stored.
        let Ok(SetOp::Limits { tail: Some(t), .. }) = extended_set(0, &|d| put64(d, 112, 5)) else { panic!() };
        assert!(t.iter().all(|&x| x == 0));
    }

    #[test]
    fn set_limits_refuse_values_the_kernel_refuses() {
        let bad = Err(STATUS_INVALID_PARAMETER);
        assert_eq!(basic_set(0x20, &|d| put32(d, 56, 7)), bad);
        assert!(basic_set(0x20, &|d| put32(d, 56, 6)).is_ok());
        assert_eq!(basic_set(0x80, &|d| put32(d, 60, 10)), bad);
        assert!(basic_set(0x80, &|d| put32(d, 60, 9)).is_ok());
        assert_eq!(basic_set(0x10, &|d| put64(d, 48, 0)), bad);
        assert_eq!(basic_set(0x10, &|d| put64(d, 48, u64::MAX)), bad);
        assert!(basic_set(0x10, &|d| put64(d, 48, 3)).is_ok());
        assert_eq!(basic_set(0x2, &|_| {}), bad);
        assert!(basic_set(0x2, &|d| put64(d, 0, u64::MAX)).is_ok());
        let ws = |min: u64, max: u64| {
            basic_set(0x1, &|d| {
                put64(d, 24, min);
                put64(d, 32, max);
            })
        };
        assert_eq!(ws(1 << 21, 1 << 20), bad);
        assert_eq!(ws(12345, 1 << 20), bad);
        assert_eq!(ws(0, 0), bad);
        assert!(ws(1 << 20, 1 << 20).is_ok());
        assert!(ws(1 << 20, 2 << 20).is_ok());
        // The basic structure refuses the extended flags and the unknown ones.
        for flags in [0x800, 0x1000, 0x400, 0x2000, 0x100, 0x200, 0x4000, 0x8000, 0x10000, 0xFFFF_FFFF] {
            assert_eq!(basic_set(flags, &|_| {}), bad, "{flags:#x}");
        }
        // The extended one takes them; memory limits need their value; 0x4000 needs 0x10.
        for flags in [0x800, 0x1000, 0x400, 0x2000, 0x3c00] {
            assert!(extended_set(flags, &|_| {}).is_ok(), "{flags:#x}");
        }
        assert_eq!(extended_set(0x100, &|_| {}), bad);
        assert!(extended_set(0x100, &|d| put64(d, 112, 64 << 20)).is_ok());
        assert_eq!(extended_set(0x8000, &|_| {}), bad);
        assert_eq!(extended_set(0x4000, &|_| {}), bad);
        assert!(extended_set(0x4000 | 0x10, &|d| put64(d, 48, 1)).is_ok());
    }

    #[test]
    fn set_checks_class_lengths_and_small_values() {
        // Lengths: only the structure's own, per class.
        assert_eq!(set_length(BASIC_LIMIT, 64), Ok(64));
        for len in [0, 4, 63, 65, 144] {
            assert_eq!(set_length(BASIC_LIMIT, len), Err(STATUS_INFO_LENGTH_MISMATCH), "{len}");
        }
        assert_eq!(set_length(EXTENDED_LIMIT, 144), Ok(144));
        assert_eq!(set_length(EXTENDED_LIMIT, 152), Ok(152));
        assert_eq!(set_length(EXTENDED_LIMIT, 64), Err(STATUS_INFO_LENGTH_MISMATCH));
        assert_eq!(set_length(EXTENDED_LIMIT, 145), Err(STATUS_INFO_LENGTH_MISMATCH));
        // Query-only classes, class 0, class 5.
        assert_eq!(set_length(0, 4), Err(STATUS_INVALID_INFO_CLASS));
        for (class, len) in [(1, 48), (3, 16), (8, 96), (10, 4), (34, 104)] {
            assert_eq!(set_length(class, len), Err(STATUS_INVALID_INFO_CLASS), "{class}");
            assert_eq!(set_length(class, len + 1), Err(STATUS_INFO_LENGTH_MISMATCH), "{class}");
        }
        assert_eq!(set_length(5, 40), Err(STATUS_NOT_SUPPORTED));
        // Accepted ones.
        assert_eq!(set_length(7, 16), Ok(16));
        assert_eq!(set_length(7, 15), Err(STATUS_INFO_LENGTH_MISMATCH));
        assert_eq!(set_length(14, 0), Ok(0));
        assert_eq!(set_length(14, 32), Ok(32));
        assert_eq!(set_length(14, 8), Err(STATUS_INFO_LENGTH_MISMATCH));
        assert_eq!(parse_set(15, &[0; 8]), Err(STATUS_INVALID_PARAMETER));
        assert_eq!(parse_set(18, &[0; 16]), Err(STATUS_INVALID_PARAMETER));
        assert_eq!(parse_set(35, &[]), Err(STATUS_INVALID_PARAMETER));
        assert_eq!(parse_set(11, &[0; 4]), Err(STATUS_INVALID_PARAMETER));
        assert_eq!(parse_set(11, &[0; 2]), Ok(SetOp::Accepted));
        // The scan of the visible test: class 11 at 2 bytes, other even lengths invalid, odd ones
        // and 0 a mismatch; class 14 at multiples of 16; class 19 never; class 13 and 20 at 0 are
        // query-only; 25 and 27 at 0 are taken; unknown classes are not classes.
        assert_eq!(set_length(11, 2), Ok(2));
        assert_eq!(set_length(11, 6), Ok(6));
        assert_eq!(set_length(11, 7), Err(STATUS_INFO_LENGTH_MISMATCH));
        assert_eq!(set_length(11, 0), Err(STATUS_INFO_LENGTH_MISMATCH));
        assert_eq!(set_length(14, 288), Ok(288));
        assert_eq!(set_length(19, 8), Err(STATUS_INFO_LENGTH_MISMATCH));
        assert_eq!(set_length(13, 0), Err(STATUS_INVALID_INFO_CLASS));
        assert_eq!(set_length(25, 0), Ok(0));
        assert_eq!(set_length(27, 0), Ok(0));
        assert_eq!(set_length(24, 8), Ok(8));
        assert_eq!(set_length(24, 7), Err(STATUS_INFO_LENGTH_MISMATCH));
        assert_eq!(set_length(99, 4), Err(STATUS_INVALID_INFO_CLASS));
        // UI restrictions and the end-of-job action.
        for v in [0u32, 1, 0x3f, 0xff, 0x100, 0x1ff] {
            assert_eq!(parse_set(UI_RESTRICTIONS, &v.to_le_bytes()), Ok(SetOp::Ui(v)));
        }
        assert_eq!(parse_set(UI_RESTRICTIONS, &0x8000_0000u32.to_le_bytes()), Err(STATUS_INVALID_PARAMETER));
        assert_eq!(parse_set(END_OF_JOB_TIME, &1u32.to_le_bytes()), Ok(SetOp::EndOfJob(1)));
        assert_eq!(parse_set(END_OF_JOB_TIME, &2u32.to_le_bytes()), Err(STATUS_INVALID_PARAMETER));
        assert_eq!(parse_set(END_OF_JOB_TIME, &3u32.to_le_bytes()), Err(STATUS_INVALID_PARAMETER));
    }

    #[test]
    fn a_process_that_never_ran_leaves_no_exit_message() {
        let (mut jobs, mut io, mut om) = world();
        new_job(&mut jobs, &mut om, 1);
        let p = ProcessId(9);
        jobs.mark_unstarted(p);
        jobs.assign(&mut io, &mut om, 1, p).unwrap();
        jobs.process_exited(&mut io, &mut om, p, NtStatus(0x42));
        assert!(jobs.job(1).unwrap().active.is_empty());
        // `started` clears the mark.
        let q = ProcessId(10);
        jobs.mark_unstarted(q);
        jobs.started(q);
        assert!(!jobs.unstarted.contains(&q));
    }
}

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
use alloc::vec::Vec;
use poly_abi::SyscallRegs;
use poly_ntobj::access::*;
use poly_ntobj::status::*;
use poly_ntobj::{Handle, NewObject, NoParse, NtStatus, OBJ_OPENIF, ObjectId, ObjectManager, ObjectType, ProcessId};

pub type Result<T> = core::result::Result<T, NtStatus>;

/// `JOB_OBJECT_LIMIT_*` (`winnt.h`).
pub const LIMIT_ACTIVE_PROCESS: u32 = 0x8;
pub const LIMIT_BREAKAWAY_OK: u32 = 0x800;
pub const LIMIT_SILENT_BREAKAWAY_OK: u32 = 0x1000;
pub const LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x2000;
/// Every limit flag Windows defines for the basic and extended structures.
const VALID_LIMITS: u32 = 0x7FFF | 0x8000 | 0x10000 | 0x20000;
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
    pub object: ObjectId,
    /// The members still running.
    pub active: BTreeSet<ProcessId>,
    pub total: u32,
    pub terminated: u32,
    /// `JOBOBJECT_BASIC_LIMIT_INFORMATION` as set (64 bytes).
    pub basic: [u8; 64],
    /// The rest of `JOBOBJECT_EXTENDED_LIMIT_INFORMATION` as set (80 bytes).
    pub extended_tail: [u8; 80],
    pub ui: u32,
    pub end_of_job: u32,
    /// The completion port and key (it holds a reference on the port object).
    pub port: Option<PortAssoc>,
}

impl Job {
    pub fn limit_flags(&self) -> u32 {
        u32::from_le_bytes(self.basic[16..20].try_into().expect("4 bytes"))
    }

    pub fn active_limit(&self) -> u32 {
        u32::from_le_bytes(self.basic[40..44].try_into().expect("4 bytes"))
    }
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
}

impl Jobs {
    pub fn create(&mut self, cookie: u64, object: ObjectId) {
        self.jobs.insert(
            cookie,
            Job {
                object,
                active: BTreeSet::new(),
                total: 0,
                terminated: 0,
                basic: [0; 64],
                extended_tail: [0; 80],
                ui: 0,
                end_of_job: 0,
                port: None,
            },
        );
    }

    pub fn job(&self, cookie: u64) -> Option<&Job> {
        self.jobs.get(&cookie)
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
    pub fn assign(
        &mut self,
        io: &mut crate::io::IoState,
        om: &mut ObjectManager,
        job: u64,
        pid: ProcessId,
    ) -> Result<()> {
        let j = self.jobs.get_mut(&job).ok_or(STATUS_INVALID_HANDLE)?;
        if j.active.contains(&pid) {
            return Ok(());
        }
        if j.limit_flags() & LIMIT_ACTIVE_PROCESS != 0 && j.active.len() as u32 >= j.active_limit() {
            self.post(io, om, job, MSG_ACTIVE_PROCESS_LIMIT, 0);
            return Err(STATUS_QUOTA_EXCEEDED);
        }
        j.active.insert(pid);
        j.total += 1;
        let list = self.member_of.entry(pid).or_default();
        if !list.contains(&job) {
            list.push(job);
        }
        self.post(io, om, job, MSG_NEW_PROCESS, u64::from(pid.0));
        Ok(())
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
        for job in self.jobs_of(pid).to_vec() {
            let Some(j) = self.jobs.get_mut(&job) else { continue };
            if !j.active.remove(&pid) {
                continue;
            }
            let empty = j.active.is_empty();
            let abnormal = status.0 & 0xF000_0000 == 0xC000_0000;
            let message = if abnormal { MSG_ABNORMAL_EXIT_PROCESS } else { MSG_EXIT_PROCESS };
            self.post(io, om, job, message, u64::from(pid.0));
            if empty {
                self.post(io, om, job, MSG_ACTIVE_PROCESS_ZERO, 0);
            }
        }
    }

    /// The process object of `pid` was deleted: forget its membership.
    pub fn process_forgotten(&mut self, pid: ProcessId) {
        self.member_of.remove(&pid);
    }

    /// `NtTerminateJobObject`: every member still running is to end.
    pub fn terminate(&mut self, job: u64, status: NtStatus) {
        if let Some(j) = self.jobs.get_mut(&job) {
            let members: Vec<ProcessId> = j.active.iter().copied().collect();
            j.terminated += members.len() as u32;
            self.kills.extend(members.into_iter().map(|p| (p, status)));
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

/// The job behind handle `h`, which must have `access`.
fn job_of(sys: &mut crate::NtSystem, caller: &poly_ntobj::Caller, h: u64, access: u32) -> Result<u64> {
    let obj = sys.objects.object_by_handle(caller, Handle(h), access, Some(ObjectType::Job))?;
    sys.objects.cookie(obj).ok_or(STATUS_INVALID_HANDLE)
}

/// `NtCreateJobObject(JobHandle*, DesiredAccess, ObjectAttributes*)`: a name
/// that is taken by a job opens it (`STATUS_OBJECT_NAME_EXISTS`).
pub fn create_job_object<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [out, access, attrs] = abi::args::<K, 3>(k, regs)?;
    let attrs = abi::read_attributes(k, attrs)?;
    let caller = k.caller();
    let created = k.with_state(|sys, _, b| {
        let cookie = sys.io.cookie();
        let new = NewObject::Opaque { kind: ObjectType::Job, cookie, signaled: None, parse: false };
        let mut oa = attrs.as_object_attributes();
        oa.attributes |= OBJ_OPENIF;
        let r = sys.objects.create_object(&caller, &oa, access as u32, new, &mut NoParse);
        if let Ok(c) = &r
            && sys.objects.cookie(c.object) == Some(cookie)
        {
            sys.jobs.create(cookie, c.object);
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
        sys.jobs.terminate(cookie, NtStatus(status as u32));
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

/// The structure a query class returns, by class number: its exact size
/// (`None`: variable). Classes in this table that are not answered fail
/// with `STATUS_INFO_LENGTH_MISMATCH` as the spec sheet observed.
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
/// JobInformation, JobInformationLength, ReturnLength*)`.
pub fn query_information_job_object<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, class, buf, len, ret] = abi::args::<K, 5>(k, regs)?;
    let (class, len) = (class as u32, len as u32 as usize);
    let caller = k.caller();
    let bytes = k.with_state(|sys, _, _| -> Result<Vec<u8>> {
        let cookie = job_of(sys, &caller, h, JOB_OBJECT_QUERY)?;
        if class == 0 || class > 15 && class != 0 {
            return Err(STATUS_INVALID_INFO_CLASS);
        }
        let j = sys.jobs.job(cookie).ok_or(STATUS_INVALID_HANDLE)?;
        let exact = |n: usize| if len == n { Ok(()) } else { Err(STATUS_INFO_LENGTH_MISMATCH) };
        Ok(match class {
            BASIC_ACCOUNTING | BASIC_AND_IO => {
                exact(if class == BASIC_ACCOUNTING { 48 } else { 96 })?;
                let mut b = alloc::vec![0u8; len];
                b[36..40].copy_from_slice(&j.total.to_le_bytes());
                b[40..44].copy_from_slice(&(j.active.len() as u32).to_le_bytes());
                b[44..48].copy_from_slice(&j.terminated.to_le_bytes());
                b
            }
            BASIC_LIMIT => {
                exact(64)?;
                j.basic.to_vec()
            }
            EXTENDED_LIMIT => {
                exact(144)?;
                let mut b = j.basic.to_vec();
                b.extend_from_slice(&j.extended_tail);
                b
            }
            UI_RESTRICTIONS => {
                exact(4)?;
                j.ui.to_le_bytes().to_vec()
            }
            END_OF_JOB_TIME => {
                exact(4)?;
                j.end_of_job.to_le_bytes().to_vec()
            }
            PROCESS_ID_LIST => {
                if len < 16 {
                    return Err(STATUS_INFO_LENGTH_MISMATCH);
                }
                let room = (len - 8) / 8;
                let ids: Vec<u64> = j.active.iter().take(room).map(|p| u64::from(p.0)).collect();
                let mut b = (j.active.len() as u32).to_le_bytes().to_vec();
                b.extend_from_slice(&(ids.len() as u32).to_le_bytes());
                for id in ids {
                    b.extend_from_slice(&id.to_le_bytes());
                }
                b
            }
            SECURITY_LIMIT => alloc::vec![0u8; len],
            GROUP_INFORMATION => Vec::new(),
            GROUP_INFORMATION_EX => alloc::vec![0u8; 16],
            _ => return Err(STATUS_INFO_LENGTH_MISMATCH),
        })
    })?;
    if !bytes.is_empty() {
        abi::write(k, buf, &bytes)?;
    }
    if ret != 0 {
        abi::write_u32(k, ret, bytes.len() as u32)?;
    }
    Ok(STATUS_SUCCESS)
}

/// `NtSetInformationJobObject(JobHandle, JobInformationClass,
/// JobInformation, JobInformationLength)`: the basic (2) and extended (9)
/// limits, the UI restrictions (4), the end-of-job action (6) and the
/// completion port (7, once).
pub fn set_information_job_object<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, class, buf, len] = abi::args::<K, 4>(k, regs)?;
    let (class, len) = (class as u32, len as u32 as usize);
    let size = match class {
        BASIC_LIMIT => 64,
        EXTENDED_LIMIT => 144,
        UI_RESTRICTIONS | END_OF_JOB_TIME => 4,
        ASSOCIATE_PORT => 16,
        _ => 0,
    };
    let caller = k.caller();
    // The rights come before the class (an unusable handle first).
    k.with_state(|sys, _, _| job_of(sys, &caller, h, JOB_OBJECT_SET_ATTRIBUTES).map(|_| ()))?;
    if size == 0 {
        return Err(STATUS_INVALID_PARAMETER);
    }
    if len != size {
        return Err(STATUS_INFO_LENGTH_MISMATCH);
    }
    let mut data = alloc::vec![0u8; size];
    abi::read(k, buf, &mut data)?;
    k.with_state(|sys, _, b| -> Result<()> {
        let cookie = job_of(sys, &caller, h, JOB_OBJECT_SET_ATTRIBUTES)?;
        let r = match class {
            BASIC_LIMIT | EXTENDED_LIMIT => {
                let flags = u32::from_le_bytes(data[16..20].try_into().expect("4 bytes"));
                if flags & !VALID_LIMITS != 0 {
                    return Err(STATUS_INVALID_PARAMETER);
                }
                let j = sys.jobs.jobs.get_mut(&cookie).ok_or(STATUS_INVALID_HANDLE)?;
                j.basic.copy_from_slice(&data[..64]);
                if class == EXTENDED_LIMIT {
                    j.extended_tail.copy_from_slice(&data[64..]);
                }
                Ok(())
            }
            UI_RESTRICTIONS => {
                sys.jobs.jobs.get_mut(&cookie).ok_or(STATUS_INVALID_HANDLE)?.ui =
                    u32::from_le_bytes(data[0..4].try_into().expect("4 bytes"));
                Ok(())
            }
            END_OF_JOB_TIME => {
                sys.jobs.jobs.get_mut(&cookie).ok_or(STATUS_INVALID_HANDLE)?.end_of_job =
                    u32::from_le_bytes(data[0..4].try_into().expect("4 bytes"));
                Ok(())
            }
            _ => {
                // JOBOBJECT_ASSOCIATE_COMPLETION_PORT: key, then port.
                let key = u64::from_le_bytes(data[0..8].try_into().expect("8 bytes"));
                let port = u64::from_le_bytes(data[8..16].try_into().expect("8 bytes"));
                if sys.jobs.job(cookie).is_some_and(|j| j.port.is_some()) {
                    return Err(STATUS_INVALID_PARAMETER);
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

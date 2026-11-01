//! procfs in the kernel: [`KernelProc`], the kernel's
//! [`ProcSource`](poly_procfs::ProcSource) over its process table, memory
//! maps, descriptor tables, mounts, clocks and CPUID; what the kernel
//! remembers about each program it started ([`record_exec`]); and the
//! mounts made at boot: procfs on `/proc`, sysfs on `/sys` (`sysfs.rs`),
//! a tmpfs on `/dev/shm`. Decisions are in ADR 0043.
//!
//! Every method runs in the thread that reads `/proc`, with no VFS or
//! [`Files`](super::Files) lock held (path walks work on a copy of the
//! context), so it may lock any process's files and memory map.
//!
//! What is not tracked yet reads as 0: CPU time of the system as a whole
//! (all of it counts as idle; processes' own CPU time is `crate::cputime`'s),
//! load averages, context switches, page faults. A process the kernel did not start through
//! [`record_exec`] (the kernel tests' bare processes) has no command line
//! and is named `?`; its memory map is shown only to itself.

use super::vfs;
use crate::process::{self, ExitStatus, Process};
use crate::sync::Mutex;
use crate::user::memory::{Backing, Kind, Prot, Vma};
use crate::user::{self, FileRef, Memory};
use crate::{arch, kprintln, random, thread, time};
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use poly_personality_linux::admin::SystemName;
use poly_personality_linux::ns;
use poly_procfs::{
    CpuInfo, CpuTimes, FdInfo, IdMapFile, KernelInfo, KernelName, MapEntry, MapName, MountEntry, Owner, ProcFs,
    ProcSource, ProcState, ProcessInfo, SignalSets, SystemInfo,
};
use poly_vfs::{At, Cred, FileType, FsContext, MountFlags, OpenFile, Vfs};

/// What the kernel remembers about the program a process runs.
#[derive(Clone)]
struct Record {
    /// The executable, while something keeps it open (its mappings do).
    exe_file: Weak<OpenFile>,
    /// Its path at exec, for when the file is gone.
    exe: Vec<u8>,
    /// The command name: the file name, at most 15 bytes.
    comm: Vec<u8>,
    memory: Weak<Memory>,
    start_ns: u64,
    start_stack: u64,
    arg: (u64, u64),
    env: (u64, u64),
}

static RECORDS: Mutex<BTreeMap<u32, Record>> = Mutex::new(BTreeMap::new());

/// The extent of strings laid out one after another from `start`, each
/// with its NUL.
fn extent(start: u64, strings: &[&[u8]]) -> (u64, u64) {
    let len: u64 = strings.iter().map(|s| s.len() as u64 + 1).sum();
    (start, start + len)
}

/// Note that `process` now runs the program in `file`, with memory
/// `memory` whose initial stack holds `argv` and `envp` as `stack`
/// describes. `program::prepare` and `execve` call this.
pub fn record_exec(
    process: &Arc<Process>,
    file: &Arc<OpenFile>,
    memory: &Arc<Memory>,
    stack: &poly_elf::StackLayout,
    argv: &[&[u8]],
    envp: &[&[u8]],
) {
    let pointer = |at: u64| {
        let mut b = [0u8; 8];
        memory.read(at, &mut b).ok().map(|()| u64::from_le_bytes(b))
    };
    let arg = match argv.first().and(pointer(stack.argv)) {
        Some(p) => extent(p, argv),
        None => (0, 0),
    };
    let env = match envp.first().and(pointer(stack.envp)) {
        Some(p) => extent(p, envp),
        None => (arg.1, arg.1),
    };
    let ctx = vfs().root_context().ok();
    let exe = ctx.and_then(|c| vfs().file_path(&c, file).ok()).unwrap_or_default();
    let name = exe.rsplit(|&b| b == b'/').next().unwrap_or(&exe);
    let record = Record {
        exe_file: Arc::downgrade(file),
        comm: name[..name.len().min(15)].to_vec(),
        exe,
        memory: Arc::downgrade(memory),
        start_ns: time::now_ns(),
        start_stack: stack.rsp,
        arg,
        env,
    };
    insert(process.pid, record);
}

/// Keep `record` for `pid`, forgetting the records of processes that are
/// gone (no memory left, not in the process table, not a zombie).
fn insert(pid: u32, record: Record) {
    let mut records = RECORDS.lock();
    records.retain(|&p, r| r.memory.strong_count() > 0 || find(p).is_some());
    records.insert(pid, record);
}

/// `fork`: process `child` runs what `parent` ran, in `memory` (a copy of
/// the parent's, so the strings are at the same addresses).
pub fn record_fork(parent: u32, child: u32, memory: &Arc<Memory>) {
    let Some(mut record) = RECORDS.lock().get(&parent).cloned() else { return };
    record.memory = Arc::downgrade(memory);
    record.start_ns = time::now_ns();
    insert(child, record);
}

/// Process `pid`: a live one, or a zombie waiting for its parent.
fn find(pid: u32) -> Option<Arc<Process>> {
    process::find(pid).or_else(|| crate::linux::task::zombie(pid))
}

/// The kernel's facts for procfs.
#[derive(Debug, Default)]
pub struct KernelProc;

/// The root context paths are shown from: the reading process's root
/// directory (chroot(2): a chrooted shell reads `/proc/self/exe` as
/// `/usr/bin/readlink`, not with the chroot's own path in front, ADR 0059),
/// the system's for kernel threads. A file outside that root is named from
/// the system's root.
fn root() -> Option<FsContext> {
    match user::current_process() {
        Some(p) => Some(p.files.context()),
        None => vfs().root_context().ok(),
    }
}

fn with_record<R>(pid: u32, f: impl FnOnce(&Record) -> R) -> Option<R> {
    RECORDS.lock().get(&pid).map(f)
}

/// The command name of process `pid` (`/proc/<pid>/comm`), if it was
/// started from a program.
pub fn comm(pid: u32) -> Option<Vec<u8>> {
    let recorded = with_record(pid, |r| r.comm.clone())?;
    Some(find(pid).and_then(|p| crate::linux::cred::process_comm(&p)).unwrap_or(recorded))
}

/// The memory map of process `pid`: the caller's own, or the one recorded
/// at exec.
pub(crate) fn memory_of(pid: u32) -> Option<Arc<Memory>> {
    if user::current_process().is_some_and(|p| p.pid == pid) {
        return user::current_memory();
    }
    with_record(pid, |r| r.memory.upgrade()).flatten()
}

/// What `/proc/<pid>/fd/<n>` links to for a file without a path.
fn anonymous_target(file: &OpenFile) -> Vec<u8> {
    if let Some(name) = file.object().and_then(|o| o.anon_name()) {
        return alloc::format!("anon_inode:{name}").into_bytes();
    }
    let what = match file.kind() {
        FileType::Fifo => "pipe",
        FileType::Socket => "socket",
        _ => "anon_inode",
    };
    alloc::format!("{what}:[{}]", file.node().ino()).into_bytes()
}

/// `MapName` for a mapping of `vma`.
fn map_name(vma: &Vma<FileRef>, ctx: Option<&FsContext>) -> (MapName, (u32, u32), u64) {
    match (&vma.backing, vma.kind) {
        (_, Kind::Heap) => (MapName::Heap, (0, 0), 0),
        (_, Kind::Stack) => (MapName::Stack, (0, 0), 0),
        (Backing::Anonymous, _) => (MapName::None, (0, 0), 0),
        (Backing::File { file, .. }, _) => {
            let source = file.source();
            if let Some(open) = source.file() {
                let node = open.node();
                let dev = node.mount().dev();
                let path = ctx.and_then(|c| vfs().file_path(c, open).ok()).unwrap_or_default();
                (MapName::Path(path), (dev.major, dev.minor), node.ino())
            } else if let Some(label) = source.label() {
                (MapName::Label(String::from(label)), (0, 0), 0)
            } else {
                (MapName::None, (0, 0), 0)
            }
        }
    }
}

/// The waitpid(2) status word of a process that ended.
fn wait_status(s: ExitStatus) -> i32 {
    match s {
        ExitStatus::Exited(code) => i32::from(code) << 8,
        ExitStatus::Signaled { signal, core } => signal as i32 | if core { 0x80 } else { 0 },
        ExitStatus::Faulted(_) => 0,
    }
}

fn signal_sets(p: &Process) -> (SignalSets, u64) {
    use poly_personality_linux::abi::signal::{SIG_DFL, SIG_IGN};
    use poly_personality_linux::fs::rlimit::RLIMIT_RSS;
    let Some(linux) = p.linux.as_ref() else { return (SignalSets::default(), u64::MAX) };
    linux.with(|lp| {
        let main = lp.thread_ids().next();
        let mut s = SignalSets {
            shared_pending: lp.shared.set().0,
            queue_limit: lp.queue_limit as u64,
            queued: lp.shared.len() as u64,
            ..SignalSets::default()
        };
        if let Some(tid) = main {
            let t = lp.thread(tid);
            s.pending = t.pending.set().0;
            s.blocked = t.mask.0;
            s.queued += t.pending.len() as u64;
        }
        for signal in 1..=poly_personality_linux::abi::signal::NSIG {
            let bit = 1u64 << (signal - 1);
            match lp.actions.get(signal).handler {
                SIG_IGN => s.ignored |= bit,
                SIG_DFL => {}
                _ => s.caught |= bit,
            }
        }
        (s, lp.limits.get(RLIMIT_RSS).map_or(u64::MAX, |l| l.cur))
    })
}

impl ProcSource for KernelProc {
    fn current_pid(&self) -> Option<u32> {
        user::current_process().map(|p| p.pid)
    }

    fn pids(&self) -> Vec<u32> {
        process::all().iter().map(|p| p.pid).collect()
    }

    fn process(&self, pid: u32) -> Option<ProcessInfo> {
        let p = find(pid)?;
        let (pgrp, session) = crate::linux::task::group_and_session(pid);
        let current = user::current_process().is_some_and(|c| c.pid == pid);
        let state = if p.has_ended() {
            ProcState::Zombie
        } else if p.is_stopped() {
            ProcState::Stopped
        } else if current {
            ProcState::Running
        } else {
            ProcState::Sleeping
        };
        // proc(5): real, effective, saved set and file system ids, of
        // the main thread (ADR 0045: credentials are per thread).
        let cred = crate::linux::cred::process_creds(&p);
        let fds = p.files.fd_list();
        let slots = fds.last().map_or(0, |(fd, _, _)| *fd as u32 + 1).next_multiple_of(64).max(64);
        let memory = memory_of(pid);
        let (signals, rss_limit) = signal_sets(&p);
        let cpu = crate::cputime::process(&p);
        let rec = with_record(pid, |r| (r.comm.clone(), r.start_ns, r.start_stack, r.arg, r.env, r.exe.clone()));
        let (comm, start_ns, start_stack, arg, env, exe) =
            rec.unwrap_or_else(|| (b"?".to_vec(), 0, 0, (0, 0), (0, 0), Vec::new()));
        // The name prctl(PR_SET_NAME) may have changed (ADR 0045).
        let comm = crate::linux::cred::process_comm(&p).unwrap_or(comm);
        Some(ProcessInfo {
            pid,
            ppid: crate::linux::task::parent_of(&p),
            pgrp: pgrp as u32,
            session: session as u32,
            comm,
            state,
            uids: [cred.uid, cred.euid, cred.suid, cred.fsuid],
            gids: [cred.gid, cred.egid, cred.sgid, cred.fsgid],
            umask: p.files.current_umask(),
            threads: p.thread_count() as u32,
            fd_slots: slots,
            start_ns,
            user_ns: cpu.user_ns,
            system_ns: cpu.system_ns,
            resident_pages: memory.as_ref().map_or(0, |m| m.resident_pages()),
            resident_file_pages: memory.as_ref().map_or(0, |m| m.cached_pages()),
            rss_limit,
            start_stack,
            start_brk: memory.as_ref().and_then(|m| m.brk_start()).unwrap_or(0),
            arg,
            env,
            exe: self.exe(pid).or(Some(exe)),
            signals,
            exit_code: p.status().map_or(0, wait_status),
        })
    }

    fn owner(&self, pid: u32) -> Option<Owner> {
        let p = find(pid)?;
        // proc(5): owned by the process's effective user and group ids.
        let cred = crate::linux::cred::process_creds(&p);
        Some(Owner { uid: cred.euid, gid: cred.egid, start_ns: with_record(pid, |r| r.start_ns).unwrap_or(0) })
    }

    fn threads(&self, pid: u32) -> Vec<u32> {
        let Some(p) = find(pid) else { return Vec::new() };
        match p.linux.as_ref() {
            Some(l) => l.with(|s| s.thread_ids().map(|t| t as u32).collect()),
            None => alloc::vec![pid],
        }
    }

    fn thread_comm(&self, pid: u32, tid: u32) -> Option<Vec<u8>> {
        let p = find(pid)?;
        let comm = p.linux.as_ref()?.with(|s| s.has_thread(tid as i32).then(|| s.thread(tid as i32).comm))?;
        let len = comm.iter().position(|&b| b == 0).unwrap_or(comm.len());
        Some(comm[..len].to_vec())
    }

    fn maps(&self, pid: u32) -> Vec<MapEntry> {
        let Some(memory) = memory_of(pid) else { return Vec::new() };
        let ctx = root();
        memory
            .regions()
            .iter()
            .map(|vma| {
                let (name, dev, ino) = map_name(vma, ctx.as_ref());
                let offset = match vma.backing {
                    Backing::File { offset, .. } => offset,
                    Backing::Anonymous => 0,
                };
                MapEntry {
                    start: vma.start,
                    end: vma.end,
                    read: vma.prot.contains(Prot::READ),
                    write: vma.prot.contains(Prot::WRITE),
                    exec: vma.prot.contains(Prot::EXEC),
                    shared: vma.is_shared(),
                    offset,
                    dev,
                    ino,
                    name,
                    locked: vma.locked,
                }
            })
            .collect()
    }

    fn cmdline(&self, pid: u32) -> Vec<u8> {
        self.strings(pid, |r| r.arg)
    }

    fn environ(&self, pid: u32) -> Vec<u8> {
        self.strings(pid, |r| r.env)
    }

    fn exe(&self, pid: u32) -> Option<Vec<u8>> {
        let (file, path) = with_record(pid, |r| (r.exe_file.upgrade(), r.exe.clone()))?;
        match (file, root()) {
            (Some(f), Some(ctx)) => vfs().file_path(&ctx, &f).ok().or(Some(path)),
            _ => Some(path),
        }
    }

    fn cwd(&self, pid: u32) -> Option<Vec<u8>> {
        let p = find(pid)?;
        let cwd = p.files.context().cwd().clone();
        vfs().path_of(&root()?, &cwd).ok()
    }

    fn root(&self, pid: u32) -> Option<Vec<u8>> {
        let p = find(pid)?;
        let dir = p.files.context().root().clone();
        vfs().path_of(&root()?, &dir).ok()
    }

    fn fds(&self, pid: u32) -> Vec<u32> {
        find(pid).map_or_else(Vec::new, |p| p.files.fd_list().iter().map(|(fd, _, _)| *fd as u32).collect())
    }

    fn fd(&self, pid: u32, fd: u32) -> Option<FdInfo> {
        let p = find(pid)?;
        let (_, file, cloexec) = p.files.fd_list().into_iter().find(|(n, _, _)| *n as u32 == fd)?;
        let target =
            root().and_then(|ctx| vfs().file_path(&ctx, &file).ok()).unwrap_or_else(|| anonymous_target(&file));
        let mut flags = file.flags().bits();
        if cloexec {
            flags |= poly_procfs::format::O_CLOEXEC;
        }
        Some(FdInfo {
            target,
            // Unknown while a transfer of this description is under way
            // (perhaps the very read of this file): then 0.
            pos: file.offset_now().unwrap_or(0),
            flags,
            mnt_id: file.node().mount().id(),
            ino: file.node().ino(),
            read: file.readable(),
            write: file.writable(),
        })
    }

    fn mounts(&self, pid: u32) -> Vec<MountEntry> {
        // The mounts of the process's mount namespace, as paths from its
        // root (ADR 0054).
        let Some(ctx) = find(pid).map(|p| p.files.context()).or_else(root) else { return Vec::new() };
        let namespace = vfs().namespace_of(&ctx);
        vfs()
            .namespace_mounts(namespace)
            .iter()
            .filter_map(|m| {
                let at = vfs().mount_path(&ctx, m.id).ok()?;
                Some(MountEntry {
                    id: m.id,
                    parent: m.parent.map_or(m.id, |(p, _)| p),
                    dev: (m.dev.major, m.dev.minor),
                    root: b"/".to_vec(),
                    mount_point: at,
                    source: super::mounts::source(m.id)
                        .unwrap_or_else(|| super::disk::source_of(m.id, m.fs_name).into_bytes()),
                    fstype: String::from(m.fs_name),
                    read_only: m.flags.read_only,
                    no_suid: m.flags.no_suid,
                    no_dev: m.flags.no_dev,
                    no_exec: m.flags.no_exec,
                })
            })
            .collect()
    }

    fn system(&self) -> SystemInfo {
        let now = time::now_ns();
        let (total, free) =
            crate::frames::FRAMES.lock().as_ref().map_or((0, 0), |f| (f.total_frames() * PAGE, f.free_frames() * PAGE));
        let c = super::cache::cache().stats();
        let cached = c.pages as u64 * PAGE;
        let dirty = c.dirty as u64 * PAGE;
        let threads = thread::list();
        let runnable = threads
            .iter()
            .filter(|t| matches!(t.state, poly_sched::State::Running | poly_sched::State::Runnable))
            .count() as u32;
        let procs = process::all();
        SystemInfo {
            uptime_ns: now,
            idle_ns: now,
            boot_time: (time::realtime_ns().saturating_sub(now)) / time::NANOS_PER_SEC,
            mem_total: total,
            mem_free: free,
            mem_available: free + cached.saturating_sub(dirty),
            buffers: 0,
            cached,
            dirty,
            shmem: 0,
            kernel_stack: 0,
            page_tables: 0,
            loads: [0; 3],
            runnable,
            threads: threads.len() as u32,
            last_pid: procs.iter().map(|p| p.pid).max().unwrap_or(0),
            procs_running: runnable,
            procs_blocked: 0,
            cpu: CpuTimes { idle: now, ..CpuTimes::default() },
            interrupts: time::ticks(),
            context_switches: 0,
            forks: procs.iter().map(|p| u64::from(p.pid)).max().unwrap_or(0),
        }
    }

    fn boot_time(&self) -> u64 {
        time::realtime_ns().saturating_sub(time::now_ns()) / time::NANOS_PER_SEC
    }

    fn cpus(&self) -> Vec<CpuInfo> {
        let khz = arch::time::info().and_then(|i| i.tsc_hz).map_or(0, |hz| hz / 1000);
        crate::arch::percpu::all()
            .filter(|c| crate::smp::online_cpus().contains(c.index()))
            .map(|c| arch::cpuinfo::cpu_info(c.index() as u32, c.apic_id(), khz))
            .collect()
    }

    fn filesystems(&self) -> Vec<(String, bool)> {
        ["sysfs", "tmpfs", "proc"]
            .iter()
            .map(|n| (String::from(*n), true))
            .chain(["ext2", "ext3", "ext4"].iter().map(|n| (String::from(*n), false)))
            .collect()
    }

    fn kernel(&self) -> KernelInfo {
        use poly_personality_linux::sys::system::{UNAME_RELEASE, UNAME_SYSNAME, UNAME_VERSION};
        // The names of the reader's UTS namespace (ADR 0054).
        let uts = user::current_process().and_then(|p| p.linux.as_ref().and_then(|l| l.with(|s| s.ns.uts.clone())));
        let (hostname, domainname) = match uts {
            Some(u) => {
                let n = u.names();
                (n.hostname, n.domainname)
            }
            None => (crate::linux::admin::hostname(), crate::linux::admin::domainname()),
        };
        KernelInfo {
            ostype: String::from(UNAME_SYSNAME),
            osrelease: String::from(UNAME_RELEASE),
            version: String::from(UNAME_VERSION),
            hostname,
            domainname,
        }
    }

    fn random_uuid(&self) -> [u8; 16] {
        let mut b = [0u8; 16];
        random::fill(&mut b);
        b
    }

    fn boot_id(&self) -> [u8; 16] {
        random::boot_id()
    }

    fn entropy_avail(&self) -> u32 {
        random::entropy_avail()
    }

    fn namespace(&self, pid: u32, kind: usize) -> Option<u32> {
        let kind = ns::Kind::from_index(kind as u32)?;
        let p = find(pid)?;
        let (namespaces, user_ns) = namespaces_of(&p)?;
        namespaces.entry_inum(kind, &user_ns)
    }

    fn namespace_exists(&self, inum: u32, kind: usize) -> bool {
        ns::Kind::from_index(kind as u32).is_some_and(|k| ns::lookup(inum, k).is_some())
    }

    fn pin_namespace(&self, inum: u32, kind: usize, pin: bool) {
        if let Some(k) = ns::Kind::from_index(kind as u32) {
            ns::pin(inum, k, pin);
        }
    }

    fn write_kernel_name(&self, which: KernelName, _offset: u64, data: &[u8]) -> Result<usize, poly_vfs::Errno> {
        use poly_personality_linux::sys::admin::{may_set_names, sysctl_name};
        // The writer's UTS namespace, with sethostname(2)'s privilege
        // (sethostname's spec sheet: "with the same permission").
        let creds = crate::linux::cred::current_creds().ok_or(poly_vfs::Errno::EPERM)?;
        let uts = user::current_process().and_then(|p| p.linux.as_ref().and_then(|l| l.with(|s| s.ns.uts.clone())));
        if !may_set_names(&creds, &uts) {
            return Err(poly_vfs::Errno::EPERM);
        }
        let name = sysctl_name(data);
        match (uts, which) {
            (Some(u), KernelName::Host) => u.set_hostname(name),
            (Some(u), KernelName::Domain) => u.set_domainname(name),
            (None, KernelName::Host) => crate::linux::admin::set_name(SystemName::Host, name),
            (None, KernelName::Domain) => crate::linux::admin::set_name(SystemName::Domain, name),
        }
        Ok(data.len())
    }

    fn id_map(&self, pid: u32, file: IdMapFile) -> Vec<u8> {
        let Some((_, target)) = find(pid).and_then(|p| namespaces_of(&p)) else { return Vec::new() };
        let reader = crate::linux::cred::current_creds().and_then(|c| c.user_ns);
        match file {
            IdMapFile::Uid => ns::user::format_map(&target, ns::user::IdKind::Uid, &reader),
            IdMapFile::Gid => ns::user::format_map(&target, ns::user::IdKind::Gid, &reader),
            IdMapFile::Setgroups => ns::user::format_setgroups(&target),
        }
    }

    fn write_id_map(&self, pid: u32, file: IdMapFile, offset: u64, data: &[u8]) -> Result<usize, poly_vfs::Errno> {
        let (_, target) = find(pid).and_then(|p| namespaces_of(&p)).ok_or(poly_vfs::Errno::ESRCH)?;
        let writer = crate::linux::cred::current_creds().ok_or(poly_vfs::Errno::EPERM)?;
        let written = match file {
            IdMapFile::Uid => ns::user::write_map(&target, ns::user::IdKind::Uid, data, offset, &writer),
            IdMapFile::Gid => ns::user::write_map(&target, ns::user::IdKind::Gid, data, offset, &writer),
            IdMapFile::Setgroups => ns::user::write_setgroups(&target, data, offset, &writer),
        };
        written.map_err(|e| poly_vfs::Errno::from_code(i32::from(e.get())).unwrap_or(poly_vfs::Errno::EIO))
    }
}

/// Process `p`'s namespaces and its main thread's user namespace (ADR
/// 0054), if it is a Linux process.
fn namespaces_of(p: &Process) -> Option<(ns::Namespaces, ns::user::UserNsRef)> {
    let pid = p.pid as i32;
    p.linux.as_ref()?.with(|s| Some((s.ns.clone(), s.process_creds(pid)?.user_ns.clone())))
}

/// Bytes per page.
const PAGE: u64 = 4096;

/// The most a `cmdline` or `environ` read returns.
const STRINGS_MAX: u64 = 128 * 1024;

impl KernelProc {
    /// The bytes of `range` (from the record) in the process's memory.
    fn strings(&self, pid: u32, range: impl FnOnce(&Record) -> (u64, u64)) -> Vec<u8> {
        let Some((start, end)) = with_record(pid, range) else { return Vec::new() };
        let Some(memory) = memory_of(pid) else { return Vec::new() };
        if find(pid).is_none_or(|p| p.has_ended()) {
            return Vec::new();
        }
        let len = end.saturating_sub(start).min(STRINGS_MAX) as usize;
        let mut buf = alloc::vec![0u8; len];
        match memory.read(start, &mut buf) {
            Ok(()) => buf,
            Err(_) => Vec::new(),
        }
    }
}

/// A procfs over the kernel's processes: what `/proc` has, and what a
/// mount(2) of `proc` gets (ADR 0051).
pub fn new_procfs() -> Arc<dyn poly_vfs::FileSystem> {
    Arc::new(ProcFs::new(Arc::new(KernelProc)))
}

/// A file system mounted at boot: path, file system, mode of its root,
/// `MS_NOEXEC`.
type BootMount = (&'static [u8], Arc<dyn poly_vfs::FileSystem>, u16, bool);

/// The memory file systems a Linux system has at boot (ADR 0122): procfs
/// on `/proc`, sysfs on `/sys`, tmpfs on `/dev/shm`, the POSIX message
/// queue file system on `/dev/mqueue` (mq_overview(7); a tmpfs named
/// `mqueue`), tmpfs on `/run` and `/run/lock`. Each directory is made if
/// missing, in this order (`/run/lock` on the new `/run`). Called once at
/// boot by `fs::init`.
pub fn mount_all(vfs: &Vfs, ctx: &FsContext) {
    let root = Cred::root();
    let mut made = ctx.clone();
    made.set_umask(0);
    let clock = super::clock;
    let tmpfs = || -> Arc<dyn poly_vfs::FileSystem> { super::memfs::cached(super::memfs::new_tmpfs(clock())) };
    let mounts: [BootMount; 6] = [
        (b"/proc", new_procfs(), 0o555, true),
        (b"/sys", super::sysfs::build(), 0o555, true),
        (b"/dev/shm", tmpfs(), 0o1777, false),
        (b"/dev/mqueue", Arc::new(poly_vfs::Tmpfs::new(clock()).named("mqueue", poly_vfs::MQUEUE_MAGIC)), 0o1777, true),
        (b"/run", tmpfs(), 0o755, false),
        (b"/run/lock", tmpfs(), 0o1777, true),
    ];
    let total = mounts.len();
    let mut ok = 0;
    for (path, fs, perm, no_exec) in mounts {
        let _ = vfs.mkdir(&made, &root, At::Cwd, path, perm);
        let flags = MountFlags { no_suid: true, no_dev: true, no_exec, read_only: false };
        match vfs.mount(ctx, &root, At::Cwd, path, fs, flags) {
            Ok(_) => {
                ok += 1;
                // A new tmpfs's root is 0755; set what the directory is for.
                let _ = vfs.chmod(ctx, &root, At::Cwd, path, perm, true);
            }
            Err(e) => kprintln!("fs: mounting on {}: {}", String::from_utf8_lossy(path), e.name()),
        }
    }
    kprintln!("fs: proc, sysfs, tmpfs on /dev/shm, /run, /run/lock, mqueue on /dev/mqueue; {ok} of {total} mounted");
}

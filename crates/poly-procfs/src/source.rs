//! What procfs needs from the kernel: the [`ProcSource`] trait and the
//! plain data it hands out. procfs formats; the kernel only reports facts.

use alloc::string::String;
use alloc::vec::Vec;
use poly_vfs::Errno;

/// Clock ticks per second in the files that count ticks (`/proc/stat`,
/// `/proc/<pid>/stat`): `USER_HZ`, which `sysconf(_SC_CLK_TCK)` reports as
/// 100 on x86-64 (proc_pid_stat(5), proc_stat(5): "1/100ths of a second on
/// most architectures").
pub const USER_HZ: u64 = 100;

/// Nanoseconds per clock tick.
pub const NANOS_PER_TICK: u64 = 1_000_000_000 / USER_HZ;

/// A process's state, as `/proc/<pid>/stat` and `status` show it
/// (proc_pid_stat(5)).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProcState {
    /// `R`: running or runnable.
    Running,
    /// `S`: sleeping in an interruptible wait.
    #[default]
    Sleeping,
    /// `T`: stopped by a signal.
    Stopped,
    /// `Z`: ended, not yet waited for.
    Zombie,
}

impl ProcState {
    /// The letter of `/proc/<pid>/stat`.
    pub const fn letter(self) -> char {
        match self {
            ProcState::Running => 'R',
            ProcState::Sleeping => 'S',
            ProcState::Stopped => 'T',
            ProcState::Zombie => 'Z',
        }
    }

    /// The text of the `State:` line of `/proc/<pid>/status`.
    pub const fn status(self) -> &'static str {
        match self {
            ProcState::Running => "R (running)",
            ProcState::Sleeping => "S (sleeping)",
            ProcState::Stopped => "T (stopped)",
            ProcState::Zombie => "Z (zombie)",
        }
    }
}

/// Signal sets of a process, bit `n - 1` for signal `n`
/// (`/proc/<pid>/status`: `SigPnd`, `ShdPnd`, `SigBlk`, `SigIgn`, `SigCgt`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SignalSets {
    /// Pending for the thread reported on (the main thread).
    pub pending: u64,
    /// Pending for the process as a whole.
    pub shared_pending: u64,
    pub blocked: u64,
    pub ignored: u64,
    pub caught: u64,
    /// Signals queued, and the most that may be (`SigQ`).
    pub queued: u64,
    pub queue_limit: u64,
}

/// Facts about one process.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProcessInfo {
    pub pid: u32,
    pub ppid: u32,
    pub pgrp: u32,
    pub session: u32,
    /// The command name (`comm`), at most 15 bytes are shown.
    pub comm: Vec<u8>,
    pub state: ProcState,
    /// Real, effective, saved and file-system user ids, and the same for
    /// groups.
    pub uids: [u32; 4],
    pub gids: [u32; 4],
    pub umask: u16,
    pub threads: u32,
    /// Descriptor slots of the table (`FDSize`).
    pub fd_slots: u32,
    /// When it started, in nanoseconds after boot.
    pub start_ns: u64,
    /// CPU time in user and kernel mode, in nanoseconds (0 where the
    /// kernel does not account it).
    pub user_ns: u64,
    pub system_ns: u64,
    /// Pages with a frame (`VmRSS`), and those of them that map files
    /// (`RssFile`).
    pub resident_pages: u64,
    pub resident_file_pages: u64,
    /// The soft `RLIMIT_RSS` in bytes (`rsslim`).
    pub rss_limit: u64,
    /// Where the initial stack pointer was, the program break started, and
    /// the argument and environment strings lie (0 where unknown).
    pub start_stack: u64,
    pub start_brk: u64,
    pub arg: (u64, u64),
    pub env: (u64, u64),
    /// The executable's path, to tell its mappings (`VmExe`, `startcode`)
    /// from libraries'.
    pub exe: Option<Vec<u8>>,
    pub signals: SignalSets,
    /// The exit status in `waitpid` form, once it has ended.
    pub exit_code: i32,
}

/// What names a mapping in `/proc/<pid>/maps`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum MapName {
    /// Anonymous memory: nothing.
    #[default]
    None,
    /// A file, by its path (`" (deleted)"` already appended if it was).
    Path(Vec<u8>),
    /// `[heap]`: the program break area.
    Heap,
    /// `[stack]`: the main thread's stack.
    Stack,
    /// Any other pseudo-path, such as `[vdso]`, written as given.
    Label(String),
}

/// Who owns a process directory (see [`ProcSource::owner`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Owner {
    pub uid: u32,
    pub gid: u32,
    pub start_ns: u64,
}

/// One mapping of `/proc/<pid>/maps`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MapEntry {
    pub start: u64,
    pub end: u64,
    pub read: bool,
    pub write: bool,
    pub exec: bool,
    /// `MAP_SHARED`: `s`, else `p`.
    pub shared: bool,
    /// File offset of `start` (0 for anonymous memory).
    pub offset: u64,
    /// The file's device (major, minor) and inode; zeros for anonymous
    /// memory.
    pub dev: (u32, u32),
    pub ino: u64,
    pub name: MapName,
}

impl MapEntry {
    pub fn size(&self) -> u64 {
        self.end - self.start
    }

    /// Whether a file backs it.
    pub fn is_file(&self) -> bool {
        matches!(self.name, MapName::Path(_))
    }
}

/// One open descriptor of a process.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FdInfo {
    /// What `/proc/<pid>/fd/<n>` links to: a path, or `type:[inode]` for
    /// files without one (proc_pid_fd(5)).
    pub target: Vec<u8>,
    /// The file offset (`pos`).
    pub pos: u64,
    /// Access mode and status flags, with `O_CLOEXEC` when the descriptor
    /// has close-on-exec set (proc_pid_fdinfo(5)).
    pub flags: u32,
    pub mnt_id: u32,
    pub ino: u64,
    /// Whether it is open for reading and for writing (the link's mode).
    pub read: bool,
    pub write: bool,
}

/// One mount, for `/proc/<pid>/mounts` and `mountinfo`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MountEntry {
    pub id: u32,
    /// The mount it is mounted on (itself for the root).
    pub parent: u32,
    pub dev: (u32, u32),
    /// The directory of the file system shown there (`/` but for bind
    /// mounts).
    pub root: Vec<u8>,
    /// Where it is mounted, from the process's root.
    pub mount_point: Vec<u8>,
    /// What is mounted (`virtio2`, `proc`, `none`).
    pub source: Vec<u8>,
    pub fstype: String,
    pub read_only: bool,
    pub no_suid: bool,
    pub no_dev: bool,
    pub no_exec: bool,
}

/// CPU time of the system in nanoseconds, by what it was spent on
/// (`/proc/stat`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuTimes {
    pub user: u64,
    pub nice: u64,
    pub system: u64,
    pub idle: u64,
    pub iowait: u64,
    pub irq: u64,
    pub softirq: u64,
    pub steal: u64,
}

/// System-wide facts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SystemInfo {
    /// Nanoseconds since boot, and spent idle.
    pub uptime_ns: u64,
    pub idle_ns: u64,
    /// The wall-clock time of boot, in seconds since the epoch (`btime`).
    pub boot_time: u64,
    /// Memory in bytes (`/proc/meminfo`).
    pub mem_total: u64,
    pub mem_free: u64,
    pub mem_available: u64,
    pub buffers: u64,
    pub cached: u64,
    pub dirty: u64,
    pub shmem: u64,
    pub kernel_stack: u64,
    pub page_tables: u64,
    /// Load averages over 1, 5 and 15 minutes, fixed point with 16
    /// fractional bits (sysinfo(2)'s `loads`).
    pub loads: [u64; 3],
    /// Scheduling entities runnable and existing, and the newest pid
    /// (`/proc/loadavg`).
    pub runnable: u32,
    pub threads: u32,
    pub last_pid: u32,
    /// Processes running and blocked (`/proc/stat`).
    pub procs_running: u32,
    pub procs_blocked: u32,
    pub cpu: CpuTimes,
    /// Interrupts serviced, context switches, processes created.
    pub interrupts: u64,
    pub context_switches: u64,
    pub forks: u64,
}

/// One CPU as CPUID describes it (`/proc/cpuinfo`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CpuInfo {
    /// The CPU's number.
    pub index: u32,
    /// The vendor string (CPUID leaf 0), the brand string (leaves
    /// 0x80000002-4, trimmed).
    pub vendor: String,
    pub model_name: String,
    /// Family, model and stepping as the SDM combines them (extended
    /// family and model included).
    pub family: u32,
    pub model: u32,
    pub stepping: u32,
    /// The highest standard leaf.
    pub cpuid_level: u32,
    /// The clock in kHz (from the TSC calibration).
    pub khz: u64,
    pub apic_id: u32,
    /// CLFLUSH line size in bytes.
    pub clflush: u32,
    /// Physical and linear address bits (leaf 0x80000008).
    pub phys_bits: u32,
    pub virt_bits: u32,
    /// The feature registers flags are named from: leaf 1 EDX and ECX,
    /// leaf 7 subleaf 0 EBX, ECX and EDX, leaf 0x80000001 EDX and ECX.
    pub features: CpuFeatures,
}

/// Raw CPUID feature registers (see [`crate::cpuflags`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuFeatures {
    pub leaf1_edx: u32,
    pub leaf1_ecx: u32,
    pub leaf7_ebx: u32,
    pub leaf7_ecx: u32,
    pub leaf7_edx: u32,
    pub ext1_edx: u32,
    pub ext1_ecx: u32,
}

/// The kernel's identity (`/proc/version`, `/proc/sys/kernel/*`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KernelInfo {
    /// `ostype`: `Linux` for the Linux personality.
    pub ostype: String,
    /// `osrelease`, as uname(2) reports it.
    pub osrelease: String,
    /// `version`, as uname(2) reports it.
    pub version: String,
    pub hostname: Vec<u8>,
    pub domainname: Vec<u8>,
}

/// The facts procfs shows, provided by the kernel. Every method is called
/// without procfs or VFS locks held, from the thread that reads `/proc`,
/// so the kernel may take its own locks (not the reader's file-system
/// context lock: a path walk can be in progress) and allocate. Methods
/// about a process return `None` (or nothing) when there is no such
/// process.
pub trait ProcSource: Send + Sync {
    /// The process (thread group) the calling thread belongs to: what
    /// `/proc/self` names. `None` for kernel threads.
    fn current_pid(&self) -> Option<u32>;

    /// Every process, in any order.
    fn pids(&self) -> Vec<u32>;

    fn process(&self, pid: u32) -> Option<ProcessInfo>;

    /// Whether process `pid` exists, and its effective user and group ids
    /// and start time (nanoseconds after boot): what `stat` of its
    /// directory needs, cheaply. It must not take the process's file
    /// system context lock: a `chdir` into `/proc` holds it while it asks.
    fn owner(&self, pid: u32) -> Option<Owner>;

    /// The mappings, in address order.
    fn maps(&self, pid: u32) -> Vec<MapEntry>;

    /// The argument and environment strings, each with its NUL, as they
    /// are in the process's memory (proc_pid_cmdline(5)).
    fn cmdline(&self, pid: u32) -> Vec<u8>;
    fn environ(&self, pid: u32) -> Vec<u8>;

    /// The executable's, working directory's and root directory's paths.
    fn exe(&self, pid: u32) -> Option<Vec<u8>>;
    fn cwd(&self, pid: u32) -> Option<Vec<u8>>;
    fn root(&self, pid: u32) -> Option<Vec<u8>>;

    /// The open descriptors, in any order.
    fn fds(&self, pid: u32) -> Vec<u32>;
    fn fd(&self, pid: u32, fd: u32) -> Option<FdInfo>;

    /// The mounts the process sees, in mount order.
    fn mounts(&self, pid: u32) -> Vec<MountEntry>;

    fn system(&self) -> SystemInfo;
    /// The wall-clock time of boot in seconds since the epoch (what
    /// [`SystemInfo::boot_time`] holds), cheaply: every `stat` of a procfs
    /// file asks.
    fn boot_time(&self) -> u64;
    fn cpus(&self) -> Vec<CpuInfo>;
    /// The file system types, and whether each needs no device (`nodev`).
    fn filesystems(&self) -> Vec<(String, bool)>;
    fn kernel(&self) -> KernelInfo;

    /// 16 random bytes (for `/proc/sys/kernel/random/uuid`, fresh each
    /// read) and the boot's id (`boot_id`, the same all boot).
    fn random_uuid(&self) -> [u8; 16];
    fn boot_id(&self) -> [u8; 16];
    /// Bits of entropy the random generator holds (`entropy_avail`).
    fn entropy_avail(&self) -> u32;

    /// The inode number of process `pid`'s namespace of kind `kind` (an
    /// index into [`NS_KINDS`](crate::NS_KINDS)): what `/proc/<pid>/ns/`
    /// shows. `None` (the default): the process has none of that kind.
    fn namespace(&self, pid: u32, kind: usize) -> Option<u32> {
        let _ = (pid, kind);
        None
    }

    /// Whether a namespace of kind `kind` with inode number `inum` exists.
    fn namespace_exists(&self, inum: u32, kind: usize) -> bool {
        let _ = (inum, kind);
        false
    }

    /// The text of process `pid`'s `uid_map`, `gid_map` or `setgroups`
    /// as the calling thread sees it (user_namespaces(7)).
    fn id_map(&self, pid: u32, file: IdMapFile) -> Vec<u8> {
        let _ = (pid, file);
        Vec::new()
    }

    /// A write of `data` at `offset` to one of those files by the calling
    /// thread: the bytes taken, or the error. `EROFS` by default.
    fn write_id_map(&self, pid: u32, file: IdMapFile, offset: u64, data: &[u8]) -> Result<usize, Errno> {
        let _ = (pid, file, offset, data);
        Err(Errno::EROFS)
    }
}

/// The files of a process directory that set up its user namespace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdMapFile {
    Uid,
    Gid,
    Setgroups,
}

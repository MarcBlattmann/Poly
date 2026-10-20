//! A fake kernel for host tests: user memory as a map of 4 KiB pages with
//! protections, files as a map of fds, VMAs as the set of mapped pages,
//! fixed clocks and a deterministic random source, processes with their
//! [`LinuxProcess`] state, and a futex table.
//!
//! The fake stands for one calling thread (`pid`, `tid`) at a time. Other
//! threads act through [`FakeKernel::as_thread`], typically from a
//! "meanwhile" step ([`FakeKernel::meanwhile`]): when the calling thread
//! blocks, the fake runs the next step, then reports what woke the thread
//! (its signal notification, an unblock, or the deadline).

extern crate std;

use crate::Linux;
use crate::abi::{FileStat, PAGE_SIZE, Timespec, ioctl, mode};
use crate::cred::Creds;
use crate::errno::Errno;
use crate::futex::FutexTable;
use crate::kernel::{
    Backing, Clock, CpuTimes, CpuTimesOf, Credentials, Deadline, FpuState, LinuxKernel, MapRequest, Placement,
    ProcessInfo, ProcessSelector, Prot, RandomMode, SigSet, SignalActions, Target, UserContext, UserPtr, Wakeup,
};
use crate::signal::{ExitToUser, LinuxProcess, LinuxThread};
use crate::time::TimerWakeup;
use poly_abi::{Outcome, SyscallRegs};
use std::boxed::Box;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::vec;
use std::vec::Vec;

/// Fake user space ends here (the x86-64 user half minus its top page).
pub const USER_END: u64 = 0x7fff_ffff_f000;
/// The lowest address a fixed mapping may use (like `mmap_min_addr`).
pub const MIN_ADDR: u64 = 0x1_0000;
/// Where the kernel's own choices for mappings start.
pub const MMAP_BASE: u64 = 0x7f00_0000_0000;

#[derive(Clone, Debug)]
pub struct Page {
    pub prot: u32,
    pub data: Box<[u8; PAGE_SIZE as usize]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Pipe,
    Tty,
    /// A character device that is not a terminal, like `/dev/null`.
    NullDevice,
    Regular,
}

#[derive(Clone, Debug)]
pub struct File {
    pub kind: Kind,
    pub readable: bool,
    pub writable: bool,
    /// Everything written so far.
    pub written: Vec<u8>,
    /// What reads return, consumed from the front.
    pub input: Vec<u8>,
    /// Accept at most this many bytes per write (a full pipe).
    pub max_write: Option<usize>,
    /// Fail every write once this many bytes have been written.
    pub fail_after: Option<(usize, Errno)>,
}

impl File {
    pub fn new(kind: Kind) -> Self {
        File {
            kind,
            readable: true,
            writable: true,
            written: Vec::new(),
            input: Vec::new(),
            max_write: None,
            fail_after: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    Thread(u8),
    Group(u8),
    /// kill_process(signal, core).
    Signal(u32, bool),
}

/// A process the fake knows.
#[derive(Clone, Debug)]
pub struct FakeProc {
    pub info: ProcessInfo,
    pub state: LinuxProcess,
}

/// What other threads do while the calling thread is blocked.
pub type Step = Box<dyn FnMut(&mut FakeKernel)>;

/// The fake's user stack: 64 KiB below this address.
pub const STACK_TOP: u64 = 0x1001_0000;
pub const STACK_SIZE: u64 = 0x1_0000;
/// The selectors the fake's threads run with.
pub const USER_CS: u16 = 0x33;
pub const USER_SS: u16 = 0x2b;
/// Where the fake's syscall instructions end (rip after `syscall`).
pub const SYSCALL_RIP: u64 = 0x40_1002;

pub struct FakeKernel {
    pub pages: BTreeMap<u64, Page>,
    pub files: BTreeMap<i32, File>,
    pub start_brk: u64,
    pub cur_brk: u64,
    pub fs_base: u64,
    pub gs_base: u64,
    pub exit: Option<Exit>,
    pub pid: i32,
    pub tid: i32,
    pub ppid: i32,
    pub realtime: Timespec,
    pub monotonic: Timespec,
    pub random_ready: bool,
    pub random_next: u8,
    /// Every mmap request that reached the kernel.
    pub maps: Vec<MapRequest>,
    /// Every (fd, request, arg) ioctl that reached the kernel.
    pub ioctls: Vec<(i32, u32, u64)>,
    /// Every madvise that reached the kernel (ADR 0049).
    pub advised: Vec<(u64, u64, crate::sys::madvise::Advice)>,
    /// Every mprotect that reached the kernel.
    pub protects: Vec<(u64, u64, Prot)>,
    /// Every process, by pid.
    pub procs: BTreeMap<i32, FakeProc>,
    /// Threads whose signal notification is set.
    pub notified: BTreeSet<i32>,
    /// Threads with an unblock waiting.
    pub unblocked: BTreeSet<i32>,
    /// Steps of other threads, run one per block of the calling thread.
    pub steps: VecDeque<Step>,
    /// Calls to block().
    pub blocks: usize,
    /// The deadline of every block() call.
    pub deadlines: Vec<Option<Deadline>>,
    /// The calling thread's user registers.
    pub ctx: UserContext,
    pub fpu: Vec<u8>,
    pub xfeatures: Option<u64>,
    /// FPU images loaded by rt_sigreturn, and resets.
    pub fpu_loads: Vec<Vec<u8>>,
    pub fpu_resets: usize,
    /// Refuse FPU images (as a kernel would refuse bad MXCSR bits).
    pub refuse_fpu: bool,
    pub futexes: FutexTable,
    futexes_locked: bool,
    /// stop_process calls (signal), and the pids continue_process got.
    pub stops: Vec<u32>,
    pub continued: Vec<i32>,
    /// The file system services (`None`: `ENOSYS`), and what sysinfo
    /// reports.
    pub fs: Option<Box<dyn crate::fs::LinuxFs>>,
    pub sysinfo: Option<crate::fs::SysInfo>,
    /// The stored time zone (settimeofday).
    pub timezone: (i32, i32),
    /// CPU times the kernel accounts: the calling thread, the process, the
    /// children (`None`: not accounted).
    pub cpu: Option<[CpuTimes; 3]>,
    /// Every timers_changed report.
    pub timer_wakeups: Vec<TimerWakeup>,
    /// Children, programs and the process tree (`tests/task.rs`).
    pub tasks: super::task::Tasks,
    /// Pipes, waits and event files (`tests/fileio.rs`).
    pub io: super::fileio::FakeIo,
}

impl Default for FakeKernel {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeKernel {
    /// A process with stdin (tty), stdout (pipe), stderr (tty), a 64 KiB
    /// read-write "data" area at 0x40_0000 and a break at 0x50_0000.
    pub fn new() -> Self {
        let mut k = FakeKernel {
            pages: BTreeMap::new(),
            files: BTreeMap::new(),
            start_brk: 0x50_0000,
            cur_brk: 0x50_0000,
            fs_base: 0,
            gs_base: 0,
            exit: None,
            pid: 100,
            tid: 100,
            ppid: 1,
            realtime: Timespec { sec: 1_700_000_000, nsec: 123 },
            monotonic: Timespec { sec: 42, nsec: 999_999_999 },
            random_ready: true,
            random_next: 0,
            maps: Vec::new(),
            ioctls: Vec::new(),
            protects: Vec::new(),
            advised: Vec::new(),
            procs: BTreeMap::new(),
            notified: BTreeSet::new(),
            unblocked: BTreeSet::new(),
            steps: VecDeque::new(),
            blocks: 0,
            deadlines: Vec::new(),
            ctx: UserContext {
                rsp: STACK_TOP - 0x100,
                rip: SYSCALL_RIP,
                rflags: 0x246,
                cs: USER_CS,
                ss: USER_SS,
                ..Default::default()
            },
            fpu: (0..512).map(|i| i as u8).collect(),
            xfeatures: None,
            fpu_loads: Vec::new(),
            fpu_resets: 0,
            refuse_fpu: false,
            futexes: FutexTable::new(),
            futexes_locked: false,
            stops: Vec::new(),
            continued: Vec::new(),
            fs: None,
            sysinfo: None,
            timezone: (0, 0),
            cpu: None,
            timer_wakeups: Vec::new(),
            tasks: Default::default(),
            io: Default::default(),
        };
        k.files.insert(0, File::new(Kind::Tty));
        k.files.insert(1, File::new(Kind::Pipe));
        k.files.insert(2, File::new(Kind::Tty));
        k.map_range(DATA, 16 * PAGE_SIZE, RW);
        k.map_range(STACK_TOP - STACK_SIZE, STACK_SIZE, RW);
        let info = ProcessInfo {
            pid: 100,
            pgid: 100,
            sid: 100,
            uid: 1000,
            euid: 1001,
            suid: 1001,
            gid: 2000,
            egid: 2001,
            sgid: 2001,
            zombie: false,
        };
        k.procs.insert(100, FakeProc { info, state: LinuxProcess::new(100) });
        // Real uid 1000, effective and saved 1001; gids 2000 and 2001; no
        // capabilities.
        let mut creds = Creds::user(1000, 2000);
        creds.euid = 1001;
        creds.suid = 1001;
        creds.fsuid = 1001;
        creds.egid = 2001;
        creds.sgid = 2001;
        creds.fsgid = 2001;
        k.me().thread_mut(100).creds = creds;
        k
    }

    /// The calling thread's credentials, to change.
    pub fn creds_mut(&mut self) -> &mut Creds {
        let tid = self.tid;
        &mut self.me().thread_mut(tid).creds
    }

    /// The calling thread's credentials.
    pub fn creds(&self) -> &Creds {
        &self.procs[&self.pid].state.thread(self.tid).creds
    }

    /// Add a process `pid` with main thread `pid`, owned by `uid`, in
    /// process group `pgid` and session `sid`.
    pub fn add_process(&mut self, pid: i32, uid: u32, pgid: i32, sid: i32) -> &mut FakeProc {
        let info =
            ProcessInfo { pid, pgid, sid, uid, euid: uid, suid: uid, gid: uid, egid: uid, sgid: uid, zombie: false };
        let mut state = LinuxProcess::new(pid);
        state.thread_mut(pid).creds = Creds::user(uid, uid);
        self.procs.insert(pid, FakeProc { info, state });
        self.procs.get_mut(&pid).unwrap()
    }

    /// Add thread `tid` to the calling process (cloned from the calling
    /// thread, sharing memory).
    pub fn add_thread(&mut self, tid: i32) {
        let me = self.tid;
        self.me().add_thread(me, tid, true);
    }

    /// The calling process's state.
    pub fn me(&mut self) -> &mut LinuxProcess {
        &mut self.procs.get_mut(&self.pid).expect("calling process").state
    }

    /// A process's state.
    pub fn proc_state(&mut self, pid: i32) -> &mut LinuxProcess {
        &mut self.procs.get_mut(&pid).expect("process").state
    }

    /// The calling thread's state.
    pub fn thread_state(&self) -> &LinuxThread {
        self.procs[&self.pid].state.thread(self.tid)
    }

    /// The calling thread's mask.
    pub fn mask(&self) -> SigSet {
        self.thread_state().mask
    }

    /// The calling process's dispositions.
    pub fn actions(&self) -> SignalActions {
        self.procs[&self.pid].state.actions
    }

    /// The process that has thread `tid`.
    pub fn pid_of(&self, tid: i32) -> Option<i32> {
        self.procs.iter().find(|(_, p)| p.state.has_thread(tid)).map(|(&pid, _)| pid)
    }

    /// Run `f` as thread `tid` (of whichever process has it), with the
    /// calling thread's identity and registers put back afterwards.
    pub fn as_thread<R>(&mut self, tid: i32, f: impl FnOnce(&mut FakeKernel) -> R) -> R {
        let saved = (self.pid, self.tid, self.ctx);
        self.pid = self.pid_of(tid).expect("thread exists");
        self.tid = tid;
        let r = f(self);
        (self.pid, self.tid, self.ctx) = saved;
        r
    }

    /// Queue a step other threads take when the calling thread blocks.
    pub fn meanwhile(&mut self, step: impl FnMut(&mut FakeKernel) + 'static) {
        self.steps.push_back(Box::new(step));
    }

    /// A 32-bit word of user memory.
    pub fn peek_u32(&self, addr: u64) -> u32 {
        u32::from_le_bytes(self.peek(addr, 4).try_into().unwrap())
    }

    /// Map `[addr, addr + len)` zero-filled with `prot`, replacing pages.
    pub fn map_range(&mut self, addr: u64, len: u64, prot: u32) {
        for page in (addr / PAGE_SIZE)..(addr + len).div_ceil(PAGE_SIZE) {
            self.pages.insert(page, Page { prot, data: Box::new([0; PAGE_SIZE as usize]) });
        }
    }

    pub fn unmap_range(&mut self, addr: u64, len: u64) {
        for page in (addr / PAGE_SIZE)..(addr + len).div_ceil(PAGE_SIZE) {
            self.pages.remove(&page);
        }
    }

    fn range_is_free(&self, addr: u64, len: u64) -> bool {
        self.pages.range(addr / PAGE_SIZE..(addr + len).div_ceil(PAGE_SIZE)).next().is_none()
    }

    pub fn is_mapped(&self, addr: u64) -> bool {
        self.pages.contains_key(&(addr / PAGE_SIZE))
    }

    pub fn prot_at(&self, addr: u64) -> Option<u32> {
        self.pages.get(&(addr / PAGE_SIZE)).map(|p| p.prot)
    }

    /// Write bytes into user memory, ignoring protections (the "program"
    /// storing data). Panics on unmapped memory.
    pub fn poke(&mut self, addr: u64, bytes: &[u8]) {
        for (i, &b) in bytes.iter().enumerate() {
            let a = addr + i as u64;
            self.pages.get_mut(&(a / PAGE_SIZE)).expect("poke into unmapped memory").data[(a % PAGE_SIZE) as usize] = b;
        }
    }

    /// Read bytes from user memory, ignoring protections.
    pub fn peek(&self, addr: u64, len: usize) -> Vec<u8> {
        (0..len as u64)
            .map(|i| {
                let a = addr + i;
                self.pages.get(&(a / PAGE_SIZE)).expect("peek at unmapped memory").data[(a % PAGE_SIZE) as usize]
            })
            .collect()
    }

    pub fn peek_u64(&self, addr: u64) -> u64 {
        u64::from_le_bytes(self.peek(addr, 8).try_into().unwrap())
    }

    pub fn file(&self, fd: i32) -> &File {
        &self.files[&fd]
    }

    pub fn file_mut(&mut self, fd: i32) -> &mut File {
        self.files.get_mut(&fd).unwrap()
    }

    fn user_range(&self, ptr: UserPtr, len: usize, need: u32) -> Result<(), Errno> {
        if len == 0 {
            return Ok(());
        }
        let end = ptr.0.checked_add(len as u64).ok_or(Errno::EFAULT)?;
        if end > USER_END {
            return Err(Errno::EFAULT);
        }
        for page in (ptr.0 / PAGE_SIZE)..end.div_ceil(PAGE_SIZE) {
            match self.pages.get(&page) {
                Some(p) if p.prot & need == need => {}
                _ => return Err(Errno::EFAULT),
            }
        }
        Ok(())
    }

    fn find_free(&self, len: u64) -> Option<u64> {
        let mut addr = MMAP_BASE;
        while addr + len <= USER_END {
            if self.range_is_free(addr, len) {
                return Some(addr);
            }
            addr += PAGE_SIZE;
        }
        None
    }
}

/// The fake's data area and break.
pub const DATA: u64 = 0x40_0000;
pub const RW: u32 = crate::abi::prot::PROT_READ | crate::abi::prot::PROT_WRITE;

impl LinuxKernel for FakeKernel {
    fn pid(&self) -> i32 {
        self.pid
    }

    fn tid(&self) -> i32 {
        self.tid
    }

    fn ppid(&self) -> i32 {
        self.ppid
    }

    fn credentials(&self) -> Credentials {
        self.creds().summary()
    }

    fn copy_from_user(&self, src: UserPtr, dst: &mut [u8]) -> Result<(), Errno> {
        self.user_range(src, dst.len(), crate::abi::prot::PROT_READ)?;
        dst.copy_from_slice(&self.peek(src.0, dst.len()));
        Ok(())
    }

    fn copy_to_user(&mut self, dst: UserPtr, src: &[u8]) -> Result<(), Errno> {
        self.user_range(dst, src.len(), crate::abi::prot::PROT_WRITE)?;
        self.poke(dst.0, src);
        Ok(())
    }

    fn write(&mut self, fd: i32, buf: &[u8]) -> Result<usize, Errno> {
        let f = self.files.get_mut(&fd).ok_or(Errno::EBADF)?;
        if !f.writable {
            return Err(Errno::EBADF);
        }
        if let Some((limit, e)) = f.fail_after
            && f.written.len() >= limit
        {
            return Err(e);
        }
        let n = buf.len().min(f.max_write.unwrap_or(usize::MAX));
        f.written.extend_from_slice(&buf[..n]);
        Ok(n)
    }

    fn read(&mut self, fd: i32, buf: &mut [u8]) -> Result<usize, Errno> {
        let f = self.files.get_mut(&fd).ok_or(Errno::EBADF)?;
        if !f.readable {
            return Err(Errno::EBADF);
        }
        let n = buf.len().min(f.input.len());
        buf[..n].copy_from_slice(&f.input[..n]);
        f.input.drain(..n);
        Ok(n)
    }

    fn close(&mut self, fd: i32) -> Result<(), Errno> {
        self.files.remove(&fd).map(|_| ()).ok_or(Errno::EBADF)
    }

    fn fstat(&mut self, fd: i32) -> Result<FileStat, Errno> {
        let f = self.files.get(&fd).ok_or(Errno::EBADF)?;
        let (kind, rdev) = match f.kind {
            Kind::Pipe => (mode::S_IFIFO | 0o600, 0),
            Kind::Tty => (mode::S_IFCHR | 0o620, 0x8800),
            Kind::NullDevice => (mode::S_IFCHR | 0o666, 0x103),
            Kind::Regular => (mode::S_IFREG | 0o644, 0),
        };
        Ok(FileStat {
            dev: 7,
            ino: 1000 + fd as u64,
            nlink: 1,
            mode: kind,
            uid: self.creds().uid,
            gid: self.creds().gid,
            rdev,
            size: f.written.len() as i64,
            blksize: 4096,
            blocks: 0,
            atime: self.realtime,
            mtime: self.realtime,
            ctime: self.realtime,
        })
    }

    fn ioctl(&mut self, fd: i32, request: u32, arg: u64) -> Result<u64, Errno> {
        self.ioctls.push((fd, request, arg));
        let kind = self.files.get(&fd).ok_or(Errno::EBADF)?.kind;
        match (kind, request) {
            // A zeroed kernel `struct termios` (36 bytes, asm-generic/termbits.h).
            (Kind::Tty, ioctl::TCGETS) => self.copy_to_user(UserPtr(arg), &[0; 36]).map(|()| 0),
            (Kind::Pipe, ioctl::FIONREAD) => {
                let n = self.files[&fd].input.len() as u32;
                self.copy_to_user(UserPtr(arg), &n.to_le_bytes()).map(|()| 0)
            }
            _ => Err(Errno::ENOTTY),
        }
    }

    fn mmap(&mut self, r: &MapRequest) -> Result<u64, Errno> {
        self.maps.push(*r);
        let file_data = match r.backing {
            Backing::Anonymous => None,
            Backing::File { fd, offset } => {
                let f = self.files.get(&fd).ok_or(Errno::EBADF)?;
                if f.kind != Kind::Regular {
                    return Err(Errno::ENODEV);
                }
                if !f.readable {
                    return Err(Errno::EACCES);
                }
                Some(f.written.iter().copied().skip(offset as usize).take(r.len as usize).collect::<Vec<u8>>())
            }
        };
        let addr = match r.placement {
            Placement::Fixed | Placement::FixedNoReplace => {
                let end = r.addr.checked_add(r.len).ok_or(Errno::ENOMEM)?;
                if end > USER_END {
                    return Err(Errno::ENOMEM);
                }
                if r.addr < MIN_ADDR {
                    return Err(Errno::EPERM);
                }
                if r.placement == Placement::FixedNoReplace && !self.range_is_free(r.addr, r.len) {
                    return Err(Errno::EEXIST);
                }
                r.addr
            }
            Placement::Hint => {
                let hint = r.addr & !(PAGE_SIZE - 1);
                if hint >= MIN_ADDR && hint + r.len <= USER_END && self.range_is_free(hint, r.len) {
                    hint
                } else {
                    self.find_free(r.len).ok_or(Errno::ENOMEM)?
                }
            }
        };
        self.map_range(addr, r.len, r.prot.0);
        if let Some(data) = file_data {
            for (i, b) in data.iter().enumerate() {
                let a = addr + i as u64;
                self.pages.get_mut(&(a / PAGE_SIZE)).unwrap().data[(a % PAGE_SIZE) as usize] = *b;
            }
        }
        Ok(addr)
    }

    fn munmap(&mut self, addr: u64, len: u64) -> Result<(), Errno> {
        if addr + len > USER_END {
            return Err(Errno::EINVAL);
        }
        self.unmap_range(addr, len);
        Ok(())
    }

    fn madvise(&mut self, addr: u64, len: u64, advice: crate::sys::madvise::Advice) -> Result<(), Errno> {
        self.advised.push((addr, len, advice));
        let pages = addr / PAGE_SIZE..(addr + len).div_ceil(PAGE_SIZE);
        if advice == crate::sys::madvise::Advice::DontNeed {
            for p in pages.clone() {
                if let Some(page) = self.pages.get_mut(&p) {
                    page.data.fill(0);
                }
            }
        }
        if pages.into_iter().any(|p| !self.pages.contains_key(&p)) {
            return Err(Errno::ENOMEM);
        }
        Ok(())
    }

    fn mprotect(&mut self, addr: u64, len: u64, prot: Prot) -> Result<(), Errno> {
        self.protects.push((addr, len, prot));
        if addr + len > USER_END {
            return Err(Errno::ENOMEM);
        }
        let pages = addr / PAGE_SIZE..(addr + len) / PAGE_SIZE;
        if pages.clone().any(|p| !self.pages.contains_key(&p)) {
            return Err(Errno::ENOMEM);
        }
        let access =
            prot.0 & (crate::abi::prot::PROT_READ | crate::abi::prot::PROT_WRITE | crate::abi::prot::PROT_EXEC);
        for p in pages {
            self.pages.get_mut(&p).unwrap().prot = access;
        }
        Ok(())
    }

    fn brk(&self) -> u64 {
        self.cur_brk
    }

    fn set_brk(&mut self, addr: u64) -> Result<u64, Errno> {
        if addr < self.start_brk || addr > MMAP_BASE {
            return Err(Errno::ENOMEM);
        }
        let old_end = self.cur_brk.div_ceil(PAGE_SIZE) * PAGE_SIZE;
        let new_end = addr.div_ceil(PAGE_SIZE) * PAGE_SIZE;
        if new_end > old_end {
            if !self.range_is_free(old_end, new_end - old_end) {
                return Err(Errno::ENOMEM);
            }
            self.map_range(old_end, new_end - old_end, RW);
        } else if new_end < old_end {
            self.unmap_range(new_end, old_end - new_end);
        }
        self.cur_brk = addr;
        Ok(addr)
    }

    fn fs_base(&self) -> u64 {
        self.fs_base
    }

    fn set_fs_base(&mut self, base: u64) {
        self.fs_base = base;
    }

    fn gs_base(&self) -> u64 {
        self.gs_base
    }

    fn set_gs_base(&mut self, base: u64) {
        self.gs_base = base;
    }

    fn pgid(&self) -> i32 {
        self.procs[&self.pid].info.pgid
    }

    fn sid(&self) -> i32 {
        self.procs[&self.pid].info.sid
    }

    fn with_process(
        &mut self,
        target: Target,
        f: &mut dyn FnMut(&mut LinuxProcess, &ProcessInfo),
    ) -> Result<(), Errno> {
        let pid = match target {
            Target::Current => self.pid,
            Target::Process(pid) => pid,
            Target::Thread(tid) => self.pid_of(tid).ok_or(Errno::ESRCH)?,
        };
        let p = self.procs.get_mut(&pid).ok_or(Errno::ESRCH)?;
        f(&mut p.state, &p.info);
        Ok(())
    }

    fn process_ids(&self, selector: ProcessSelector) -> Vec<i32> {
        self.procs
            .values()
            .filter(|p| match selector {
                ProcessSelector::All => true,
                ProcessSelector::Group(g) => p.info.pgid == g,
            })
            .map(|p| p.info.pid)
            .collect()
    }

    fn notify_signal(&mut self, tid: i32) {
        self.notified.insert(tid);
    }

    fn clear_signal_notification(&mut self) {
        self.notified.remove(&self.tid);
    }

    fn stop_process(&mut self, signal: u32) {
        self.stops.push(signal);
    }

    fn continue_process(&mut self, pid: i32) {
        self.continued.push(pid);
    }

    fn kill_process(&mut self, signal: u32, core_dump: bool) {
        self.exit = Some(Exit::Signal(signal, core_dump));
    }

    fn user_context(&self) -> UserContext {
        self.ctx
    }

    fn set_user_context(&mut self, ctx: &UserContext) {
        self.ctx = *ctx;
    }

    fn fpu_state(&self) -> FpuState<'_> {
        FpuState { image: &self.fpu, xfeatures: self.xfeatures }
    }

    fn set_fpu_state(&mut self, image: &[u8]) -> Result<(), Errno> {
        if self.refuse_fpu {
            return Err(Errno::EINVAL);
        }
        self.fpu_loads.push(image.to_vec());
        Ok(())
    }

    fn reset_fpu_state(&mut self) {
        self.fpu_resets += 1;
    }

    fn block(&mut self, deadline: Option<Deadline>) -> Wakeup {
        self.blocks += 1;
        self.deadlines.push(deadline);
        loop {
            if self.notified.contains(&self.tid) {
                return Wakeup::Signal;
            }
            if self.unblocked.remove(&self.tid) {
                return Wakeup::Woken;
            }
            if let Some(mut step) = self.steps.pop_front() {
                let saved = (self.pid, self.tid, self.ctx);
                step(self);
                (self.pid, self.tid, self.ctx) = saved;
                continue;
            }
            if deadline.is_some() {
                return Wakeup::TimedOut;
            }
            panic!("thread {} blocked with nothing left to wake it", self.tid);
        }
    }

    fn unblock(&mut self, tid: i32) {
        self.unblocked.insert(tid);
    }

    fn file_io(&mut self) -> Option<&mut dyn crate::fileio::FileIo> {
        Some(self)
    }

    fn signal_queued(&mut self, pid: i32) {
        self.io.queued.push(pid);
    }

    fn with_futexes(&mut self, f: &mut dyn FnMut(&mut FutexTable, &mut dyn LinuxKernel)) {
        assert!(!self.futexes_locked, "with_futexes called inside with_futexes");
        let mut table = core::mem::take(&mut self.futexes);
        self.futexes_locked = true;
        f(&mut table, self);
        self.futexes_locked = false;
        self.futexes = table;
    }

    fn address_space_id(&self) -> u64 {
        self.pid as u64
    }

    fn futex_cmpxchg(&mut self, addr: UserPtr, expected: u32, new: u32) -> Result<u32, Errno> {
        let mut b = [0; 4];
        self.copy_from_user(addr, &mut b)?;
        self.user_range(addr, 4, crate::abi::prot::PROT_WRITE)?;
        let found = u32::from_le_bytes(b);
        if found == expected {
            self.poke(addr.0, &new.to_le_bytes());
        }
        Ok(found)
    }

    fn exit_thread(&mut self, status: u8) {
        self.exit = Some(Exit::Thread(status));
    }

    fn exit_group(&mut self, status: u8) {
        self.exit = Some(Exit::Group(status));
    }

    fn clock(&self, clock: Clock) -> Result<Timespec, Errno> {
        match clock {
            Clock::Realtime => Ok(self.realtime),
            Clock::Monotonic => Ok(self.monotonic),
            Clock::ProcessCpuTime => self.cpu.map(|c| Timespec::from_nanos(c[1].total_ns())).ok_or(Errno::EINVAL),
            Clock::ThreadCpuTime => self.cpu.map(|c| Timespec::from_nanos(c[0].total_ns())).ok_or(Errno::EINVAL),
        }
    }

    fn set_realtime(&mut self, at: Timespec) -> Result<(), Errno> {
        if (at.sec, at.nsec) < (self.monotonic.sec, self.monotonic.nsec) {
            return Err(Errno::EINVAL);
        }
        self.realtime = at;
        Ok(())
    }

    fn timezone(&self) -> (i32, i32) {
        self.timezone
    }

    fn set_timezone(&mut self, minuteswest: i32, dsttime: i32) {
        self.timezone = (minuteswest, dsttime);
    }

    fn tick_ns(&self) -> u64 {
        4_000_000
    }

    fn cpu_times(&self, of: CpuTimesOf) -> Option<CpuTimes> {
        let c = self.cpu?;
        match of {
            CpuTimesOf::Thread(tid) if tid == self.tid => Some(c[0]),
            CpuTimesOf::Thread(tid) => self.procs[&self.pid].state.has_thread(tid).then_some(CpuTimes::default()),
            CpuTimesOf::Process => Some(c[1]),
            CpuTimesOf::Children => Some(c[2]),
        }
    }

    fn timers_changed(&mut self, next: TimerWakeup) {
        self.timer_wakeups.push(next);
    }

    fn random_bytes(&mut self, buf: &mut [u8], mode: RandomMode) -> Result<usize, Errno> {
        if !self.random_ready && mode == RandomMode::NonBlock {
            return Err(Errno::EAGAIN);
        }
        for b in buf.iter_mut() {
            *b = self.random_next;
            self.random_next = self.random_next.wrapping_add(1);
        }
        Ok(buf.len())
    }

    fn hostname(&self) -> &[u8] {
        b"testhost"
    }

    fn fs(&mut self) -> Option<&mut dyn crate::fs::LinuxFs> {
        match &mut self.fs {
            Some(fs) => Some(&mut **fs),
            None => None,
        }
    }

    fn system_info(&mut self) -> Result<crate::fs::SysInfo, Errno> {
        self.sysinfo.ok_or(Errno::ENOSYS)
    }

    fn clone_task(&mut self, req: &crate::task::clone::CloneRequest) -> Result<i32, Errno> {
        super::task::clone_task(self, req)
    }

    fn execve(&mut self, req: &crate::task::exec::ExecRequest<'_>) -> Result<(), Errno> {
        super::task::execve(self, req)
    }

    fn with_tree(&mut self, f: &mut dyn FnMut(&mut crate::task::tree::ProcessTree)) -> Result<(), Errno> {
        f(&mut self.tasks.tree);
        Ok(())
    }

    fn wait_tree(&mut self, generation: u64) -> Wakeup {
        super::task::wait_tree(self, generation)
    }
}

/// Run one system call through the dispatcher.
pub fn call(k: &mut FakeKernel, number: u64, args: &[u64]) -> (Outcome, u64) {
    let mut a = [0u64; 6];
    a[..args.len()].copy_from_slice(args);
    let mut regs = SyscallRegs {
        rax: number,
        rdi: a[0],
        rsi: a[1],
        rdx: a[2],
        r10: a[3],
        r8: a[4],
        r9: a[5],
        ..Default::default()
    };
    let outcome = Linux::dispatch_with(k, &mut regs);
    (outcome, regs.rax)
}

/// Run system call `number` the way the kernel would: the arguments and
/// number go into the calling thread's registers, the dispatcher runs,
/// `rax` is written, then the kernel's way back to user mode
/// ([`Linux::exit_to_user`]) runs. Returns the registers the thread
/// returns to user mode with.
pub fn run(k: &mut FakeKernel, number: u64, args: &[u64]) -> (ExitToUser, UserContext) {
    let mut a = [0u64; 6];
    a[..args.len()].copy_from_slice(args);
    k.ctx.rax = number;
    k.ctx.rdi = a[0];
    k.ctx.rsi = a[1];
    k.ctx.rdx = a[2];
    k.ctx.r10 = a[3];
    k.ctx.r8 = a[4];
    k.ctx.r9 = a[5];
    k.ctx.rcx = k.ctx.rip;
    k.ctx.r11 = k.ctx.rflags;
    let mut regs = SyscallRegs {
        rax: number,
        rdi: a[0],
        rsi: a[1],
        rdx: a[2],
        r10: a[3],
        r8: a[4],
        r9: a[5],
        rflags: k.ctx.rflags,
        rsp: k.ctx.rsp,
    };
    let outcome = Linux::dispatch_with(k, &mut regs);
    assert_eq!(outcome, Outcome::Handled, "syscall {number} ({:?}) not handled", crate::table::name(number));
    k.ctx.rax = regs.rax;
    let exit = Linux::exit_to_user(k);
    (exit, k.ctx)
}

/// Run an implemented system call and return `rax` as `Ok(value)` or
/// `Err(errno)`.
pub fn sys(k: &mut FakeKernel, number: u64, args: &[u64]) -> Result<u64, Errno> {
    let (outcome, rax) = call(k, number, args);
    assert_eq!(outcome, Outcome::Handled, "syscall {number} ({:?}) not handled", crate::table::name(number));
    match Errno::from_rax(rax) {
        Some(e) => Err(e),
        None => Ok(rax),
    }
}

/// Address of a scratch buffer in the data area.
pub const fn scratch(offset: u64) -> u64 {
    DATA + offset
}

/// An unmapped user address.
pub const UNMAPPED: u64 = 0x3000_0000;

/// A read-only page set up by [`read_only_page`].
pub fn read_only_page(k: &mut FakeKernel) -> u64 {
    let at = 0x20_0000;
    k.map_range(at, PAGE_SIZE, crate::abi::prot::PROT_READ);
    at
}

/// Store iovecs at `at` and return its address.
pub fn put_iovecs(k: &mut FakeKernel, at: u64, iovs: &[(u64, u64)]) -> u64 {
    let mut raw = vec![];
    for &(base, len) in iovs {
        raw.extend_from_slice(&base.to_le_bytes());
        raw.extend_from_slice(&len.to_le_bytes());
    }
    k.poke(at, &raw);
    at
}

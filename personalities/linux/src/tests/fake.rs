//! A fake kernel for host tests: user memory as a map of 4 KiB pages with
//! protections, files as a map of fds, VMAs as the set of mapped pages, and
//! fixed clocks and a deterministic random source.

extern crate std;

use crate::Linux;
use crate::abi::{FileStat, PAGE_SIZE, Timespec, ioctl, mode};
use crate::errno::Errno;
use crate::kernel::{
    Backing, Clock, Credentials, LinuxKernel, MapRequest, Placement, Prot, RandomMode, SigSet, SignalActions, UserPtr,
};
use poly_abi::{Outcome, SyscallRegs};
use std::boxed::Box;
use std::collections::BTreeMap;
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
}

pub struct FakeKernel {
    pub pages: BTreeMap<u64, Page>,
    pub files: BTreeMap<i32, File>,
    pub start_brk: u64,
    pub cur_brk: u64,
    pub fs_base: u64,
    pub gs_base: u64,
    pub clear_child_tid: UserPtr,
    pub mask: SigSet,
    pub actions: SignalActions,
    pub exit: Option<Exit>,
    pub pid: i32,
    pub tid: i32,
    pub ppid: i32,
    pub creds: Credentials,
    pub realtime: Timespec,
    pub monotonic: Timespec,
    pub random_ready: bool,
    pub random_next: u8,
    /// Every mmap request that reached the kernel.
    pub maps: Vec<MapRequest>,
    /// Every (fd, request, arg) ioctl that reached the kernel.
    pub ioctls: Vec<(i32, u32, u64)>,
    /// Every mprotect that reached the kernel.
    pub protects: Vec<(u64, u64, Prot)>,
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
            clear_child_tid: UserPtr::NULL,
            mask: SigSet::EMPTY,
            actions: SignalActions::new(),
            exit: None,
            pid: 100,
            tid: 100,
            ppid: 1,
            creds: Credentials { uid: 1000, euid: 1001, gid: 2000, egid: 2001 },
            realtime: Timespec { sec: 1_700_000_000, nsec: 123 },
            monotonic: Timespec { sec: 42, nsec: 999_999_999 },
            random_ready: true,
            random_next: 0,
            maps: Vec::new(),
            ioctls: Vec::new(),
            protects: Vec::new(),
        };
        k.files.insert(0, File::new(Kind::Tty));
        k.files.insert(1, File::new(Kind::Pipe));
        k.files.insert(2, File::new(Kind::Tty));
        k.map_range(DATA, 16 * PAGE_SIZE, RW);
        k
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
        self.creds
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
            uid: self.creds.uid,
            gid: self.creds.gid,
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

    fn set_clear_child_tid(&mut self, addr: UserPtr) {
        self.clear_child_tid = addr;
    }

    fn signal_mask(&mut self) -> &mut SigSet {
        &mut self.mask
    }

    fn signal_actions(&mut self) -> &mut SignalActions {
        &mut self.actions
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
            Clock::ProcessCpuTime | Clock::ThreadCpuTime => Err(Errno::EINVAL),
        }
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

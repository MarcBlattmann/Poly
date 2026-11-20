//! The BSD layer against a fake kernel: user memory is one buffer from
//! [`BASE`], files are a few names, children are scripted.

use super::file::{Access, DirEntry, OpenHow, Stat, Status, dt};
use super::signal::{self, Signals};
use super::*;
use std::collections::BTreeMap;
use std::vec;
use std::vec::Vec;

const BASE: u64 = 0x10000;
const SIZE: usize = 0x10000;

/// A spawn the fake kernel saw: path, argv, envp.
type Spawned = (Vec<u8>, Vec<Vec<u8>>, Vec<Vec<u8>>);

#[derive(Default)]
pub(crate) struct Fake {
    mem: Vec<u8>,
    files: BTreeMap<i32, (Vec<u8>, u64, bool)>, // contents, offset, is a directory
    cloexec: BTreeMap<i32, bool>,
    pub(crate) out: Vec<u8>,
    pub(crate) exited: Option<u8>,
    killed_by: Option<u32>,
    signals: Signals,
    maps: Vec<MapRequest>,
    spawned: Vec<Spawned>,
    children: Vec<(u32, ChildStatus)>,
    dir_pos: usize,
}

impl Fake {
    /// A buffer address in the fake user memory.
    pub(crate) const BUF: u64 = BUF;

    pub(crate) fn new() -> Fake {
        let mut f = Fake { mem: vec![0; SIZE], ..Fake::default() };
        f.files.insert(0, (Vec::new(), 0, false));
        f.files.insert(1, (Vec::new(), 0, false));
        f.files.insert(5, (Vec::new(), 0, true));
        f
    }

    fn poke(&mut self, addr: u64, b: &[u8]) {
        let o = (addr - BASE) as usize;
        self.mem[o..o + b.len()].copy_from_slice(b);
    }

    fn peek(&self, addr: u64, n: usize) -> &[u8] {
        let o = (addr - BASE) as usize;
        &self.mem[o..o + n]
    }

    fn u64_at(&self, addr: u64) -> u64 {
        u64::from_le_bytes(self.peek(addr, 8).try_into().unwrap())
    }

    fn call(&mut self, n: u64, args: &[u64]) -> Result<Ret, u32> {
        let mut a = [0u64; 8];
        a[..args.len()].copy_from_slice(args);
        super::call(self, n, &a).expect("implemented")
    }
}

impl BsdKernel for Fake {
    fn copy_in(&self, addr: u64, buf: &mut [u8]) -> Result<(), ()> {
        let o = addr.checked_sub(BASE).ok_or(())? as usize;
        let src = self.mem.get(o..o + buf.len()).ok_or(())?;
        buf.copy_from_slice(src);
        Ok(())
    }

    fn copy_out(&mut self, addr: u64, buf: &[u8]) -> Result<(), ()> {
        let o = addr.checked_sub(BASE).ok_or(())? as usize;
        self.mem.get_mut(o..o + buf.len()).ok_or(())?.copy_from_slice(buf);
        Ok(())
    }

    fn exit(&mut self, status: u8) {
        self.exited = Some(status);
    }

    fn write(&mut self, fd: i32, buf: &[u8], _offset: Option<u64>) -> Result<usize, u32> {
        if fd != 1 {
            return Err(EBADF);
        }
        self.out.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn read(&mut self, fd: i32, buf: &mut [u8], offset: Option<u64>) -> Result<usize, u32> {
        let (data, pos, dir) = self.files.get_mut(&fd).ok_or(EBADF)?;
        if *dir {
            return Err(EISDIR);
        }
        let at = offset.unwrap_or(*pos) as usize;
        let n = buf.len().min(data.len().saturating_sub(at));
        buf[..n].copy_from_slice(&data[at..at + n]);
        if offset.is_none() {
            *pos += n as u64;
        }
        Ok(n)
    }

    fn open(&mut self, _dirfd: i32, path: &[u8], how: &OpenHow) -> Result<i32, u32> {
        match path {
            b"hello.txt" => {
                let fd = (0..).find(|f| !self.files.contains_key(f)).unwrap();
                self.files.insert(fd, (b"hello\n".to_vec(), 0, false));
                self.cloexec.insert(fd, how.cloexec);
                Ok(fd)
            }
            _ => Err(ENOENT),
        }
    }

    fn close(&mut self, fd: i32) -> Result<(), u32> {
        self.files.remove(&fd).map(|_| ()).ok_or(EBADF)
    }

    fn lseek(&mut self, fd: i32, offset: i64, whence: u32) -> Result<u64, u32> {
        let (data, pos, _) = self.files.get_mut(&fd).ok_or(EBADF)?;
        *pos = match whence {
            0 => offset as u64,
            1 => (*pos as i64 + offset) as u64,
            _ => (data.len() as i64 + offset) as u64,
        };
        Ok(*pos)
    }

    fn stat(&mut self, of: StatOf<'_>) -> Result<Stat, u32> {
        let (size, dir) = match of {
            StatOf::Fd(fd) => self.files.get(&fd).map(|f| (f.0.len() as u64, f.2)).ok_or(EBADF)?,
            StatOf::Path { path: b"hello.txt", .. } => (6, false),
            StatOf::Path { .. } => return Err(ENOENT),
        };
        Ok(Stat { mode: if dir { 0o040755 } else { 0o100644 }, size, nlink: 1, ino: 42, ..Stat::default() })
    }

    fn read_dir(&mut self, fd: i32, emit: &mut dyn FnMut(&DirEntry<'_>) -> bool) -> Result<(), u32> {
        if !self.files.get(&fd).ok_or(EBADF)?.2 {
            return Err(ENOTDIR);
        }
        let names: [(&[u8], u8); 3] = [(b".", dt::DIR), (b"..", dt::DIR), (b"hello.txt", dt::REG)];
        while let Some((n, kind)) = names.get(self.dir_pos) {
            if !emit(&DirEntry { ino: 10 + self.dir_pos as u64, kind: *kind, name: n }) {
                break;
            }
            self.dir_pos += 1;
        }
        Ok(())
    }

    fn dup(&mut self, fd: i32, min: i32, cloexec: bool) -> Result<i32, u32> {
        let f = self.files.get(&fd).cloned().ok_or(EBADF)?;
        let n = (min..).find(|f| !self.files.contains_key(f)).unwrap();
        self.files.insert(n, f);
        self.cloexec.insert(n, cloexec);
        Ok(n)
    }

    fn dup2(&mut self, old: i32, new: i32) -> Result<i32, u32> {
        let f = self.files.get(&old).cloned().ok_or(EBADF)?;
        self.files.insert(new, f);
        Ok(new)
    }

    fn cloexec(&self, fd: i32) -> Result<bool, u32> {
        self.files.get(&fd).ok_or(EBADF)?;
        Ok(self.cloexec.get(&fd).copied().unwrap_or(false))
    }

    fn set_cloexec(&mut self, fd: i32, on: bool) -> Result<(), u32> {
        self.files.get(&fd).ok_or(EBADF)?;
        self.cloexec.insert(fd, on);
        Ok(())
    }

    fn status(&self, fd: i32) -> Result<Status, u32> {
        self.files.get(&fd).ok_or(EBADF)?;
        Ok(Status { access: Access::ReadWrite, append: false, nonblock: false })
    }

    fn set_status(&mut self, fd: i32, _append: bool, _nonblock: bool) -> Result<(), u32> {
        self.files.get(&fd).map(|_| ()).ok_or(EBADF)
    }

    fn path_of(&self, fd: i32) -> Result<Vec<u8>, u32> {
        self.files.get(&fd).ok_or(EBADF)?;
        Ok(b"/tmp/hello.txt".to_vec())
    }

    fn chdir(&mut self, _path: &[u8]) -> Result<(), u32> {
        Ok(())
    }

    fn fchdir(&mut self, _fd: i32) -> Result<(), u32> {
        Ok(())
    }

    fn pipe(&mut self) -> Result<(i32, i32), u32> {
        Ok((3, 4))
    }

    fn mmap(&mut self, req: &MapRequest) -> Result<u64, u32> {
        self.maps.push(*req);
        Ok(if req.fixed { req.addr } else { 0x7000_0000 })
    }

    fn munmap(&mut self, _addr: u64, _len: u64) -> Result<(), u32> {
        Ok(())
    }

    fn page_size(&self) -> u64 {
        4096
    }

    fn pid(&self) -> u32 {
        7
    }

    fn ppid(&self) -> u32 {
        1
    }

    fn thread_id(&self) -> u64 {
        700
    }

    fn ids(&self) -> (u32, u32) {
        (501, 20)
    }

    fn process_exists(&self, pid: u32) -> bool {
        pid == 7 || pid == 8
    }

    fn random(&mut self, buf: &mut [u8]) {
        buf.fill(0xa5);
    }

    fn system(&self) -> sysctl::System {
        sysctl::System { ncpu: 2, memsize: 1 << 31, page_size: 4096, machine: "x86_64", ..sysctl::System::default() }
    }

    fn with_signals(&mut self, f: &mut dyn FnMut(&mut Signals)) {
        f(&mut self.signals);
    }

    fn exit_signal(&mut self, sig: u32) {
        self.killed_by = Some(sig);
    }

    fn kill(&mut self, pid: u32, _sig: u32) -> Result<(), u32> {
        if pid == 8 { Ok(()) } else { Err(ESRCH) }
    }

    fn spawn(&mut self, path: &[u8], argv: &[Vec<u8>], envp: &[Vec<u8>]) -> Result<u32, u32> {
        if path != b"/bin/child" {
            return Err(ENOENT);
        }
        self.spawned.push((path.to_vec(), argv.to_vec(), envp.to_vec()));
        self.children.push((8, ChildStatus::Exited(7)));
        Ok(8)
    }

    fn wait(&mut self, pid: i32, nohang: bool) -> Result<Option<(u32, ChildStatus)>, u32> {
        let i = self.children.iter().position(|c| pid == -1 || c.0 == pid as u32).ok_or(ECHILD)?;
        let _ = nohang;
        Ok(Some(self.children.remove(i)))
    }
}

const PATH: u64 = BASE + 0x100;
const BUF: u64 = BASE + 0x1000;
const PTR: u64 = BASE + 0x2000;

#[test]
fn open_read_lseek_close() {
    let mut k = Fake::new();
    k.poke(PATH, b"hello.txt\0");
    let fd = k.call(nr::OPEN, &[PATH, 0]).unwrap().value;
    assert_eq!(fd, 2);
    assert_eq!(k.call(nr::READ, &[fd, BUF, 100]).unwrap().value, 6);
    assert_eq!(k.peek(BUF, 6), b"hello\n");
    assert_eq!(k.call(nr::READ, &[fd, BUF, 100]).unwrap().value, 0);
    assert_eq!(k.call(nr::LSEEK, &[fd, 1, 0]).unwrap().value, 1);
    assert_eq!(k.call(nr::PREAD, &[fd, BUF, 2, 3]).unwrap().value, 2);
    assert_eq!(k.peek(BUF, 2), b"lo");
    assert_eq!(k.call(nr::LSEEK, &[fd, 0, 1]).unwrap().value, 1, "pread leaves the offset");
    assert_eq!(k.call(nr::LSEEK, &[fd, 0, 7]), Err(EINVAL));
    assert_eq!(k.call(nr::CLOSE, &[fd]).unwrap().value, 0);
    assert_eq!(k.call(nr::CLOSE, &[fd]), Err(EBADF));
    // Errors before the kernel: a bad pointer, the empty path, a bad mode.
    assert_eq!(k.call(nr::OPEN, &[8, 0]), Err(EFAULT));
    k.poke(PATH, b"\0");
    assert_eq!(k.call(nr::OPEN, &[PATH, 0]), Err(ENOENT));
    k.poke(PATH, b"hello.txt\0");
    assert_eq!(k.call(nr::OPEN, &[PATH, 3]), Err(EINVAL));
    // A component too long for PATH_MAX.
    let long = vec![b'a'; 2000];
    k.poke(BUF, &long);
    k.poke(BUF + 2000, b"\0");
    assert_eq!(k.call(nr::OPEN, &[BUF, 0]), Err(ENAMETOOLONG));
    // A read into memory that is not there.
    k.call(nr::OPEN, &[PATH, 0]).unwrap();
    assert_eq!(k.call(nr::READ, &[2, 8, 4]), Err(EFAULT));
}

#[test]
fn write_and_exit() {
    let mut k = Fake::new();
    k.poke(BUF, b"hi there");
    assert_eq!(k.call(nr::WRITE, &[1, BUF, 8]).unwrap().value, 8);
    assert_eq!(k.out, b"hi there");
    assert_eq!(k.call(nr::WRITE, &[1, BUF, 0]).unwrap().value, 0);
    assert_eq!(k.call(nr::WRITE, &[9, BUF, 1]), Err(EBADF));
    assert_eq!(k.call(nr::WRITE, &[1, 8, 4]), Err(EFAULT));
    k.call(nr::EXIT, &[263]).unwrap();
    assert_eq!(k.exited, Some(7));
}

#[test]
fn stat_calls() {
    let mut k = Fake::new();
    k.poke(PATH, b"hello.txt\0");
    assert_eq!(k.call(nr::STAT64, &[PATH, BUF]).unwrap().value, 0);
    let st = k.peek(BUF, 144).to_vec();
    assert_eq!(u16::from_le_bytes([st[4], st[5]]), 0o100644);
    assert_eq!(u64::from_le_bytes(st[96..104].try_into().unwrap()), 6);
    assert_eq!(k.call(nr::FSTATAT64, &[file::AT_FDCWD as u64, PATH, BUF, 0x20]).unwrap().value, 0);
    assert_eq!(k.call(nr::FSTATAT64, &[file::AT_FDCWD as u64, PATH, BUF, 0x1000]), Err(EINVAL));
    assert_eq!(k.call(nr::FSTAT64, &[5, BUF]).unwrap().value, 0);
    assert_eq!(k.call(nr::FSTAT64, &[55, BUF]), Err(EBADF));
    assert_eq!(k.call(nr::FSTAT64, &[5, 8]), Err(EFAULT));
    k.poke(PATH, b"nope\0");
    assert_eq!(k.call(nr::LSTAT64, &[PATH, BUF]), Err(ENOENT));
}

#[test]
fn directory_listing() {
    let mut k = Fake::new();
    // 64 bytes: two records per call; basep 0, then 0x7fffffff at the end.
    assert_eq!(k.call(nr::GETDIRENTRIES64, &[5, BUF, 32, PTR]), Err(EINVAL));
    assert_eq!(k.call(nr::GETDIRENTRIES64, &[5, BUF, 64, PTR]).unwrap().value, 64);
    assert_eq!(&k.peek(BUF + 21, 2), b".\0");
    assert_eq!(k.u64_at(PTR), 0);
    assert_eq!(k.call(nr::GETDIRENTRIES64, &[5, BUF, 64, PTR]).unwrap().value, 32);
    assert_eq!(&k.peek(BUF + 21, 10), b"hello.txt\0");
    assert_eq!(k.peek(BUF + 20, 1)[0], dt::REG);
    assert_eq!(k.call(nr::GETDIRENTRIES64, &[5, BUF, 64, PTR]).unwrap().value, 0);
    assert_eq!(k.u64_at(PTR), 0x7fff_ffff);
    // Not a directory; a bad basep.
    assert_eq!(k.call(nr::GETDIRENTRIES64, &[1, BUF, 64, PTR]), Err(EINVAL));
    assert_eq!(k.call(nr::GETDIRENTRIES64, &[5, BUF, 64, 0]), Err(EFAULT));
    assert_eq!(k.call(nr::GETDIRENTRIES64, &[99, BUF, 64, PTR]), Err(EBADF));
}

#[test]
fn fcntl_and_dup2() {
    let mut k = Fake::new();
    assert_eq!(k.call(nr::FCNTL, &[1, 0, 20]).unwrap().value, 20);
    assert_eq!(k.call(nr::FCNTL, &[1, 0, 20]).unwrap().value, 21);
    assert_eq!(k.call(nr::FCNTL, &[1, 67, 30]).unwrap().value, 30);
    assert_eq!(k.call(nr::FCNTL, &[30, 1, 0]).unwrap().value, 1);
    k.call(nr::FCNTL, &[30, 2, 0]).unwrap();
    assert_eq!(k.call(nr::FCNTL, &[30, 1, 0]).unwrap().value, 0);
    assert_eq!(k.call(nr::FCNTL, &[1, 3, 0]).unwrap().value, 2);
    assert_eq!(k.call(nr::FCNTL, &[1, 50, BUF]).unwrap().value, 0);
    assert_eq!(k.peek(BUF, 15), b"/tmp/hello.txt\0");
    assert_eq!(k.call(nr::FCNTL, &[1, 50, 8]), Err(EFAULT));
    assert_eq!(k.call(nr::FCNTL, &[1, 999, 0]), Err(ENODEV));
    assert_eq!(k.call(nr::FCNTL, &[77, 999, 0]), Err(EBADF));
    assert_eq!(k.call(nr::DUP2, &[1, 50]).unwrap().value, 50);
    assert_eq!(k.call(nr::DUP2, &[1, -1i64 as u64]), Err(EBADF));
}

#[test]
fn mmap_arguments() {
    let mut k = Fake::new();
    let anon = u64::from(map::ANON | map::PRIVATE);
    assert_eq!(k.call(nr::MMAP, &[0, 1, 3, anon, 0, 0]).unwrap().value, 0x7000_0000);
    assert_eq!(k.maps[0].len, 4096, "a byte maps a page");
    assert_eq!(k.maps[0].file, None);
    assert_eq!(k.call(nr::MMAP, &[0, 0, 3, anon, 0, 0]).unwrap().value, 0, "length 0");
    assert_eq!(k.call(nr::MMAP, &[0, -4096i64 as u64, 3, anon, 0, 0]), Err(EINVAL));
    assert_eq!(k.call(nr::MMAP, &[0, 1 << 62, 3, anon, 0, 0]), Err(ENOMEM));
    let fixed = anon | u64::from(map::FIXED);
    assert_eq!(k.call(nr::MMAP, &[0x5001, 4096, 3, fixed, 0, 0]), Err(EINVAL));
    assert_eq!(k.call(nr::MMAP, &[0x5000, 4096, 3, fixed, 0, 0]).unwrap().value, 0x5000);
    // A file at an unaligned offset: the address is that far into a page.
    let private = u64::from(map::PRIVATE);
    assert_eq!(k.call(nr::MMAP, &[0, 10, 1, private, 1, 4099]).unwrap().value, 0x7000_0003);
    assert_eq!(k.maps.last().unwrap().file, Some((1, 4096)));
    assert_eq!(k.call(nr::MMAP, &[0, 10, 1, private, -1i64 as u64, 0]), Err(EBADF));
    assert_eq!(k.call(nr::MMAP, &[0, 10, 1, private, 5, 0]), Err(EINVAL), "a directory");
    assert_eq!(k.call(nr::MUNMAP, &[0x5001, 4096]), Err(EINVAL));
    assert_eq!(k.call(nr::MUNMAP, &[0x5000, 0]), Err(EINVAL));
    assert_eq!(k.call(nr::MUNMAP, &[0x5000, 1 << 62]).unwrap().value, 0);
}

#[test]
fn ids_entropy_csops() {
    let mut k = Fake::new();
    assert_eq!(k.call(nr::GETPID, &[]).unwrap().value, 7);
    assert_eq!(k.call(nr::GETPPID, &[]).unwrap().value, 1);
    assert_eq!(k.call(nr::GETUID, &[]).unwrap().value, 501);
    assert_eq!(k.call(nr::GETEGID, &[]).unwrap().value, 20);
    assert_eq!(k.call(nr::ISSETUGID, &[1, 2, 3]).unwrap().value, 0);
    assert_eq!(k.call(nr::GETENTROPY, &[BUF, 16]).unwrap().value, 0);
    assert_eq!(k.peek(BUF, 16), [0xa5; 16]);
    assert_eq!(k.call(nr::GETENTROPY, &[BUF, 257]), Err(EINVAL));
    assert_eq!(k.call(nr::GETENTROPY, &[8, 0]).unwrap().value, 0);
    assert_eq!(k.call(nr::GETENTROPY, &[8, 16]), Err(EFAULT));
    assert_eq!(k.call(nr::CSOPS, &[0, 0, BUF, 4]).unwrap().value, 0);
    assert_eq!(k.peek(BUF, 4), CS_STATUS_ADHOC.to_le_bytes());
    assert_eq!(k.call(nr::CSOPS, &[99999, 0, BUF, 4]), Err(ESRCH));
    assert_eq!(k.call(nr::CSOPS, &[0, 9999, BUF, 4]), Err(EINVAL));
    assert_eq!(k.call(nr::CSOPS, &[0, 0, 8, 4]), Err(EFAULT));
}

#[test]
fn sysctl_by_number_and_name() {
    let mut k = Fake::new();
    let name = BASE + 0x300;
    let len = BASE + 0x400;
    k.poke(name, &[1, 0, 0, 0, 1, 0, 0, 0]);
    // Size query, then the value.
    assert_eq!(k.call(nr::SYSCTL, &[name, 2, 0, len, 0, 0]).unwrap().value, 0);
    assert_eq!(k.u64_at(len), 7);
    assert_eq!(k.call(nr::SYSCTL, &[name, 2, BUF, len, 0, 0]).unwrap().value, 0);
    assert_eq!(k.peek(BUF, 7), b"Darwin\0");
    // Too small: ENOMEM and the length becomes 0.
    k.poke(len, &3u64.to_le_bytes());
    assert_eq!(k.call(nr::SYSCTL, &[name, 2, BUF, len, 0, 0]), Err(ENOMEM));
    assert_eq!(k.u64_at(len), 0);
    // Read-only, unknown, bad lengths.
    k.poke(len, &8u64.to_le_bytes());
    assert_eq!(k.call(nr::SYSCTL, &[name, 2, BUF, len, BUF, 7]), Err(EPERM));
    assert_eq!(k.call(nr::SYSCTL, &[name, 1, BUF, len, 0, 0]), Err(EINVAL));
    assert_eq!(k.call(nr::SYSCTL, &[name, 13, BUF, len, 0, 0]), Err(EINVAL));
    k.poke(name, &[6, 0, 0, 0, 7, 0, 0, 0]);
    k.poke(len, &8u64.to_le_bytes());
    assert_eq!(k.call(nr::SYSCTL, &[name, 2, BUF, len, 0, 0]).unwrap().value, 0);
    assert_eq!(k.u64_at(len), 4, "hw.pagesize by number is 4 bytes");
    // Name to numbers.
    k.poke(name, &[0, 0, 0, 0, 3, 0, 0, 0]);
    k.poke(PATH, b"hw.memsize");
    k.poke(len, &64u64.to_le_bytes());
    assert_eq!(k.call(nr::SYSCTL, &[name, 2, BUF, len, PATH, 10]).unwrap().value, 0);
    assert_eq!(k.peek(BUF, 8), [6, 0, 0, 0, 24, 0, 0, 0]);
    // By name, with or without the NUL counted.
    k.poke(PATH, b"hw.pagesize\0");
    k.poke(len, &8u64.to_le_bytes());
    assert_eq!(k.call(nr::SYSCTLBYNAME, &[PATH, 11, BUF, len, 0, 0]).unwrap().value, 0);
    assert_eq!(k.u64_at(len), 8);
    assert_eq!(k.call(nr::SYSCTLBYNAME, &[PATH, 12, BUF, len, 0, 0]).unwrap().value, 0);
    assert_eq!(k.call(nr::SYSCTLBYNAME, &[PATH, 5, BUF, len, 0, 0]), Err(ENOENT));
    assert_eq!(k.call(nr::SYSCTLBYNAME, &[PATH, 1999, BUF, len, 0, 0]), Err(ENAMETOOLONG));
    assert_eq!(k.call(nr::SYSCTLBYNAME, &[8, 11, BUF, len, 0, 0]), Err(EFAULT));
}

#[test]
fn signals() {
    let mut k = Fake::new();
    // An action reads back as {handler, mask, flags}.
    let act = PTR;
    k.poke(act, &0x4000u64.to_le_bytes());
    k.poke(act + 8, &0x5000u64.to_le_bytes());
    k.poke(act + 16, &4u32.to_le_bytes());
    k.poke(act + 20, &0x42u32.to_le_bytes());
    assert_eq!(k.call(nr::SIGACTION, &[30, act, 0]).unwrap().value, 0);
    assert_eq!(k.call(nr::SIGACTION, &[30, 0, BUF]).unwrap().value, 0);
    assert_eq!(k.u64_at(BUF), 0x4000);
    assert_eq!(k.peek(BUF + 8, 8), [4, 0, 0, 0, 0x42, 0, 0, 0]);
    assert_eq!(k.call(nr::SIGACTION, &[9, 0, BUF]), Err(EINVAL));
    assert_eq!(k.call(nr::SIGACTION, &[32, 0, BUF]), Err(EINVAL));
    // The mask.
    k.poke(PTR, &u32::MAX.to_le_bytes());
    assert_eq!(k.call(nr::SIGPROCMASK, &[3, PTR, BUF]).unwrap().value, 0);
    assert_eq!(k.call(nr::SIGPROCMASK, &[1, 0, BUF]).unwrap().value, 0);
    assert_eq!(k.peek(BUF, 4), 0xfffe_feffu32.to_le_bytes());
    assert_eq!(k.call(nr::SIGPROCMASK, &[9, PTR, BUF]), Err(EINVAL));
    assert_eq!(k.call(nr::SIGPROCMASK, &[1, 8, BUF]), Err(EFAULT));
    // kill: blocked signals wait; the signal number is checked first.
    assert_eq!(k.call(nr::KILL, &[0, 30]).unwrap().value, 0);
    k.call(nr::SIGPENDING, &[BUF]).unwrap();
    assert_eq!(k.peek(BUF, 4), signal::bit(30).to_le_bytes());
    assert_eq!(k.call(nr::KILL, &[99_999_999, 32]), Err(EINVAL));
    assert_eq!(k.call(nr::KILL, &[99_999_999, 15]), Err(ESRCH));
    assert_eq!(k.call(nr::KILL, &[8, 0]).unwrap().value, 0);
    assert_eq!(k.call(nr::KILL, &[8, 15]).unwrap().value, 0);
    // A default-terminating signal to itself ends the caller.
    k.poke(PTR, &0u32.to_le_bytes());
    k.call(nr::SIGPROCMASK, &[3, PTR, 0]).unwrap();
    assert_eq!(k.call(nr::KILL, &[7, 20]).unwrap().value, 0);
    assert_eq!(k.killed_by, None, "SIGCHLD is ignored by default");
    k.call(nr::KILL, &[7, 15]).unwrap();
    assert_eq!(k.killed_by, Some(15));
}

#[test]
fn spawn_and_wait() {
    let mut k = Fake::new();
    let path = BASE + 0x500;
    let argv = BASE + 0x600;
    let envp = BASE + 0x700;
    k.poke(path, b"/bin/child\0");
    k.poke(BASE + 0x800, b"child\0");
    k.poke(BASE + 0x810, b"x\0");
    k.poke(BASE + 0x820, b"A=1\0");
    k.poke(argv, &(BASE + 0x800).to_le_bytes());
    k.poke(argv + 8, &(BASE + 0x810).to_le_bytes());
    k.poke(argv + 16, &0u64.to_le_bytes());
    k.poke(envp, &(BASE + 0x820).to_le_bytes());
    k.poke(envp + 8, &0u64.to_le_bytes());
    assert_eq!(k.call(nr::POSIX_SPAWN, &[PTR, path, 0, argv, envp]).unwrap().value, 0);
    assert_eq!(k.peek(PTR, 4), 8i32.to_le_bytes());
    let (p, a, e) = &k.spawned[0];
    assert_eq!(p, b"/bin/child");
    assert_eq!(a, &[b"child".to_vec(), b"x".to_vec()]);
    assert_eq!(e, &[b"A=1".to_vec()]);
    // wait4: an unwritable status first, then the exit code in bits 8..15.
    assert_eq!(k.call(nr::WAIT4, &[8, 8, 0, 0]), Err(EFAULT));
    assert_eq!(k.call(nr::WAIT4, &[-1i64 as u64, BUF, 0, 0]).unwrap().value, 8);
    assert_eq!(k.peek(BUF, 4), 0x700u32.to_le_bytes());
    assert_eq!(k.call(nr::WAIT4, &[-1i64 as u64, BUF, 0, 0]), Err(ECHILD));
    k.poke(path, b"/bin/nope\0");
    assert_eq!(k.call(nr::POSIX_SPAWN, &[PTR, path, 0, argv, envp]), Err(ENOENT));
    assert_eq!(ChildStatus::Signaled(15).word(), 15);
}

#[test]
fn registers() {
    let mut k = Fake::new();
    let mut r = SyscallRegs { rax: 0x200_0000 | nr::PIPE, ..SyscallRegs::default() };
    assert_eq!(dispatch_with(&mut k, &mut r), Some(Outcome::Handled));
    assert_eq!((r.rax, r.rdx, r.rflags & RFLAGS_CF), (3, 4, 0));
    let mut r = SyscallRegs { rax: 0x200_0000 | nr::CLOSE, rdi: 99, ..SyscallRegs::default() };
    dispatch_with(&mut k, &mut r);
    assert_eq!((r.rax, r.rflags & RFLAGS_CF != 0), (u64::from(EBADF), true));
    // Not this layer's: the Mach core's table or ENOSYS next.
    let mut r = SyscallRegs { rax: 0x200_0000 | 74, ..SyscallRegs::default() };
    assert_eq!(dispatch_with(&mut k, &mut r), None);
    let mut r = SyscallRegs { rax: 0x100_001f, ..SyscallRegs::default() };
    assert_eq!(dispatch_with(&mut k, &mut r), None);
}

// ----- ADR 0246 addendum: waitid, vfork, execve's order, xattr names, ioctl -----

#[test]
fn waitid_checks_its_options_first() {
    use crate::bsd::nr::WAITID;
    let mut f = Fake::new();
    // No option at all, an unknown bit: EINVAL; a group id 0: ECHILD; a
    // negative one: EINVAL.
    assert_eq!(f.call(WAITID, &[0, 0, Fake::BUF, 0]), Err(EINVAL));
    assert_eq!(f.call(WAITID, &[0, 0, Fake::BUF, 0x4 | 0x1000]), Err(EINVAL));
    assert_eq!(f.call(WAITID, &[2, 0, Fake::BUF, 0x4]), Err(ECHILD));
    assert_eq!(f.call(WAITID, &[2, (-3i64) as u64, Fake::BUF, 0x4]), Err(EINVAL));
    // Only WNOHANG: accepted, nothing reported.
    assert_eq!(f.call(WAITID, &[0, 0, Fake::BUF, 0x1]).map(|r| r.value), Ok(0));
}

#[test]
fn vfork_ends_the_caller_with_sigsys() {
    let mut f = Fake::new();
    let _ = f.call(crate::bsd::nr::VFORK, &[]);
    assert_eq!(f.killed_by, Some(crate::bsd::signal::SIGSYS));
}

#[test]
fn execve_reads_its_arguments_first() {
    use crate::bsd::nr::EXECVE;
    let mut f = Fake::new();
    // An unreadable path is EFAULT; an empty one ENOENT.
    assert_eq!(f.call(EXECVE, &[8, 0, 0]), Err(EFAULT));
    f.poke(Fake::BUF, b"\0");
    assert_eq!(f.call(EXECVE, &[Fake::BUF, 0, 0]), Err(ENOENT));
}

#[test]
fn xattr_names_are_checked() {
    use crate::bsd::nr::GETXATTR;
    let mut f = Fake::new();
    f.poke(Fake::BUF, b"file\0");
    // An empty name: EINVAL; 128 bytes: ENAMETOOLONG; not UTF-8: EINVAL.
    f.poke(Fake::BUF + 64, b"\0");
    assert_eq!(f.call(GETXATTR, &[Fake::BUF, Fake::BUF + 64, 0, 0, 0, 0]), Err(EINVAL));
    let long = [b'a'; 128];
    f.poke(Fake::BUF + 64, &long);
    f.poke(Fake::BUF + 64 + 128, b"\0");
    assert_eq!(f.call(GETXATTR, &[Fake::BUF, Fake::BUF + 64, 0, 0, 0, 0]), Err(ENAMETOOLONG));
    f.poke(Fake::BUF + 64, b"\xff\xfe\0");
    assert_eq!(f.call(GETXATTR, &[Fake::BUF, Fake::BUF + 64, 0, 0, 0, 0]), Err(EINVAL));
    // A position on an attribute other than the resource fork: EINVAL.
    f.poke(Fake::BUF + 64, b"name\0");
    assert_eq!(f.call(GETXATTR, &[Fake::BUF, Fake::BUF + 64, 0, 0, 3, 0]), Err(EINVAL));
    // The descriptor forms take no XATTR_NOFOLLOW.
    assert_eq!(f.call(crate::bsd::nr::FGETXATTR, &[1, Fake::BUF + 64, 0, 0, 0, 1]), Err(EINVAL));
}

#[test]
fn ioctl_requests() {
    use crate::bsd::nr::IOCTL;
    let mut f = Fake::new();
    assert_eq!(f.call(IOCTL, &[42, 0x1234_5678, 0]), Err(EBADF));
    assert_eq!(f.call(IOCTL, &[1, 0x1234_5678, 0]), Err(25));
    assert_eq!(f.call(IOCTL, &[1, u64::from(crate::bsd::FIOCLEX), 0]).map(|r| r.value), Ok(0));
    assert_eq!(f.cloexec(1), Ok(true));
    assert_eq!(f.call(IOCTL, &[1, u64::from(crate::bsd::FIONCLEX), 0]).map(|r| r.value), Ok(0));
    assert_eq!(f.cloexec(1), Ok(false));
}

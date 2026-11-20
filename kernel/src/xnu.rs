//! The XNU personality's processes: starting a Mach-O program with Poly's
//! own dyld, and its first BSD system calls (ADR 0237).
//!
//! [`prepare`] links the program and its libraries with `poly-dyld`
//! (reading them through the VFS), maps every segment, writes every fixup,
//! gives each image with thread-local variables its block for the main
//! thread, and maps Poly's start code (`poly_dyld::start`) with its data
//! block: the first thread runs the initializers, then `main`, then
//! libSystem's `exit`. A static program (`LC_UNIXTHREAD`, no
//! `LC_LOAD_DYLINKER`) starts at its own entry point instead. The stack is
//! `poly_dyld::stack`'s: the header address below `argc` when dyld runs,
//! `apple[0]` = `executable_path=<path>`.
//!
//! System calls go to the BSD layer of the personality
//! (`poly_personality_xnu::bsd`, ADR 0246) with this module's [`Thread`] as
//! its kernel: files on the process's descriptor table, mappings in its
//! `Memory`, sysctl values, signals and `posix_spawn` children in
//! [`XnuState`].
//!
//! arm64 programs run in the translator's runtime ([`arm64`], ADR 0257).
//! The files are read whole into the kernel heap and their segments copied
//! into anonymous memory (file-backed mappings come with larger programs).

pub mod arm64;

use crate::fs::cache::FileSource;
use crate::fs::poll::ThreadBlocker;
use crate::fs::{FileType, OpenFile};
use crate::process::{self, ExitStatus, Process, ThreadFlags};
use crate::sync::SpinLock;
use crate::user::memory::{Layout, Memory, MemoryError, Placement, Prot};
use crate::user::{self, Entry, FileRef};
use crate::{fs, program, thread};
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_abi::{Outcome, SyscallRegs};
use poly_dyld::{Program, start};
use poly_personality_xnu::bsd::signal::{self, Signals};
use poly_personality_xnu::bsd::{self, BsdKernel, errno, file};
use poly_vfs::{At, Cred, OpenFlags, Whence};

/// The largest file the loader reads (it reads files whole).
const MAX_FILE: usize = 64 << 20;
/// The initial stack region.
const STACK_SIZE: u64 = 512 << 10;

/// A Mach-O program ready to start.
pub struct Loaded {
    pub memory: Arc<Memory>,
    pub entry: Entry,
}

/// Files for the loader, read whole through the VFS as the kernel.
pub(crate) struct VfsFiles;

impl poly_dyld::FileSystem for VfsFiles {
    fn read(&mut self, path: &str) -> Option<Vec<u8>> {
        let vfs = fs::vfs();
        let ctx = vfs.root_context().ok()?;
        let file = vfs.open(&ctx, &Cred::root(), At::Cwd, path.as_bytes(), OpenFlags::RDONLY, 0).ok()?;
        let mut out = Vec::new();
        let mut chunk = alloc::vec![0u8; 64 * 1024];
        loop {
            match file.read_at(out.len() as u64, &mut chunk) {
                Ok(0) => return Some(out),
                Ok(n) if out.len() + n <= MAX_FILE => out.extend_from_slice(&chunk[..n]),
                _ => return None,
            }
        }
    }
}

fn prot(read: bool, write: bool, exec: bool) -> Prot {
    let bits = u64::from(read) | u64::from(write) << 1 | u64::from(exec) << 2;
    Prot::from_bits(bits).expect("three bits")
}

fn page_up(x: u64) -> u64 {
    (x + 4095) & !4095
}

/// Link and map the Mach-O program at `path`; its first thread is not
/// started. Errors are dyld-style messages.
pub fn prepare(path: &[u8], argv: &[&[u8]], envp: &[&[u8]]) -> Result<Loaded, String> {
    let path = core::str::from_utf8(path).map_err(|_| "the path is not UTF-8".to_string())?;
    let program = poly_dyld::link(&mut VfsFiles, path, &poly_dyld::Options::X86_64).map_err(|e| e.to_string())?;
    let memory = Memory::new(Layout::default()).map_err(|e| format!("no memory: {e:?}"))?;
    map_images(&memory, &program)?;
    let mut failed = None;
    program
        .fixups(&mut |addr, value| {
            if failed.is_none() && memory.write(addr, &value.to_le_bytes()).is_err() {
                failed = Some(addr);
            }
        })
        .map_err(|e| e.to_string())?;
    if let Some(addr) = failed {
        return Err(format!("a fixup at {addr:#x} is outside the mapped segments"));
    }

    let header = program.images[0].base;
    let (rip, with_header) = match (program.dynamic, program.entry) {
        (false, poly_dyld::Entry::Thread { pc, .. }) => (pc, false),
        (_, entry) => (map_start(&memory, &program, entry)?, true),
    };
    protect_images(&memory, &program)?;

    // The stack: the header address (when dyld runs), argc, argv, envp,
    // apple, the strings.
    let apple0 = format!("executable_path={path}");
    let apple: [&[u8]; 1] = [apple0.as_bytes()];
    let layout = memory.layout();
    memory.setup_stack(STACK_SIZE, Prot::RW).map_err(|e| format!("no stack: {e:?}"))?;
    let (stack, _) = poly_dyld::stack::build(layout.stack_top, argv, envp, &apple, with_header.then_some(header))
        .ok_or("the arguments do not fit the stack")?;
    if stack.bytes.len() as u64 > STACK_SIZE / 2 {
        return Err("the arguments do not fit the stack".into());
    }
    memory.write(stack.sp, &stack.bytes).map_err(|_| "the stack cannot be written".to_string())?;
    Ok(Loaded { memory: Arc::new(memory), entry: Entry { rip, rsp: stack.sp, args: [0; 3] } })
}

/// Every segment, writable, holding its file bytes (protections come after
/// the fixups).
fn map_images(memory: &Memory, program: &Program) -> Result<(), String> {
    for im in &program.images {
        for s in &im.segments {
            memory
                .map_anonymous(Placement::FixedNoReplace(s.addr), s.mem_size, Prot::RW)
                .map_err(|e| format!("{}: segment {} at {:#x}: {e:?}", im.path, s.name, s.addr))?;
            let n = s.file_size.min(s.mem_size) as usize;
            let from = s.file_offset as usize;
            let bytes = im.file.get(from..from + n).ok_or_else(|| format!("{}: segment outside the file", im.path))?;
            memory.write(s.addr, bytes).map_err(|_| format!("{}: segment {} cannot be written", im.path, s.name))?;
        }
    }
    Ok(())
}

fn protect_images(memory: &Memory, program: &Program) -> Result<(), String> {
    for im in &program.images {
        for s in &im.segments {
            let p = prot(s.prot.read, s.prot.write && !s.read_only_after_fixups, s.prot.execute);
            memory.protect(s.addr, s.mem_size, p).map_err(|e| format!("{}: {e:?}", im.path))?;
        }
    }
    Ok(())
}

/// Map the start code and its data block, and the thread-local blocks of
/// the main thread; returns where the thread starts.
fn map_start(memory: &Memory, program: &Program, entry: poly_dyld::Entry) -> Result<u64, String> {
    let (target, mode) = match entry {
        poly_dyld::Entry::Main { addr, .. } => (addr, start::Mode::CallMain),
        poly_dyld::Entry::Thread { pc, .. } => (pc, start::Mode::Jump),
    };
    let data = start::StartData {
        entry: target,
        mode,
        exit: program.exit_function().map_err(|e| e.to_string())?.unwrap_or(0),
        initializers: program.initializers(),
    }
    .to_bytes();
    let len = start::DATA_OFFSET + page_up(data.len() as u64);
    let code = memory.map_anonymous(Placement::Hint(0), len, Prot::RW).map_err(|e| format!("no memory: {e:?}"))?;
    let w = |at: u64, bytes: &[u8]| memory.write(at, bytes).map_err(|_| "the start code cannot be written".to_string());
    w(code, &start::X86_64_START)?;
    w(code + start::DATA_OFFSET, &data)?;
    memory.protect(code, start::DATA_OFFSET, Prot::from_bits(0x5).expect("r-x")).map_err(|e| format!("{e:?}"))?;
    memory.protect(code + start::DATA_OFFSET, len - start::DATA_OFFSET, Prot::READ).map_err(|e| format!("{e:?}"))?;

    // Thread-local variables: one block per image for the main thread, a
    // copy of its template (as the fixups left it).
    let thunk = code + start::X86_64_TLV_GET;
    for (i, im) in program.images.iter().enumerate() {
        let Some(t) = &im.tlv else { continue };
        let size = page_up(t.size.max(1));
        let block =
            memory.map_anonymous(Placement::Hint(0), size, Prot::RW).map_err(|e| format!("no memory: {e:?}"))?;
        if t.init_size > 0 {
            let mut init = alloc::vec![0u8; t.init_size as usize];
            memory.read(t.template, &mut init).map_err(|_| format!("{}: thread-local template unreadable", im.path))?;
            w(block, &init)?;
        }
        let mut failed = false;
        program.tlv_writes(i, block, thunk, &mut |a, v| failed |= memory.write(a, &v.to_le_bytes()).is_err());
        if failed {
            return Err(format!("{}: thread-local descriptors cannot be written", im.path));
        }
    }
    Ok(code)
}

/// The per-process state of the XNU personality: signals and the children
/// `posix_spawn` started that were not waited for yet.
#[derive(Debug)]
pub struct XnuState {
    pub signals: SpinLock<Signals>,
    pub children: SpinLock<BTreeMap<u32, Arc<Process>>>,
}

impl Default for XnuState {
    fn default() -> Self {
        XnuState { signals: SpinLock::new(Signals::default()), children: SpinLock::new(BTreeMap::new()) }
    }
}

/// A system call of an XNU process: the BSD layer first, then the "not
/// implemented" answers (the Mach core's table joins here, ADR 0238).
pub fn syscall(process: &Process, regs: &mut SyscallRegs) -> Outcome {
    let Some(flags) = user::current_flags() else {
        return <poly_personality_xnu::Xnu as poly_abi::SyscallDispatch>::dispatch(regs);
    };
    let Some(state) = process.xnu.as_ref() else {
        return <poly_personality_xnu::Xnu as poly_abi::SyscallDispatch>::dispatch(regs);
    };
    match bsd::dispatch_with(&mut Thread { process, state, flags: Arc::clone(&flags) }, regs) {
        Some(outcome) => outcome,
        // Not a call of the BSD layer (the registers are untouched): the
        // Mach core's (ADR 0238).
        None => {
            let tid = user::current_tid().unwrap_or(0);
            poly_personality_xnu::dispatch_with(&mut crate::xnu_mach::MachThread { process, flags, tid }, regs)
        }
    }
}

struct Thread<'a> {
    process: &'a Process,
    state: &'a XnuState,
    flags: Arc<ThreadFlags>,
}

/// A VFS (Linux-numbered) errno as Darwin's.
fn de(e: poly_vfs::Errno) -> u32 {
    errno::from_linux(e as u32)
}

fn mem_err(e: MemoryError) -> u32 {
    errno::from_linux(e.linux_errno() as u32)
}

/// Darwin's `AT_FDCWD` (-2) as the kernel's.
fn at(dirfd: i32) -> i32 {
    if dirfd == file::AT_FDCWD { fs::AT_FDCWD } else { dirfd }
}

fn darwin_stat(st: &poly_vfs::Stat) -> file::Stat {
    let dev = |d: poly_vfs::DeviceId| (d.major << 24) | (d.minor & 0xff_ffff);
    let t = |t: poly_vfs::Timespec| (t.sec, i64::from(t.nsec));
    file::Stat {
        dev: dev(st.dev),
        ino: st.ino,
        mode: st.mode,
        nlink: st.nlink,
        uid: st.uid,
        gid: st.gid,
        rdev: dev(st.rdev),
        size: st.size,
        blocks: st.blocks,
        blksize: st.blksize,
        times: [t(st.atime), t(st.mtime), t(st.ctime), t(st.ctime)],
    }
}

impl Thread<'_> {
    fn blocker(&self) -> ThreadBlocker<'_> {
        ThreadBlocker::new(self.process, Arc::clone(&self.flags))
    }

    fn memory(&self) -> Result<Arc<Memory>, u32> {
        user::current_memory().ok_or(errno::ENOMEM)
    }
}

impl BsdKernel for Thread<'_> {
    fn copy_in(&self, addr: u64, buf: &mut [u8]) -> Result<(), ()> {
        if buf.is_empty() {
            return Ok(());
        }
        user::copy_from_user(buf, addr).map_err(|_| ())
    }

    fn copy_out(&mut self, addr: u64, buf: &[u8]) -> Result<(), ()> {
        if buf.is_empty() {
            return Ok(());
        }
        user::copy_to_user(addr, buf).map_err(|_| ())
    }

    fn exit(&mut self, status: u8) {
        crate::xnu_mach::task_ended(self.process.pid);
        let status = self.process.group_exit(ExitStatus::Exited(status));
        let _ = user::request_exit(status);
    }

    fn write(&mut self, fd: i32, buf: &[u8], offset: Option<u64>) -> Result<usize, u32> {
        let file = self.process.files.file(fd).map_err(de)?;
        self.process.files.before_write(&file, buf.len()).map_err(de)?;
        match offset {
            Some(o) => file.write_at(o, buf).map_err(de),
            None => {
                let mut blocker = self.blocker();
                file.write_wait(buf, &mut blocker).map_err(de)
            }
        }
    }

    fn read(&mut self, fd: i32, buf: &mut [u8], offset: Option<u64>) -> Result<usize, u32> {
        let file = self.process.files.file(fd).map_err(de)?;
        if !file.readable() {
            return Err(errno::EBADF);
        }
        if file.kind() == FileType::Directory {
            return Err(errno::EISDIR);
        }
        match offset {
            Some(o) => file.read_at(o, buf).map_err(de),
            None => {
                let mut blocker = self.blocker();
                file.read_wait(buf, &mut blocker).map_err(de)
            }
        }
    }

    fn open(&mut self, dirfd: i32, path: &[u8], how: &file::OpenHow) -> Result<i32, u32> {
        let mut flags = match how.access {
            file::Access::Read => OpenFlags::RDONLY,
            file::Access::Write => OpenFlags::WRONLY,
            file::Access::ReadWrite => OpenFlags::RDWR,
        };
        for (on, f) in [
            (how.create, OpenFlags::CREAT),
            (how.truncate, OpenFlags::TRUNC),
            (how.exclusive, OpenFlags::EXCL),
            (how.append, OpenFlags::APPEND),
            (how.nonblock, OpenFlags::NONBLOCK),
            (how.directory, OpenFlags::DIRECTORY),
            (how.nofollow, OpenFlags::NOFOLLOW),
            (how.cloexec, OpenFlags::CLOEXEC),
            (how.noctty, OpenFlags::NOCTTY),
        ] {
            if on {
                flags = flags | f;
            }
        }
        self.process.files.openat(at(dirfd), path, flags, how.mode).map_err(de)
    }

    fn close(&mut self, fd: i32) -> Result<(), u32> {
        self.process.files.close(fd).map_err(de)
    }

    fn lseek(&mut self, fd: i32, offset: i64, whence: u32) -> Result<u64, u32> {
        let whence = match whence {
            0 => Whence::Set,
            1 => Whence::Cur,
            _ => Whence::End,
        };
        self.process.files.lseek(fd, offset, whence).map_err(de)
    }

    fn stat(&mut self, of: bsd::StatOf<'_>) -> Result<file::Stat, u32> {
        let st = match of {
            bsd::StatOf::Fd(fd) => self.process.files.fstat(fd),
            bsd::StatOf::Path { dirfd, path, follow } => self.process.files.statat(at(dirfd), path, follow),
        };
        st.map(|s| darwin_stat(&s)).map_err(de)
    }

    fn read_dir(&mut self, fd: i32, emit: &mut dyn FnMut(&file::DirEntry<'_>) -> bool) -> Result<(), u32> {
        self.process
            .files
            .getdents(fd, &mut |e| {
                // Darwin's d_type values are Linux's (S_IFMT >> 12).
                emit(&file::DirEntry { ino: e.ino, kind: e.kind.dirent_type(), name: e.name })
            })
            .map(|_| ())
            .map_err(de)
    }

    fn dup(&mut self, fd: i32, min: i32, cloexec: bool) -> Result<i32, u32> {
        self.process.files.dup_from(fd, min, cloexec).map_err(de)
    }

    fn dup2(&mut self, old: i32, new: i32) -> Result<i32, u32> {
        self.process.files.dup2(old, new).map_err(de)
    }

    fn cloexec(&self, fd: i32) -> Result<bool, u32> {
        self.process.files.cloexec(fd).map_err(de)
    }

    fn set_cloexec(&mut self, fd: i32, on: bool) -> Result<(), u32> {
        self.process.files.set_cloexec(fd, on).map_err(de)
    }

    fn status(&self, fd: i32) -> Result<file::Status, u32> {
        let f = self.process.files.file(fd).map_err(de)?;
        let flags = f.flags();
        let access = match (f.readable(), f.writable()) {
            (true, true) => file::Access::ReadWrite,
            (false, true) => file::Access::Write,
            _ => file::Access::Read,
        };
        Ok(file::Status {
            access,
            append: flags.contains(OpenFlags::APPEND),
            nonblock: flags.contains(OpenFlags::NONBLOCK),
        })
    }

    fn set_status(&mut self, fd: i32, append: bool, nonblock: bool) -> Result<(), u32> {
        let files = &self.process.files;
        let mut flags = files.status_flags(fd).map_err(de)?;
        for (on, f) in [(append, OpenFlags::APPEND), (nonblock, OpenFlags::NONBLOCK)] {
            flags = if on { flags | f } else { flags.without(f) };
        }
        files.set_status_flags(fd, flags).map_err(de)
    }

    fn path_of(&self, fd: i32) -> Result<Vec<u8>, u32> {
        let file = self.process.files.file(fd).map_err(de)?;
        let ctx = self.process.files.context();
        fs::vfs().file_path(&ctx, &file).map_err(de)
    }

    fn chdir(&mut self, path: &[u8]) -> Result<(), u32> {
        self.process.files.chdir(path).map_err(de)
    }

    fn fchdir(&mut self, fd: i32) -> Result<(), u32> {
        self.process.files.fchdir(fd).map_err(de)
    }

    fn pipe(&mut self) -> Result<(i32, i32), u32> {
        let (r, w) = poly_vfs::pipe::Pipe::pair(false);
        let files = &self.process.files;
        let node = fs::vfs().anon_node(FileType::Fifo, 0o660, &files.cred()).map_err(de)?;
        let read = Arc::new(OpenFile::on_object(node.clone(), Arc::new(r), OpenFlags::RDONLY));
        let write = Arc::new(OpenFile::on_object(node, Arc::new(w), OpenFlags::WRONLY));
        let rfd = files.install(read, false).map_err(de)?;
        match files.install(write, false) {
            Ok(wfd) => Ok((rfd, wfd)),
            Err(e) => {
                let _ = files.close(rfd);
                Err(de(e))
            }
        }
    }

    fn mmap(&mut self, r: &bsd::MapRequest) -> Result<u64, u32> {
        let memory = self.memory()?;
        let prot = Prot::from_bits(u64::from(r.prot)).ok_or(errno::EINVAL)?;
        let req = match r.file {
            None if r.shared => {
                let object = FileRef::new(Arc::new(user::AnonShared::new(r.len)));
                poly_vm::MapRequest::file(r.len, prot, object, 0, true)
            }
            None => poly_vm::MapRequest::anonymous(r.len, prot),
            Some((fd, offset)) => {
                let file = self.process.files.file(fd).map_err(|_| errno::EBADF)?;
                if file.kind() == FileType::CharDevice || file.object().is_some() {
                    return Err(errno::ENODEV);
                }
                if file.kind() != FileType::Regular {
                    return Err(errno::EACCES);
                }
                let source = FileSource::of(&file).backed(r.shared && r.prot & 2 != 0);
                poly_vm::MapRequest::file(r.len, prot, FileRef::new(Arc::new(source)), offset, r.shared)
            }
        };
        let placement = if r.fixed { Placement::Fixed(r.addr) } else { Placement::Hint(r.addr) };
        memory.map(req.at(placement)).map_err(mem_err)
    }

    fn munmap(&mut self, addr: u64, len: u64) -> Result<(), u32> {
        self.memory()?.unmap(addr, len).map_err(mem_err)
    }

    fn page_size(&self) -> u64 {
        4096
    }

    fn pid(&self) -> u32 {
        self.process.pid
    }

    fn ppid(&self) -> u32 {
        self.process.ppid
    }

    fn thread_id(&self) -> u64 {
        u64::from(user::current_tid().unwrap_or(self.process.pid))
    }

    fn ids(&self) -> (u32, u32) {
        let c = self.process.files.cred();
        (c.uid, c.gid)
    }

    fn process_exists(&self, pid: u32) -> bool {
        process::find(pid).is_some_and(|p| !p.has_ended())
    }

    fn random(&mut self, buf: &mut [u8]) {
        crate::random::fill(buf);
    }

    fn system(&self) -> bsd::sysctl::System {
        use poly_procfs::ProcSource;
        let s = fs::proc::KernelProc::default().system();
        let boot = crate::time::realtime_offset_ns();
        bsd::sysctl::System {
            ncpu: crate::smp::cpus_online() as u32,
            memsize: s.mem_total,
            page_size: 4096,
            hostname: "poly".into(),
            boottime: ((boot / 1_000_000_000) as i64, ((boot % 1_000_000_000) / 1000) as i64),
            machine: "x86_64",
            translated: false,
        }
    }

    fn with_signals(&mut self, f: &mut dyn FnMut(&mut Signals)) {
        f(&mut self.state.signals.lock());
    }

    fn exit_signal(&mut self, sig: u32) {
        crate::xnu_mach::task_ended(self.process.pid);
        let status = self.process.group_exit(ExitStatus::Signaled { signal: sig, core: false });
        let _ = user::request_exit(status);
    }

    fn kill(&mut self, pid: u32, sig: u32) -> Result<(), u32> {
        let target = process::find(pid).filter(|p| !p.has_ended()).ok_or(errno::ESRCH)?;
        let Some(state) = target.xnu.as_ref() else {
            // A process of another personality: the signals both name the
            // same way end it as killed by that signal.
            if matches!(sig, signal::SIGHUP | signal::SIGINT | signal::SIGKILL | signal::SIGTERM) {
                target.group_exit(ExitStatus::Signaled { signal: sig, core: false });
            }
            return Ok(());
        };
        let d = state.signals.lock().deliver(sig);
        if d == signal::Disposition::Terminate {
            target.group_exit(ExitStatus::Signaled { signal: sig, core: false });
        }
        Ok(())
    }

    fn spawn(&mut self, path: &[u8], argv: &[Vec<u8>], envp: &[Vec<u8>]) -> Result<u32, u32> {
        let prepared = prepare_child_deep(path, argv, envp, self.process).map_err(|e| match e {
            program::ExecPathError::Open(e) => de(e),
            program::ExecPathError::Unsupported(_) => errno::ENOEXEC,
            program::ExecPathError::Exec(e) => errno::from_linux(e.linux_errno() as u32),
        })?;
        let child = Arc::clone(prepared.process());
        // What an exec keeps: descriptors without close-on-exec, the mask
        // and ignored signals (the posix_spawn sheet).
        for (fd, _, _) in child.files.fd_list() {
            let _ = child.files.close(fd);
        }
        for (fd, file, cloexec) in self.process.files.fd_list() {
            if cloexec {
                continue;
            }
            let at = child.files.install(file, false).map_err(de)?;
            if at != fd {
                child.files.dup2(at, fd).map_err(de)?;
                let _ = child.files.close(at);
            }
        }
        if let Some(cs) = child.xnu.as_ref() {
            *cs.signals.lock() = self.state.signals.lock().for_exec();
        }
        let child = prepared.start().map_err(|_| errno::ENOEXEC)?;
        let pid = child.pid;
        self.state.children.lock().insert(pid, child);
        Ok(pid)
    }

    fn wait(&mut self, pid: i32, nohang: bool) -> Result<Option<(u32, bsd::ChildStatus)>, u32> {
        let matching = |c: &BTreeMap<u32, Arc<Process>>| -> Vec<Arc<Process>> {
            c.values().filter(|p| pid == -1 || p.pid == pid as u32).cloned().collect()
        };
        if matching(&self.state.children.lock()).is_empty() {
            return Err(errno::ECHILD);
        }
        let me = self.process;
        let ended = || matching(&self.state.children.lock()).into_iter().find(|p| p.has_ended());
        let child = match ended() {
            Some(c) => c,
            None if nohang => return Ok(None),
            None => {
                process::wait_event(|| ended().is_some() || me.is_exiting());
                ended().ok_or(errno::EINTR)?
            }
        };
        self.state.children.lock().remove(&child.pid);
        let status = match child.status() {
            Some(ExitStatus::Exited(c)) => bsd::ChildStatus::Exited(c),
            Some(ExitStatus::Signaled { signal, .. }) => bsd::ChildStatus::Signaled(signal),
            _ => bsd::ChildStatus::Signaled(signal::SIGSEGV),
        };
        Ok(Some((child.pid, status)))
    }
}

/// What [`prepare_child_deep`] hands its helper thread, and what comes
/// back.
struct SpawnJob<'a> {
    path: &'a [u8],
    argv: &'a [Vec<u8>],
    envp: &'a [Vec<u8>],
    parent: &'a Process,
    result: Option<Result<program::Prepared, program::ExecPathError>>,
}

fn spawn_job(arg: usize) -> usize {
    // SAFETY: `arg` is the address of the `SpawnJob` of `prepare_child_deep`,
    // which waits for this thread before the job goes away, and nothing
    // else touches the job meanwhile.
    let job = unsafe { &mut *(arg as *mut SpawnJob<'_>) };
    let argv: Vec<&[u8]> = job.argv.iter().map(Vec::as_slice).collect();
    let envp: Vec<&[u8]> = job.envp.iter().map(Vec::as_slice).collect();
    job.result = Some(program::prepare_child(job.path, &argv, &envp, job.parent));
    0
}

/// `program::prepare_child` on a kernel thread of its own: loading a
/// program (dyld's linking above all) is deep, and a system call already
/// sits deep in its thread's 16 KiB kernel stack.
fn prepare_child_deep(
    path: &[u8],
    argv: &[Vec<u8>],
    envp: &[Vec<u8>],
    parent: &Process,
) -> Result<program::Prepared, program::ExecPathError> {
    let mut job = SpawnJob { path, argv, envp, parent, result: None };
    let addr = &raw mut job as usize;
    let unsupported = || program::ExecPathError::Unsupported("no thread to load the program on".into());
    let tid = thread::spawn_named("xnu-spawn", spawn_job, addr).map_err(|_| unsupported())?;
    // The job lives until the helper has ended.
    let _ = thread::join(tid);
    job.result.take().unwrap_or_else(|| Err(unsupported()))
}

/// A new XNU process record for a Mach-O program.
pub fn new_process(ppid: u32, format: poly_binfmt::Format, traced: bool) -> Arc<Process> {
    Arc::new(Process::new_xnu(process::alloc_pid(), ppid, format, traced))
}

/// Start `loaded`'s first thread in `process`.
pub fn start(process: &Arc<Process>, loaded: Loaded) -> Result<(), thread::SpawnError> {
    let tid = user::spawn_in(Arc::clone(process), loaded.memory, loaded.entry, user::DEFAULT_HOOK)?;
    let _ = thread::detach(tid);
    Ok(())
}

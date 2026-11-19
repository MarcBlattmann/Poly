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
//! Only x86-64 programs run: arm64 ones wait for the translator (ADR 0151).
//! The files are read whole into the kernel heap and their segments copied
//! into anonymous memory (file-backed mappings come with larger programs).

use crate::fs::poll::ThreadBlocker;
use crate::process::{self, ExitStatus, Process, ThreadFlags};
use crate::user::memory::{Layout, Memory, Placement, Prot};
use crate::user::{self, Entry};
use crate::{fs, thread};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_abi::{Outcome, SyscallRegs};
use poly_dyld::{Program, start};
use poly_personality_xnu::bsd::{self, BsdKernel};
use poly_vfs::{At, Cred, OpenFlags};

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
struct VfsFiles;

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

/// A system call of an XNU process.
pub fn syscall(process: &Process, regs: &mut SyscallRegs) -> Outcome {
    let Some(flags) = user::current_flags() else {
        return <poly_personality_xnu::Xnu as poly_abi::SyscallDispatch>::dispatch(regs);
    };
    bsd::dispatch_with(&mut Thread { process, flags }, regs)
}

struct Thread<'a> {
    process: &'a Process,
    flags: Arc<ThreadFlags>,
}

/// A VFS errno as Darwin's: 1 to 34 are the same numbers on both; the
/// would-block error is 35 on Darwin (`<sys/errno.h>`).
fn darwin_errno(e: poly_vfs::Errno) -> u64 {
    match e {
        poly_vfs::Errno::EAGAIN => 35,
        e if (e as u64) <= 34 => e as u64,
        _ => bsd::EINVAL,
    }
}

impl BsdKernel for Thread<'_> {
    fn write(&mut self, fd: i32, buf: u64, len: u64) -> Result<u64, u64> {
        let file = self.process.files.file(fd).map_err(darwin_errno)?;
        let mut done = 0u64;
        let mut chunk = alloc::vec![0u8; len.min(64 * 1024) as usize];
        while done < len {
            let n = (len - done).min(chunk.len() as u64) as usize;
            if user::copy_from_user(&mut chunk[..n], buf.wrapping_add(done)).is_err() {
                return if done == 0 { Err(bsd::EFAULT) } else { Ok(done) };
            }
            self.process.files.before_write(&file, n).map_err(darwin_errno)?;
            let mut blocker = ThreadBlocker::new(self.process, Arc::clone(&self.flags));
            match file.write_wait(&chunk[..n], &mut blocker) {
                Ok(w) => {
                    done += w as u64;
                    if w < n {
                        break;
                    }
                }
                Err(e) if done == 0 => return Err(darwin_errno(e)),
                Err(_) => break,
            }
        }
        Ok(done)
    }

    fn exit(&mut self, status: u8) {
        let status = self.process.group_exit(ExitStatus::Exited(status));
        let _ = user::request_exit(status);
    }
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

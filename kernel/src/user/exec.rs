//! `exec` of static ELF executables: a new [`Memory`] laid out from the
//! file, the initial stack, and a thread at the entry point.
//!
//! The plan comes from `poly_elf` (ADR 0024), the regions from `poly_vm`
//! (ADR 0026); this module joins them (ADR 0036):
//!
//! 1. Parse and validate the file ([`poly_elf::Elf::parse`]). Images with a
//!    program interpreter (`PT_INTERP`, dynamically linked) are refused with
//!    [`ExecError::Interpreter`] until the kernel loads `ld.so` as well.
//! 2. Plan the mappings ([`poly_elf::LoadPlan`]): `ET_EXEC` at its link
//!    addresses, a static PIE (`ET_DYN`) at `poly_elf::DEFAULT_DYN_BASE`.
//! 3. One region per mapping, nothing copied: the file-backed part of a
//!    segment maps the file through a [`FileRef::window`] that ends at
//!    `p_offset + p_filesz`, so the rest of its last page reads as zero;
//!    the rest of `.bss` is anonymous memory. Pages come in on the first
//!    access.
//! 4. The program break starts at the page-aligned end of the highest
//!    segment.
//! 5. A stack region of [`INITIAL_STACK`] bytes (more if the arguments need
//!    it) below the top of user space, which grows on demand to the stack
//!    limit and keeps the guard gap of `poly_vm` free below it.
//! 6. The initial stack, as the System V AMD64 psABI §3.4.1 describes:
//!    `argc`, `argv`, `envp`, the auxiliary vector (`AT_PHDR`, `AT_PHENT`,
//!    `AT_PHNUM`, `AT_PAGESZ`, `AT_ENTRY`, `AT_RANDOM`, `AT_UID`/`EUID`/
//!    `GID`/`EGID`, `AT_SECURE`, `AT_EXECFN`, `AT_PLATFORM`, `AT_HWCAP`,
//!    `AT_CLKTCK`, `AT_BASE`, `AT_FLAGS`, `AT_SYSINFO_EHDR` for the vDSO
//!    mapped with its data page by `crate::vdso`, `AT_NULL`) and the strings, built
//!    by [`poly_elf::build_initial_stack`] and written into the stack pages.
//! 7. A thread at the entry point with `rsp` at `argc`; every other
//!    register is 0 (`rdx` 0: no function for `atexit`, as the psABI
//!    allows).
//!
//! The arguments and environment may take at most a quarter of the stack
//! limit, as execve(2) documents for Linux (`E2BIG`).
//!
//! What `execve(2)` does around this (replacing the calling process's
//! memory, ending its other threads, closing `O_CLOEXEC` files, resolving
//! `#!`) is the system call's: [`load`] only builds the new memory and
//! never touches a running process.

use super::memory::{FileRef, Layout, Memory, MemoryError, MemoryImage, PageSource, Placement, Prot};
use super::{Entry, SyscallHook, spawn_in};
use crate::arch;
use crate::process::Process;
use crate::thread::{SpawnError, ThreadId};
use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_elf::{
    AuxInfo, Credentials, Elf, LoadOptions, LoadPlan, PLATFORM_X86_64, StackError, StackLayout, StackParams, Warnings,
    build_initial_stack, initial_stack_size,
};

/// Size of the stack region exec creates (128 KiB), unless the initial
/// stack needs more. It grows on demand below that, up to the layout's
/// stack limit.
pub const INITIAL_STACK: u64 = 128 << 10;

/// Largest file [`load`] reads into memory to parse when its source is not
/// in memory already (1 GiB).
const MAX_FILE_COPY: u64 = 1 << 30;

/// What the new program gets besides its file.
#[derive(Clone, Copy, Debug)]
pub struct ExecParams<'a> {
    /// Argument strings, without their NULs; `argv[0]` is the program name
    /// by convention.
    pub argv: &'a [&'a [u8]],
    /// Environment strings (`NAME=value`), without their NULs.
    pub envp: &'a [&'a [u8]],
    /// The path the program was started by (`AT_EXECFN`).
    pub execfn: &'a [u8],
    /// User and group IDs (`AT_UID` ... `AT_EGID`; `AT_SECURE` when real
    /// and effective IDs differ).
    pub creds: Credentials,
}

impl<'a> ExecParams<'a> {
    /// Running as root (all IDs 0), the only user there is so far.
    pub fn new(argv: &'a [&'a [u8]], envp: &'a [&'a [u8]], execfn: &'a [u8]) -> ExecParams<'a> {
        ExecParams { argv, envp, execfn, creds: Credentials::default() }
    }
}

/// Why exec failed. Nothing of the new program exists then.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecError {
    /// Not a valid x86-64 ELF executable.
    Elf(poly_elf::Error),
    /// A dynamically linked executable (`PT_INTERP`): not supported yet.
    Interpreter,
    /// The arguments and environment are too large for the stack, or
    /// contain a NUL.
    Arguments(StackError),
    /// No memory for the address space, a region or the initial stack.
    Memory(MemoryError),
    /// Reading the file failed.
    Io,
    /// No thread could be started.
    Spawn(SpawnError),
}

impl ExecError {
    /// The Linux errno for `execve`, from `asm-generic/errno-base.h`:
    /// `ENOEXEC` (8) for a file that is not a loadable executable (and,
    /// for now, a dynamic one), `E2BIG` (7) for oversized arguments,
    /// `EINVAL` (22) for an argument with a NUL in it, `ENOMEM` (12), `EIO`
    /// (5), `EAGAIN` (11) when no thread can be created.
    pub const fn linux_errno(self) -> u64 {
        match self {
            ExecError::Elf(_) | ExecError::Interpreter => 8,
            ExecError::Arguments(StackError::InteriorNul) => 22,
            ExecError::Arguments(_) => 7,
            ExecError::Memory(_) => 12,
            ExecError::Io => 5,
            ExecError::Spawn(_) => 11,
        }
    }
}

impl From<MemoryError> for ExecError {
    fn from(e: MemoryError) -> ExecError {
        ExecError::Memory(e)
    }
}

/// Where things went in the new memory.
#[derive(Clone, Copy, Debug)]
pub struct ImageInfo {
    /// Entry point (`AT_ENTRY`).
    pub entry: u64,
    /// What was added to the link-time addresses (0 for `ET_EXEC`).
    pub bias: u64,
    /// Lowest and end address of the mapped image.
    pub start: u64,
    pub end: u64,
    /// Start of the program break.
    pub brk: u64,
    /// End of the stack region.
    pub stack_top: u64,
    /// Where `argc`, `argv`, `envp`, the auxiliary vector and the strings
    /// went; `stack.rsp` is the initial stack pointer.
    pub stack: StackLayout,
    /// W^X violations and the like that the plan allowed.
    pub warnings: Warnings,
}

/// A program ready to run: its memory and where its first thread starts.
#[derive(Debug)]
pub struct Image {
    pub memory: Arc<Memory>,
    pub entry: Entry,
    pub info: ImageInfo,
}

/// Build the memory of the static executable in `file` with its initial
/// stack (see the module docs), without starting it.
pub fn load(file: Arc<dyn PageSource>, params: &ExecParams<'_>) -> Result<Image, ExecError> {
    let copy;
    let bytes = match file.contents() {
        Some(bytes) => bytes,
        None => {
            copy = read_all(&*file)?;
            &copy[..]
        }
    };
    let elf = Elf::parse(bytes).map_err(ExecError::Elf)?;
    if elf.interp().is_some() {
        return Err(ExecError::Interpreter);
    }
    // About 1.6 KiB: off the 16 KiB kernel stack.
    let plan = Box::new(LoadPlan::new(&elf, &LoadOptions::DEFAULT).map_err(ExecError::Elf)?);

    let layout = Layout::default();
    let memory = Memory::new(layout)?;
    for m in plan.mappings() {
        let prot = vm_prot(m.prot);
        let at = Placement::FixedNoReplace(m.start);
        match m.file {
            Some(f) => {
                let window = FileRef::window(Arc::clone(&file), f.offset + f.len);
                memory.map_file(at, m.size(), prot, window, f.offset)?;
            }
            None => {
                memory.map_anonymous(at, m.size(), prot)?;
            }
        }
    }
    memory.init_brk(plan.brk())?;

    // The vDSO and its data page (ADR 0044), announced as AT_SYSINFO_EHDR.
    let sysinfo_ehdr = crate::vdso::map_into(&memory);
    let random = arch::user::random_bytes();
    let aux = AuxInfo { hwcap: arch::user::hwcap(), sysinfo_ehdr, ..AuxInfo::new(&plan, None, params.creds) };
    let stack = StackParams {
        argv: params.argv,
        envp: params.envp,
        execfn: params.execfn,
        platform: PLATFORM_X86_64,
        random: &random,
        aux,
        phdr_table: elf.program_header_table(),
        extra_aux: &[],
    };
    let size = initial_stack_size(&stack).ok_or(ExecError::Arguments(StackError::TooSmall))?;
    if size as u64 > layout.stack_max / 4 {
        return Err(ExecError::Arguments(StackError::TooSmall));
    }
    let region = poly_vm::page_up(size as u64).unwrap_or(u64::MAX).max(INITIAL_STACK);
    let stack_prot = if plan.stack_executable() { Prot::ALL } else { Prot::RW };
    memory.setup_stack(region, stack_prot)?;
    let mut buf = Vec::new();
    buf.try_reserve_exact(size).map_err(|_| ExecError::Memory(MemoryError::OutOfMemory))?;
    buf.resize(size, 0);
    let stack_top = layout.stack_top;
    let built = build_initial_stack(&mut buf, stack_top, &stack).map_err(ExecError::Arguments)?;
    let used = built.used(stack_top) as usize;
    memory.write(built.rsp, &buf[size - used..]).map_err(|_| ExecError::Memory(MemoryError::OutOfMemory))?;

    let info = ImageInfo {
        entry: plan.entry(),
        bias: plan.bias(),
        start: plan.start(),
        end: plan.end(),
        brk: plan.brk(),
        stack_top,
        stack: built,
        warnings: plan.warnings(),
    };
    let entry = Entry { rip: plan.entry(), rsp: built.rsp, args: [0; 3] };
    Ok(Image { memory: Arc::new(memory), entry, info })
}

/// [`load`] the executable in `file` and start its first thread in
/// `process`, its system calls handled by `hook` (`user::DEFAULT_HOOK` for
/// the process's personality).
pub fn exec(
    process: Arc<Process>,
    file: Arc<dyn PageSource>,
    params: &ExecParams<'_>,
    hook: SyscallHook,
) -> Result<ThreadId, ExecError> {
    let image = load(file, params)?;
    spawn_in(process, image.memory, image.entry, hook).map_err(ExecError::Spawn)
}

/// [`exec`] an executable held in kernel memory (until files come from the
/// VFS, and for programs built into the kernel).
pub fn exec_image(
    process: Arc<Process>,
    image: Arc<[u8]>,
    params: &ExecParams<'_>,
    hook: SyscallHook,
) -> Result<ThreadId, ExecError> {
    exec(process, Arc::new(MemoryImage(image)), params, hook)
}

/// The region protection for a segment's `p_flags`.
fn vm_prot(p: poly_elf::Prot) -> Prot {
    let mut prot = Prot::NONE;
    for (has, bit) in [(p.readable(), Prot::READ), (p.writable(), Prot::WRITE), (p.executable(), Prot::EXEC)] {
        if has {
            prot = prot | bit;
        }
    }
    prot
}

/// The whole of a source that is not in memory, to parse its headers.
fn read_all(file: &dyn PageSource) -> Result<Vec<u8>, ExecError> {
    let len = file.len();
    if len > MAX_FILE_COPY {
        return Err(ExecError::Memory(MemoryError::OutOfMemory));
    }
    let mut buf = Vec::new();
    buf.try_reserve_exact(len as usize).map_err(|_| ExecError::Memory(MemoryError::OutOfMemory))?;
    buf.resize(len as usize, 0);
    let mut done = 0;
    while done < buf.len() {
        match file.read_at(done as u64, &mut buf[done..]) {
            Ok(0) | Err(_) => return Err(ExecError::Io),
            Ok(n) => done += n,
        }
    }
    Ok(buf)
}

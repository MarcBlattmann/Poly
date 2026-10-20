//! Starting programs from the file system: [`exec_path`] (what `execve`
//! will build on), the first program (`poly.init=<path>`, [`run_init`]) and
//! the shell's `run`.
//!
//! [`exec_path`] opens the file through the VFS with the kernel's root
//! credentials (`Vfs::open_exec`: a regular file with execute permission),
//! decides the personality from its first bytes (`poly-binfmt`), and for a
//! static Linux ELF makes a Linux process (pid from `process::alloc_pid`,
//! the console on fds 0, 1 and 2) and starts it with `user::exec::load`
//! reading the file through a [`FileSource`]. Other formats are refused
//! with a reason: PE and Mach-O need their personalities' loaders, `#!`
//! scripts and dynamically linked ELF files (`PT_INTERP`) come with
//! `execve` and `ld.so` support (Phase 4).

use crate::boot_info::BootInfo;
use crate::process::{self, ExitStatus, Process};
use crate::user::exec::{self, ExecError, ExecParams};
use crate::user::{self, FileSource};
use crate::{fs, kprintln, qemu, testrun, thread};
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_abi::Personality;
use poly_vfs::{At, Cred};

/// Bytes read from the start of a file to decide its format.
const HEADER_BYTES: usize = 4096;

/// Why [`exec_path`] did not start a program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecPathError {
    /// Opening the file failed (`ENOENT`, `EACCES`: not a regular file or
    /// not executable, ...).
    Open(poly_vfs::Errno),
    /// The file is not something Poly can run yet.
    Unsupported(String),
    /// Loading or starting it failed.
    Exec(ExecError),
}

impl core::fmt::Display for ExecPathError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ExecPathError::Open(e) => f.write_str(e.name()),
            ExecPathError::Unsupported(why) => f.write_str(why),
            ExecPathError::Exec(ExecError::Interpreter) => {
                f.write_str("a dynamically linked executable (PT_INTERP): not supported yet")
            }
            ExecPathError::Exec(ExecError::Elf(e)) => write!(f, "not a loadable x86-64 ELF executable ({e:?})"),
            ExecPathError::Exec(e) => write!(f, "exec failed: {e:?}"),
        }
    }
}

/// Start the program at `path` with `argv` and `envp` as a new process
/// (see the module docs) and return it; its first thread is running.
/// `traced`: it reports what it does on the serial port (ADR 0027).
pub fn exec_path(path: &[u8], argv: &[&[u8]], envp: &[&[u8]], traced: bool) -> Result<Arc<Process>, ExecPathError> {
    prepare(path, argv, envp, traced)?.start()
}

/// A program loaded by [`prepare`], not started yet.
#[derive(Debug)]
pub struct Prepared {
    process: Arc<Process>,
    image: exec::Image,
}

impl Prepared {
    /// Start the program's first thread; returns its process.
    pub fn start(self) -> Result<Arc<Process>, ExecPathError> {
        let tid = user::spawn_in(Arc::clone(&self.process), self.image.memory, self.image.entry, user::DEFAULT_HOOK)
            .map_err(|e| ExecPathError::Exec(ExecError::Spawn(e)))?;
        // Nobody joins the thread: the process is what callers wait for.
        let _ = thread::detach(tid);
        Ok(self.process)
    }
}

/// Everything [`exec_path`] does but starting the thread.
pub fn prepare(path: &[u8], argv: &[&[u8]], envp: &[&[u8]], traced: bool) -> Result<Prepared, ExecPathError> {
    let vfs = fs::vfs();
    let ctx = vfs.root_context().map_err(ExecPathError::Open)?;
    let file = vfs.open_exec(&ctx, &Cred::root(), At::Cwd, path).map_err(ExecPathError::Open)?;
    drop(ctx);
    let mut header = alloc::vec![0u8; HEADER_BYTES];
    let mut len = 0;
    while len < header.len() {
        match file.read_at(len as u64, &mut header[len..]) {
            Ok(0) => break,
            Ok(n) => len += n,
            Err(e) => return Err(ExecPathError::Open(e)),
        }
    }
    let format = poly_binfmt::detect(&header[..len])
        .map_err(|_| ExecPathError::Unsupported("not an executable Poly knows (no ELF, PE or Mach-O header)".into()))?;
    match format.personality() {
        Some(Personality::Linux) => {}
        Some(p) => return Err(ExecPathError::Unsupported(format!("{} programs cannot run yet", p.name()))),
        None => return Err(ExecPathError::Unsupported("#! scripts are not supported yet".into())),
    }
    let source = FileSource::new(file).ok_or(ExecPathError::Open(poly_vfs::Errno::EACCES))?;
    let image = exec::load(Arc::new(source), &ExecParams::new(argv, envp, path)).map_err(ExecPathError::Exec)?;
    let process = Arc::new(Process::new_linux(process::alloc_pid(), 0, format, traced));
    Ok(Prepared { process, image })
}

/// `path` and arguments split from a command line (`run`, `poly.init`
/// takes only a path).
pub fn split_args(path: &str, args: &[String]) -> Vec<Vec<u8>> {
    let mut argv = Vec::with_capacity(args.len() + 1);
    argv.push(path.as_bytes().to_vec());
    argv.extend(args.iter().map(|a| a.as_bytes().to_vec()));
    argv
}

/// Run the program at `path` with `args` and wait for it to end.
pub fn run(path: &str, args: &[String], traced: bool) -> Result<ExitStatus, ExecPathError> {
    let argv = split_args(path, args);
    let argv: Vec<&[u8]> = argv.iter().map(Vec::as_slice).collect();
    let process = exec_path(path.as_bytes(), &argv, &[], traced)?;
    Ok(process.wait())
}

/// `poly.init=<path>`: run that program as the first process before
/// anything else (the self-test, the shell), with the records of ADR 0027
/// (`poly.test.capture` adds the syscall, output and signal records). With
/// `poly.qemu_exit` QEMU then exits with the success status, the
/// program's own status being in the `POLY-EXIT` record. Returns whether
/// `poly.init` was given.
pub fn run_init(boot: &BootInfo) -> bool {
    let Some(path) = boot.arg(testrun::INIT_ARG) else { return false };
    let traced = boot.has_flag(testrun::CAPTURE_FLAG);
    kprintln!("init: running {path}{}", if traced { " (traced)" } else { "" });
    match run_first(path, traced) {
        Ok((pid, status)) => {
            testrun::exit(pid, status);
            kprintln!("init: {path} (pid {pid}) ended: {status}");
        }
        Err(e) => {
            testrun::unsupported(&format!("{path}: {e}"));
            kprintln!("init: cannot run {path}: {e}");
        }
    }
    if boot.has_flag("poly.qemu_exit") {
        qemu::exit(qemu::ExitCode::Success);
    }
    true
}

fn run_first(path: &str, traced: bool) -> Result<(u32, ExitStatus), ExecPathError> {
    let argv: [&[u8]; 1] = [path.as_bytes()];
    let prepared = prepare(path.as_bytes(), &argv, &[], traced)?;
    // The record goes out before the program's first instruction runs.
    testrun::init(path);
    let process = prepared.start()?;
    Ok((process.pid, process.wait()))
}

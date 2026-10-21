//! Starting programs from the file system: [`exec_path`] (what `execve`
//! will build on), the first program (`poly.init=<path>`, [`run_init`]) and
//! the shell's `run`.
//!
//! [`exec_path`] opens the file through the VFS with the kernel's root
//! credentials (`Vfs::open_exec`: a regular file with execute permission),
//! decides the personality from its first bytes (`poly-binfmt`), and for a
//! Linux ELF makes a Linux process (pid from `process::alloc_pid`, the
//! console on fds 0, 1 and 2) and starts it with `user::exec::load_dynamic`
//! reading the file through a [`FileSource`]; a dynamically linked one gets
//! the interpreter its `PT_INTERP` names, opened the same way (ADR 0049).
//! Other formats are refused with a reason: PE and Mach-O need their
//! personalities' loaders; `#!` scripts are left to `execve` (a program can
//! run one).

use crate::boot_info::BootInfo;
use crate::fs::cache::FileSource;
use crate::process::{self, ExitStatus, Process};
use crate::user;
use crate::user::exec::{self, ExecError, ExecParams};
use crate::user::memory::PageSource;
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
            ExecPathError::Exec(ExecError::InterpreterOpen(errno)) => {
                let name = poly_vfs::Errno::from_code(i32::from(*errno)).map_or("error", poly_vfs::Errno::name);
                write!(f, "its interpreter (PT_INTERP) cannot be opened: {name}")
            }
            ExecPathError::Exec(ExecError::BadInterpreter(e)) => {
                write!(f, "its interpreter (PT_INTERP) is not a loadable ELF image ({e:?})")
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
pub enum Prepared {
    /// A Linux ELF: its process record and loaded image.
    Linux { process: Arc<Process>, image: exec::Image },
    /// A PE executable: its process record and bytes, started by `crate::nt`.
    Nt { process: Arc<Process>, file: Vec<u8> },
}

impl Prepared {
    /// The process record, already made (its files can be set up before
    /// [`start`](Self::start)).
    pub fn process(&self) -> &Arc<Process> {
        match self {
            Prepared::Linux { process, .. } | Prepared::Nt { process, .. } => process,
        }
    }

    /// Start the program's first thread; returns its process.
    pub fn start(self) -> Result<Arc<Process>, ExecPathError> {
        match self {
            Prepared::Linux { process, image } => {
                let tid = user::spawn_in(Arc::clone(&process), image.memory, image.entry, user::DEFAULT_HOOK)
                    .map_err(|e| ExecPathError::Exec(ExecError::Spawn(e)))?;
                // Nobody joins the thread: the process is what callers wait for.
                let _ = thread::detach(tid);
                Ok(process)
            }
            Prepared::Nt { process, file } => {
                crate::nt::spawn_pe(&process, &file, user::DEFAULT_HOOK)
                    .map_err(|e| ExecPathError::Unsupported(format!("PE executable could not start: {e:?}")))?;
                Ok(process)
            }
        }
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
        Some(Personality::Nt) => {
            // Read the whole image; the NT loader maps it as a section.
            let mut file_bytes = header;
            file_bytes.truncate(len);
            loop {
                let at = file_bytes.len();
                file_bytes.resize(at + HEADER_BYTES, 0);
                match file.read_at(at as u64, &mut file_bytes[at..]) {
                    Ok(0) => {
                        file_bytes.truncate(at);
                        break;
                    }
                    Ok(n) => file_bytes.truncate(at + n),
                    Err(e) => return Err(ExecPathError::Open(e)),
                }
            }
            let process = Arc::new(Process::new_nt(process::alloc_pid(), format, traced));
            return Ok(Prepared::Nt { process, file: file_bytes });
        }
        Some(p) => return Err(ExecPathError::Unsupported(format!("{} programs cannot run yet", p.name()))),
        None => return Err(ExecPathError::Unsupported("#! scripts are not supported yet".into())),
    }
    let source = FileSource::of(&file);
    let open_interp = |interp: &[u8]| -> Result<Arc<dyn PageSource>, ExecError> {
        let ctx = vfs.root_context().map_err(|e| ExecError::InterpreterOpen(e.code() as u16))?;
        let file = vfs.open_exec(&ctx, &Cred::root(), At::Cwd, interp);
        let file = file.map_err(|e| ExecError::InterpreterOpen(e.code() as u16))?;
        Ok(Arc::new(FileSource::of(&file)))
    };
    let params = ExecParams::new(argv, envp, path);
    let image = exec::load_dynamic(Arc::new(source), &params, &open_interp).map_err(ExecPathError::Exec)?;
    let process = Arc::new(Process::new_linux(process::alloc_pid(), 0, format, traced));
    if let Some(linux) = &process.linux {
        // The program's name (prctl(2) PR_GET_NAME); it runs as root.
        let comm = poly_personality_linux::sys::prctl::comm_from_path(path);
        let pid = process.pid as i32;
        linux.with(|p| p.thread_mut(pid).comm = comm);
    }
    fs::proc::record_exec(&process, &file, &image.memory, &image.info.stack, argv, envp);
    Ok(Prepared::Linux { process, image })
}

/// `path` and arguments split from a command line (`run`, `poly.init`
/// takes only a path).
pub fn split_args(path: &str, args: &[String]) -> Vec<Vec<u8>> {
    let mut argv = Vec::with_capacity(args.len() + 1);
    argv.push(path.as_bytes().to_vec());
    argv.extend(args.iter().map(|a| a.as_bytes().to_vec()));
    argv
}

/// Run the program at `path` with `args` and the environment `envp`
/// (`NAME=value` strings: the shell's `run NAME=value ... <program>`, for
/// `LD_DEBUG` and the like, ADR 0049) on the terminal `terminal` (its fds
/// 0, 1 and 2; `None`: the console) and wait for it to end: the shell's
/// `run`. The program leads a session of its own, so opening the terminal
/// makes it the program's controlling terminal (ADR 0046).
pub fn run_on(
    path: &str,
    args: &[String],
    envp: &[String],
    terminal: Option<&str>,
) -> Result<ExitStatus, ExecPathError> {
    let argv = split_args(path, args);
    let argv: Vec<&[u8]> = argv.iter().map(Vec::as_slice).collect();
    let envp: Vec<&[u8]> = envp.iter().map(String::as_bytes).collect();
    let prepared = prepare(path.as_bytes(), &argv, &envp, false)?;
    if let Some(tty) = terminal {
        let files = &prepared.process().files;
        for fd in 0..3 {
            let _ = files.close(fd);
        }
        let opened = files
            .open(tty.as_bytes(), poly_vfs::OpenFlags::RDWR, 0)
            .and_then(|_| files.dup(0))
            .and_then(|_| files.dup(0));
        if let Err(e) = opened {
            kprintln!("run: {tty}: {}", e.name());
        }
    }
    Ok(prepared.start()?.wait())
}

/// `poly.init=<path>`: run that program as the first process before
/// anything else (the self-test, the shell), with the records of ADR 0027
/// (`poly.test.capture` adds the syscall, output and signal records). With
/// `poly.qemu_exit` QEMU then exits with the success status, the
/// program's own status being in the `POLY-EXIT` record. Returns whether
/// `poly.init` was given.
pub fn run_init(boot: &BootInfo) -> bool {
    crate::linux::strace::configure(boot);
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

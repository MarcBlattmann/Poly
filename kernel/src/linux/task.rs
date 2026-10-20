//! Processes and threads of the Linux personality in the kernel (ADR
//! 0041): `clone` / `fork` / `vfork`, `execve`, the process tree with its
//! zombies, waiting for children, and what happens when a process ends.
//!
//! - **clone** ([`clone_task`]): a thread is another user thread in the
//!   caller's process and memory; a process gets a copy of the memory
//!   (`Memory::fork`, copy on write) or the same one (`CLONE_VM`), a copy
//!   of the open files and of the signal state (`LinuxProcess::fork`), and
//!   a node in the [`TREE`]. Either way the child starts with the caller's
//!   registers (rax 0, the new stack and TLS if given) and FPU state
//!   (`user::spawn_child`). The tid stores and `clear_child_tid` are done
//!   before it runs. A `CLONE_VFORK` caller then waits ([`vfork_release`]).
//! - **execve** ([`execve`]): open the file through the process's own
//!   directories, follow `#!` lines, load the ELF (`user::exec::load`);
//!   then end the other threads, take over the pid, swap the memory, close
//!   close-on-exec descriptors and reset what execve(2) resets.
//! - **The tree** is the personality's `ProcessTree` behind a spinlock
//!   ([`with_tree`]); a generation counter and a wait queue let `wait4`
//!   sleep until a child changes ([`wait_tree`]). Processes the tree keeps
//!   as zombies keep their record in [`ZOMBIES`] (so `kill` still finds
//!   them, as kill(2) says) until their parent reaps them.
//! - **The end of a process** ([`process_exit`], from `user::end_thread`
//!   once the last thread left): the personality updates the tree and
//!   signals the parent.
//!
//! Not done yet: `CLONE_FILES`, `CLONE_FS` and `CLONE_SIGHAND` between
//! *processes* (without `CLONE_THREAD`) copy instead of sharing, since a
//! process owns its files and signal state; threads share all three.
//! `CLONE_PTRACE`, `CLONE_IO`, `CLONE_SYSVSEM` have no effect. Resource
//! usage (`wait4`'s `rusage`) is all zero.

use super::services::vfs_errno;
use super::{LinuxState, Thread};
use crate::fs::cache::FileSource;
use crate::process::{self, ExitStatus, Process};
use crate::sync::SpinLock;
use crate::thread;
use crate::user::exec::{self, ExecParams};
use crate::user::{self, UserRegs};
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use poly_abi::Personality;
use poly_personality_linux::abi::SigAction;
use poly_personality_linux::abi::signal::{NSIG, SIG_DFL, SIG_IGN, SIGSEGV};
use poly_personality_linux::errno::Errno;
use poly_personality_linux::kernel::{LinuxKernel, Wakeup};
use poly_personality_linux::task::clone::CloneRequest;
use poly_personality_linux::task::exec::{ELOOP, ExecRequest, MAX_SCRIPT_DEPTH, parse_shebang, script_argv};
use poly_personality_linux::task::tree::{ProcessTree, Termination};

/// The process tree of every Linux process.
static TREE: SpinLock<ProcessTree> = SpinLock::new(ProcessTree::new());
/// The tree's generation as of the last change, readable without the lock.
static GENERATION: AtomicU64 = AtomicU64::new(0);
/// The records of processes that ended and wait for their parent, by pid.
static ZOMBIES: SpinLock<BTreeMap<u32, Arc<Process>>> = SpinLock::new(BTreeMap::new());
/// vfork children whose parent waits, by the child's thread id.
static VFORKS: SpinLock<BTreeMap<u32, Arc<AtomicBool>>> = SpinLock::new(BTreeMap::new());

/// Bytes read from the start of a file to decide what it is.
const HEADER_BYTES: usize = 4096;

/// Run `f` on the process tree, locked; then wake waiters if it changed and
/// let go of the records of the processes it reaped.
pub fn with_tree<R>(f: impl FnOnce(&mut ProcessTree) -> R) -> R {
    let (r, reaped, generation) = {
        let mut t = TREE.lock();
        let r = f(&mut t);
        (r, t.take_reaped(), t.generation())
    };
    if GENERATION.swap(generation, Ordering::SeqCst) != generation {
        process::wake_waiters();
    }
    if !reaped.is_empty() {
        let gone: Vec<Arc<Process>> = {
            let mut zombies = ZOMBIES.lock();
            reaped.iter().filter_map(|&pid| zombies.remove(&(pid as u32))).collect()
        };
        // The last reference to a process record may go here: its file
        // system context may take sleeping locks, so not under the lock.
        drop(gone);
    }
    r
}

/// A process the kernel started: the root of a tree of its own (its
/// parent is the kernel).
pub fn register_root(pid: u32) {
    with_tree(|t| t.insert_root(pid as i32, 0));
}

/// The record of zombie `pid`, if it is one.
pub fn zombie(pid: u32) -> Option<Arc<Process>> {
    ZOMBIES.lock().get(&pid).cloned()
}

/// The current parent of `process` (its creator's pid until that ended,
/// then init's or 0).
pub fn parent_of(process: &Process) -> u32 {
    TREE.lock().parent(process.pid as i32).map_or(process.ppid, |p| p as u32)
}

/// The process group and session of `pid` (its own pid if the tree does
/// not know it).
pub fn group_and_session(pid: u32) -> (i32, i32) {
    let t = TREE.lock();
    let pid = pid as i32;
    (t.pgid(pid).unwrap_or(pid), t.sid(pid).unwrap_or(pid))
}

/// `LinuxKernel::wait_tree`: block until the tree's generation moves, or
/// the thread has a signal to act on (or must end).
pub fn wait_tree(k: &Thread<'_>, generation: u64) -> Wakeup {
    let flags = &k.flags;
    let process = k.process;
    let interrupted =
        || flags.notified.load(Ordering::SeqCst) || flags.killed.load(Ordering::SeqCst) || process.is_exiting();
    // The process events queue: signal notifications, group exits and
    // `execve` ending threads wake it too.
    process::wait_event(|| GENERATION.load(Ordering::SeqCst) != generation || interrupted());
    if GENERATION.load(Ordering::SeqCst) != generation { Wakeup::Woken } else { Wakeup::Signal }
}

/// The personality state of a child process of `parent`'s thread
/// `parent_tid`: `LinuxProcess::fork`, and with `clear_handlers`
/// (`CLONE_CLEAR_SIGHAND`) every caught signal back at `SIG_DFL`.
fn child_state(parent: &LinuxState, parent_tid: u32, child: u32, clear_handlers: bool) -> LinuxState {
    let mut state = parent.with(|p| p.fork(parent_tid as i32, child as i32));
    if clear_handlers {
        for signal in 1..=NSIG {
            let a = state.actions.get(signal);
            if a.handler != SIG_DFL && a.handler != SIG_IGN {
                state.actions.set(signal, SigAction::default());
            }
        }
    }
    LinuxState { process: SpinLock::new(state) }
}

/// Register a vfork child (thread `tid`): its parent waits on the flag.
fn vfork_register(tid: u32) -> Arc<AtomicBool> {
    let done = Arc::new(AtomicBool::new(false));
    VFORKS.lock().insert(tid, Arc::clone(&done));
    done
}

/// Thread `tid` exec'd or ended: a parent waiting for it in vfork goes on.
pub fn vfork_release(tid: u32) {
    let done = VFORKS.lock().remove(&tid);
    if let Some(done) = done {
        done.store(true, Ordering::SeqCst);
        process::wake_waiters();
    }
}

/// `LinuxKernel::clone_task` (see the module docs).
pub fn clone_task(k: &mut Thread<'_>, req: &CloneRequest) -> Result<i32, Errno> {
    let process = user::current_process().ok_or(Errno::EAGAIN)?;
    let memory = user::current_memory().ok_or(Errno::ENOMEM)?;
    let hook = user::current_hook().ok_or(Errno::EAGAIN)?;
    let mut regs = user::registers();
    regs.rax = 0;
    if let Some(stack) = req.stack {
        regs.rsp = stack;
    }
    let (fpu, _) = user::fpu_image().unwrap_or_default();
    let fs_base = req.tls.unwrap_or_else(user::fs_base);
    let gs_base = user::gs_base();
    let clear_child_tid = req.clear_child_tid.unwrap_or_default();
    let tid = if req.thread() {
        let tid = process::alloc_pid();
        k.linux.with(|p| {
            p.add_thread(k.tid as i32, tid as i32, req.drops_altstack());
            p.thread_mut(tid as i32).clear_child_tid = clear_child_tid;
        });
        store_tids(k, req, tid, None);
        let done = req.vfork_wait().then(|| vfork_register(tid));
        let started = user::spawn_child(process, memory, &regs, &fpu, fs_base, gs_base, hook, Some(tid));
        if started.is_err() {
            k.linux.with(|p| p.remove_thread(tid as i32));
            VFORKS.lock().remove(&tid);
            return Err(Errno::EAGAIN);
        }
        start(started, done, k)?;
        tid
    } else {
        let pid = process::alloc_pid();
        let memory = if req.shares_memory() { memory } else { Arc::new(memory.fork().map_err(|_| Errno::ENOMEM)?) };
        let linux = child_state(k.linux, k.tid, pid, req.clears_handlers());
        linux.with(|p| p.thread_mut(pid as i32).clear_child_tid = clear_child_tid);
        let files = k.process.files.fork();
        let child = Process::new_linux_child(pid, k.process.pid, k.process.format, k.process.traced, files, linux);
        let me = k.process.pid as i32;
        let uid = k.credentials().uid;
        with_tree(|t| {
            let parent = if req.sibling() { t.parent(me).unwrap_or(0) } else { me };
            t.insert_child(me, parent, pid as i32, req.exit_signal, uid);
        });
        store_tids(k, req, pid, (!req.shares_memory()).then_some(&memory));
        let done = req.vfork_wait().then(|| vfork_register(pid));
        let started = user::spawn_child(Arc::new(child), memory, &regs, &fpu, fs_base, gs_base, hook, None);
        if started.is_err() {
            with_tree(|t| t.remove(pid as i32));
            VFORKS.lock().remove(&pid);
            return Err(Errno::EAGAIN);
        }
        start(started, done, k)?;
        pid
    };
    Ok(tid as i32)
}

/// The child is running: let it go, and for vfork wait until it exec'd or
/// ended (vfork(2): "until the child terminates ... or it makes a call to
/// execve(2)"), or this thread must end.
fn start(
    started: Result<(thread::ThreadId, u32), thread::SpawnError>,
    done: Option<Arc<AtomicBool>>,
    k: &Thread<'_>,
) -> Result<(), Errno> {
    let (id, _) = started.map_err(|_| Errno::EAGAIN)?;
    let _ = thread::detach(id);
    if let Some(done) = done {
        let flags = &k.flags;
        process::wait_event(|| {
            done.load(Ordering::SeqCst) || flags.killed.load(Ordering::SeqCst) || k.process.is_exiting()
        });
    }
    Ok(())
}

/// `CLONE_PARENT_SETTID` and `CLONE_CHILD_SETTID`: the child's id, as an
/// `int`, in the caller's memory and in the child's (`child_memory`, when
/// it is not the caller's). Failures are ignored (to be confirmed by the
/// diff runner).
fn store_tids(k: &mut Thread<'_>, req: &CloneRequest, tid: u32, child_memory: Option<&Arc<user::Memory>>) {
    let bytes = (tid as i32).to_le_bytes();
    if let Some(ptr) = req.parent_tid {
        let _ = k.copy_to_user(ptr, &bytes);
    }
    if let Some(ptr) = req.child_tid {
        match child_memory {
            Some(memory) => {
                let _ = memory.write(ptr.0, &bytes);
            }
            None => {
                let _ = k.copy_to_user(ptr, &bytes);
            }
        }
    }
}

/// How the personality's tree records an exit status.
fn termination(status: ExitStatus) -> Termination {
    match status {
        ExitStatus::Exited(code) => Termination::Exited(code),
        ExitStatus::Signaled { signal, core } => Termination::Killed { signal, core },
        // A Linux process's faults are signals; a bare fault reads as one.
        ExitStatus::Faulted(_) => Termination::Killed { signal: SIGSEGV, core: false },
    }
}

/// The last thread of Linux process `process` (the running thread) left it
/// with `status`: the personality updates the tree and tells the parent;
/// the record stays (for `kill` and the parent's `wait`) while the process
/// is a zombie.
pub fn process_exit(process: &Process, status: ExitStatus) {
    let Some(me) = user::current_process() else { return };
    let pid = process.pid;
    // Listed first, so a parent that reaps at once finds it to let go of.
    ZOMBIES.lock().insert(pid, me);
    // Terminals: hangups and orphaned process groups (ADR 0046).
    let jobs = crate::tty::before_exit(pid);
    if let Some(mut k) = Thread::current(process) {
        poly_personality_linux::task::process_exited(&mut k, termination(status));
        crate::tty::after_exit(&mut k, pid, jobs);
    }
    if !with_tree(|t| t.is_zombie(pid as i32)) {
        let gone = ZOMBIES.lock().remove(&pid);
        drop(gone);
    }
}

/// `LinuxKernel::execve` (see the module docs).
pub fn execve(k: &mut Thread<'_>, req: &ExecRequest<'_>) -> Result<(), Errno> {
    let mut path = req.path.to_vec();
    let mut argv = req.argv.to_vec();
    let mut dirfd = req.dirfd;
    let mut depth = 0;
    let file = loop {
        let file = k.process.files.open_exec_at(dirfd, &path).map_err(vfs_errno)?;
        let mut header = alloc::vec![0u8; HEADER_BYTES];
        let mut len = 0;
        while len < header.len() {
            match file.read_at(len as u64, &mut header[len..]) {
                Ok(0) => break,
                Ok(n) => len += n,
                Err(e) => return Err(vfs_errno(e)),
            }
        }
        let Some(script) = parse_shebang(&header[..len])? else {
            match poly_binfmt::detect(&header[..len]).map(|f| f.personality()) {
                Ok(Some(Personality::Linux)) => break file,
                _ => return Err(Errno::ENOEXEC),
            }
        };
        depth += 1;
        if depth > MAX_SCRIPT_DEPTH {
            return Err(ELOOP);
        }
        argv = script_argv(&script, &path, &argv);
        path = script.interpreter;
        // The interpreter's path is the system's, not relative to dirfd.
        dirfd = crate::fs::AT_FDCWD;
    };
    let argv_refs: Vec<&[u8]> = argv.iter().map(Vec::as_slice).collect();
    let envp_refs: Vec<&[u8]> = req.envp.iter().map(Vec::as_slice).collect();
    let params = ExecParams::new(&argv_refs, &envp_refs, &path);
    let image = exec::load(Arc::new(FileSource::of(&file)), &params)
        .map_err(|e| Errno::new(e.linux_errno() as u16).unwrap_or(Errno::ENOEXEC))?;
    drop(file);
    // The point of no return: the other threads go (execve(2): "All
    // threads other than the calling thread are destroyed").
    if !k.process.end_other_threads(k.tid, &k.flags) {
        // This thread is being ended itself; it never returns to user mode.
        return Err(Errno::EAGAIN);
    }
    let pid = k.process.pid;
    let old_tid = k.tid;
    if old_tid != pid {
        // "the calling thread ... takes the thread group leader's PID".
        k.process.rename_thread(old_tid, pid);
        user::set_current_tid(pid);
        k.linux.with(|p| {
            p.add_thread(old_tid as i32, pid as i32, false);
            p.remove_thread(old_tid as i32);
        });
        k.tid = pid;
    }
    k.linux.with(|p| p.exec(pid as i32));
    let old = user::replace_memory(image.memory);
    // Dropping the old memory may take file system locks: fine here.
    drop(old);
    k.process.files.close_on_exec();
    user::reset_fpu();
    let _ = user::set_fs_base(0);
    let _ = user::set_gs_base(0);
    // The psABI's initial process state: every register 0 but rsp (rdx 0:
    // no function for atexit) and rip at the entry point.
    let regs = UserRegs { rip: image.entry.rip, rsp: image.entry.rsp, rflags: 0x202, ..UserRegs::default() };
    if user::set_registers(&regs).is_ok() {
        k.replaced = true;
    }
    vfork_release(old_tid);
    vfork_release(pid);
    Ok(())
}

//! Processes and threads (ADR 0041): clone, fork, vfork, clone3, execve,
//! wait4, waitid, process groups and sessions, and the parent's `SIGCHLD`,
//! against the fake kernel with a process tree of its own.

extern crate std;

use super::fake::{FakeKernel, UNMAPPED, scratch, sys};
use crate::abi::SigAction;
use crate::abi::SigInfo;
use crate::abi::signal::*;
use crate::errno::Errno;
use crate::kernel::{LinuxKernel, UserPtr, Wakeup};
use crate::nr;
use crate::task::clone::*;
use crate::task::exec::ExecRequest;
use crate::task::tree::{ProcessTree, Termination};
use crate::task::wait::*;
use crate::task::{process_continued, process_exited, process_stopped};
use std::boxed::Box;
use std::collections::VecDeque;
use std::vec::Vec;

/// An exec the fake kernel saw: path, argv, envp.
pub type Exec = (Vec<u8>, Vec<Vec<u8>>, Vec<Vec<u8>>);
/// Something that happens in the tree while the caller waits.
pub type TreeStep = Box<dyn FnMut(&mut ProcessTree)>;

/// What the fake kernel keeps for these tests.
#[derive(Default)]
pub struct Tasks {
    pub tree: ProcessTree,
    /// Every clone request, in order; children get pids 200, 201, ...
    pub clones: Vec<CloneRequest>,
    /// Every exec: path, argv, envp.
    pub execs: Vec<Exec>,
    /// What execve fails with, if anything.
    pub exec_error: Option<Errno>,
    /// What happens in the tree while the caller waits: one step per
    /// `wait_tree`.
    pub steps: VecDeque<TreeStep>,
    pub waits: usize,
}

pub fn clone_task(k: &mut FakeKernel, req: &CloneRequest) -> Result<i32, Errno> {
    let pid = 200 + k.tasks.clones.len() as i32;
    k.tasks.clones.push(*req);
    if !req.thread() {
        let me = k.pid;
        let parent = if req.sibling() { k.tasks.tree.parent(me).unwrap_or(0) } else { me };
        k.tasks.tree.insert_child(me, parent, pid, req.exit_signal, k.creds.uid);
    }
    if let Some(ptr) = req.parent_tid {
        let _ = k.copy_to_user(ptr, &pid.to_le_bytes());
    }
    Ok(pid)
}

pub fn execve(k: &mut FakeKernel, req: &ExecRequest<'_>) -> Result<(), Errno> {
    k.tasks.execs.push((req.path.to_vec(), req.argv.to_vec(), req.envp.to_vec()));
    k.tasks.exec_error.map_or(Ok(()), Err)
}

pub fn wait_tree(k: &mut FakeKernel, generation: u64) -> Wakeup {
    k.tasks.waits += 1;
    loop {
        if k.notified.contains(&k.tid) {
            return Wakeup::Signal;
        }
        if k.tasks.tree.generation() != generation {
            return Wakeup::Woken;
        }
        let Some(mut step) = k.tasks.steps.pop_front() else { panic!("waiting with nothing left to happen") };
        step(&mut k.tasks.tree);
    }
}

/// The fake's process 100 as a child of init (1), with children 11 and 12
/// made by fork.
fn family() -> FakeKernel {
    let mut k = FakeKernel::new();
    k.tasks.tree.insert_root(1, 0);
    k.tasks.tree.insert_child(1, 1, 100, SIGCHLD, 1000);
    k.add_process(1, 0, 1, 1);
    for _ in 0..2 {
        sys(&mut k, nr::FORK, &[]).unwrap();
    }
    k
}

#[test]
fn fork_vfork_and_clone_reach_the_kernel() {
    let mut k = family();
    assert_eq!(k.tasks.clones, [CloneRequest::fork(), CloneRequest::fork()]);
    assert_eq!(k.tasks.tree.children(100), [200, 201]);
    assert_eq!(sys(&mut k, nr::VFORK, &[]), Ok(202));
    assert_eq!(k.tasks.clones[2], CloneRequest::vfork());
    // pthread_create's flags, with the child's tid stored for the parent.
    let flags = CLONE_VM
        | CLONE_FS
        | CLONE_FILES
        | CLONE_SIGHAND
        | CLONE_THREAD
        | CLONE_SYSVSEM
        | CLONE_SETTLS
        | CLONE_PARENT_SETTID
        | CLONE_CHILD_CLEARTID;
    let (ptid, ctid) = (scratch(0), scratch(8));
    assert_eq!(sys(&mut k, nr::CLONE, &[flags, 0x7000_0000, ptid, ctid, 0x7700_0000]), Ok(203));
    assert_eq!(k.peek_u32(ptid), 203);
    let r = k.tasks.clones[3];
    assert!(r.thread() && r.tls == Some(0x7700_0000) && r.clear_child_tid == Some(UserPtr(ctid)));
    assert_eq!(k.tasks.tree.children(100), [200, 201, 202], "threads are not children");
    assert_eq!(sys(&mut k, nr::CLONE, &[CLONE_THREAD, 0, 0, 0, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::CLONE, &[CLONE_SETTLS | 17, 0, 0, 0, 1 << 47]), Err(Errno::EPERM));
    assert_eq!(k.tasks.clones.len(), 4, "refused clones never reach the kernel");
}

#[test]
fn clone_parent_makes_a_sibling() {
    let mut k = family();
    assert_eq!(sys(&mut k, nr::CLONE, &[CLONE_PARENT | 17, 0, 0, 0, 0]), Ok(202));
    assert_eq!(k.tasks.tree.parent(202), Some(1));
}

#[test]
fn clone3_reads_its_structure() {
    let mut k = family();
    let args = scratch(0x100);
    let mut raw = [0u8; 96];
    raw[0..8].copy_from_slice(&(CLONE_VM | CLONE_VFORK).to_le_bytes());
    raw[32..40].copy_from_slice(&u64::from(SIGCHLD).to_le_bytes());
    k.poke(args, &raw);
    assert_eq!(sys(&mut k, nr::CLONE3, &[args, 64]), Ok(202));
    assert_eq!(k.tasks.clones[2], CloneRequest::vfork());
    assert_eq!(sys(&mut k, nr::CLONE3, &[args, 96]), Ok(203), "zero bytes past the known structure");
    k.poke(args + 90, &[1]);
    assert_eq!(sys(&mut k, nr::CLONE3, &[args, 96]), Err(Errno::E2BIG));
    assert_eq!(sys(&mut k, nr::CLONE3, &[args, 32]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::CLONE3, &[UNMAPPED, 64]), Err(Errno::EFAULT));
}

/// Put `strings` and a NULL-terminated pointer array for them at `at`;
/// returns the array's address.
fn put_strings(k: &mut FakeKernel, at: u64, strings: &[&[u8]]) -> u64 {
    let mut text = at + 0x100;
    let mut ptrs = Vec::new();
    for s in strings {
        k.poke(text, s);
        k.poke(text + s.len() as u64, &[0]);
        ptrs.extend_from_slice(&text.to_le_bytes());
        text += s.len() as u64 + 1;
    }
    ptrs.extend_from_slice(&0u64.to_le_bytes());
    k.poke(at, &ptrs);
    at
}

#[test]
fn execve_reads_path_arguments_and_environment() {
    let mut k = family();
    let path = scratch(0);
    k.poke(path, b"/bin/sh\0");
    let argv = put_strings(&mut k, scratch(0x1000), &[b"sh", b"-c", b"true"]);
    let envp = put_strings(&mut k, scratch(0x2000), &[b"HOME=/"]);
    assert_eq!(sys(&mut k, nr::EXECVE, &[path, argv, envp]), Ok(0));
    let (p, a, e) = &k.tasks.execs[0];
    assert_eq!(p, b"/bin/sh");
    assert_eq!(a, &[b"sh".to_vec(), b"-c".to_vec(), b"true".to_vec()]);
    assert_eq!(e, &[b"HOME=/".to_vec()]);
    // NULL lists are empty; the exec marks the process for setpgid.
    assert_eq!(sys(&mut k, nr::EXECVE, &[path, 0, 0]), Ok(0));
    assert!(k.tasks.execs[1].1.is_empty() && k.tasks.execs[1].2.is_empty());
    assert_eq!(k.tasks.tree.setpgid(1, 100, 0), Err(Errno::EACCES));
    // Errors in reading order, and the kernel's own.
    let n = k.tasks.execs.len();
    assert_eq!(sys(&mut k, nr::EXECVE, &[UNMAPPED, argv, envp]), Err(Errno::EFAULT));
    k.poke(scratch(0x3000), &[0]);
    assert_eq!(sys(&mut k, nr::EXECVE, &[scratch(0x3000), argv, envp]), Err(Errno::ENOENT));
    assert_eq!(sys(&mut k, nr::EXECVE, &[path, UNMAPPED, envp]), Err(Errno::EFAULT));
    let bad = put_strings(&mut k, scratch(0x4000), &[b"x"]);
    k.poke(bad + 8, &UNMAPPED.to_le_bytes());
    assert_eq!(sys(&mut k, nr::EXECVE, &[path, argv, bad]), Err(Errno::EFAULT), "no NULL where it should be");
    assert_eq!(k.tasks.execs.len(), n);
    k.tasks.exec_error = Some(Errno::ENOEXEC);
    assert_eq!(sys(&mut k, nr::EXECVE, &[path, argv, envp]), Err(Errno::ENOEXEC));
}

#[test]
fn execve_refuses_huge_arguments() {
    let mut k = family();
    let path = scratch(0);
    k.poke(path, b"/bin/sh\0");
    // A string of 33 pages without its NUL in reach.
    let big = 0x100_0000;
    k.map_range(big, 34 * 4096, super::fake::RW);
    k.poke(big, &std::vec![b'a'; 33 * 4096]);
    let argv = scratch(0x1000);
    k.poke(argv, &big.to_le_bytes());
    k.poke(argv + 8, &0u64.to_le_bytes());
    assert_eq!(sys(&mut k, nr::EXECVE, &[path, argv, 0]), Err(Errno::E2BIG));
    let mut long_path = std::vec![b'p'; 5000];
    long_path.push(0);
    k.poke(big, &long_path);
    assert_eq!(sys(&mut k, nr::EXECVE, &[big, 0, 0]), Err(Errno::ENAMETOOLONG));
}

#[test]
fn wait4_reaps_an_ended_child() {
    let mut k = family();
    assert_eq!(sys(&mut k, nr::WAIT4, &[u64::MAX, 0, WNOHANG as u64, 0]), Ok(0), "children, none ended");
    k.tasks.steps.push_back(Box::new(|t| {
        t.exit(201, Termination::Exited(42), false);
    }));
    let (status, usage) = (scratch(0), scratch(0x100));
    k.poke(usage, &[0xff; RUSAGE_SIZE]);
    assert_eq!(sys(&mut k, nr::WAIT4, &[u64::MAX, status, 0, usage]), Ok(201));
    assert_eq!(k.peek_u32(status), 42 << 8);
    assert_eq!(k.peek(usage, RUSAGE_SIZE), [0; RUSAGE_SIZE]);
    assert_eq!(k.tasks.waits, 1);
    assert!(!k.tasks.tree.contains(201), "reaped");
    assert_eq!(sys(&mut k, nr::WAIT4, &[201, 0, 0, 0]), Err(Errno::ECHILD));
    assert_eq!(sys(&mut k, nr::WAIT4, &[u64::MAX, 0, 0x10, 0]), Err(Errno::EINVAL));
    // A specific child, killed by a signal.
    k.tasks.tree.exit(200, Termination::Killed { signal: SIGKILL, core: false }, false);
    assert_eq!(sys(&mut k, nr::WAIT4, &[200, status, 0, 0]), Ok(200));
    assert_eq!(k.peek_u32(status), SIGKILL);
    assert_eq!(sys(&mut k, nr::WAIT4, &[u64::MAX, status, 0, 0]), Err(Errno::ECHILD));
}

#[test]
fn wait4_is_interrupted_by_signals() {
    let mut k = family();
    k.tasks.steps.push_back(Box::new(|_| {}));
    k.notified.insert(100);
    assert_eq!(sys(&mut k, nr::WAIT4, &[u64::MAX, 0, 0, 0]), Err(Errno::EINTR));
    let restart = k.thread_state().restart;
    assert!(restart.is_some(), "restartable with SA_RESTART");
}

#[test]
fn wait4_reports_stops_and_continues_when_asked() {
    let mut k = family();
    k.tasks.tree.stopped(200, SIGTSTP);
    let status = scratch(0);
    assert_eq!(sys(&mut k, nr::WAIT4, &[u64::MAX, status, WNOHANG as u64, 0]), Ok(0));
    assert_eq!(sys(&mut k, nr::WAIT4, &[u64::MAX, status, (WNOHANG | WUNTRACED) as u64, 0]), Ok(200));
    assert_eq!(k.peek_u32(status), (SIGTSTP << 8) | 0x7f);
    k.tasks.tree.continued(200);
    assert_eq!(sys(&mut k, nr::WAIT4, &[200, status, WCONTINUED as u64, 0]), Ok(200));
    assert_eq!(k.peek_u32(status), 0xffff);
}

#[test]
fn waitid_fills_siginfo_and_can_leave_the_child() {
    let mut k = family();
    let info = scratch(0);
    k.poke(info, &[0xff; SigInfo::SIZE]);
    assert_eq!(sys(&mut k, nr::WAITID, &[u64::from(P_ALL), 0, info, (WEXITED | WNOHANG) as u64, 0]), Ok(0));
    assert_eq!(k.peek(info, SigInfo::SIZE), [0; SigInfo::SIZE], "nothing to report: zeroed");
    k.tasks.tree.exit(201, Termination::Exited(7), false);
    let peek = (WEXITED | WNOWAIT) as u64;
    assert_eq!(sys(&mut k, nr::WAITID, &[u64::from(P_PID), 201, info, peek, 0]), Ok(0));
    let got = SigInfo::from_bytes(&k.peek(info, SigInfo::SIZE).try_into().unwrap());
    assert_eq!((got.signal(), got.code, got.pid(), got.uid()), (SIGCHLD, CLD_EXITED, 201, 1000));
    assert_eq!(i32::from_le_bytes(got.fields[8..12].try_into().unwrap()), 7);
    assert!(k.tasks.tree.contains(201), "WNOWAIT leaves the zombie");
    assert_eq!(sys(&mut k, nr::WAITID, &[u64::from(P_PID), 201, 0, WEXITED as u64, 0]), Ok(0));
    assert!(!k.tasks.tree.contains(201));
    assert_eq!(sys(&mut k, nr::WAITID, &[u64::from(P_ALL), 0, info, 0, 0]), Err(Errno::EINVAL));
}

#[test]
fn an_ended_child_signals_its_parent() {
    let mut k = family();
    // Child 200 ends: the parent (100) gets SIGCHLD with the details, if
    // it handles it (the default ignores it).
    let handler = SigAction { handler: 0x40_1000, flags: SA_RESTORER, restorer: 0x40_2000, mask: 0 };
    k.me().set_action(SIGCHLD, handler);
    k.add_process(200, 1000, 1, 1);
    k.as_thread(200, |k| process_exited(k, Termination::Exited(3)));
    let info = k.me().dequeue(100).expect("SIGCHLD queued");
    assert_eq!((info.signal(), info.code, info.pid()), (SIGCHLD, CLD_EXITED, 200));
    assert!(k.notified.contains(&100));
    assert!(k.tasks.tree.is_zombie(200));
    // With SIG_IGN, the next one is reaped at once and nothing is queued.
    k.me().set_action(SIGCHLD, SigAction { handler: SIG_IGN, ..SigAction::default() });
    k.add_process(201, 1000, 1, 1);
    k.as_thread(201, |k| process_exited(k, Termination::Killed { signal: SIGTERM, core: false }));
    assert!(!k.tasks.tree.contains(201));
    assert_eq!(k.me().dequeue(100), None);
}

#[test]
fn stops_reach_the_parent_unless_it_set_sa_nocldstop() {
    let mut k = family();
    let handler = SigAction { handler: 0x40_1000, flags: SA_RESTORER, restorer: 0x40_2000, mask: 0 };
    k.me().set_action(SIGCHLD, handler);
    k.add_process(200, 1000, 1, 1);
    k.as_thread(200, |k| process_stopped(k, SIGSTOP));
    let info = k.me().dequeue(100).expect("SIGCHLD for the stop");
    assert_eq!((info.code, i32::from_le_bytes(info.fields[8..12].try_into().unwrap())), (CLD_STOPPED, 19));
    process_continued(&mut k, 200);
    assert_eq!(k.me().dequeue(100).map(|i| i.code), Some(CLD_CONTINUED));
    k.me().set_action(SIGCHLD, SigAction { flags: SA_RESTORER | SA_NOCLDSTOP, ..handler });
    k.as_thread(200, |k| process_stopped(k, SIGSTOP));
    assert_eq!(k.me().dequeue(100), None);
    let status = scratch(0);
    assert_eq!(sys(&mut k, nr::WAIT4, &[200, status, WUNTRACED as u64, 0]), Ok(200), "waiters still see it");
}

#[test]
fn process_groups_and_sessions() {
    let mut k = family();
    assert_eq!(sys(&mut k, nr::GETPGID, &[0]), Ok(1));
    assert_eq!(sys(&mut k, nr::GETSID, &[200]), Ok(1));
    assert_eq!(sys(&mut k, nr::SETPGID, &[0, 0]), Ok(0));
    assert_eq!(sys(&mut k, nr::GETPGID, &[0]), Ok(100));
    assert_eq!(sys(&mut k, nr::SETPGID, &[200, 100]), Ok(0));
    assert_eq!(sys(&mut k, nr::SETPGID, &[1, 0]), Err(Errno::ESRCH));
    assert_eq!(sys(&mut k, nr::SETPGID, &[0, (-1i64) as u64]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::SETSID, &[]), Err(Errno::EPERM), "a group leader");
    assert_eq!(sys(&mut k, nr::GETPGID, &[999]), Err(Errno::ESRCH));
    assert_eq!(sys(&mut k, nr::GETPGRP, &[]), Ok(100), "the fake's own record");
    let mut k2 = family();
    k2.pid = 201;
    k2.tid = 201;
    k2.add_process(201, 1000, 1, 1);
    assert_eq!(sys(&mut k2, nr::SETSID, &[]), Ok(201));
    assert_eq!(sys(&mut k2, nr::GETSID, &[0]), Ok(201));
}

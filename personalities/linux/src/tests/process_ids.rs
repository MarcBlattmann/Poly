//! Process ids as the spec sheets of getpgid, getsid and getppid describe
//! them: a thread id answers for its process, and orphans go to the
//! nearest child subreaper (prctl(2) `PR_SET_CHILD_SUBREAPER`).

use super::fake::{DATA, FakeKernel, UNMAPPED, sys};
use crate::abi::signal::SIGCHLD;
use crate::errno::Errno;
use crate::nr;
use crate::sys::subreaper::{PR_GET_CHILD_SUBREAPER, PR_SET_CHILD_SUBREAPER};
use crate::task::tree::Termination;

/// init (1), the caller (100) in group 100 of session 1.
fn caller() -> FakeKernel {
    let mut k = FakeKernel::new();
    k.tasks.tree.insert_root(1, 0);
    k.tasks.tree.insert_child(1, 1, 100, SIGCHLD, 0);
    k.add_process(1, 0, 1, 1);
    k.tasks.tree.setpgid(100, 100, 0).unwrap();
    k
}

#[test]
fn a_thread_id_answers_for_its_process() {
    let mut k = caller();
    k.add_thread(101);
    assert_eq!(sys(&mut k, nr::GETPGID, &[101]), Ok(100));
    assert_eq!(sys(&mut k, nr::GETSID, &[101]), Ok(1));
    assert_eq!(sys(&mut k, nr::GETPGID, &[102]), Err(Errno::ESRCH));
    assert_eq!(sys(&mut k, nr::GETSID, &[(-1i64) as u64]), Err(Errno::ESRCH));
    assert_eq!(sys(&mut k, nr::GETPGID, &[(-5i64) as u64]), Err(Errno::ESRCH));
}

#[test]
fn prctl_marks_the_caller_a_child_subreaper() {
    let mut k = caller();
    let out = DATA + 0x100;
    let get = |k: &mut FakeKernel| {
        k.poke(out, &7i32.to_le_bytes());
        assert_eq!(sys(k, nr::PRCTL, &[PR_GET_CHILD_SUBREAPER as u64, out]), Ok(0));
        i32::from_le_bytes(k.peek(out, 4).try_into().unwrap())
    };
    assert_eq!(get(&mut k), 0);
    assert_eq!(sys(&mut k, nr::PRCTL, &[PR_SET_CHILD_SUBREAPER as u64, 5]), Ok(0), "any nonzero value");
    assert_eq!(get(&mut k), 1);
    assert!(k.tasks.tree.is_subreaper(100));
    // A grandchild orphaned below the caller comes to it, not to init.
    k.tasks.tree.insert_child(100, 100, 200, SIGCHLD, 0);
    k.tasks.tree.insert_child(200, 200, 300, SIGCHLD, 0);
    k.tasks.tree.exit(200, Termination::Exited(0), false);
    assert_eq!(k.tasks.tree.parent(300), Some(100));
    assert_eq!(sys(&mut k, nr::PRCTL, &[PR_SET_CHILD_SUBREAPER as u64, 0]), Ok(0));
    assert_eq!(get(&mut k), 0);
    assert_eq!(sys(&mut k, nr::PRCTL, &[PR_GET_CHILD_SUBREAPER as u64, UNMAPPED]), Err(Errno::EFAULT));
}

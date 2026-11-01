/* clone3 result fields: CLONE_PIDFD (0x1000) stores a new pidfd for the
 * child (an int, the lowest free descriptor, FD_CLOEXEC set) at the
 * address in args.pidfd; CLONE_PARENT_SETTID stores the child's id (an
 * int) at args.parent_tid in the parent; CLONE_CHILD_SETTID stores it at
 * args.child_tid in the child's memory (a copy, so the parent's word is
 * unchanged). The pidfd becomes readable (POLLIN) when the child exits,
 * and waitid(P_PIDFD, pidfd) reaps it. CLONE_VFORK suspends the parent
 * until the child exits.
 * poly-diff: syscalls=off (two processes run at once; their calls interleave by scheduling)
 */
#include "a_sys.h"

#define SYS_poll 7
#define SYS_clone3 435
#define CLONE_PIDFD 0x00001000
#define P_PIDFD 3
#define POLLIN 0x1

struct k_clone_args {
    u64 flags, pidfd, child_tid, parent_tid, exit_signal, stack, stack_size, tls, set_tid, set_tid_size, cgroup;
};
struct k_pollfd {
    i32 fd;
    short events, revents;
};

static i32 pidfd = -1, ptid = -1, ctid = -1;

int test_main(void) {
    a_block_sigchld();
    volatile i32 *sh = a_shared();
    struct k_clone_args a;
    memset(&a, 0, sizeof a);
    a.flags = CLONE_PIDFD | CLONE_PARENT_SETTID | CLONE_CHILD_SETTID;
    a.pidfd = (u64)&pidfd;
    a.parent_tid = (u64)&ptid;
    a.child_tid = (u64)&ctid;
    a.exit_signal = SIGCHLD;
    i64 pid = syscall2(SYS_clone3, &a, sizeof a);
    if (pid == 0) {
        sh[0] = ctid == (i32)syscall0(SYS_getpid);
        while (sh[1] == 0) a_relax();
        syscall1(SYS_exit, 5);
    }
    expect("pidfd_is_3", pidfd, 3);
    expect("pidfd_cloexec", syscall2(SYS_fcntl, pidfd, F_GETFD), FD_CLOEXEC);
    expect("parent_tid_written", ptid, pid);
    expect("child_tid_not_in_parent", ctid, -1);
    struct k_pollfd p = { pidfd, POLLIN, 0 };
    expect("not_readable_while_running", syscall3(SYS_poll, &p, 1, 0), 0);
    sh[1] = 1;
    p.revents = 0;
    expect("readable_after_exit", syscall3(SYS_poll, &p, 1, 5000), 1);
    expect("pollin", p.revents, POLLIN);
    struct a_siginfo si;
    memset(&si, 0, sizeof si);
    expect("waitid_pidfd", syscall5(SYS_waitid, P_PIDFD, pidfd, &si, WEXITED, 0), 0);
    expect("waitid_pid", si.si_pid, pid);
    expect("waitid_status", si.si_status, 5);
    expect("child_saw_child_tid", sh[0], 1);

    /* CLONE_VFORK: the parent resumes after the child's exit. */
    memset(&a, 0, sizeof a);
    a.flags = CLONE_VFORK;
    a.exit_signal = SIGCHLD;
    sh[2] = 0;
    pid = syscall2(SYS_clone3, &a, sizeof a);
    if (pid == 0) {
        a_spin_ticks(100000000UL);
        sh[2] = 1;
        syscall1(SYS_exit, 6);
    }
    expect("vfork_parent_after_child", sh[2], 1);
    i32 status = -1;
    syscall4(SYS_wait4, pid, &status, 0, 0);
    expect("vfork_child_status", A_WEXITSTATUS(status), 6);
    return finish();
}

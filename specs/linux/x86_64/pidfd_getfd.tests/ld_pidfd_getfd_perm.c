/* pidfd_getfd needs ptrace attach access to the target: a process that
 * switched to uid/gid 4242 holding a pidfd of a root process (inherited
 * across fork) gets EPERM, also for a targetfd the target does not have
 * open (the access check comes before the descriptor lookup); root itself
 * may take the descriptor.
 * poly-diff: needs=root (switches to another uid)
 */
#include "a_sys.h"

#define SYS_setgroups 116
#define SYS_setresuid 117
#define SYS_setresgid 119
#define SYS_pidfd_getfd 438
#define CLONE_PIDFD 0x1000

static volatile i32 done, stop;
static int child(void *unused) {
    (void)unused;
    while (!stop) a_relax();
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    i64 fd = a_open("/tmp/ld_pidfd_getfd_perm", O_RDWR | O_CREAT | O_TRUNC, 0600);
    expect("file_fd3", fd, 3);
    static i32 pidfd = -1;
    done = -1;
    i64 target = a_clone(CLONE_VM | CLONE_PIDFD | CLONE_CHILD_CLEARTID | SIGCHLD, a_stacks[0] + A_STACK_SIZE,
                         &pidfd, (i32 *)&done, 0, child, 0);
    check("clone", target > 0, target);

    i64 pid = a_fork_sync();
    if (pid == 0) {
        syscall2(SYS_setgroups, 0, 0);
        syscall3(SYS_setresgid, 4242, 4242, 4242);
        if (syscall3(SYS_setresuid, 4242, 4242, 4242) != 0) syscall1(SYS_exit, 9);
        expect("user_eperm", syscall3(SYS_pidfd_getfd, pidfd, 3, 0), -EPERM);
        expect("user_eperm_before_ebadf", syscall3(SYS_pidfd_getfd, pidfd, 40, 0), -EPERM);
        expect("user_einval_before_eperm", syscall3(SYS_pidfd_getfd, pidfd, 3, 1), -EINVAL);
        syscall1(SYS_exit, finish());
    }
    i32 status = -1;
    syscall4(SYS_wait4, pid, &status, 0, 0);
    expect("child_passed", status, 0);
    expect("root_may", syscall3(SYS_pidfd_getfd, pidfd, 3, 0), 5);

    stop = 1;
    a_spin_until(&done, 0, A_SPIN_LIMIT);
    syscall4(SYS_wait4, target, 0, 0, 0);
    a_unlink("/tmp/ld_pidfd_getfd_perm");
    return finish();
}

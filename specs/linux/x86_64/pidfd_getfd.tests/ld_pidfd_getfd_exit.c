/* pidfd_getfd on a process that has ended: once the target has exited
 * (a zombie not yet reaped) and after it has been reaped, the pidfd stays
 * a valid pidfd but pidfd_getfd is ESRCH (flags are still checked first:
 * EINVAL). The copy taken while the target lived stays usable.
 */
#include "a_sys.h"

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
    i64 fd = a_open("/tmp/ld_pidfd_getfd_exit", O_RDWR | O_CREAT | O_TRUNC, 0600);
    expect("file_fd3", fd, 3);
    static i32 pidfd = -1;
    done = -1;
    i64 pid = a_clone(CLONE_VM | CLONE_PIDFD | CLONE_CHILD_CLEARTID | SIGCHLD, a_stacks[0] + A_STACK_SIZE, &pidfd,
                      (i32 *)&done, 0, child, 0);
    check("clone", pid > 0, pid);
    i64 g = syscall3(SYS_pidfd_getfd, pidfd, 3, 0);
    expect("alive_copy_5", g, 5);
    stop = 1;
    a_spin_until(&done, 0, A_SPIN_LIMIT);
    expect("zombie_esrch", syscall3(SYS_pidfd_getfd, pidfd, 3, 0), -ESRCH);
    expect("zombie_stdout_esrch", syscall3(SYS_pidfd_getfd, pidfd, 1, 0), -ESRCH);
    expect("zombie_bad_flags_einval", syscall3(SYS_pidfd_getfd, pidfd, 3, 1), -EINVAL);
    expect("reap", syscall4(SYS_wait4, pid, 0, 0, 0), pid);
    expect("reaped_esrch", syscall3(SYS_pidfd_getfd, pidfd, 3, 0), -ESRCH);
    expect("reaped_target_not_open_esrch", syscall3(SYS_pidfd_getfd, pidfd, 40, 0), -ESRCH);
    expect("copy_still_works", a_write(g, "x", 1), 1);
    expect("pidfd_still_open", syscall2(SYS_fcntl, pidfd, F_GETFD), FD_CLOEXEC);
    a_unlink("/tmp/ld_pidfd_getfd_exit");
    return finish();
}

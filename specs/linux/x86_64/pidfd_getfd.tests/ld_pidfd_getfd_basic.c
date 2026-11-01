/* pidfd_getfd(pidfd, targetfd, 0) duplicates descriptor targetfd of the
 * process the pidfd refers to into the caller's table, like dup across
 * processes: it returns the lowest free descriptor, always with
 * FD_CLOEXEC, referring to the same open file description (same status
 * flags, shared file offset: a write through the copy moves the target's
 * offset). Closing the copy does not affect the target. The target here is
 * a CLONE_VM child with its own descriptor table, made with CLONE_PIDFD;
 * it opens a file at a descriptor the parent does not have.
 */
#include "a_sys.h"

#define SYS_pidfd_getfd 438
#define CLONE_PIDFD 0x1000 /* linux/sched.h */

static volatile i32 done, cmd;
static volatile i64 res;
static int child(void *unused) {
    (void)unused;
    for (;;) {
        while (!cmd) a_relax();
        if (cmd == 1) res = a_open("/tmp/ld_pidfd_getfd_basic", O_RDWR | O_APPEND, 0);
        if (cmd == 2) res = syscall3(SYS_lseek, 3, 0, SEEK_CUR);
        if (cmd == 3) res = syscall3(SYS_lseek, 4, 0, SEEK_CUR);
        if (cmd == 9) { cmd = 0; return 0; }
        cmd = 0;
    }
}
static i64 tell(int c) {
    cmd = c;
    while (cmd) a_relax();
    return res;
}

int test_main(void) {
    a_block_sigchld();
    i64 fd = a_open("/tmp/ld_pidfd_getfd_basic", O_RDWR | O_CREAT | O_TRUNC, 0600);
    expect("file_fd3", fd, 3);
    static i32 pidfd = -1;
    done = -1;
    i64 pid = a_clone(CLONE_VM | CLONE_PIDFD | CLONE_CHILD_CLEARTID | SIGCHLD, a_stacks[0] + A_STACK_SIZE, &pidfd,
                      (i32 *)&done, 0, child, 0);
    check("clone", pid > 0, pid);
    expect("pidfd_4", pidfd, 4);
    expect("child_opens_fd4", tell(1), 4);

    i64 g = syscall3(SYS_pidfd_getfd, pidfd, 3, 0);
    expect("dup_child_fd3_lowest_5", g, 5);
    expect("copy_cloexec", syscall2(SYS_fcntl, g, F_GETFD), FD_CLOEXEC);
    expect("copy_status_flags", syscall2(SYS_fcntl, g, F_GETFL), O_RDWR | O_LARGEFILE);
    expect("write_copy", a_write(g, "abcdef", 6), 6);
    expect("child_offset_moved", tell(2), 6);
    expect("own_fd3_offset_moved", syscall3(SYS_lseek, 3, 0, SEEK_CUR), 6);

    i64 h = syscall3(SYS_pidfd_getfd, pidfd, 4, 0);
    expect("dup_child_fd4_lowest_6", h, 6);
    expect("copy_append_flag", syscall2(SYS_fcntl, h, F_GETFL), O_RDWR | O_APPEND | O_LARGEFILE);
    expect("copy_cloexec_again", syscall2(SYS_fcntl, h, F_GETFD), FD_CLOEXEC);
    expect("seek_copy", syscall3(SYS_lseek, h, 2, SEEK_SET), 2);
    expect("child_fd4_offset_2", tell(3), 2);
    expect("close_copy", a_close(h), 0);
    expect("child_fd4_still_open", tell(3), 2);
    expect("again_lowest_6", syscall3(SYS_pidfd_getfd, pidfd, 4, 0), 6);
    expect("dup_stdout", syscall3(SYS_pidfd_getfd, pidfd, 1, 0), 7);

    tell(9);
    a_spin_until(&done, 0, A_SPIN_LIMIT);
    syscall4(SYS_wait4, pid, 0, 0, 0);
    a_unlink("/tmp/ld_pidfd_getfd_basic");
    return finish();
}

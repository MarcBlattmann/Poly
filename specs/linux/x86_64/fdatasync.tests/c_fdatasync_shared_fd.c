/* fdatasync through descriptors that share an open file: a dup (also at a
 * high number and after the original is closed) and a fork child's
 * inherited copy all return 0.
 * poly-diff: syscalls=off (a child process)
 */
#include "a_sys.h"

#define F "/tmp/c_fdatasync_shared_fd"

int test_main(void) {
    i64 fd = a_open(F, O_RDWR | O_CREAT | O_TRUNC, 0644);
    expect("write", a_write(fd, "abc", 3), 3);
    i64 d = syscall1(SYS_dup, fd);
    expect("dup", syscall1(SYS_fdatasync, d), 0);
    i64 h = syscall3(SYS_fcntl, fd, F_DUPFD, 500);
    expect("dupfd_500", h, 500);
    expect("high", syscall1(SYS_fdatasync, h), 0);
    a_close(fd);
    expect("original_closed", syscall1(SYS_fdatasync, fd), -EBADF);
    expect("dup_survives", syscall1(SYS_fdatasync, d), 0);
    i64 pid = syscall0(SYS_fork);
    if (pid == 0) {
        i64 r1 = syscall1(SYS_fdatasync, h);
        i64 r2 = syscall1(SYS_fdatasync, 499);
        exit_group(r1 == 0 && r2 == -EBADF ? 0 : 3);
    }
    i32 status = -1;
    expect("wait", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    check("child_ok", A_WIFEXITED(status) && A_WEXITSTATUS(status) == 0, status);
    expect("parent_after_child", syscall1(SYS_fdatasync, h), 0);
    a_close(d);
    a_close(h);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}

/* fork: the child inherits open descriptors sharing the open file (and
 * its offset) and their FD_CLOEXEC flags; its own closes and opens do not
 * affect the parent's table. (Child reports through shared memory, then
 * spins until killed.) */
#include "a_sys.h"

#define F "/tmp/a_fork_fds"

int test_main(void) {
    a_block_sigchld();
    volatile i32 *sh = a_shared();
    a_make_file(F, "0123456789", 0644);
    i64 fd = a_open(F, O_RDONLY, 0);
    i64 cfd = a_open(F, O_RDONLY | O_CLOEXEC, 0);
    i64 pid = syscall0(SYS_fork);
    if (pid == 0) {
        char buf[4];
        sh[1] = (i32)a_read(fd, buf, 4);
        sh[2] = (i32)syscall3(SYS_fcntl, cfd, F_GETFD, 0);
        sh[3] = (i32)a_close(cfd);
        sh[4] = (i32)a_open("/tmp", O_RDONLY | O_DIRECTORY, 0);
        sh[0] = 1;
        for (;;) a_relax();
    }
    a_spin_until(&sh[0], 1, A_SPIN_LIMIT);
    syscall2(SYS_kill, pid, SIGKILL);
    i32 status;
    expect("wait4", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    check("fork", pid > 0, pid);
    expect("child_read_inherited_fd", sh[1], 4);
    expect("offset_shared_with_child", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 4);
    expect("child_saw_cloexec_flag", sh[2], FD_CLOEXEC);
    expect("child_close_ok", sh[3], 0);
    expect("parent_fd_still_open", syscall3(SYS_fcntl, cfd, F_GETFD, 0), FD_CLOEXEC);
    expect("child_open_reused_its_number", sh[4], cfd);
    a_close(fd);
    a_close(cfd);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}

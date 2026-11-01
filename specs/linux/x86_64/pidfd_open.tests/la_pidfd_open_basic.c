/* pidfd_open(pid, 0) returns the lowest free descriptor referring to
 * process pid: open O_RDWR, always FD_CLOEXEC; fstat st_mode 0700 (no
 * type bits); link "anon_inode:[pidfd]". PIDFD_NONBLOCK (O_NONBLOCK)
 * adds O_NONBLOCK. The caller's own pid works. poll on it reports
 * nothing while the process runs and POLLIN once it has exited (also as
 * a zombie); pidfd_open of a zombie works; waitid(P_PIDFD, fd) reaps it;
 * after that the pid is ESRCH. The descriptor stays valid after the
 * process is gone.
 * poly-diff: syscalls=off (two processes run at once)
 */
#include "a_sys.h"

#define SYS_poll 7
#define SYS_pidfd_open 434
#define PIDFD_NONBLOCK O_NONBLOCK
#define P_PIDFD 3
#define POLLIN 0x1

struct k_pollfd {
    i32 fd;
    short events, revents;
};

static i64 poll1(i64 fd, i64 timeout) {
    struct k_pollfd p = { (i32)fd, POLLIN, 0 };
    i64 r = syscall3(SYS_poll, &p, 1, timeout);
    return r <= 0 ? r : p.revents;
}

int test_main(void) {
    a_block_sigchld();
    volatile i32 *sh = a_shared();
    i64 pid = syscall0(SYS_fork);
    if (pid == 0) {
        while (sh[0] == 0) a_relax();
        syscall1(SYS_exit, 4);
    }
    i64 fd = syscall2(SYS_pidfd_open, pid, 0);
    expect("fd_3", fd, 3);
    expect("getfl_rdwr", syscall2(SYS_fcntl, fd, F_GETFL), O_RDWR);
    expect("getfd_cloexec", syscall2(SYS_fcntl, fd, F_GETFD), FD_CLOEXEC);
    struct a_stat st;
    memset(&st, 0, sizeof st);
    syscall2(SYS_fstat, fd, &st);
    expect("mode_0700", st.st_mode, 0700);
    char link[64];
    i64 n = syscall4(SYS_readlinkat, AT_FDCWD, "/proc/self/fd/3", link, sizeof link - 1);
    link[n > 0 ? n : 0] = 0;
    check("link_text", str_eq(link, "anon_inode:[pidfd]"), n);

    i64 nb = syscall2(SYS_pidfd_open, pid, PIDFD_NONBLOCK);
    expect("nonblock_fd_4", nb, 4);
    expect("nonblock_getfl", syscall2(SYS_fcntl, nb, F_GETFL), O_RDWR | O_NONBLOCK);
    i64 self = syscall2(SYS_pidfd_open, syscall0(SYS_getpid), 0);
    expect("self_fd_5", self, 5);

    expect("running_not_readable", poll1(fd, 0), 0);
    sh[0] = 1;
    expect("exited_pollin", poll1(fd, 5000), POLLIN);
    i64 zfd = syscall2(SYS_pidfd_open, pid, 0);
    expect("zombie_fd_6", zfd, 6);
    expect("zombie_pollin", poll1(zfd, 0), POLLIN);

    struct a_siginfo si;
    memset(&si, 0, sizeof si);
    expect("waitid_pidfd", syscall5(SYS_waitid, P_PIDFD, fd, &si, WEXITED, 0), 0);
    expect("waitid_pid", si.si_pid, pid);
    expect("waitid_code", si.si_code, CLD_EXITED);
    expect("waitid_status", si.si_status, 4);
    expect("reaped_esrch", syscall2(SYS_pidfd_open, pid, 0), -ESRCH);
    expect("old_fd_still_open", syscall2(SYS_fcntl, fd, F_GETFD), FD_CLOEXEC);
    expect("waitid_again_echild", syscall5(SYS_waitid, P_PIDFD, fd, &si, WEXITED, 0), -ECHILD);
    return finish();
}

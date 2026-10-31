/* A descriptor of a process's /proc/<pid> directory (opened with
 * O_DIRECTORY) is accepted by pidfd_send_signal like a pidfd and
 * signals that process; other /proc directories (/proc itself,
 * /proc/<pid>/fd) are EBADF.
 * poly-diff: syscalls=names (the /proc path holds the pid, which differs between runs)
 */
#include "a_sys.h"

#define SYS_pidfd_send_signal 424
#define SYS_rt_sigtimedwait 128

static i64 pss(i64 fd, i64 sig) { return syscall4(SYS_pidfd_send_signal, fd, sig, 0, 0); }

int test_main(void) {
    u64 block = A_SIGBIT(SIGUSR1);
    syscall4(SYS_rt_sigprocmask, SIG_BLOCK, &block, 0, 8);
    char path[40];
    char *p = str_cpy(path, "/proc/");
    a_utoa(p, (u64)syscall0(SYS_getpid));
    i64 dir = a_open(path, O_RDONLY | O_DIRECTORY, 0);
    expect("proc_pid_dir_fd", dir, 3);
    expect("proc_pid_dir_signal", pss(dir, SIGUSR1), 0);
    u64 pending = 0;
    syscall2(SYS_rt_sigpending, &pending, 8);
    check("signal_pending", (pending & A_SIGBIT(SIGUSR1)) != 0, (i64)pending);
    u64 set = A_SIGBIT(SIGUSR1);
    struct a_siginfo si;
    expect("take_it", syscall4(SYS_rt_sigtimedwait, &set, &si, 0, 8), SIGUSR1);
    expect("proc_pid_dir_sig_0", pss(dir, 0), 0);

    i64 proc = a_open("/proc", O_RDONLY | O_DIRECTORY, 0);
    expect("proc_root_ebadf", pss(proc, 0), -EBADF);
    str_cpy(p + str_len(p), "/fd");
    i64 fddir = a_open(path, O_RDONLY | O_DIRECTORY, 0);
    expect("proc_pid_fd_dir_ebadf", pss(fddir, 0), -EBADF);
    return finish();
}

/* PR_GET_SECCOMP / PR_SET_SECCOMP, strict mode only. GET returns the
 * mode, 0 for a normal process. SET with mode 0 or 3 is EINVAL; mode 2
 * (filter) with a NULL program is EFAULT. SET mode 1 (strict) returns 0;
 * from then on only read, write, exit and rt_sigreturn are allowed: any
 * other system call (getpid, exit_group, prctl itself) kills the process
 * with SIGKILL at once, before it does anything. Each case runs in a
 * fork child.
 * poly-diff: syscalls=off (several processes)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_prctl 157
/* linux/prctl.h */
#define PR_GET_SECCOMP 21
#define PR_SET_SECCOMP 22
/* linux/seccomp.h */
#define SECCOMP_MODE_DISABLED 0
#define SECCOMP_MODE_STRICT 1
#define SECCOMP_MODE_FILTER 2

static i64 pr(i64 o, i64 a, i64 b, i64 c, i64 d) { return syscall5(SYS_prctl, o, a, b, c, d); }

static int pipefd[2];

/* Run case `which` in a child that enters strict mode; returns its wait
 * status. Results the child wants to report go through the pipe. */
static int strict_child(int which) {
    long c = a_fork_sync();
    if (c == 0) {
        i64 r = pr(PR_SET_SECCOMP, SECCOMP_MODE_STRICT, 0, 0, 0);
        char b = r == 0 ? 'y' : 'n';
        syscall3(SYS_write, pipefd[1], &b, 1);
        switch (which) {
        case 0: /* allowed calls only */
            syscall3(SYS_read, 999, &b, 1);
            syscall3(SYS_write, pipefd[1], "w", 1);
            syscall1(SYS_exit, 5);
            break;
        case 1:
            syscall0(SYS_getpid);
            break;
        case 2:
            syscall1(SYS_exit_group, 6);
            break;
        case 3:
            pr(PR_GET_SECCOMP, 0, 0, 0, 0);
            break;
        }
        /* Not reached if killed. */
        syscall3(SYS_write, pipefd[1], "!", 1);
        syscall1(SYS_exit, 9);
    }
    int st = -1;
    syscall4(SYS_wait4, c, &st, 0, 0);
    return st;
}

static int drain(char *buf, int n) {
    int fl = (int)syscall3(SYS_fcntl, pipefd[0], 3 /* F_GETFL */, 0);
    syscall3(SYS_fcntl, pipefd[0], 4 /* F_SETFL */, fl | O_NONBLOCK);
    i64 r = syscall3(SYS_read, pipefd[0], buf, n - 1);
    syscall3(SYS_fcntl, pipefd[0], 4, fl);
    if (r < 0) r = 0;
    buf[r] = 0;
    return (int)r;
}

int test_main(void) {
    expect("get: disabled", pr(PR_GET_SECCOMP, 0, 0, 0, 0), SECCOMP_MODE_DISABLED);
    expect("set mode 0", pr(PR_SET_SECCOMP, SECCOMP_MODE_DISABLED, 0, 0, 0), -EINVAL);
    expect("set mode 3", pr(PR_SET_SECCOMP, 3, 0, 0, 0), -EINVAL);
    expect("set filter NULL", pr(PR_SET_SECCOMP, SECCOMP_MODE_FILTER, 0, 0, 0), -EFAULT);
    expect("still disabled", pr(PR_GET_SECCOMP, 0, 0, 0, 0), SECCOMP_MODE_DISABLED);

    expect("pipe", syscall2(SYS_pipe2, pipefd, 0), 0);
    char buf[16];

    int st = strict_child(0);
    drain(buf, sizeof buf);
    check("allowed: read, write, exit", !A_WIFSIGNALED(st) && A_WEXITSTATUS(st) == 5, st);
    check("allowed: output", str_eq(buf, "yw"), buf[0]);

    st = strict_child(1);
    drain(buf, sizeof buf);
    check("getpid: SIGKILL", A_WIFSIGNALED(st) && A_WTERMSIG(st) == SIGKILL, st);
    check("getpid: nothing after", str_eq(buf, "y"), buf[0]);

    st = strict_child(2);
    drain(buf, sizeof buf);
    check("exit_group: SIGKILL", A_WIFSIGNALED(st) && A_WTERMSIG(st) == SIGKILL, st);

    st = strict_child(3);
    drain(buf, sizeof buf);
    check("prctl: SIGKILL", A_WIFSIGNALED(st) && A_WTERMSIG(st) == SIGKILL, st);
    check("prctl: nothing after", str_eq(buf, "y"), buf[0]);

    expect("parent unaffected", pr(PR_GET_SECCOMP, 0, 0, 0, 0), SECCOMP_MODE_DISABLED);
    return finish();
}

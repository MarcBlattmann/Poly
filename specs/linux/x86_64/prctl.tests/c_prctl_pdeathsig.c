/* PR_SET_PDEATHSIG / PR_GET_PDEATHSIG: the signal (1..64, or 0 for
 * none) a process gets when its parent dies. Other values, also with
 * upper bits set, are EINVAL and change nothing; arguments 3 to 5 are
 * ignored. PR_GET_PDEATHSIG stores an int at arg2 (EFAULT if it cannot)
 * and returns 0. Delivery: a grandchild sets SIGUSR2 (default action:
 * terminate), its parent exits, and the grandchild is killed by SIGUSR2;
 * the test process is a child subreaper, so it can wait for it. With 0
 * set, nothing is sent.
 * poly-diff: syscalls=off (three processes)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_prctl 157
/* linux/prctl.h */
#define PR_SET_PDEATHSIG 1
#define PR_GET_PDEATHSIG 2
#define PR_SET_CHILD_SUBREAPER 36

static i64 pr(i64 o, i64 a, i64 b, i64 c, i64 d) { return syscall5(SYS_prctl, o, a, b, c, d); }

static int get(void) {
    int v = -7;
    pr(PR_GET_PDEATHSIG, (i64)&v, 0, 0, 0);
    return v;
}

/* Fork a middle process that forks a grandchild; the grandchild sets
 * `sig` as its parent-death signal, the middle process then exits.
 * Returns the grandchild's wait status. */
static int orphan_status(int sig, i64 *gpid) {
    volatile i32 *sh = a_shared();
    sh[0] = 0;
    sh[1] = 0;
    sh[2] = 0;
    long mid = syscall0(SYS_fork);
    if (mid == 0) {
        long g = syscall0(SYS_fork);
        if (g == 0) {
            pr(PR_SET_PDEATHSIG, sig, 0, 0, 0);
            sh[1] = 1;
            /* Wait for the end of the parent, then a little longer. */
            a_spin_until(&sh[2], 1, 10000000000UL);
            a_spin_ticks(200000000UL);
            syscall1(SYS_exit, 42);
        }
        sh[0] = (i32)g;
        a_spin_until(&sh[1], 1, 10000000000UL);
        syscall1(SYS_exit, 0);
    }
    int st = -1;
    syscall4(SYS_wait4, mid, &st, 0, 0);
    sh[2] = 1;
    *gpid = sh[0];
    st = -1;
    i64 r = syscall4(SYS_wait4, sh[0], &st, 0, 0);
    return r == sh[0] ? st : -1000;
}

int test_main(void) {
    expect("initially 0: get", pr(PR_GET_PDEATHSIG, (i64) & (int){0}, 0, 0, 0), 0);
    expect("initially 0", get(), 0);
    expect("set 1", pr(PR_SET_PDEATHSIG, 1, 0, 0, 0), 0);
    expect("get 1", get(), 1);
    expect("set 64", pr(PR_SET_PDEATHSIG, 64, 0, 0, 0), 0);
    expect("get 64", get(), 64);
    expect("set 65", pr(PR_SET_PDEATHSIG, 65, 0, 0, 0), -EINVAL);
    expect("set -1", pr(PR_SET_PDEATHSIG, -1, 0, 0, 0), -EINVAL);
    expect("set 1<<32 | 9", pr(PR_SET_PDEATHSIG, (1L << 32) | 9, 0, 0, 0), -EINVAL);
    expect("failed sets keep 64", get(), 64);
    expect("set 9, extra args", pr(PR_SET_PDEATHSIG, 9, -1, -1, -1), 0);
    expect("get 9", get(), 9);
    expect("set 0", pr(PR_SET_PDEATHSIG, 0, 0, 0, 0), 0);
    expect("get 0", get(), 0);
    expect("get NULL", pr(PR_GET_PDEATHSIG, 0, 0, 0, 0), -EFAULT);
    expect("get read-only", pr(PR_GET_PDEATHSIG, (i64) "constant", 0, 0, 0), -EFAULT);

    expect("become subreaper", pr(PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0), 0);
    i64 g = 0;
    int st = orphan_status(SIGUSR2, &g);
    check("grandchild waited for", st != -1000, g);
    check("killed by SIGUSR2", A_WIFSIGNALED(st) && A_WTERMSIG(st) == SIGUSR2, st);
    st = orphan_status(0, &g);
    check("no signal: grandchild waited for", st != -1000, g);
    check("no signal: normal exit", !A_WIFSIGNALED(st) && A_WEXITSTATUS(st) == 42, st);
    return finish();
}

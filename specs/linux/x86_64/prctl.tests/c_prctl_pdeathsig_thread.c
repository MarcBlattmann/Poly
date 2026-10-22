/* The parent-death signal follows the thread that created the child,
 * not the process: a child forked by a second thread gets its signal
 * when that thread exits, although the process lives on. The process
 * can still wait for the child.
 * poly-diff: syscalls=off (two threads and a child)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_prctl 157
/* linux/prctl.h */
#define PR_SET_PDEATHSIG 1
#define PR_GET_PDEATHSIG 2

static i64 pr(i64 o, i64 a, i64 b, i64 c, i64 d) { return syscall5(SYS_prctl, o, a, b, c, d); }

static volatile i32 done;

static int thread_fn(void *arg) {
    (void)arg;
    volatile i32 *sh = a_shared();
    long c = syscall0(SYS_fork);
    if (c == 0) {
        int v = -1;
        i64 r = pr(PR_SET_PDEATHSIG, SIGUSR1, 0, 0, 0);
        pr(PR_GET_PDEATHSIG, (i64)&v, 0, 0, 0);
        sh[2] = (r == 0 && v == SIGUSR1) ? 1 : 2;
        sh[1] = 1;
        /* Long enough to be killed first; exits 42 if it never is. */
        a_spin_ticks(30000000000UL);
        syscall1(SYS_exit, 42);
    }
    sh[0] = (i32)c;
    a_spin_until(&sh[1], 1, 10000000000UL);
    return 0;
}

int test_main(void) {
    volatile i32 *sh = a_shared();
    done = -1;
    long t = a_clone(CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM |
                         CLONE_CHILD_CLEARTID,
                     a_stacks[0] + A_STACK_SIZE, 0, (i32 *)&done, 0, thread_fn, 0);
    check("thread created", t > 0, t);
    if (t <= 0) return finish();
    while (done) a_relax();
    expect("child set SIGUSR1", sh[2], 1);
    int st = -1;
    i64 r = syscall4(SYS_wait4, sh[0], &st, 0, 0);
    check("waited for child", r == sh[0], r);
    check("child killed by SIGUSR1", A_WIFSIGNALED(st) && A_WTERMSIG(st) == SIGUSR1, st);
    return finish();
}

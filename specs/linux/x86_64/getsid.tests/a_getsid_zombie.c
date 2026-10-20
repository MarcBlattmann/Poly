/* getsid of a zombie child still answers; once reaped, ESRCH. */
#include "a_sys.h"

static volatile i32 done;

static int child(void *arg) {
    (void)arg;
    syscall0(SYS_setsid);
    return 0;
}

/* The checks run in a child that leads a new session and process group
 * (a_in_session), so they do not depend on how the test was started. */
static int body(void) {
    a_block_sigchld();
    long pid = a_spawn(0, &done, child, 0);
    a_spin_until(&done, 0, A_SPIN_LIMIT);
    struct a_siginfo si;
    expect("peek", syscall5(SYS_waitid, P_PID, pid, &si, WEXITED | WNOWAIT, 0), 0);
    expect("zombie_session", syscall1(SYS_getsid, pid), pid);
    i32 st;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("reaped_esrch", syscall1(SYS_getsid, pid), -ESRCH);
    return finish();
}

int test_main(void) { return a_in_session(body); }

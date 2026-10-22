/* What a fork child inherits: name, dumpable, keepcaps, TSC mode, timer
 * slack, machine-check kill policy, no_new_privs and THP-disable are
 * copied; the parent-death signal and child-subreaper are not (0 in the
 * child). The child's default timer slack is the slack its parent had at
 * the fork: PR_SET_TIMERSLACK 0 in the child goes back to that value.
 * poly-diff: syscalls=off (two processes)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_prctl 157
/* linux/prctl.h */
#define PR_SET_PDEATHSIG 1
#define PR_GET_PDEATHSIG 2
#define PR_GET_DUMPABLE 3
#define PR_SET_DUMPABLE 4
#define PR_GET_KEEPCAPS 7
#define PR_SET_KEEPCAPS 8
#define PR_SET_NAME 15
#define PR_GET_NAME 16
#define PR_GET_TSC 25
#define PR_SET_TSC 26
#define PR_TSC_SIGSEGV 2
#define PR_SET_TIMERSLACK 29
#define PR_GET_TIMERSLACK 30
#define PR_MCE_KILL 33
#define PR_MCE_KILL_GET 34
#define PR_MCE_KILL_SET 1
#define PR_MCE_KILL_LATE 0
#define PR_SET_CHILD_SUBREAPER 36
#define PR_GET_CHILD_SUBREAPER 37
#define PR_SET_NO_NEW_PRIVS 38
#define PR_GET_NO_NEW_PRIVS 39
#define PR_SET_THP_DISABLE 41
#define PR_GET_THP_DISABLE 42

static i64 pr(i64 o, i64 a, i64 b, i64 c, i64 d) { return syscall5(SYS_prctl, o, a, b, c, d); }

static int child(void) {
    char name[17] = {0};
    int v;
    pr(PR_GET_NAME, (i64)name, 0, 0, 0);
    check("child: name inherited", str_eq(name, "forkparent"), name[0]);
    v = -1;
    expect("child: get pdeathsig", pr(PR_GET_PDEATHSIG, (i64)&v, 0, 0, 0), 0);
    expect("child: pdeathsig cleared", v, 0);
    expect("child: dumpable inherited", pr(PR_GET_DUMPABLE, 0, 0, 0, 0), 0);
    expect("child: keepcaps inherited", pr(PR_GET_KEEPCAPS, 0, 0, 0, 0), 1);
    v = -1;
    pr(PR_GET_TSC, (i64)&v, 0, 0, 0);
    expect("child: tsc mode inherited", v, PR_TSC_SIGSEGV);
    expect("child: timer slack inherited", pr(PR_GET_TIMERSLACK, 0, 0, 0, 0), 4321);
    expect("child: mce kill inherited", pr(PR_MCE_KILL_GET, 0, 0, 0, 0), PR_MCE_KILL_LATE);
    v = -1;
    pr(PR_GET_CHILD_SUBREAPER, (i64)&v, 0, 0, 0);
    expect("child: subreaper cleared", v, 0);
    expect("child: no_new_privs inherited", pr(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), 1);
    expect("child: thp disable inherited", pr(PR_GET_THP_DISABLE, 0, 0, 0, 0), 1);
    pr(PR_SET_TIMERSLACK, 99, 0, 0, 0);
    expect("child: reset slack", pr(PR_SET_TIMERSLACK, 0, 0, 0, 0), 0);
    expect("child: default is parent's slack at fork", pr(PR_GET_TIMERSLACK, 0, 0, 0, 0), 4321);
    return failed ? 1 : 0;
}

int test_main(void) {
    i64 rc = 0;
    rc |= pr(PR_SET_NAME, (i64) "forkparent", 0, 0, 0);
    rc |= pr(PR_SET_PDEATHSIG, SIGUSR1, 0, 0, 0);
    rc |= pr(PR_SET_DUMPABLE, 0, 0, 0, 0);
    rc |= pr(PR_SET_KEEPCAPS, 1, 0, 0, 0);
    rc |= pr(PR_SET_TSC, PR_TSC_SIGSEGV, 0, 0, 0);
    rc |= pr(PR_SET_TIMERSLACK, 4321, 0, 0, 0);
    rc |= pr(PR_MCE_KILL, PR_MCE_KILL_SET, PR_MCE_KILL_LATE, 0, 0);
    rc |= pr(PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0);
    rc |= pr(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
    rc |= pr(PR_SET_THP_DISABLE, 1, 0, 0, 0);
    expect("parent: settings made", rc, 0);

    long pid = a_fork_sync();
    if (pid == 0) exit_group(child());
    int status = -1;
    syscall4(SYS_wait4, pid, &status, 0, 0);
    expect("child checks passed", status, 0);

    int v = -1;
    pr(PR_GET_PDEATHSIG, (i64)&v, 0, 0, 0);
    expect("parent: pdeathsig kept", v, SIGUSR1);
    v = -1;
    pr(PR_GET_CHILD_SUBREAPER, (i64)&v, 0, 0, 0);
    expect("parent: subreaper kept", v, 1);
    pr(PR_SET_DUMPABLE, 1, 0, 0, 0);
    return finish();
}

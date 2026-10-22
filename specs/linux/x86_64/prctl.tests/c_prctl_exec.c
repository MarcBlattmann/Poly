/* What execve does to prctl settings (an ordinary program, no set-id
 * bits). The thread name becomes the last component of the path given
 * to execve, cut to 15 bytes. Dumpable goes back to 1 and keepcaps to
 * 0. The parent-death signal, child-subreaper, no_new_privs,
 * THP-disable, TSC mode, timer slack and machine-check kill policy are
 * kept. The program runs itself again with an extra argument.
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
#define PR_MCE_KILL_EARLY 1
#define PR_SET_CHILD_SUBREAPER 36
#define PR_GET_CHILD_SUBREAPER 37
#define PR_SET_NO_NEW_PRIVS 38
#define PR_GET_NO_NEW_PRIVS 39
#define PR_SET_THP_DISABLE 41
#define PR_GET_THP_DISABLE 42

static i64 pr(i64 o, i64 a, i64 b, i64 c, i64 d) { return syscall5(SYS_prctl, o, a, b, c, d); }

static int after_exec(void) {
    /* The expected name: the last path component, at most 15 bytes. */
    const char *path = a_self_path();
    const char *base = path;
    for (const char *p = path; *p; p++)
        if (*p == '/') base = p + 1;
    char want[16];
    int i = 0;
    for (; i < 15 && base[i]; i++) want[i] = base[i];
    want[i] = 0;
    char name[17] = {0};
    pr(PR_GET_NAME, (i64)name, 0, 0, 0);
    check("name is the program's file name", str_eq(name, want), name[0]);
    check("name is not the old one", !str_eq(name, "beforeexec"), 0);

    expect("dumpable reset to 1", pr(PR_GET_DUMPABLE, 0, 0, 0, 0), 1);
    expect("keepcaps reset to 0", pr(PR_GET_KEEPCAPS, 0, 0, 0, 0), 0);
    int v = -1;
    pr(PR_GET_PDEATHSIG, (i64)&v, 0, 0, 0);
    expect("pdeathsig kept", v, SIGUSR2);
    v = -1;
    pr(PR_GET_CHILD_SUBREAPER, (i64)&v, 0, 0, 0);
    expect("subreaper kept", v, 1);
    expect("no_new_privs kept", pr(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), 1);
    expect("thp disable kept", pr(PR_GET_THP_DISABLE, 0, 0, 0, 0), 1);
    v = -1;
    pr(PR_GET_TSC, (i64)&v, 0, 0, 0);
    expect("tsc mode kept", v, PR_TSC_SIGSEGV);
    expect("timer slack kept", pr(PR_GET_TIMERSLACK, 0, 0, 0, 0), 2468);
    expect("mce kill kept", pr(PR_MCE_KILL_GET, 0, 0, 0, 0), PR_MCE_KILL_EARLY);
    return finish();
}

int test_main(void) {
    if (a_argc() > 1) return after_exec();
    i64 rc = 0;
    rc |= pr(PR_SET_NAME, (i64) "beforeexec", 0, 0, 0);
    rc |= pr(PR_SET_PDEATHSIG, SIGUSR2, 0, 0, 0);
    rc |= pr(PR_SET_DUMPABLE, 0, 0, 0, 0);
    rc |= pr(PR_SET_KEEPCAPS, 1, 0, 0, 0);
    rc |= pr(PR_SET_TSC, PR_TSC_SIGSEGV, 0, 0, 0);
    rc |= pr(PR_SET_TIMERSLACK, 2468, 0, 0, 0);
    rc |= pr(PR_MCE_KILL, PR_MCE_KILL_SET, PR_MCE_KILL_EARLY, 0, 0);
    rc |= pr(PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0);
    rc |= pr(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
    rc |= pr(PR_SET_THP_DISABLE, 1, 0, 0, 0);
    expect("settings made", rc, 0);
    const char *path = a_self_path();
    char *argv[] = {(char *)path, "after-exec", 0};
    char *envp[] = {0};
    i64 r = syscall3(SYS_execve, path, argv, envp);
    expect("execve", r, 0);
    return finish();
}

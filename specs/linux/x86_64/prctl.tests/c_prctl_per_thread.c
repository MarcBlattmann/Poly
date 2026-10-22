/* Which prctl settings belong to a thread and which to the whole
 * process. A new thread starts with its creator's name. A thread
 * changing its name, timer slack, no_new_privs, keepcaps, TSC mode or
 * machine-check kill policy changes only its own; a thread changing
 * dumpable, THP-disable or child-subreaper changes them for the whole
 * process.
 * poly-diff: syscalls=off (two threads)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_prctl 157
/* linux/prctl.h */
#define PR_GET_DUMPABLE 3
#define PR_SET_DUMPABLE 4
#define PR_GET_KEEPCAPS 7
#define PR_SET_KEEPCAPS 8
#define PR_SET_NAME 15
#define PR_GET_NAME 16
#define PR_GET_TSC 25
#define PR_SET_TSC 26
#define PR_TSC_ENABLE 1
#define PR_TSC_SIGSEGV 2
#define PR_SET_TIMERSLACK 29
#define PR_GET_TIMERSLACK 30
#define PR_MCE_KILL 33
#define PR_MCE_KILL_GET 34
#define PR_MCE_KILL_SET 1
#define PR_MCE_KILL_EARLY 1
#define PR_MCE_KILL_DEFAULT 2
#define PR_SET_CHILD_SUBREAPER 36
#define PR_GET_CHILD_SUBREAPER 37
#define PR_SET_NO_NEW_PRIVS 38
#define PR_GET_NO_NEW_PRIVS 39
#define PR_SET_THP_DISABLE 41
#define PR_GET_THP_DISABLE 42

static i64 pr(i64 o, i64 a, i64 b, i64 c, i64 d) { return syscall5(SYS_prctl, o, a, b, c, d); }

static volatile i32 done;
static char thread_start_name[17];
static i64 thread_slack_start;
static i64 thread_rc;

static int thread_fn(void *arg) {
    (void)arg;
    pr(PR_GET_NAME, (i64)thread_start_name, 0, 0, 0);
    thread_slack_start = pr(PR_GET_TIMERSLACK, 0, 0, 0, 0);
    thread_rc |= pr(PR_SET_NAME, (i64) "worker", 0, 0, 0);
    thread_rc |= pr(PR_SET_TIMERSLACK, 12345, 0, 0, 0);
    thread_rc |= pr(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
    thread_rc |= pr(PR_SET_KEEPCAPS, 1, 0, 0, 0);
    thread_rc |= pr(PR_SET_TSC, PR_TSC_SIGSEGV, 0, 0, 0);
    thread_rc |= pr(PR_MCE_KILL, PR_MCE_KILL_SET, PR_MCE_KILL_EARLY, 0, 0);
    thread_rc |= pr(PR_SET_DUMPABLE, 0, 0, 0, 0);
    thread_rc |= pr(PR_SET_THP_DISABLE, 1, 0, 0, 0);
    thread_rc |= pr(PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0);
    return 0;
}

int test_main(void) {
    pr(PR_SET_NAME, (i64) "mainthread", 0, 0, 0);
    pr(PR_SET_TIMERSLACK, 777, 0, 0, 0);
    done = -1;
    long t = a_clone(CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM |
                         CLONE_CHILD_CLEARTID,
                     a_stacks[0] + A_STACK_SIZE, 0, (i32 *)&done, 0, thread_fn, 0);
    check("thread created", t > 0, t);
    if (t > 0)
        while (done) a_relax();
    expect("thread calls succeeded", thread_rc, 0);
    check("thread started with creator's name", str_eq(thread_start_name, "mainthread"), thread_start_name[0]);
    expect("thread started with creator's slack", thread_slack_start, 777);

    char name[17] = {0};
    pr(PR_GET_NAME, (i64)name, 0, 0, 0);
    check("name is per thread", str_eq(name, "mainthread"), name[0]);
    expect("timer slack is per thread", pr(PR_GET_TIMERSLACK, 0, 0, 0, 0), 777);
    expect("no_new_privs is per thread", pr(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), 0);
    expect("keepcaps is per thread", pr(PR_GET_KEEPCAPS, 0, 0, 0, 0), 0);
    int tsc = -1;
    pr(PR_GET_TSC, (i64)&tsc, 0, 0, 0);
    expect("tsc mode is per thread", tsc, PR_TSC_ENABLE);
    expect("mce kill is per thread", pr(PR_MCE_KILL_GET, 0, 0, 0, 0), PR_MCE_KILL_DEFAULT);
    expect("dumpable is per process", pr(PR_GET_DUMPABLE, 0, 0, 0, 0), 0);
    expect("thp disable is per process", pr(PR_GET_THP_DISABLE, 0, 0, 0, 0), 1);
    int sub = -1;
    pr(PR_GET_CHILD_SUBREAPER, (i64)&sub, 0, 0, 0);
    expect("subreaper is per process", sub, 1);
    pr(PR_SET_DUMPABLE, 1, 0, 0, 0);
    return finish();
}

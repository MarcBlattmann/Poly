/* Under the fair policies sched_setparam accepts only priority 0 and
 * changes nothing else: the policy, the nice value and
 * SCHED_RESET_ON_FORK stay (SCHED_BATCH with the flag and nice 3 is
 * still SCHED_BATCH | SCHED_RESET_ON_FORK at nice 3 afterwards); under
 * SCHED_IDLE too, and priority 1 is EINVAL there. In a child process.
 * (An unprivileged caller may not clear the flag, so it stays set.)
 * poly-diff: syscalls=off (a child process)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_getpriority 140
#define SYS_setpriority 141
#define SYS_sched_setparam 142
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145
/* linux/sched.h */
#define SCHED_OTHER 0
#define SCHED_BATCH 3
#define SCHED_IDLE 5
#define SCHED_RESET_ON_FORK 0x40000000
/* linux/resource.h */
#define PRIO_PROCESS 0

static i64 getsched(void) { return syscall1(SYS_sched_getscheduler, 0); }
static i64 setsched(i64 policy, int prio) {
    int p = prio;
    return syscall3(SYS_sched_setscheduler, 0, policy, &p);
}
static i64 setparam(i64 pid, int prio) {
    int p = prio;
    return syscall2(SYS_sched_setparam, pid, &p);
}

static void body(void) {
    i64 r = setparam(0, 0);
    check("other_0_ok", r == 0, r);
    r = syscall3(SYS_setpriority, PRIO_PROCESS, 0, 3);
    check("nice_3", r == 0, r);
    r = setsched(SCHED_BATCH | SCHED_RESET_ON_FORK, 0);
    check("batch_reset", r == 0, r);
    r = setparam(0, 0);
    check("batch_0_ok", r == 0, r);
    r = setparam(getpid(), 1);
    check("batch_1_einval", r == -EINVAL, r);
    r = getsched();
    check("policy_and_flag_kept", r == (SCHED_BATCH | SCHED_RESET_ON_FORK), r);
    r = syscall2(SYS_getpriority, PRIO_PROCESS, 0);
    check("nice_kept", r == 17, r);
    /* (keeping the flag: an unprivileged caller may not clear it) */
    r = setsched(SCHED_IDLE | SCHED_RESET_ON_FORK, 0);
    check("idle", r == 0, r);
    r = setparam(0, 0);
    check("idle_0_ok", r == 0, r);
    r = setparam(0, 1);
    check("idle_1_einval", r == -EINVAL, r);
    r = getsched();
    check("still_idle", r == (SCHED_IDLE | SCHED_RESET_ON_FORK), r);
}

int test_main(void) {
    i64 pid = fork();
    if (pid == 0) {
        int before = failed;
        body();
        exit_group(failed != before);
    }
    int code = wait_exit(pid);
    check("child_checks", code == 0, code);
    i64 r = getsched();
    check("parent_other", r == SCHED_OTHER, r);
    return finish();
}

/* Switching between the fair policies keeps the nice value: after
 * setpriority to nice 4, SCHED_OTHER -> SCHED_BATCH -> SCHED_IDLE all
 * leave getpriority at 16 (20 - nice); under SCHED_IDLE the nice value
 * can still be raised (6 -> 14). Setting the policy a task already has
 * succeeds. Leaving SCHED_IDLE may need privilege, so each sequence
 * runs in a child.
 * poly-diff: syscalls=off (child processes)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_getpriority 140
#define SYS_setpriority 141
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145
/* linux/sched.h */
#define SCHED_OTHER 0
#define SCHED_BATCH 3
#define SCHED_IDLE 5
/* linux/resource.h */
#define PRIO_PROCESS 0

static i64 getprio(i64 which, i64 who) { return syscall2(SYS_getpriority, which, who); }
static i64 setprio(i64 which, i64 who, i64 nice) { return syscall3(SYS_setpriority, which, who, nice); }
static i64 getsched(i64 pid) { return syscall1(SYS_sched_getscheduler, pid); }
static i64 setsched(i64 pid, i64 policy, int prio) {
    int p = prio;
    return syscall3(SYS_sched_setscheduler, pid, policy, &p);
}
static void in_child(const char *name, void (*fn)(void)) {
    i64 pid = fork();
    if (pid == 0) {
        int before = failed;
        fn();
        exit_group(failed != before);
    }
    int code = wait_exit(pid);
    check(name, code == 0, code);
}

static void sequence(void) {
    i64 r = setprio(PRIO_PROCESS, 0, 4);
    check("nice_4", r == 0, r);
    r = setsched(0, SCHED_OTHER, 0);
    check("other_again_ok", r == 0, r);
    r = getprio(PRIO_PROCESS, 0);
    check("other_keeps_nice", r == 16, r);
    r = setsched(0, SCHED_BATCH, 0);
    check("batch_ok", r == 0, r);
    r = getprio(PRIO_PROCESS, 0);
    check("batch_keeps_nice", r == 16, r);
    r = setsched(0, SCHED_BATCH, 0);
    check("batch_again_ok", r == 0, r);
    r = setsched(0, SCHED_IDLE, 0);
    check("idle_ok", r == 0, r);
    r = getprio(PRIO_PROCESS, 0);
    check("idle_keeps_nice", r == 16, r);
    r = setsched(0, SCHED_IDLE, 0);
    check("idle_again_ok", r == 0, r);
    r = setprio(PRIO_PROCESS, 0, 6);
    check("nice_6_under_idle", r == 0, r);
    r = getprio(PRIO_PROCESS, 0);
    check("idle_nice_6", r == 14, r);
    r = getsched(0);
    check("still_idle", r == SCHED_IDLE, r);
}

int test_main(void) {
    i64 r = getprio(PRIO_PROCESS, 0);
    check("start_nice_0", r == 20, r);
    in_child("sequence", sequence);
    r = getprio(PRIO_PROCESS, 0);
    check("parent_nice_0", r == 20, r);
    r = getsched(0);
    check("parent_other", r == SCHED_OTHER, r);
    return finish();
}

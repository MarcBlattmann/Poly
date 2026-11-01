/* The policies any caller may choose, as sched_getscheduler reports
 * them: SCHED_BATCH (3), SCHED_IDLE (5), and SCHED_RESET_ON_FORK
 * (0x40000000) or-ed into the result while that flag is set. Leaving
 * SCHED_IDLE and clearing the flag may need privilege, so each step runs
 * in its own child process. In a fork child the flag is clear while the
 * (non-real-time) policy is kept.
 * poly-diff: syscalls=off (child processes)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145
/* linux/sched.h */
#define SCHED_OTHER 0
#define SCHED_BATCH 3
#define SCHED_IDLE 5
#define SCHED_RESET_ON_FORK 0x40000000

static i64 getsched(i64 pid) { return syscall1(SYS_sched_getscheduler, pid); }
static i64 setsched(i64 pid, i64 policy, int prio) {
    int p = prio;
    return syscall3(SYS_sched_setscheduler, pid, policy, &p);
}
/* Run fn in a child process; the check passes when all its checks did. */
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

static void idle(void) {
    i64 r = setsched(0, SCHED_IDLE, 0);
    check("set_idle", r == 0, r);
    r = getsched(0);
    check("is_idle", r == SCHED_IDLE, r);
}

static void reset_flag(void) {
    i64 r = setsched(0, SCHED_OTHER | SCHED_RESET_ON_FORK, 0);
    check("set_other_reset", r == 0, r);
    r = getsched(0);
    check("is_other_reset", r == (SCHED_OTHER | SCHED_RESET_ON_FORK), r);
    r = setsched(0, SCHED_BATCH | SCHED_RESET_ON_FORK, 0);
    check("set_batch_reset", r == 0, r);
    r = getsched(0);
    check("is_batch_reset", r == (SCHED_BATCH | SCHED_RESET_ON_FORK), r);
    i64 pid = fork();
    if (pid == 0) exit_group(getsched(0) == SCHED_BATCH ? 0 : 1);
    check("fork_child_batch_without_flag", wait_exit(pid) == 0, 0);
}

static void idle_reset(void) {
    i64 r = setsched(0, SCHED_IDLE | SCHED_RESET_ON_FORK, 0);
    check("set_idle_reset", r == 0, r);
    r = getsched(0);
    check("is_idle_reset", r == (SCHED_IDLE | SCHED_RESET_ON_FORK), r);
    i64 pid = fork();
    if (pid == 0) exit_group(getsched(0) == SCHED_IDLE ? 0 : 1);
    check("fork_child_idle_without_flag", wait_exit(pid) == 0, 0);
}

int test_main(void) {
    i64 r = setsched(0, SCHED_BATCH, 0);
    check("set_batch", r == 0, r);
    r = getsched(0);
    check("is_batch", r == SCHED_BATCH, r);
    r = setsched(0, SCHED_OTHER, 0);
    check("set_other", r == 0, r);
    in_child("idle_child", idle);
    in_child("reset_flag_child", reset_flag);
    in_child("idle_reset_child", idle_reset);
    r = getsched(0);
    check("parent_still_other", r == SCHED_OTHER, r);
    return finish();
}

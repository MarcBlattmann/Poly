/* What a caller without CAP_SYS_NICE may do with its own policy, with
 * RLIMIT_RTPRIO and RLIMIT_NICE both 0 (a child that drops to uid/gid
 * 65534 after setting those limits):
 * - SCHED_FIFO/SCHED_RR at any valid priority: EPERM (an invalid
 *   priority is still EINVAL: the range check comes first);
 * - SCHED_OTHER, SCHED_BATCH, SCHED_IDLE, with or without
 *   SCHED_RESET_ON_FORK: allowed;
 * - clearing SCHED_RESET_ON_FORK once set: EPERM (sched_setparam, which
 *   keeps the flag, is allowed);
 * - leaving SCHED_IDLE for SCHED_OTHER or SCHED_BATCH: EPERM, since nice
 *   0 is beyond RLIMIT_NICE 0; a failed call changes nothing.
 * poly-diff: needs=root (sets resource limits and drops to another user)
 * poly-diff: syscalls=off (child processes)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_sched_setparam 142
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145
#define SYS_setrlimit 160
/* linux/sched.h */
#define SCHED_OTHER 0
#define SCHED_FIFO 1
#define SCHED_RR 2
#define SCHED_BATCH 3
#define SCHED_IDLE 5
#define SCHED_RESET_ON_FORK 0x40000000
/* asm-generic/resource.h */
#define RLIMIT_NICE 13
#define RLIMIT_RTPRIO 14

static i64 getsched(i64 pid) { return syscall1(SYS_sched_getscheduler, pid); }
static i64 setsched(i64 pid, i64 policy, int prio) {
    int p = prio;
    return syscall3(SYS_sched_setscheduler, pid, policy, &p);
}
static i64 setparam(i64 pid, int prio) {
    int p = prio;
    return syscall2(SYS_sched_setparam, pid, &p);
}
static i64 set_rlimit(int res, u64 cur, u64 max) {
    u64 r[2] = { cur, max };
    return syscall2(SYS_setrlimit, res, r);
}
static i64 become(unsigned id) {
    i64 r = syscall2(SYS_setgroups, 0, 0);
    if (r == 0) r = syscall3(SYS_setresgid, id, id, id);
    if (r == 0) r = syscall3(SYS_setresuid, id, id, id);
    return r;
}
static void in_child(const char *name, void (*fn)(void)) {
    i64 pid = fork();
    if (pid == 0) {
        int before = failed;
        i64 r = set_rlimit(RLIMIT_NICE, 0, 0);
        if (r == 0) r = set_rlimit(RLIMIT_RTPRIO, 0, 0);
        if (r == 0) r = become(65534);
        check("dropped", r == 0, r);
        if (r == 0) fn();
        exit_group(failed != before);
    }
    int code = wait_exit(pid);
    check(name, code == 0, code);
}

static void realtime(void) {
    i64 r = setsched(0, SCHED_FIFO, 1);
    check("fifo_eperm", r == -EPERM, r);
    r = setsched(0, SCHED_RR, 99);
    check("rr_eperm", r == -EPERM, r);
    r = setsched(0, SCHED_RR | SCHED_RESET_ON_FORK, 10);
    check("rr_reset_eperm", r == -EPERM, r);
    r = setsched(0, SCHED_FIFO, 0);
    check("fifo_0_einval_first", r == -EINVAL, r);
    r = setsched(0, SCHED_RR, 100);
    check("rr_100_einval_first", r == -EINVAL, r);
    r = getsched(0);
    check("still_other", r == SCHED_OTHER, r);
}

static void reset_flag(void) {
    i64 r = setsched(0, SCHED_OTHER | SCHED_RESET_ON_FORK, 0);
    check("set_flag_ok", r == 0, r);
    r = setparam(0, 0);
    check("setparam_keeps_flag_ok", r == 0, r);
    r = getsched(0);
    check("flag_kept", r == (SCHED_OTHER | SCHED_RESET_ON_FORK), r);
    r = setsched(0, SCHED_OTHER, 0);
    check("clear_flag_eperm", r == -EPERM, r);
    r = setsched(0, SCHED_BATCH, 0);
    check("clear_flag_batch_eperm", r == -EPERM, r);
    r = setsched(0, SCHED_BATCH | SCHED_RESET_ON_FORK, 0);
    check("batch_with_flag_ok", r == 0, r);
    r = getsched(0);
    check("batch_flag", r == (SCHED_BATCH | SCHED_RESET_ON_FORK), r);
}

static void idle(void) {
    i64 r = setsched(0, SCHED_BATCH, 0);
    check("batch_ok", r == 0, r);
    r = setsched(0, SCHED_OTHER, 0);
    check("batch_to_other_ok", r == 0, r);
    r = setsched(0, SCHED_IDLE, 0);
    check("idle_ok", r == 0, r);
    r = setsched(0, SCHED_IDLE, 0);
    check("idle_again_ok", r == 0, r);
    r = setparam(0, 0);
    check("idle_setparam_ok", r == 0, r);
    r = setsched(0, SCHED_OTHER, 0);
    check("idle_to_other_eperm", r == -EPERM, r);
    r = setsched(0, SCHED_BATCH, 0);
    check("idle_to_batch_eperm", r == -EPERM, r);
    r = getsched(0);
    check("still_idle", r == SCHED_IDLE, r);
}

int test_main(void) {
    in_child("realtime", realtime);
    in_child("reset_flag", reset_flag);
    in_child("idle", idle);
    return finish();
}

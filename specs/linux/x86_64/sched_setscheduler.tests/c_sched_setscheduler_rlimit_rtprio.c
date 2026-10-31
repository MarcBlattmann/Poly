/* RLIMIT_RTPRIO lets a caller without CAP_SYS_NICE use real-time
 * policies (a child that sets the limit to 10 and drops to uid 65534):
 * - SCHED_FIFO up to the limit (10) is allowed, 11 is EPERM; likewise
 *   sched_setparam;
 * - with the soft limit then lowered to 0, a real-time task may still
 *   lower its priority and keep its policy, but raising the priority or
 *   switching to the other real-time policy is EPERM; returning to
 *   SCHED_OTHER is allowed.
 * - with RLIMIT_NICE 25 (nice down to -5 allowed), leaving SCHED_IDLE at
 *   nice 0 is allowed.
 * poly-diff: needs=root (sets resource limits and drops to another user)
 * poly-diff: syscalls=off (a child process)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_sched_setparam 142
#define SYS_sched_getparam 143
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145
#define SYS_setrlimit 160
/* linux/sched.h */
#define SCHED_OTHER 0
#define SCHED_FIFO 1
#define SCHED_RR 2
#define SCHED_IDLE 5
/* asm-generic/resource.h */
#define RLIMIT_NICE 13
#define RLIMIT_RTPRIO 14

static i64 getsched(void) { return syscall1(SYS_sched_getscheduler, 0); }
static i64 setsched(i64 policy, int prio) {
    int p = prio;
    return syscall3(SYS_sched_setscheduler, 0, policy, &p);
}
static i64 setparam(int prio) {
    int p = prio;
    return syscall2(SYS_sched_setparam, 0, &p);
}
static i64 getparam(void) {
    int p = -77;
    i64 r = syscall2(SYS_sched_getparam, 0, &p);
    return r < 0 ? r : p;
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

static void body(void) {
    i64 r = set_rlimit(RLIMIT_RTPRIO, 10, 10);
    if (r == 0) r = set_rlimit(RLIMIT_NICE, 25, 25);
    if (r == 0) r = become(65534);
    check("dropped", r == 0, r);
    r = setsched(SCHED_FIFO, 11);
    check("fifo_11_eperm", r == -EPERM, r);
    r = setsched(SCHED_FIFO, 10);
    check("fifo_10_ok", r == 0, r);
    r = setparam(11);
    check("setparam_11_eperm", r == -EPERM, r);
    r = setparam(3);
    check("setparam_3_ok", r == 0, r);
    r = setparam(10);
    check("setparam_10_ok", r == 0, r);
    r = setsched(SCHED_RR, 5);
    check("rr_5_ok", r == 0, r);
    r = getsched();
    check("is_rr", r == SCHED_RR, r);

    r = set_rlimit(RLIMIT_RTPRIO, 0, 10);
    check("soft_limit_0", r == 0, r);
    r = setparam(4);
    check("lower_to_4_ok", r == 0, r);
    r = setparam(6);
    check("raise_to_6_eperm", r == -EPERM, r);
    r = getparam();
    check("prio_4", r == 4, r);
    r = setsched(SCHED_FIFO, 4);
    check("switch_rt_policy_eperm", r == -EPERM, r);
    r = setsched(SCHED_RR, 4);
    check("same_policy_ok", r == 0, r);
    r = setsched(SCHED_OTHER, 0);
    check("to_other_ok", r == 0, r);
    r = setsched(SCHED_FIFO, 1);
    check("back_to_fifo_eperm", r == -EPERM, r);

    r = setsched(SCHED_IDLE, 0);
    check("idle_ok", r == 0, r);
    r = setsched(SCHED_OTHER, 0);
    check("leave_idle_with_rlimit_nice_ok", r == 0, r);
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
    return finish();
}

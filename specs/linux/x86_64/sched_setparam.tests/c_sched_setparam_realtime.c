/* Under SCHED_FIFO and SCHED_RR sched_setparam changes the real-time
 * priority within 1..99 and keeps the policy (and SCHED_RESET_ON_FORK);
 * 0 and 100 are EINVAL and change nothing. A parent may change its
 * real-time child's priority. In child processes.
 * poly-diff: needs=root (only a privileged caller may choose a real-time policy without RLIMIT_RTPRIO)
 * poly-diff: syscalls=off (child processes)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_sched_setparam 142
#define SYS_sched_getparam 143
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145
/* linux/sched.h */
#define SCHED_FIFO 1
#define SCHED_RR 2
#define SCHED_RESET_ON_FORK 0x40000000

static i64 getsched(i64 pid) { return syscall1(SYS_sched_getscheduler, pid); }
static i64 setsched(i64 pid, i64 policy, int prio) {
    int p = prio;
    return syscall3(SYS_sched_setscheduler, pid, policy, &p);
}
static i64 setparam(i64 pid, int prio) {
    int p = prio;
    return syscall2(SYS_sched_setparam, pid, &p);
}
static i64 getparam(i64 pid) {
    int p = -77;
    i64 r = syscall2(SYS_sched_getparam, pid, &p);
    return r < 0 ? r : p;
}

static void body(void) {
    i64 r = setsched(0, SCHED_FIFO, 10);
    check("fifo_10", r == 0, r);
    r = setparam(0, 99);
    check("setparam_99", r == 0, r);
    r = getparam(0);
    check("reads_99", r == 99, r);
    r = getsched(0);
    check("still_fifo", r == SCHED_FIFO, r);
    r = setparam(0, 1);
    check("setparam_1", r == 0, r);
    r = getparam(0);
    check("reads_1", r == 1, r);
    r = setparam(0, 0);
    check("setparam_0_einval", r == -EINVAL, r);
    r = setparam(0, 100);
    check("setparam_100_einval", r == -EINVAL, r);
    r = getparam(0);
    check("still_1", r == 1, r);
    r = setsched(0, SCHED_RR | SCHED_RESET_ON_FORK, 5);
    check("rr_reset_5", r == 0, r);
    r = setparam(0, 60);
    check("rr_setparam_60", r == 0, r);
    r = getsched(0);
    check("rr_flag_kept", r == (SCHED_RR | SCHED_RESET_ON_FORK), r);
    r = getparam(0);
    check("reads_60", r == 60, r);
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

    volatile unsigned *g = shared_page(4096);
    pid = fork();
    if (pid == 0) {
        setsched(0, SCHED_RR, 20);
        gate_open(&g[0]);
        gate_wait(&g[1]);
        exit_group(getparam(0) == 33 ? 0 : 1);
    }
    gate_wait(&g[0]);
    i64 r = setparam(pid, 33);
    check("set_child_33", r == 0, r);
    gate_open(&g[1]);
    code = wait_exit(pid);
    check("child_saw_33", code == 0, code);
    return finish();
}

/* sched_rr_get_interval under the fair policies returns 0 and stores a
 * struct timespec (two 64-bit fields, 16 bytes; bytes after it are left
 * alone) with tv_sec 0 and tv_nsec in 0..999999999. The value is the
 * task's time slice rounded down to whole clock ticks, so it depends
 * on the kernel's tick rate (0 on a 250 Hz kernel, a few ms at 1000
 * Hz); it is the same for SCHED_OTHER, SCHED_BATCH and SCHED_IDLE and
 * never more than the SCHED_RR interval (100 ms by default). pid 0, the
 * caller's pid and tid name the caller; only the low 32 bits count.
 * poly-diff: syscalls=off (a child process)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_sched_setscheduler 144
#define SYS_sched_rr_get_interval 148
/* linux/sched.h */
#define SCHED_BATCH 3
#define SCHED_IDLE 5

static i64 setsched(i64 pid, i64 policy, int prio) {
    int p = prio;
    return syscall3(SYS_sched_setscheduler, pid, policy, &p);
}
/* The interval in ns, or the error. */
static i64 interval(i64 pid) {
    i64 t[3] = { 77, 77, 99 };
    i64 r = syscall2(SYS_sched_rr_get_interval, pid, t);
    if (r < 0) return r;
    if (t[2] != 99) return -1000;
    if (t[0] != 0 || t[1] < 0 || t[1] > 999999999) return -2000;
    return t[1];
}

int test_main(void) {
    i64 other = interval(0);
    check("other_ok", other >= 0, other);
    check("other_at_most_100ms", other <= 100000000, other);
    i64 r = interval(getpid());
    check("own_pid_same", r == other, r);
    r = interval(gettid());
    check("own_tid_same", r == other, r);
    r = interval(1L << 32);
    check("high_bits_pid0_same", r == other, r);

    i64 pid = fork();
    if (pid == 0) {
        if (setsched(0, SCHED_BATCH, 0) != 0 || interval(0) != other) exit_group(1);
        if (setsched(0, SCHED_IDLE, 0) != 0 || interval(0) != other) exit_group(2);
        exit_group(0);
    }
    int code = wait_exit(pid);
    check("batch_and_idle_same", code == 0, code);
    return finish();
}

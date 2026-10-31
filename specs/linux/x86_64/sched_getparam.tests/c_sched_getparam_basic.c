/* sched_getparam stores the static priority in struct sched_param (one
 * int, linux/sched/types.h) and returns 0: 0 under SCHED_OTHER,
 * SCHED_BATCH and SCHED_IDLE. Exactly 4 bytes are written: the bytes
 * after the int are left alone. pid 0, the caller's pid and its thread
 * id are the caller; only the low 32 bits of pid count.
 * poly-diff: syscalls=off (one step runs in a child process)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_sched_setscheduler 144
#define SYS_sched_getparam 143
/* linux/sched.h */
#define SCHED_OTHER 0
#define SCHED_BATCH 3
#define SCHED_IDLE 5

static i64 setsched(i64 pid, i64 policy, int prio) {
    int p = prio;
    return syscall3(SYS_sched_setscheduler, pid, policy, &p);
}

int test_main(void) {
    int buf[4] = { 77, 88, 99, 111 };
    i64 r = syscall2(SYS_sched_getparam, 0, buf);
    check("self_ok", r == 0, r);
    check("other_prio_0", buf[0] == 0, buf[0]);
    check("only_4_bytes_written", buf[1] == 88 && buf[2] == 99 && buf[3] == 111, buf[1]);

    buf[0] = 5;
    r = syscall2(SYS_sched_getparam, getpid(), buf);
    check("own_pid", r == 0 && buf[0] == 0, buf[0]);
    buf[0] = 5;
    r = syscall2(SYS_sched_getparam, gettid(), buf);
    check("own_tid", r == 0 && buf[0] == 0, buf[0]);
    buf[0] = 5;
    r = syscall2(SYS_sched_getparam, 1L << 32, buf);
    check("high_bits_pid0", r == 0 && buf[0] == 0, r);

    r = setsched(0, SCHED_BATCH, 0);
    check("set_batch", r == 0, r);
    buf[0] = 5;
    r = syscall2(SYS_sched_getparam, 0, buf);
    check("batch_prio_0", r == 0 && buf[0] == 0, buf[0]);
    r = setsched(0, SCHED_OTHER, 0);
    check("set_other", r == 0, r);

    i64 pid = fork();
    if (pid == 0) {
        int p = 5;
        if (setsched(0, SCHED_IDLE, 0) != 0) exit_group(1);
        if (syscall2(SYS_sched_getparam, 0, &p) != 0 || p != 0) exit_group(2);
        exit_group(0);
    }
    int code = wait_exit(pid);
    check("idle_prio_0", code == 0, code);
    return finish();
}

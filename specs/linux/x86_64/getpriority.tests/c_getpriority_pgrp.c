/* PRIO_PGRP: getpriority reports the highest value (the lowest nice) of
 * all threads in the process group; who 0 is the caller's own group. A
 * child that makes its own group (setpgid 0 0) with a thread of its own
 * at nice 1, a member at nice 2 and itself at nice 6 reads 19; once the
 * thread has exited, 18; the group id read as PRIO_PROCESS is just the
 * leader (14).
 * poly-diff: syscalls=off (several processes and a thread)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_getpriority 140
#define SYS_setpriority 141
/* linux/resource.h */
#define PRIO_PROCESS 0
#define PRIO_PGRP 1

static i64 getprio(i64 which, i64 who) { return syscall2(SYS_getpriority, which, who); }
static i64 setprio(i64 which, i64 who, i64 nice) { return syscall3(SYS_setpriority, which, who, nice); }

static char stack[65536] __attribute__((aligned(16)));
static volatile int tid_word;
static volatile unsigned t_ready, t_exit;

static int thread_main(void *arg) {
    (void)arg;
    setprio(PRIO_PROCESS, 0, 1);
    gate_open(&t_ready);
    gate_wait(&t_exit);
    return 0;
}

static void body(void) {
    i64 r = syscall2(SYS_setpgid, 0, 0);
    check("own_group", r == 0, r);
    /* The thread and the member start at nice 0 and raise their own. */
    i64 t = poly_clone(THREAD_FLAGS, stack + sizeof stack, (int *)&tid_word, (int *)&tid_word, thread_main, 0);
    check("thread_started", t > 0, t);
    gate_wait(&t_ready);
    volatile unsigned *g = shared_page(4096);
    i64 member = fork();
    if (member == 0) {
        setprio(PRIO_PROCESS, 0, 2);
        gate_open(&g[0]);
        gate_wait(&g[1]);
        exit_group(0);
    }
    gate_wait(&g[0]);
    r = setprio(PRIO_PROCESS, 0, 6);
    check("self_nice_6", r == 0, r);
    r = getprio(PRIO_PGRP, 0);
    check("group_with_thread_19", r == 19, r);
    r = getprio(PRIO_PROCESS, 0);
    check("main_thread_14", r == 14, r);
    gate_open(&t_exit);
    while (__atomic_load_n(&tid_word, __ATOMIC_SEQ_CST) != 0)
        futex((volatile unsigned *)&tid_word, FUTEX_WAIT, (unsigned)t, 0, 0, 0);

    r = getprio(PRIO_PGRP, 0);
    check("group_18", r == 18, r);
    r = getprio(PRIO_PGRP, getpid());
    check("group_by_id_18", r == 18, r);
    r = getprio(PRIO_PROCESS, getpid());
    check("leader_alone_14", r == 14, r);

    gate_open(&g[1]);
    wait_exit(member);
    r = getprio(PRIO_PGRP, 0);
    check("group_alone_14", r == 14, r);
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

/* sched_rr_get_interval on another process with a real-time policy: a
 * SCHED_RR process forks a child, which inherits SCHED_RR and sees 100 ms
 * both for itself and for its (blocked) parent; under SCHED_FIFO both
 * read {0, 0}; with SCHED_RR|SCHED_RESET_ON_FORK the child is back in
 * SCHED_OTHER (below one second) while the parent still reads 100 ms.
 * Results come back as exit codes; no real-time process ever spins.
 * poly-diff: needs=root (only root may enter SCHED_FIFO/SCHED_RR here)
 */
#include "a_sys.h"

#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145
#define SYS_sched_rr_get_interval 148
#define SCHED_OTHER 0
#define SCHED_FIFO 1
#define SCHED_RR 2
#define SCHED_RESET_ON_FORK 0x40000000

struct a_ts { i64 sec, nsec; };

static int is(struct a_ts *t, i64 sec, i64 nsec) { return t->sec == sec && t->nsec == nsec; }
static int fair(struct a_ts *t) { return t->sec == 0 && t->nsec >= 0 && t->nsec < 1000000000; }

/* Forks a child that queries itself and its parent; returns the child's
 * exit code: bit 0 self call failed or wrong, bit 1 parent call. mode 0:
 * both must be want_nsec; mode 1: self must be SCHED_OTHER and fair. */
static int probe(int mode, i64 want_nsec) {
    long pid = a_fork_sync();
    if (pid == 0) {
        int bad = 0;
        struct a_ts s = {-1, -1}, p = {-1, -1};
        i64 r1 = syscall2(SYS_sched_rr_get_interval, 0, &s);
        i64 r2 = syscall2(SYS_sched_rr_get_interval, syscall0(SYS_getppid), &p);
        if (mode == 0 && (r1 != 0 || !is(&s, 0, want_nsec))) bad |= 1;
        if (mode == 1 && (r1 != 0 || !fair(&s) || syscall1(SYS_sched_getscheduler, 0) != SCHED_OTHER)) bad |= 1;
        if (r2 != 0 || !is(&p, 0, want_nsec)) bad |= 2;
        exit_group(bad);
    }
    i32 st = -1;
    if (syscall4(SYS_wait4, pid, &st, 0, 0) != pid) return 255;
    return (st >> 8) & 0xff;
}

int test_main(void) {
    a_block_sigchld();
    long pid = a_fork_sync();
    if (pid == 0) {
        i32 prio = 5;
        expect("set_rr", syscall3(SYS_sched_setscheduler, 0, SCHED_RR, &prio), 0);
        expect("rr_child", probe(0, 100000000), 0);
        expect("set_fifo", syscall3(SYS_sched_setscheduler, 0, SCHED_FIFO, &prio), 0);
        expect("fifo_child", probe(0, 0), 0);
        expect("set_rr_reset", syscall3(SYS_sched_setscheduler, 0, SCHED_RR | SCHED_RESET_ON_FORK, &prio), 0);
        expect("reset_child", probe(1, 100000000), 0);
        exit_group(failed ? 1 : 0);
    }
    i32 st = -1;
    expect("reap", syscall4(SYS_wait4, pid, &st, 0, 0), pid);
    expect("child_ok", st, 0);
    return finish();
}

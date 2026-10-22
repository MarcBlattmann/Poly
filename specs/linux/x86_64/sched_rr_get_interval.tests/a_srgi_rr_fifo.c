/* sched_rr_get_interval under the real-time policies: SCHED_FIFO reports
 * {0, 0}; SCHED_RR reports the round-robin quantum, 100 ms by default,
 * whatever the priority (1 or 99); back under SCHED_OTHER the value is
 * again below one second. Runs in a child so the parent stays SCHED_OTHER.
 * poly-diff: needs=root (only root may enter SCHED_FIFO/SCHED_RR here)
 */
#include "a_sys.h"

#define SYS_sched_setscheduler 144
#define SYS_sched_rr_get_interval 148
#define SCHED_OTHER 0
#define SCHED_FIFO 1
#define SCHED_RR 2

struct a_ts { i64 sec, nsec; };

int test_main(void) {
    a_block_sigchld();
    long pid = a_fork_sync();
    if (pid == 0) {
        struct a_ts t;
        i32 prio = 10;
        expect("set_fifo", syscall3(SYS_sched_setscheduler, 0, SCHED_FIFO, &prio), 0);
        t.sec = t.nsec = -1;
        expect("fifo_call", syscall2(SYS_sched_rr_get_interval, 0, &t), 0);
        expect("fifo_sec", t.sec, 0);
        expect("fifo_nsec", t.nsec, 0);
        prio = 1;
        expect("set_rr1", syscall3(SYS_sched_setscheduler, 0, SCHED_RR, &prio), 0);
        t.sec = t.nsec = -1;
        expect("rr1_call", syscall2(SYS_sched_rr_get_interval, 0, &t), 0);
        expect("rr1_sec", t.sec, 0);
        expect("rr1_nsec", t.nsec, 100000000);
        prio = 99;
        expect("set_rr99", syscall3(SYS_sched_setscheduler, 0, SCHED_RR, &prio), 0);
        t.sec = t.nsec = -1;
        expect("rr99_call", syscall2(SYS_sched_rr_get_interval, syscall0(SYS_getpid), &t), 0);
        expect("rr99_sec", t.sec, 0);
        expect("rr99_nsec", t.nsec, 100000000);
        prio = 0;
        expect("set_other", syscall3(SYS_sched_setscheduler, 0, SCHED_OTHER, &prio), 0);
        t.sec = t.nsec = -1;
        expect("other_call", syscall2(SYS_sched_rr_get_interval, 0, &t), 0);
        check("other_valid", t.sec == 0 && t.nsec >= 0 && t.nsec < 1000000000, t.nsec);
        exit_group(failed ? 1 : 0);
    }
    i32 st = -1;
    expect("reap", syscall4(SYS_wait4, pid, &st, 0, 0), pid);
    expect("child_ok", st, 0);
    return finish();
}

/* sched_getscheduler on the caller: pid 0, the caller's pid and its tid all
 * report SCHED_OTHER (0). The call reads only its first argument (junk in
 * the other five registers changes nothing), may be repeated, and changes
 * nothing itself: nice stays the same. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_getpriority 140
#define SYS_sched_getscheduler 145

/* linux/resource.h */
#define PRIO_PROCESS 0

int test_main(void) {
    i64 me = syscall0(SYS_getpid);
    i64 tid = syscall0(SYS_gettid);
    i64 nice_before = syscall2(SYS_getpriority, PRIO_PROCESS, 0);
    check("getpriority", nice_before > 0, nice_before);
    expect("pid0", syscall1(SYS_sched_getscheduler, 0), 0);
    expect("own_pid", syscall1(SYS_sched_getscheduler, me), 0);
    expect("own_tid", syscall1(SYS_sched_getscheduler, tid), 0);
    expect("junk_args", syscall6(SYS_sched_getscheduler, 0, 1, -1, 0x40000000, 0xffff800000000000L, 7), 0);
    expect("junk_args_pid", syscall6(SYS_sched_getscheduler, me, 5, 1, 2, 3, 4), 0);
    int all = 1;
    for (int i = 0; i < 100; i++)
        if (syscall1(SYS_sched_getscheduler, 0) != 0) all = 0;
    check("repeat_100", all, 0);
    expect("nice_unchanged", syscall2(SYS_getpriority, PRIO_PROCESS, 0), nice_before);
    expect("still_other", syscall1(SYS_sched_getscheduler, 0), 0);
    return finish();
}

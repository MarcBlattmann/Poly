/* sched_getparam on the caller: pid 0, the caller's pid and its tid all
 * return 0 and store sched_priority 0 (SCHED_OTHER), overwriting whatever
 * the int held before. Repeated calls give the same result. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_getparam 143

static i64 getp(i64 pid, i32 fill) {
    i32 prio = fill;
    i64 r = syscall2(SYS_sched_getparam, pid, &prio);
    return r < 0 ? r : prio;
}

int test_main(void) {
    i32 prio = 0x55555555;
    expect("pid0_ret", syscall2(SYS_sched_getparam, 0, &prio), 0);
    expect("pid0_val", prio, 0);
    expect("pid0_from_minus1", getp(0, -1), 0);
    expect("pid0_from_99", getp(0, 99), 0);
    expect("getpid", getp(syscall0(SYS_getpid), 0x7fffffff), 0);
    expect("gettid", getp(syscall0(SYS_gettid), (i32)0x80000000), 0);
    for (int i = 0; i < 3; i++) expect("again", getp(0, 7), 0);
    return finish();
}

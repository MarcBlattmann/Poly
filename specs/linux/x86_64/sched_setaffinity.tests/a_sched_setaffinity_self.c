/* sched_setaffinity(0, 8, mask) sets the calling thread's CPU mask and
 * returns 0; sched_getaffinity then reports exactly the CPUs set. A mask
 * of all ones gives back every CPU the thread may use (the start mask). */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_setaffinity 203
#define SYS_sched_getaffinity 204

static u64 mask_of(i64 pid) {
    u64 m[16];
    memset(m, 0, sizeof m);
    i64 r = syscall3(SYS_sched_getaffinity, pid, sizeof m, m);
    return r == 8 ? m[0] : 0;
}

static u64 highest(u64 m) {
    u64 h = 0;
    for (; m; m &= m - 1) h = m & -m;
    return h;
}

int test_main(void) {
    u64 start = mask_of(0);
    check("start_mask", start != 0, (i64)start);
    u64 lo = start & -start, hi = highest(start);

    u64 v = lo;
    expect("set_lowest", syscall3(SYS_sched_setaffinity, 0, 8, &v), 0);
    check("get_lowest", mask_of(0) == lo, 0);
    v = hi;
    expect("set_highest", syscall3(SYS_sched_setaffinity, 0, 8, &v), 0);
    check("get_highest", mask_of(0) == hi, 0);
    v = lo | hi;
    expect("set_two", syscall3(SYS_sched_setaffinity, 0, 8, &v), 0);
    check("get_two", mask_of(0) == (lo | hi), 0);
    expect("set_same_again", syscall3(SYS_sched_setaffinity, 0, 8, &v), 0);
    check("get_same_again", mask_of(0) == (lo | hi), 0);
    check("mask_not_changed_by_call", v == (lo | hi), 0);

    v = ~0UL;
    expect("set_all_ones", syscall3(SYS_sched_setaffinity, 0, 8, &v), 0);
    check("all_ones_is_start", mask_of(0) == start, 0);
    v = start;
    expect("set_start", syscall3(SYS_sched_setaffinity, 0, 8, &v), 0);
    check("get_start", mask_of(0) == start, 0);
    return finish();
}

/* sched_getaffinity reports what sched_setaffinity stored: a one-CPU
 * mask gives that bit alone, two CPUs give both bits; a requested CPU the
 * system does not have (bit 63) is not reported; restoring the start mask
 * gives the start mask back. The CPU the caller runs on is always in the
 * reported mask. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_setaffinity 203
#define SYS_sched_getaffinity 204
#define SYS_getcpu 309

static u64 mask_of(i64 pid) {
    u64 m[2] = {0, 0};
    i64 r = syscall3(SYS_sched_getaffinity, pid, sizeof m, m);
    return r == 8 ? m[0] : 0;
}

static u64 highest(u64 m) {
    u64 h = 0;
    for (; m; m &= m - 1) h = m & -m;
    return h;
}

static int on_cpu_in(u64 m) {
    u32 cpu = 0xffffffff;
    if (syscall3(SYS_getcpu, &cpu, 0, 0) != 0 || cpu >= 64) return 0;
    return (m >> cpu) & 1;
}

int test_main(void) {
    u64 start = mask_of(0);
    check("start_not_empty", start != 0, 0);
    u64 lo = start & -start, hi = highest(start);

    u64 v = hi;
    expect("set_highest", syscall3(SYS_sched_setaffinity, 0, 8, &v), 0);
    check("get_highest", mask_of(0) == hi, 0);
    check("runs_on_highest", on_cpu_in(hi), 0);
    v = lo;
    expect("set_lowest", syscall3(SYS_sched_setaffinity, 0, 8, &v), 0);
    check("get_lowest", mask_of(0) == lo, 0);
    check("runs_on_lowest", on_cpu_in(lo), 0);
    v = lo | hi;
    expect("set_both", syscall3(SYS_sched_setaffinity, 0, 8, &v), 0);
    check("get_both", mask_of(0) == (lo | hi), 0);
    v = hi | (1UL << 63);
    expect("set_with_bit63", syscall3(SYS_sched_setaffinity, 0, 8, &v), 0);
    check("get_without_bit63", mask_of(0) == hi, 0);
    v = 0;
    expect("set_empty_fails", syscall3(SYS_sched_setaffinity, 0, 8, &v), -EINVAL);
    check("get_after_failed_set", mask_of(0) == hi, 0);
    v = start;
    expect("restore", syscall3(SYS_sched_setaffinity, 0, 8, &v), 0);
    check("get_restored", mask_of(0) == start, 0);
    v = ~0UL;
    expect("set_all_ones", syscall3(SYS_sched_setaffinity, 0, 8, &v), 0);
    check("get_all_ones_is_start", mask_of(0) == start, 0);
    return finish();
}

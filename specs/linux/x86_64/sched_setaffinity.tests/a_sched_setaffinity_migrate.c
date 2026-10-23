/* A thread whose mask no longer includes the CPU it runs on is moved to a
 * CPU in the new mask before the call returns: getcpu right after the
 * call reports a CPU of the mask, for a single CPU (the lowest and the
 * highest usable one) and for a two-CPU mask. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_setaffinity 203
#define SYS_sched_getaffinity 204
#define SYS_getcpu 309

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

/* The CPU the caller runs on, as a one-bit mask (0 on failure). */
static u64 cpu_bit(void) {
    u32 cpu = 0xffffffff;
    if (syscall3(SYS_getcpu, &cpu, 0, 0) != 0 || cpu >= 64) return 0;
    return 1UL << cpu;
}

int test_main(void) {
    u64 start = mask_of(0);
    u64 lo = start & -start, hi = highest(start);
    u64 v;
    for (int i = 0; i < 3; i++) {
        v = hi;
        expect("set_highest", syscall3(SYS_sched_setaffinity, 0, 8, &v), 0);
        check("on_highest", cpu_bit() == hi, 0);
        v = lo;
        expect("set_lowest", syscall3(SYS_sched_setaffinity, 0, 8, &v), 0);
        check("on_lowest", cpu_bit() == lo, 0);
    }
    v = lo | hi;
    expect("set_two", syscall3(SYS_sched_setaffinity, 0, 8, &v), 0);
    check("on_one_of_two", (cpu_bit() & (lo | hi)) != 0, 0);
    v = start;
    expect("restore", syscall3(SYS_sched_setaffinity, 0, 8, &v), 0);
    check("on_a_start_cpu", (cpu_bit() & start) != 0, 0);
    return finish();
}

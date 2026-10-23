/* Bits for CPUs the system does not have are dropped silently as long as
 * one usable CPU is named: the stored mask is the request intersected
 * with the usable CPUs. Bytes past the kernel's 8-byte mask (a 16-byte
 * buffer's second word) are ignored, so a CPU named only there counts as
 * no CPU. */
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

int test_main(void) {
    u64 start = mask_of(0);
    u64 lo = start & -start;

    u64 v = lo | (1UL << 63);
    expect("lowest_plus_cpu63", syscall3(SYS_sched_setaffinity, 0, 8, &v), 0);
    check("cpu63_dropped", mask_of(0) == lo, 0);
    v = start | (1UL << 63) | (1UL << 62);
    expect("start_plus_high", syscall3(SYS_sched_setaffinity, 0, 8, &v), 0);
    check("high_dropped", mask_of(0) == start, 0);

    u64 two[2] = {lo, ~0UL};
    expect("second_word_ignored", syscall3(SYS_sched_setaffinity, 0, 16, two), 0);
    check("second_word_ignored_seen", mask_of(0) == lo, 0);
    two[0] = 0;
    two[1] = 1;
    expect("only_second_word", syscall3(SYS_sched_setaffinity, 0, 16, two), -EINVAL);
    u64 big[16];
    for (int i = 0; i < 16; i++) big[i] = ~0UL;
    expect("len128_all_ones", syscall3(SYS_sched_setaffinity, 0, sizeof big, big), 0);
    check("len128_is_start", mask_of(0) == start, 0);
    return finish();
}

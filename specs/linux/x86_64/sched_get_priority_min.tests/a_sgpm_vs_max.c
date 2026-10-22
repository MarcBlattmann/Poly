/* For every valid policy the minimum is <= sched_get_priority_max; for
 * SCHED_FIFO and SCHED_RR the range spans at least 32 (POSIX); both calls
 * reject the same invalid policy. */
#include "a_sys.h"

#define SYS_sched_get_priority_min 147 /* arch/x86/entry/syscalls/syscall_64.tbl */
#define A_EINVAL 22                    /* asm-generic/errno-base.h */

#define SYS_sched_get_priority_max 146

int test_main(void) {
    static const i64 pols[] = {0, 1, 2, 3, 5, 6};
    static const char *names[] = {"other_min_le_max", "fifo_min_le_max", "rr_min_le_max",
                                  "batch_min_le_max", "idle_min_le_max", "deadline_min_le_max"};
    for (int i = 0; i < 6; i++) {
        i64 lo = syscall1(SYS_sched_get_priority_min, pols[i]);
        i64 hi = syscall1(SYS_sched_get_priority_max, pols[i]);
        check(names[i], lo >= 0 && hi >= lo, lo);
    }
    i64 fifo_span = syscall1(SYS_sched_get_priority_max, 1) - syscall1(SYS_sched_get_priority_min, 1);
    check("fifo_span_ge_32", fifo_span >= 32, fifo_span);
    i64 rr_span = syscall1(SYS_sched_get_priority_max, 2) - syscall1(SYS_sched_get_priority_min, 2);
    check("rr_span_ge_32", rr_span >= 32, rr_span);
    expect("max_rejects_4_too", syscall1(SYS_sched_get_priority_max, 4), -A_EINVAL);
    return finish();
}

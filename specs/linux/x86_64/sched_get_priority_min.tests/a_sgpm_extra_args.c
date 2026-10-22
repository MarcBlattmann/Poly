/* Only the first argument matters: garbage in the other five argument
 * registers changes nothing. */
#include "a_sys.h"

#define SYS_sched_get_priority_min 147 /* arch/x86/entry/syscalls/syscall_64.tbl */
#define A_EINVAL 22                    /* asm-generic/errno-base.h */

int test_main(void) {
    i64 g = (i64)0xdeadbeefcafef00dULL;
    expect("other_extra_args", syscall6(SYS_sched_get_priority_min, 0, g, g, g, g, g), 0);
    expect("fifo_extra_args", syscall6(SYS_sched_get_priority_min, 1, -1, -1, -1, -1, -1), 1);
    expect("rr_extra_args", syscall6(SYS_sched_get_priority_min, 2, 1, 2, 3, 4, 5), 1);
    expect("bad_extra_args", syscall6(SYS_sched_get_priority_min, 4, 0, 0, 0, 0, 0), -A_EINVAL);
    return finish();
}

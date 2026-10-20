/* clock_getres: unknown clock ids are EINVAL; a NULL result pointer is
 * allowed; a bad one is EFAULT. */
#include "a_sys.h"

int test_main(void) {
    struct a_timespec t;
    expect("null_result_ok", syscall2(SYS_clock_getres, CLOCK_MONOTONIC, 0), 0);
    expect("bad_pointer_efault", syscall2(SYS_clock_getres, CLOCK_MONOTONIC, (void *)8), -EFAULT);
    expect("clock_10_einval", syscall2(SYS_clock_getres, 10, &t), -EINVAL);
    expect("clock_12_einval", syscall2(SYS_clock_getres, 12, &t), -EINVAL);
    expect("clock_100_einval", syscall2(SYS_clock_getres, 100, &t), -EINVAL);
    expect("unknown_clock_null_einval", syscall2(SYS_clock_getres, 100, 0), -EINVAL);
    /* A dynamic clock id for a file descriptor that is not open. */
    expect("fd_clock_bad_fd", syscall2(SYS_clock_getres, (i64)((~(u64)50 << 3) | 3), &t), -EINVAL);
    return finish();
}

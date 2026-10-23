/* The order of the checks: 1. len (not a multiple of 8, or 0) EINVAL;
 * 2. pid not found ESRCH; 3. buffer not writable EFAULT. So a missing pid
 * with len 0 or 7 is EINVAL, a missing or negative pid with a NULL buffer
 * is ESRCH, and NULL with a bad len is EINVAL. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_getaffinity 204

int test_main(void) {
    u64 m[2];
    expect("missing_len0", syscall3(SYS_sched_getaffinity, 0x3fffffff, 0, m), -EINVAL);
    expect("missing_len7", syscall3(SYS_sched_getaffinity, 0x3fffffff, 7, m), -EINVAL);
    expect("negative_len9", syscall3(SYS_sched_getaffinity, -1, 9, m), -EINVAL);
    expect("missing_len_4g", syscall3(SYS_sched_getaffinity, 0x3fffffff, 0x100000000L, m), -EINVAL);
    expect("null_len0", syscall3(SYS_sched_getaffinity, 0, 0, 0), -EINVAL);
    expect("null_len7", syscall3(SYS_sched_getaffinity, 0, 7, 0), -EINVAL);
    expect("missing_null_len7", syscall3(SYS_sched_getaffinity, 0x3fffffff, 7, 0), -EINVAL);
    expect("missing_null", syscall3(SYS_sched_getaffinity, 0x3fffffff, 8, 0), -ESRCH);
    expect("negative_null", syscall3(SYS_sched_getaffinity, -1, 8, 0), -ESRCH);
    expect("missing_kernel_addr", syscall3(SYS_sched_getaffinity, 0x3fffffff, 8, 0xffff800000000000UL), -ESRCH);
    expect("self_null", syscall3(SYS_sched_getaffinity, 0, 8, 0), -EFAULT);
    return finish();
}

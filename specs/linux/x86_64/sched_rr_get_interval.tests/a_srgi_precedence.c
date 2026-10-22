/* Error precedence of sched_rr_get_interval: a negative pid gives EINVAL
 * and a missing pid ESRCH even when the pointer is bad; with a valid pid
 * the bad pointer gives EFAULT. */
#include "a_sys.h"

#define SYS_sched_rr_get_interval 148

int test_main(void) {
    expect("neg_null", syscall2(SYS_sched_rr_get_interval, -1, 0), -EINVAL);
    expect("neg_kernel", syscall2(SYS_sched_rr_get_interval, -5, 0xffff800000000000ULL), -EINVAL);
    expect("missing_null", syscall2(SYS_sched_rr_get_interval, 0x7fffffff, 0), -ESRCH);
    expect("missing_low", syscall2(SYS_sched_rr_get_interval, 0x7ffffffe, 8), -ESRCH);
    expect("self_null", syscall2(SYS_sched_rr_get_interval, 0, 0), -EFAULT);
    expect("getpid_null", syscall2(SYS_sched_rr_get_interval, syscall0(SYS_getpid), 0), -EFAULT);
    return finish();
}

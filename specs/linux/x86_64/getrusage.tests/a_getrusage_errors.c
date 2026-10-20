/* getrusage: RUSAGE_THREAD works; other values of who are EINVAL; a bad
 * pointer is EFAULT. */
#include "a_sys.h"

int test_main(void) {
    struct a_rusage r;
    expect("thread_ok", syscall2(SYS_getrusage, RUSAGE_THREAD, &r), 0);
    check("thread_well_formed", r.ru_utime.tv_usec >= 0 && r.ru_utime.tv_usec < 1000000, r.ru_utime.tv_usec);
    expect("who_2_einval", syscall2(SYS_getrusage, 2, &r), -EINVAL);
    expect("who_minus_2_einval", syscall2(SYS_getrusage, -2, &r), -EINVAL);
    expect("bad_pointer_efault", syscall2(SYS_getrusage, RUSAGE_SELF, (void *)8), -EFAULT);
    expect("null_efault", syscall2(SYS_getrusage, RUSAGE_SELF, (void *)0), -EFAULT);
    return finish();
}

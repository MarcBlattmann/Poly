/* getrlimit: unknown resources are EINVAL, a bad pointer is EFAULT. */
#include "a_sys.h"

int test_main(void) {
    struct a_rlimit l;
    expect("resource_16_einval", syscall2(SYS_getrlimit, 16, &l), -EINVAL);
    expect("resource_1000_einval", syscall2(SYS_getrlimit, 1000, &l), -EINVAL);
    expect("resource_negative_einval", syscall2(SYS_getrlimit, -1, &l), -EINVAL);
    expect("bad_pointer_efault", syscall2(SYS_getrlimit, RLIMIT_NOFILE, (void *)8), -EFAULT);
    expect("null_pointer_efault", syscall2(SYS_getrlimit, RLIMIT_NOFILE, (void *)0), -EFAULT);
    expect("good_call_after_errors", syscall2(SYS_getrlimit, RLIMIT_NOFILE, &l), 0);
    return finish();
}

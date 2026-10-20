/* prlimit64: invalid resource, soft above hard, bad pointers; a failed
 * set leaves the old limits (and does not fill the old-value buffer
 * when the new value is invalid). */
#include "a_sys.h"

int test_main(void) {
    struct a_rlimit l, old, bad = {9, 3};
    expect("resource_16_einval", syscall4(SYS_prlimit64, 0, 16, 0, &l), -EINVAL);
    expect("negative_resource_einval", syscall4(SYS_prlimit64, 0, -2, 0, &l), -EINVAL);
    syscall2(SYS_getrlimit, RLIMIT_CORE, &l);
    memset(&old, 0x5a, sizeof old);
    expect("soft_above_hard_einval", syscall4(SYS_prlimit64, 0, RLIMIT_CORE, &bad, &old), -EINVAL);
    struct a_rlimit now;
    syscall2(SYS_getrlimit, RLIMIT_CORE, &now);
    check("limits_unchanged", now.rlim_cur == l.rlim_cur && now.rlim_max == l.rlim_max, 0);
    expect("bad_new_pointer_efault", syscall4(SYS_prlimit64, 0, RLIMIT_CORE, (void *)8, 0), -EFAULT);
    expect("bad_old_pointer_efault", syscall4(SYS_prlimit64, 0, RLIMIT_CORE, 0, (void *)8), -EFAULT);
    return finish();
}

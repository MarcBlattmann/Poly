/* getitimer: unknown timers are EINVAL, a bad pointer EFAULT. */
#include "a_sys.h"

int test_main(void) {
    struct a_itimerval cur;
    expect("which_3_einval", syscall2(SYS_getitimer, 3, &cur), -EINVAL);
    expect("which_negative_einval", syscall2(SYS_getitimer, -1, &cur), -EINVAL);
    expect("bad_pointer_efault", syscall2(SYS_getitimer, ITIMER_REAL, (void *)8), -EFAULT);
    expect("null_pointer_efault", syscall2(SYS_getitimer, ITIMER_VIRTUAL, (void *)0), -EFAULT);
    expect("good_after_errors", syscall2(SYS_getitimer, ITIMER_PROF, &cur), 0);
    return finish();
}

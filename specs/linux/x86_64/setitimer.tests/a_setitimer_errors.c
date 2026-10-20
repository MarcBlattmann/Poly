/* setitimer: unknown timers and microseconds outside [0, 1e6) are
 * EINVAL; bad pointers EFAULT; a NULL new value is EFAULT... except that
 * Linux treats NULL as "disarm" (with a warning) and succeeds. */
#include "a_sys.h"

int test_main(void) {
    struct a_itimerval ok = {{0, 0}, {5, 0}}, bad = {{0, 0}, {0, 1000000}}, badi = {{0, -1}, {1, 0}};
    expect("which_3_einval", syscall3(SYS_setitimer, 3, &ok, 0), -EINVAL);
    expect("which_negative_einval", syscall3(SYS_setitimer, -1, &ok, 0), -EINVAL);
    expect("usec_1e6_einval", syscall3(SYS_setitimer, ITIMER_REAL, &bad, 0), -EINVAL);
    expect("negative_interval_usec_einval", syscall3(SYS_setitimer, ITIMER_REAL, &badi, 0), -EINVAL);
    expect("bad_new_efault", syscall3(SYS_setitimer, ITIMER_REAL, (void *)8, 0), -EFAULT);
    expect("arm", syscall3(SYS_setitimer, ITIMER_REAL, &ok, 0), 0);
    expect("bad_old_efault", syscall3(SYS_setitimer, ITIMER_REAL, &ok, (void *)8), -EFAULT);
    struct a_itimerval old;
    expect("null_new_disarms", syscall3(SYS_setitimer, ITIMER_REAL, 0, &old), 0);
    check("null_new_returned_old", old.it_value.tv_sec >= 4, old.it_value.tv_sec);
    struct a_itimerval cur;
    syscall2(SYS_getitimer, ITIMER_REAL, &cur);
    check("now_disarmed", cur.it_value.tv_sec == 0 && cur.it_value.tv_usec == 0, cur.it_value.tv_sec);
    return finish();
}

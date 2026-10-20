/* timer_gettime: unknown and deleted ids are EINVAL; a bad pointer is
 * EFAULT. */
#include "a_sys.h"

int test_main(void) {
    struct a_itimerspec cur;
    expect("no_timers_einval", syscall2(SYS_timer_gettime, 0, &cur), -EINVAL);
    struct a_sigevent ev;
    memset(&ev, 0, sizeof ev);
    ev.sigev_notify = SIGEV_NONE;
    i32 id;
    syscall3(SYS_timer_create, CLOCK_REALTIME, &ev, &id);
    expect("bad_pointer_efault", syscall2(SYS_timer_gettime, id, (void *)8), -EFAULT);
    expect("other_id_einval", syscall2(SYS_timer_gettime, id + 1, &cur), -EINVAL);
    expect("negative_id_einval", syscall2(SYS_timer_gettime, -1, &cur), -EINVAL);
    syscall1(SYS_timer_delete, id);
    expect("deleted_einval", syscall2(SYS_timer_gettime, id, &cur), -EINVAL);
    return finish();
}

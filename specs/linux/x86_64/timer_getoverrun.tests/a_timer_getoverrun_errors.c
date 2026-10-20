/* timer_getoverrun: unknown and deleted timer ids are EINVAL. */
#include "a_sys.h"

int test_main(void) {
    expect("none_created_einval", syscall1(SYS_timer_getoverrun, 0), -EINVAL);
    struct a_sigevent ev;
    memset(&ev, 0, sizeof ev);
    ev.sigev_notify = SIGEV_NONE;
    i32 id;
    syscall3(SYS_timer_create, CLOCK_MONOTONIC, &ev, &id);
    expect("exists_0", syscall1(SYS_timer_getoverrun, id), 0);
    expect("other_einval", syscall1(SYS_timer_getoverrun, id + 7), -EINVAL);
    expect("negative_einval", syscall1(SYS_timer_getoverrun, -2), -EINVAL);
    syscall1(SYS_timer_delete, id);
    expect("deleted_einval", syscall1(SYS_timer_getoverrun, id), -EINVAL);
    return finish();
}

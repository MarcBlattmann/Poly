/* timer_create: unknown clocks, notify types and signal numbers are
 * EINVAL; SIGEV_THREAD_ID needs a thread of the caller; bad pointers are
 * EFAULT. A failed call leaves no timer behind. (Whether a failed call
 * uses up a timer id is unspecified, so the one successful call that the
 * test passes an id from comes first, and the ids are not looked at after
 * the failures.) */
#include "a_sys.h"

int test_main(void) {
    i32 id = -1;
    struct a_sigevent ev;
    memset(&ev, 0, sizeof ev);
    ev.sigev_notify = SIGEV_THREAD_ID | SIGEV_SIGNAL;
    ev.sigev_signo = SIGUSR1;
    ev.sigev_tid = (i32)syscall0(SYS_gettid);
    expect("thread_id_self_ok", syscall3(SYS_timer_create, CLOCK_MONOTONIC, &ev, &id), 0);
    expect("first_id_0", id, 0);
    expect("delete_it", syscall1(SYS_timer_delete, id), 0);
    ev.sigev_tid = 0x3ffffff0;
    expect("thread_id_unknown_einval", syscall3(SYS_timer_create, CLOCK_MONOTONIC, &ev, &id), -EINVAL);
    memset(&ev, 0, sizeof ev);
    ev.sigev_notify = SIGEV_NONE;
    expect("unknown_clock_einval", syscall3(SYS_timer_create, 100, &ev, &id), -EINVAL);
    expect("monotonic_raw_eopnotsupp", syscall3(SYS_timer_create, CLOCK_MONOTONIC_RAW, &ev, &id), -EOPNOTSUPP);
    ev.sigev_notify = 77;
    expect("unknown_notify_einval", syscall3(SYS_timer_create, CLOCK_MONOTONIC, &ev, &id), -EINVAL);
    ev.sigev_notify = SIGEV_SIGNAL;
    ev.sigev_signo = 0;
    expect("signal_0_einval", syscall3(SYS_timer_create, CLOCK_MONOTONIC, &ev, &id), -EINVAL);
    ev.sigev_signo = 65;
    expect("signal_65_einval", syscall3(SYS_timer_create, CLOCK_MONOTONIC, &ev, &id), -EINVAL);
    ev.sigev_notify = SIGEV_NONE;
    expect("bad_sigevent_efault", syscall3(SYS_timer_create, CLOCK_MONOTONIC, (void *)8, &id), -EFAULT);
    expect("bad_id_pointer_efault", syscall3(SYS_timer_create, CLOCK_MONOTONIC, &ev, (void *)8), -EFAULT);
    /* None of the failed calls made a timer: no id in use answers. */
    int none = 1;
    for (int i = 0; i < 16; i++) none &= syscall1(SYS_timer_getoverrun, i) == -EINVAL;
    check("failed_calls_leave_no_timer", none, 0);
    /* Creating works as before; the timer ends with the process. */
    id = -1;
    expect("create_after_failures", syscall3(SYS_timer_create, CLOCK_MONOTONIC, &ev, &id), 0);
    check("its_id_is_valid", id >= 0, id);
    return finish();
}

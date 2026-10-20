/* timer_create: unknown clocks, notify types and signal numbers are
 * EINVAL; SIGEV_THREAD_ID needs a thread of the caller; bad pointers are
 * EFAULT. */
#include "a_sys.h"

int test_main(void) {
    i32 id;
    struct a_sigevent ev;
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
    ev.sigev_notify = SIGEV_THREAD_ID | SIGEV_SIGNAL;
    ev.sigev_signo = SIGUSR1;
    ev.sigev_tid = 0x3ffffff0;
    expect("thread_id_unknown_einval", syscall3(SYS_timer_create, CLOCK_MONOTONIC, &ev, &id), -EINVAL);
    ev.sigev_tid = (i32)syscall0(SYS_gettid);
    expect("thread_id_self_ok", syscall3(SYS_timer_create, CLOCK_MONOTONIC, &ev, &id), 0);
    syscall1(SYS_timer_delete, id);
    ev.sigev_notify = SIGEV_NONE;
    expect("bad_sigevent_efault", syscall3(SYS_timer_create, CLOCK_MONOTONIC, (void *)8, &id), -EFAULT);
    expect("bad_id_pointer_efault", syscall3(SYS_timer_create, CLOCK_MONOTONIC, &ev, (void *)8), -EFAULT);
    return finish();
}

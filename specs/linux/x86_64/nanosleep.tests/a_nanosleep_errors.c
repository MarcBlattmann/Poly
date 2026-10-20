/* nanosleep: invalid times are EINVAL (nanoseconds outside [0, 1e9),
 * negative seconds); bad pointers are EFAULT; nothing sleeps. */
#include "a_sys.h"

int test_main(void) {
    struct a_timespec t;
    t.tv_sec = 0;
    t.tv_nsec = 1000000000;
    expect("nsec_1e9_einval", syscall2(SYS_nanosleep, &t, 0), -EINVAL);
    t.tv_nsec = -1;
    expect("negative_nsec_einval", syscall2(SYS_nanosleep, &t, 0), -EINVAL);
    t.tv_sec = -1;
    t.tv_nsec = 0;
    expect("negative_sec_einval", syscall2(SYS_nanosleep, &t, 0), -EINVAL);
    expect("bad_req_efault", syscall2(SYS_nanosleep, (void *)8, 0), -EFAULT);
    expect("null_req_efault", syscall2(SYS_nanosleep, (void *)0, 0), -EFAULT);
    /* rem is only written when interrupted: a bad rem is harmless here. */
    t.tv_sec = 0;
    t.tv_nsec = 1000;
    expect("bad_rem_unused", syscall2(SYS_nanosleep, &t, (void *)8), 0);
    return finish();
}

/* clock_nanosleep: unknown clocks are EINVAL; clocks that exist but
 * cannot be slept on (the thread CPU clock, CLOCK_MONOTONIC_RAW) are
 * EOPNOTSUPP; invalid times EINVAL; bad pointers EFAULT. The raw call
 * returns -errno like other calls. */
#include "a_sys.h"

int test_main(void) {
    struct a_timespec ok = {0, 1000}, bad = {0, 1000000000};
    expect("unknown_clock_einval", syscall4(SYS_clock_nanosleep, 100, 0, &ok, 0), -EINVAL);
    expect("thread_cputime_eopnotsupp", syscall4(SYS_clock_nanosleep, CLOCK_THREAD_CPUTIME_ID, 0, &ok, 0),
           -EOPNOTSUPP);
    expect("negative_clock_einval", syscall4(SYS_clock_nanosleep, -1, 0, &ok, 0), -EINVAL);
    expect("bad_nsec_einval", syscall4(SYS_clock_nanosleep, CLOCK_MONOTONIC, 0, &bad, 0), -EINVAL);
    struct a_timespec neg = {-1, 0};
    expect("negative_sec_einval", syscall4(SYS_clock_nanosleep, CLOCK_MONOTONIC, 0, &neg, 0), -EINVAL);
    expect("bad_req_efault", syscall4(SYS_clock_nanosleep, CLOCK_MONOTONIC, 0, (void *)8, 0), -EFAULT);
    expect("raw_monotonic_eopnotsupp", syscall4(SYS_clock_nanosleep, CLOCK_MONOTONIC_RAW, 0, &ok, 0), -EOPNOTSUPP);
    /* Unknown flag bits are ignored. */
    expect("unknown_flags_ignored", syscall4(SYS_clock_nanosleep, CLOCK_MONOTONIC, 0x100, &ok, 0), 0);
    return finish();
}

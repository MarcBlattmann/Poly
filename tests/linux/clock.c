/* clock_gettime: the monotonic clock never goes backwards, values are
 * normalised, the realtime clock is after 2020, bad arguments fail. */
#include "poly_test.h"

struct timespec {
    i64 tv_sec, tv_nsec;
};

static int before_or_equal(struct timespec a, struct timespec b) {
    return a.tv_sec < b.tv_sec || (a.tv_sec == b.tv_sec && a.tv_nsec <= b.tv_nsec);
}

int test_main(void) {
    struct timespec prev, now;
    i64 r = syscall2(SYS_clock_gettime, CLOCK_MONOTONIC, &prev);
    check("clock_monotonic_succeeds", r == 0, r);
    int monotonic = 1, normalised = prev.tv_nsec >= 0 && prev.tv_nsec < 1000000000;
    for (int i = 0; i < 100; i++) {
        syscall2(SYS_clock_gettime, CLOCK_MONOTONIC, &now);
        monotonic &= before_or_equal(prev, now);
        normalised &= now.tv_nsec >= 0 && now.tv_nsec < 1000000000;
        prev = now;
    }
    check("clock_monotonic_never_goes_backwards", monotonic, 0);
    check("clock_nsec_in_range", normalised, 0);

    struct timespec real;
    r = syscall2(SYS_clock_gettime, CLOCK_REALTIME, &real);
    check("clock_realtime_succeeds", r == 0, r);
    check("clock_realtime_after_2020", real.tv_sec > 1577836800, 0);

    r = syscall2(SYS_clock_gettime, 12345, &now);
    check("clock_bad_id_einval", r == -EINVAL, r);
    r = syscall2(SYS_clock_gettime, CLOCK_MONOTONIC, 8);
    check("clock_bad_pointer_efault", r == -EFAULT, r);
    return finish();
}

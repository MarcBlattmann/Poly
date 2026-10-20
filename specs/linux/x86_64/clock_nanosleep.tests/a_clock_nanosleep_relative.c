/* clock_nanosleep with flags 0: a relative sleep on CLOCK_MONOTONIC,
 * CLOCK_REALTIME, CLOCK_BOOTTIME and CLOCK_TAI, at least as long as asked;
 * the result is 0 (the raw call returns errors as -errno). */
#include "a_sys.h"

static i64 now_ns(int clk) {
    struct a_timespec t;
    syscall2(SYS_clock_gettime, clk, &t);
    return t.tv_sec * 1000000000L + t.tv_nsec;
}

int test_main(void) {
    const int clocks[] = {CLOCK_MONOTONIC, CLOCK_REALTIME, CLOCK_BOOTTIME, CLOCK_TAI};
    const char *names[] = {"monotonic", "realtime", "boottime", "tai"};
    for (int i = 0; i < 4; i++) {
        struct a_timespec req = {0, 20000000};
        i64 t0 = now_ns(CLOCK_MONOTONIC);
        i64 r = syscall4(SYS_clock_nanosleep, clocks[i], 0, &req, 0);
        i64 dt = now_ns(CLOCK_MONOTONIC) - t0;
        put(r == 0 && dt >= 20000000 ? "ok " : "not ok ");
        put(names[i]);
        put("_relative_20ms\n");
        if (r == 0 && dt >= 20000000)
            passed++;
        else
            failed++;
    }
    struct a_timespec zero = {0, 0};
    expect("zero_sleep", syscall4(SYS_clock_nanosleep, CLOCK_MONOTONIC, 0, &zero, 0), 0);
    expect("process_cputime_clock_allowed",
           syscall4(SYS_clock_nanosleep, CLOCK_PROCESS_CPUTIME_ID, 0, &zero, 0), 0);
    return finish();
}

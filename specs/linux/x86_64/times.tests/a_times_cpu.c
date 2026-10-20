/* times: about 0.3 s of CPU work shows in tms_utime + tms_stime (in
 * clock ticks of 1/100 s); the returned tick count advances too. */
#include "a_sys.h"

int test_main(void) {
    struct a_tms a, b;
    i64 r1 = syscall1(SYS_times, &a);
    a_spin_ticks(1000000000UL);
    i64 r2 = syscall1(SYS_times, &b);
    check("cpu_ticks_grew", b.tms_utime + b.tms_stime > a.tms_utime + a.tms_stime, b.tms_utime - a.tms_utime);
    check("elapsed_ticks_grew", r2 > r1, r2 - r1);
    check("cpu_not_more_than_elapsed_plus_one", (b.tms_utime + b.tms_stime) - (a.tms_utime + a.tms_stime) <= r2 - r1 + 1,
          r2 - r1);
    return finish();
}

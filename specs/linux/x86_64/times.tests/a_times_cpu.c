/* times: about 0.2 s of CPU work shows in tms_utime + tms_stime (in
 * clock ticks of 1/100 s); the returned tick count advances too. */
#include "a_sys.h"

/* A fixed amount of CPU work: a chain of dependent multiplies, about
 * 75 ms of CPU per 1e8 steps here. A spin for a wall-clock time (the TSC)
 * gets less than one 10 ms tick of CPU when the machine is loaded; this
 * uses about the same CPU time however loaded it is, and makes no system
 * calls. */
static void burn(u64 steps) {
    u64 x = 1;
    for (u64 i = 0; i < steps; i++) {
        x = x * 6364136223846793005UL + 1;
        __asm__ volatile("" : "+r"(x));
    }
}

int test_main(void) {
    struct a_tms a, b;
    i64 r1 = syscall1(SYS_times, &a);
    burn(300000000UL);
    i64 r2 = syscall1(SYS_times, &b);
    check("cpu_ticks_grew", b.tms_utime + b.tms_stime > a.tms_utime + a.tms_stime, b.tms_utime - a.tms_utime);
    check("elapsed_ticks_grew", r2 > r1, r2 - r1);
    check("cpu_not_more_than_elapsed_plus_one", (b.tms_utime + b.tms_stime) - (a.tms_utime + a.tms_stime) <= r2 - r1 + 1,
          r2 - r1);
    return finish();
}

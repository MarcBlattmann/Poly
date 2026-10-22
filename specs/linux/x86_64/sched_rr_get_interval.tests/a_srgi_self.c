/* sched_rr_get_interval(0, tp) for the calling SCHED_OTHER process:
 * returns 0, writes a normalised timespec of less than one second, and
 * gives the same value when asked again. */
#include "a_sys.h"

#define SYS_sched_rr_get_interval 148

struct a_ts { i64 sec, nsec; };

int test_main(void) {
    struct a_ts a = {-7, -7}, b = {-7, -7};
    expect("first_call", syscall2(SYS_sched_rr_get_interval, 0, &a), 0);
    check("sec_zero", a.sec == 0, a.sec);
    check("nsec_range", a.nsec >= 0 && a.nsec < 1000000000, a.nsec);
    expect("second_call", syscall2(SYS_sched_rr_get_interval, 0, &b), 0);
    check("same_sec", b.sec == a.sec, b.sec);
    check("same_nsec", b.nsec == a.nsec, b.nsec);
    return finish();
}

/* sched_rr_get_interval with the caller's own pid or tid instead of 0:
 * succeeds and reports the same interval as pid 0. */
#include "a_sys.h"

#define SYS_sched_rr_get_interval 148

struct a_ts { i64 sec, nsec; };

int test_main(void) {
    struct a_ts z = {-1, -1}, p = {-1, -1}, t = {-1, -1};
    expect("pid0", syscall2(SYS_sched_rr_get_interval, 0, &z), 0);
    expect("getpid", syscall2(SYS_sched_rr_get_interval, syscall0(SYS_getpid), &p), 0);
    expect("gettid", syscall2(SYS_sched_rr_get_interval, syscall0(SYS_gettid), &t), 0);
    check("pid_sec", p.sec == z.sec, p.sec);
    check("pid_nsec", p.nsec == z.nsec, p.nsec);
    check("tid_sec", t.sec == z.sec, t.sec);
    check("tid_nsec", t.nsec == z.nsec, t.nsec);
    return finish();
}

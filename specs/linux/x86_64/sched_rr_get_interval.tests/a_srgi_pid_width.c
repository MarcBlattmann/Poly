/* sched_rr_get_interval reads pid as a 32-bit value: bits 32-63 of the
 * register are ignored, so 0x100000000 and 0x8000000000000000 mean the
 * caller (pid 0) and 0xffffffff00000000|getpid() means the caller's pid. */
#include "a_sys.h"

#define SYS_sched_rr_get_interval 148

struct a_ts { i64 sec, nsec; };

int test_main(void) {
    struct a_ts z = {-1, -1}, a = {-1, -1}, b = {-1, -1}, c = {-1, -1};
    i64 me = syscall0(SYS_getpid);
    expect("pid0", syscall2(SYS_sched_rr_get_interval, 0, &z), 0);
    expect("high_bit32", syscall2(SYS_sched_rr_get_interval, (i64)0x100000000LL, &a), 0);
    check("same_as_pid0", a.sec == z.sec && a.nsec == z.nsec, a.nsec);
    expect("high_ones_pid", syscall2(SYS_sched_rr_get_interval, (i64)(0xffffffff00000000ULL | (u64)me), &b), 0);
    check("same_as_pid0_2", b.sec == z.sec && b.nsec == z.nsec, b.nsec);
    expect("bit63", syscall2(SYS_sched_rr_get_interval, (i64)0x8000000000000000ULL, &c), 0);
    check("same_as_pid0_3", c.sec == z.sec && c.nsec == z.nsec, c.nsec);
    return finish();
}

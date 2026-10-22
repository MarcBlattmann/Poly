/* sched_rr_get_interval for a pid with no task (a reaped child, and the
 * largest positive pid) fails with ESRCH and leaves the timespec untouched. */
#include "a_sys.h"

#define SYS_sched_rr_get_interval 148

struct a_ts { i64 sec, nsec; };

int test_main(void) {
    a_block_sigchld();
    long pid = a_fork_sync();
    if (pid == 0) exit_group(0);
    i32 st;
    expect("reap", syscall4(SYS_wait4, pid, &st, 0, 0), pid);
    struct a_ts t = {0x1234, 0x5678};
    expect("reaped_child", syscall2(SYS_sched_rr_get_interval, pid, &t), -ESRCH);
    check("untouched", t.sec == 0x1234 && t.nsec == 0x5678, t.sec);
    expect("int_max", syscall2(SYS_sched_rr_get_interval, 0x7fffffff, &t), -ESRCH);
    check("untouched2", t.sec == 0x1234 && t.nsec == 0x5678, t.sec);
    return finish();
}

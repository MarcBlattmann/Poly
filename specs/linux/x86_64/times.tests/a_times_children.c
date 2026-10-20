/* times: a reaped child's CPU time appears in tms_cutime + tms_cstime;
 * a zombie's does not yet. */
#include "a_sys.h"

int test_main(void) {
    a_block_sigchld();
    struct a_tms t;
    i64 pid = a_fork_sync();
    if (pid == 0) {
        a_spin_ticks(1000000000UL);
        syscall1(SYS_exit_group, 0);
    }
    syscall1(SYS_times, &t);
    expect("zombie_not_counted", t.tms_cutime + t.tms_cstime, 0);
    i32 st;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    syscall1(SYS_times, &t);
    check("reaped_child_counted", t.tms_cutime + t.tms_cstime > 0, t.tms_cutime);
    return finish();
}

/* times: a reaped child's CPU time appears in tms_cutime + tms_cstime;
 * a zombie's does not yet. */
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
    a_block_sigchld();
    struct a_tms t;
    i64 pid = a_fork_sync();
    if (pid == 0) {
        burn(200000000UL);
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

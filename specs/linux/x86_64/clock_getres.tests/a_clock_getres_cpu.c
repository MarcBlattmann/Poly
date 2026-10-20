/* clock_getres on CPU-time clock ids built from a pid or tid
 * ((~id << 3) | 2 for a process, | 6 for a thread; id 0 means the caller):
 * 1 ns for existing ones, EINVAL for ids that do not exist. (The ids used
 * are constants, so the system call arguments are the same on every run.) */
#include "a_sys.h"

static i64 cpu_clock(i64 pid, int thread) { return (i64)((~(u64)pid << 3) | (thread ? 6 : 2)); }

int test_main(void) {
    struct a_timespec t = {9, 9};
    expect("own_process_clock", syscall2(SYS_clock_getres, cpu_clock(0, 0), &t), 0);
    check("own_process_1ns", t.tv_sec == 0 && t.tv_nsec == 1, t.tv_nsec);
    t.tv_nsec = 9;
    expect("own_thread_clock", syscall2(SYS_clock_getres, cpu_clock(0, 1), &t), 0);
    check("own_thread_1ns", t.tv_sec == 0 && t.tv_nsec == 1, t.tv_nsec);
    expect("missing_process_einval", syscall2(SYS_clock_getres, cpu_clock(0x3ffffff0, 0), &t), -EINVAL);
    expect("missing_thread_einval", syscall2(SYS_clock_getres, cpu_clock(0x3ffffff0, 1), &t), -EINVAL);
    struct a_timespec a, b;
    expect("gettime_own_cpu_clock", syscall2(SYS_clock_gettime, cpu_clock(0, 0), &a), 0);
    check("cpu_time_well_formed", a.tv_nsec >= 0 && a.tv_nsec < 1000000000, a.tv_nsec);
    a_spin_ticks(300000000UL);
    syscall2(SYS_clock_gettime, cpu_clock(0, 0), &b);
    check("cpu_clock_advances_with_work", b.tv_sec * 1000000000L + b.tv_nsec > a.tv_sec * 1000000000L + a.tv_nsec, 0);
    return finish();
}

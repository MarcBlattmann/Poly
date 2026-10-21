/* kill: signal 0 checks that the target exists; a signal sent to the
 * caller's own pid or to its process group (pid 0) is delivered to it;
 * errors for a bad signal number (EINVAL; a missing target is reported
 * first) and a
 * pid or process group that does not exist (ESRCH). */
#include "poly_ipc.h"

static volatile int hits;
static void on_usr1(int sig) {
    (void)sig;
    hits++;
}

int test_main(void) {
    i64 pid = getpid();
    i64 r = syscall2(SYS_kill, pid, 0);
    check("signal_0_to_self", r == 0, r);
    r = syscall2(SYS_kill, 0, 0);
    check("signal_0_to_own_group", r == 0, r);

    sig_handler(SIGUSR1, on_usr1, 0);
    r = syscall2(SYS_kill, pid, SIGUSR1);
    check("kill_self_returns_zero", r == 0, r);
    check("kill_self_delivered", hits == 1, hits);
    r = syscall2(SYS_kill, 0, SIGUSR1);
    check("kill_group_zero_returns_zero", r == 0, r);
    check("kill_group_zero_delivered", hits == 2, hits);

    /* No such process or group. 4194304 is above the largest pid. */
    r = syscall2(SYS_kill, 4194304 + 7, 0);
    check("no_such_pid_esrch", r == -ESRCH, r);
    r = syscall2(SYS_kill, 4194304 + 7, SIGUSR1);
    check("no_such_pid_with_signal_esrch", r == -ESRCH, r);
    r = syscall2(SYS_kill, -(4194304 + 7), SIGUSR1);
    check("no_such_group_esrch", r == -ESRCH, r);
    r = syscall2(SYS_kill, (i64)(int)0x80000000, 0);
    check("int_min_pid_esrch", r == -ESRCH, r);

    /* Bad signal numbers: EINVAL for an existing target; the target is
     * looked up first, so a missing one is ESRCH. */
    r = syscall2(SYS_kill, pid, 65);
    check("signal_65_einval", r == -EINVAL, r);
    r = syscall2(SYS_kill, pid, -1);
    check("signal_negative_einval", r == -EINVAL, r);
    r = syscall2(SYS_kill, 0, 65);
    check("signal_65_to_group_einval", r == -EINVAL, r);
    r = syscall2(SYS_kill, 4194304 + 7, 65);
    check("missing_target_checked_before_signal", r == -ESRCH, r);
    check("errors_deliver_nothing", hits == 2, hits);

    /* The pid argument is an int: the upper half of the register is ignored. */
    r = syscall2(SYS_kill, (i64)(0x100000000UL | (u64)pid), 0);
    check("pid_is_32_bit", r == 0, r);
    /* So is the signal number. */
    r = syscall2(SYS_kill, pid, (i64)(0x100000000UL | SIGUSR1));
    check("signal_is_32_bit", r == 0 && hits == 3, r);

    /* A real-time signal by kill. */
    sig_handler(SIGRTMIN + 5, on_usr1, 0);
    r = syscall2(SYS_kill, pid, SIGRTMIN + 5);
    check("kill_rt_signal", r == 0 && hits == 4, hits);
    return finish();
}

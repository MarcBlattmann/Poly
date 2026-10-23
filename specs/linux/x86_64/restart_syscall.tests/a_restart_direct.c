/* restart_syscall (219) with nothing to restart returns -EINTR: in a fresh
 * process, whatever its argument registers hold, every time it is called,
 * after a nanosleep that completed normally, in and after a handler of a
 * signal that interrupted no system call, and in a new child that shares
 * thread. It changes nothing (the signal mask stays). */
#include "poly_test.h"
#include "poly_ipc.h"

#define SYS_restart_syscall 219

static volatile i64 in_handler = 1;
static void on_usr1(int sig) {
    (void)sig;
    in_handler = syscall0(SYS_restart_syscall);
}

static volatile i64 child_ret = 1;
static int child(void *arg) {
    (void)arg;
    child_ret = syscall0(SYS_restart_syscall);
    return 0;
}

static char child_stack[65536] __attribute__((aligned(16)));
static volatile int child_tid = -1;

int test_main(void) {
    check("fresh_process_eintr", syscall0(SYS_restart_syscall) == -EINTR, syscall0(SYS_restart_syscall));
    i64 r = syscall6(SYS_restart_syscall, 1, 2, 3, 4, 5, 6);
    check("arguments_ignored_eintr", r == -EINTR, r);
    r = syscall6(SYS_restart_syscall, -1, -1, -1, -1, -1, -1);
    check("all_ones_arguments_eintr", r == -EINTR, r);
    int all = 1;
    for (int i = 0; i < 5; i++)
        if (syscall0(SYS_restart_syscall) != -EINTR) all = 0;
    check("repeated_calls_eintr", all, 0);

    struct k_timespec t = { 0, 20000000 };
    r = syscall2(SYS_nanosleep, &t, 0);
    check("nanosleep_completed", r == 0, r);
    r = syscall0(SYS_restart_syscall);
    check("after_completed_nanosleep_eintr", r == -EINTR, r);
    r = syscall3(SYS_poll, 0, 0, 10);
    check("poll_timed_out", r == 0, r);
    r = syscall0(SYS_restart_syscall);
    check("after_completed_poll_eintr", r == -EINTR, r);

    sig_handler(SIGUSR1, on_usr1, 0);
    raise_sig(SIGUSR1);
    check("in_handler_no_interrupted_call_eintr", in_handler == -EINTR, in_handler);
    r = syscall0(SYS_restart_syscall);
    check("after_handler_eintr", r == -EINTR, r);
    check("mask_unchanged", sig_current_mask() == 0, (i64)sig_current_mask());

    /* The thread runs alone while this one spins without system calls. */
    i64 tid = poly_clone(THREAD_FLAGS, child_stack + sizeof child_stack, (int *)&child_tid, (int *)&child_tid, child, 0);
    if (tid > 0)
        while (child_tid != 0) __asm__ volatile("pause" ::: "memory");
    check("thread_started", tid > 0, tid);
    check("new_thread_eintr", child_ret == -EINTR, child_ret);
    return finish();
}

/* rt_sigqueueinfo to another process: a negative si_code other than
 * SI_TKILL (SI_QUEUE) is accepted and the child receives the info as sent;
 * a non-negative si_code (SI_USER, SI_KERNEL) or SI_TKILL is refused with
 * EPERM, even for root.
 * poly-diff: syscalls=off (child process)
 */
#include "poly_ipc.h"

static struct k_siginfo info_for(int signo, int code, u64 value) {
    struct k_siginfo si;
    memset(&si, 0, sizeof si);
    si.si_signo = signo;
    si.si_code = code;
    si.u.kill.pid = (int)getpid();
    si.u.kill.value = value;
    return si;
}

int test_main(void) {
    int ready[2];
    syscall2(SYS_pipe2, ready, 0);
    sig_mask(SIG_BLOCK, SIGBIT(SIGRTMIN + 3), 0);
    i64 pid = fork();
    if (pid == 0) {
        /* Waits for the queued signal and reports its value and code. */
        struct k_siginfo got;
        u64 set = SIGBIT(SIGRTMIN + 3);
        struct k_timespec ts = { 5, 0 };
        char c = 'r';
        write(ready[1], &c, 1);
        i64 s = syscall4(SYS_rt_sigtimedwait, &set, &got, &ts, 8);
        if (s != SIGRTMIN + 3) exit_group(1);
        if (got.si_code != SI_QUEUE) exit_group(2);
        if (got.u.kill.value != 0xbeef) exit_group(3);
        if (got.u.kill.pid != (int)syscall0(SYS_getppid)) exit_group(4);
        exit_group(0);
    }
    char c;
    read(ready[0], &c, 1);
    struct k_siginfo si = info_for(SIGRTMIN + 3, SI_USER, 1);
    i64 r = syscall3(SYS_rt_sigqueueinfo, pid, SIGRTMIN + 3, &si);
    check("other_process_si_user_eperm", r == -EPERM, r);
    si = info_for(SIGRTMIN + 3, SI_KERNEL, 1);
    r = syscall3(SYS_rt_sigqueueinfo, pid, SIGRTMIN + 3, &si);
    check("other_process_si_kernel_eperm", r == -EPERM, r);
    si = info_for(SIGRTMIN + 3, SI_TKILL, 1);
    r = syscall3(SYS_rt_sigqueueinfo, pid, SIGRTMIN + 3, &si);
    check("other_process_si_tkill_eperm", r == -EPERM, r);
    si = info_for(SIGRTMIN + 3, SI_QUEUE, 0xbeef);
    r = syscall3(SYS_rt_sigqueueinfo, pid, SIGRTMIN + 3, &si);
    check("other_process_si_queue_ok", r == 0, r);
    int code = wait_exit(pid);
    check("child_got_queued_info", code == 0, code);
    return finish();
}

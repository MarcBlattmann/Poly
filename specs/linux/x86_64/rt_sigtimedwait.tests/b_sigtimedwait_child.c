/* rt_sigtimedwait in the parent, the signal sent by a child: the wait
 * returns it with the child's pid and uid in siginfo; a queued real-time
 * signal from the child keeps its value (sigqueue). With a NULL timeout it
 * waits as long as needed.
 * poly-diff: syscalls=off (two processes)
 */
#include "poly_ipc.h"

int test_main(void) {
    sig_mask(SIG_SETMASK, SIGBIT(SIGUSR2) | SIGBIT(SIGRTMIN + 1), 0);
    i64 parent = getpid();
    int go[2];
    syscall2(SYS_pipe2, go, 0);
    i64 pid = fork();
    if (pid == 0) {
        char c;
        syscall2(SYS_kill, parent, SIGUSR2);
        read(go[0], &c, 1);
        struct k_siginfo si;
        memset(&si, 0, sizeof si);
        si.si_signo = SIGRTMIN + 1;
        si.si_code = SI_QUEUE;
        si.u.kill.pid = (int)getpid();
        si.u.kill.value = 9876;
        syscall3(SYS_rt_sigqueueinfo, parent, SIGRTMIN + 1, &si);
        exit_group(0);
    }
    u64 set = SIGBIT(SIGUSR2);
    struct k_siginfo info;
    memset(&info, 0, sizeof info);
    i64 r = syscall4(SYS_rt_sigtimedwait, &set, &info, 0, 8);
    check("got_sigusr2", r == SIGUSR2, r);
    check("sender_pid_is_child", info.u.kill.pid == pid, 0);
    check("sender_uid", info.u.kill.uid == syscall0(SYS_getuid), info.u.kill.uid);
    check("code_user", info.si_code == SI_USER, info.si_code);

    write(go[1], "x", 1);
    set = SIGBIT(SIGRTMIN + 1);
    struct k_timespec five = { 5, 0 };
    memset(&info, 0, sizeof info);
    r = syscall4(SYS_rt_sigtimedwait, &set, &info, &five, 8);
    check("got_queued_rt", r == SIGRTMIN + 1, r);
    check("queued_value", info.u.kill.value == 9876, (i64)info.u.kill.value);
    check("queued_code", info.si_code == SI_QUEUE, info.si_code);
    check("child_exit", wait_exit(pid) == 0, 0);
    return finish();
}

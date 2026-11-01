/* With a handler for SIGCHLD, the end of a child wakes a parent sleeping
 * in pause: pause returns EINTR (4) after the handler ran, and the child
 * can then be reaped. Signals sent by another process count as well: a
 * second child sends SIGUSR1 (handled) to the parent, which wakes from
 * pause the same way.
 * poly-diff: syscalls=off (two processes run at once)
 */
#include "a_sys.h"

#define SYS_pause 34

void ld_restorer(void);
__asm__(".text\n"
        ".global ld_restorer\n"
        "ld_restorer:\n"
        "    mov $15, %eax\n"
        "    syscall\n"
        "    hlt\n");

static volatile i32 chld, usr1;
static void on_chld(int s) {
    (void)s;
    chld++;
}
static void on_usr1(int s) {
    (void)s;
    usr1++;
}

int test_main(void) {
    struct a_sigaction sa = {(u64)on_chld, SA_RESTORER, (u64)ld_restorer, 0};
    syscall4(SYS_rt_sigaction, SIGCHLD, &sa, 0, 8);
    sa.handler = (u64)on_usr1;
    syscall4(SYS_rt_sigaction, SIGUSR1, &sa, 0, 8);

    i64 pid = syscall5(SYS_clone, SIGCHLD, 0, 0, 0, 0);
    if (pid == 0) {
        struct a_timespec t = {0, 200000000};
        syscall2(SYS_nanosleep, &t, 0);
        syscall1(SYS_exit_group, 5);
    }
    expect("pause_eintr_on_sigchld", syscall0(SYS_pause), -EINTR);
    expect("sigchld_handler_ran", chld, 1);
    i32 st = -1;
    expect("reaped", syscall4(SYS_wait4, pid, &st, 0, 0), pid);
    expect("child_status", A_WEXITSTATUS(st), 5);

    i64 parent = syscall0(SYS_getpid);
    pid = syscall5(SYS_clone, SIGCHLD, 0, 0, 0, 0);
    if (pid == 0) {
        struct a_timespec t = {0, 200000000};
        syscall2(SYS_nanosleep, &t, 0);
        syscall2(SYS_kill, parent, SIGUSR1);
        syscall1(SYS_exit_group, 0);
    }
    /* SIGCHLD of this child must not be what wakes the parent. */
    u64 blk = A_SIGBIT(SIGCHLD);
    syscall4(SYS_rt_sigprocmask, SIG_BLOCK, &blk, 0, 8);
    expect("pause_eintr_on_sigusr1", syscall0(SYS_pause), -EINTR);
    expect("usr1_handler_ran", usr1, 1);
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("second_child_status", st, 0);
    return finish();
}

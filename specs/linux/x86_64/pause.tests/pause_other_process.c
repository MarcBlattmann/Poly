/* pause: a signal from another process with a handler ends pause with
 * EINTR. The parent sends SIGUSR1 to the pausing child until the child's
 * pause has returned (a signal that came before pause was entered is
 * handled then, and pause waits for the next). The SA_SIGINFO handler
 * sees SI_USER and the parent as sender.
 * poly-diff: syscalls=off (two processes, the number of sends varies)
 */
#include "poly_ipc.h"

static volatile int hits, code, sender;

static void on_info(int sig, struct k_siginfo *info, void *ctx) {
    (void)sig;
    (void)ctx;
    hits++;
    code = info->si_code;
    sender = info->u.kill.pid;
}

int test_main(void) {
    volatile i64 *sh = shared_page(4096);
    i64 parent = getpid();
    i64 pid = fork();
    if (pid == 0) {
        sig_action(SIGUSR1, (u64)on_info, SA_SIGINFO, 0);
        __atomic_store_n(&sh[0], 1, __ATOMIC_SEQ_CST);
        i64 r = syscall0(SYS_pause);
        sh[1] = r;
        sh[2] = hits;
        sh[3] = code;
        sh[4] = sender == parent;
        __atomic_store_n(&sh[5], 1, __ATOMIC_SEQ_CST);
        exit_group(0);
    }
    while (__atomic_load_n(&sh[0], __ATOMIC_SEQ_CST) == 0) sleep_ms(1);
    for (int i = 0; i < 500 && __atomic_load_n(&sh[5], __ATOMIC_SEQ_CST) == 0; i++) {
        syscall2(SYS_kill, pid, SIGUSR1);
        sleep_ms(10);
    }
    check("child_exit_0", wait_exit(pid) == 0, 0);
    check("returns_eintr", sh[1] == -EINTR, sh[1]);
    check("handler_ran", sh[2] >= 1, sh[2]);
    check("si_code_si_user", sh[3] == 0, sh[3]);
    check("sender_is_parent", sh[4] == 1, sh[4]);
    return finish();
}

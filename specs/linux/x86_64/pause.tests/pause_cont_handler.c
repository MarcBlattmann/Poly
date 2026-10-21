/* pause: SIGCONT with a handler installed ends pause with EINTR, also
 * when it continues a stopped process. The parent stops the pausing child
 * with SIGSTOP, then sends SIGCONT until the child's pause has returned.
 * poly-diff: syscalls=off (two processes, the number of sends varies)
 */
#include "poly_ipc.h"

#define WUNTRACED 2

static volatile int got, hits;

static void on_cont(int sig) {
    got = sig;
    hits++;
}

int test_main(void) {
    volatile i64 *sh = shared_page(4096);
    i64 pid = fork();
    if (pid == 0) {
        sig_handler(SIGCONT, on_cont, 0);
        __atomic_store_n(&sh[0], 1, __ATOMIC_SEQ_CST);
        i64 r = syscall0(SYS_pause);
        sh[1] = r;
        sh[2] = got;
        sh[3] = hits;
        __atomic_store_n(&sh[5], 1, __ATOMIC_SEQ_CST);
        exit_group(0);
    }
    while (__atomic_load_n(&sh[0], __ATOMIC_SEQ_CST) == 0) sleep_ms(1);
    sleep_ms(50);
    syscall2(SYS_kill, pid, SIGSTOP);
    int st = 0;
    i64 w = syscall4(SYS_wait4, pid, &st, WUNTRACED, 0);
    check("child_stopped", w == pid && (st & 0xff) == 0x7f && ((st >> 8) & 0xff) == SIGSTOP, st);
    for (int i = 0; i < 500 && __atomic_load_n(&sh[5], __ATOMIC_SEQ_CST) == 0; i++) {
        syscall2(SYS_kill, pid, SIGCONT);
        sleep_ms(10);
    }
    check("child_exit_0", wait_exit(pid) == 0, 0);
    check("returns_eintr", sh[1] == -EINTR, sh[1]);
    check("ended_by_sigcont_handler", sh[2] == SIGCONT && sh[3] >= 1, sh[2]);
    return finish();
}

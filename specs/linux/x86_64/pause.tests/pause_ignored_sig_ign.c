/* pause: signals whose action is SIG_IGN do not end pause. A child sets
 * SIGUSR2, SIGTERM and SIGINT to SIG_IGN and pauses with a 400 ms
 * SIGALRM timer; the parent sends it the ignored signals while it waits.
 * pause ends only by SIGALRM, after the timer.
 * poly-diff: syscalls=off (two processes)
 */
#include "poly_ipc.h"

struct itv {
    struct k_timeval interval;
    struct k_timeval value;
};

static volatile int got, hits;

static void on_alrm(int sig) {
    got = sig;
    hits++;
}

int test_main(void) {
    volatile i64 *sh = shared_page(4096);
    i64 pid = fork();
    if (pid == 0) {
        sig_action(SIGUSR2, SIG_IGN, 0, 0);
        sig_action(SIGTERM, SIG_IGN, 0, 0);
        sig_action(SIGINT, SIG_IGN, 0, 0);
        sig_handler(SIGALRM, on_alrm, 0);
        struct itv t = { { 0, 0 }, { 0, 400000 } };
        i64 start = now_ns();
        syscall3(SYS_setitimer, 0, &t, 0);
        __atomic_store_n(&sh[0], 1, __ATOMIC_SEQ_CST);
        i64 r = syscall0(SYS_pause);
        sh[1] = r;
        sh[2] = got;
        sh[3] = hits;
        sh[4] = (now_ns() - start) / 1000000;
        exit_group(0);
    }
    while (__atomic_load_n(&sh[0], __ATOMIC_SEQ_CST) == 0) sleep_ms(1);
    sleep_ms(50);
    for (int i = 0; i < 3; i++) {
        syscall2(SYS_kill, pid, SIGUSR2);
        syscall2(SYS_kill, pid, SIGTERM);
        syscall2(SYS_kill, pid, SIGINT);
        sleep_ms(20);
    }
    check("child_exit_0", wait_exit(pid) == 0, 0);
    check("returns_eintr", sh[1] == -EINTR, sh[1]);
    check("ended_by_sigalrm", sh[2] == SIGALRM && sh[3] == 1, sh[2]);
    check("ignored_signals_did_not_end_it", sh[4] >= 380, sh[4]);
    return finish();
}

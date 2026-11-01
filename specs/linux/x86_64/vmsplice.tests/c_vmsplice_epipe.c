/* vmsplice(2) into a pipe without readers is EPIPE and raises SIGPIPE
 * once; at SIGPIPE's default disposition the process is killed. The read
 * end of a pipe whose writers are gone gives 0 (no signal).
 * poly-diff: syscalls=off (a child process)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_vmsplice 278

static i64 vmsplice(i64 fd, const struct k_iovec *iov, u64 n, u64 flags) {
    return syscall4(SYS_vmsplice, fd, iov, n, flags);
}
static void expect(const char *n, i64 got, i64 want) { check(n, got == want, got); }
static volatile int sigs;
static void on_pipe(int s) {
    (void)s;
    sigs++;
}
static char msg[] = "abc";

int test_main(void) {
    int w[2];
    expect("pipe", syscall1(SYS_pipe, w), 0);
    close(w[0]);
    struct k_iovec v = { msg, 3 };

    i64 pid = fork();
    if (pid == 0) {
        vmsplice(w[1], &v, 1, 0);
        syscall1(SYS_exit, 0);
    }
    int st = wait_status(pid);
    check("default_killed_by_sigpipe", W_SIGNALED(st) && W_TERMSIG(st) == SIGPIPE, st);

    expect("handler", sig_handler(SIGPIPE, on_pipe, 0), 0);
    sigs = 0;
    expect("epipe", vmsplice(w[1], &v, 1, 0), -EPIPE);
    expect("one_signal", sigs, 1);
    sigs = 0;
    expect("epipe_nonblock_flag", vmsplice(w[1], &v, 1, 2), -EPIPE);
    expect("one_signal_2", sigs, 1);
    expect("ignore", sig_action(SIGPIPE, SIG_IGN, 0, 0), 0);
    expect("ignored_epipe", vmsplice(w[1], &v, 1, 0), -EPIPE);
    close(w[1]);

    int r[2];
    expect("pipe_r", syscall1(SYS_pipe, r), 0);
    close(r[1]);
    char a[8];
    struct k_iovec rv = { a, 8 };
    expect("handler_again", sig_handler(SIGPIPE, on_pipe, 0), 0);
    sigs = 0;
    expect("read_end_no_writers_0", vmsplice(r[0], &rv, 1, 0), 0);
    expect("read_end_no_signal", sigs, 0);
    close(r[0]);
    return finish();
}

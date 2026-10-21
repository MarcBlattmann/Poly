/* End of file and broken pipes: once every write end is closed, reads
 * return the remaining bytes and then 0; while a duplicate of the write
 * end is open there is no EOF (a nonblocking read is EAGAIN). Writing
 * when every read end is closed raises SIGPIPE and fails with EPIPE (with
 * SIGPIPE ignored or handled the process survives; the handler runs before
 * write returns). */
#include "poly_ipc.h"

static volatile int pipes;
static void on_pipe(int sig) {
    (void)sig;
    pipes++;
}

int test_main(void) {
    int p[2];
    char buf[16];
    syscall2(SYS_pipe2, p, O_NONBLOCK);
    int dupw = (int)syscall1(SYS_dup, p[1]);
    write(p[1], "abc", 3);
    close(p[1]);
    i64 r = read(p[0], buf, sizeof buf);
    check("data_before_eof", r == 3, r);
    r = read(p[0], buf, sizeof buf);
    check("duplicate_writer_keeps_pipe_open", r == -EAGAIN, r);
    close(dupw);
    r = read(p[0], buf, sizeof buf);
    check("eof_after_last_writer_closed", r == 0, r);
    r = read(p[0], buf, sizeof buf);
    check("eof_repeats", r == 0, r);
    close(p[0]);

    /* Blocking read with no writer: returns 0 at once. */
    syscall2(SYS_pipe2, p, 0);
    close(p[1]);
    r = read(p[0], buf, sizeof buf);
    check("blocking_read_no_writer_zero", r == 0, r);
    close(p[0]);

    /* Broken pipe with a handler. */
    sig_handler(SIGPIPE, on_pipe, 0);
    syscall2(SYS_pipe2, p, 0);
    close(p[0]);
    r = write(p[1], "x", 1);
    check("write_no_reader_epipe", r == -EPIPE, r);
    check("sigpipe_raised", pipes == 1, pipes);
    r = write(p[1], buf, 0);
    check("zero_write_no_reader_returns_zero", r == 0 && pipes == 1, r);
    int before = pipes;

    /* Ignored SIGPIPE: still EPIPE. */
    sig_action(SIGPIPE, SIG_IGN, 0, 0);
    r = write(p[1], "x", 1);
    check("ignored_sigpipe_still_epipe", r == -EPIPE, r);
    check("ignored_sigpipe_not_pending", sig_pending() == 0, (i64)sig_pending());

    /* Blocked SIGPIPE: EPIPE, and the signal is left pending. */
    sig_handler(SIGPIPE, on_pipe, 0);
    sig_mask(SIG_BLOCK, SIGBIT(SIGPIPE), 0);
    r = write(p[1], "x", 1);
    check("blocked_sigpipe_epipe", r == -EPIPE, r);
    check("blocked_sigpipe_pending", sig_pending() == SIGBIT(SIGPIPE), (i64)sig_pending());
    sig_mask(SIG_UNBLOCK, SIGBIT(SIGPIPE), 0);
    check("blocked_sigpipe_delivered_on_unblock", pipes == before + 1, pipes - before);
    close(p[1]);
    return finish();
}

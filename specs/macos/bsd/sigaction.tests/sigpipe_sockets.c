#include "poly_mtest.h"
/* sigaction, BSD 46, SIGPIPE: a write to a pipe whose read end is closed fails with EPIPE (32) and sends
 * SIGPIPE to the writer: with a handler it runs once, with SIG_IGN nothing runs and the write still fails with
 * EPIPE. The same for a socket pair whose peer closed. With the socket option SO_NOSIGPIPE (0x1022 at
 * level SOL_SOCKET 0xffff, setsockopt BSD 105, value 1) the write still fails with EPIPE but no signal is
 * sent; the option on a pipe is ENOTSOCK (38). A write to a socket pair whose peer is open does not fail. The
 * default action of SIGPIPE kills the process (wait status 13). */
#define SYS_socketpair 135
#define SYS_setsockopt 105
#define SYS_getsockopt 118
#define SOL_SOCKET 0xffff
#define SO_NOSIGPIPE 0x1022
#define ENOTSOCK 38
static volatile i64 runs;
static void h(int sig, void *info, void *uc) { (void)sig; (void)info; (void)uc; runs++; }
int test_main(void) {
    sysret p = bsd0(SYS_pipe);
    bsd1(SYS_close, p.ret);
    poly_sigaction(SIGPIPE, (void *)h, 0, SA_SIGINFO, 0);
    sysret r = bsd3(SYS_write, p.ret2, "x", 1);
    check_err("a write to a pipe without reader is EPIPE", r, EPIPE);
    check("and the handler ran once", runs == 1, runs);
    poly_sigaction(SIGPIPE, SIG_IGN, 0, 0, 0);
    runs = 0;
    sysret p2 = bsd0(SYS_pipe);
    bsd1(SYS_close, p2.ret);
    r = bsd3(SYS_write, p2.ret2, "x", 1);
    check_err("with SIGPIPE ignored: still EPIPE", r, EPIPE);
    check("nothing ran", runs == 0, runs);
    poly_sigaction(SIGPIPE, (void *)h, 0, SA_SIGINFO, 0);
    int sv[2] = {0, 0};
    check_ok("socketpair", bsd4(SYS_socketpair, 1, 1, 0, sv));
    r = bsd3(SYS_write, sv[0], "ok", 2);
    check("a write with the peer open works", r.err == 0 && r.ret == 2, r.err);
    bsd1(SYS_close, sv[1]);
    runs = 0;
    r = bsd3(SYS_write, sv[0], "x", 1);
    check_err("a write to a socket whose peer closed is EPIPE", r, EPIPE);
    check("and SIGPIPE ran the handler once", runs == 1, runs);
    int sv2[2] = {0, 0};
    bsd4(SYS_socketpair, 1, 1, 0, sv2);
    i32 one = 1;
    r = bsd6(SYS_setsockopt, sv2[0], SOL_SOCKET, SO_NOSIGPIPE, &one, 4, 0);
    check_ok("SO_NOSIGPIPE", r);
    i32 got = 0, len = 4;
    r = bsd6(SYS_getsockopt, sv2[0], SOL_SOCKET, SO_NOSIGPIPE, &got, &len, 0);
    check("reads back as 1", r.err == 0 && got == 1, r.err ? r.err : got);
    bsd1(SYS_close, sv2[1]);
    runs = 0;
    r = bsd3(SYS_write, sv2[0], "x", 1);
    check_err("with SO_NOSIGPIPE the write is still EPIPE", r, EPIPE);
    check("but no signal is sent", runs == 0, runs);
    r = bsd6(SYS_setsockopt, p2.ret2, SOL_SOCKET, SO_NOSIGPIPE, &one, 4, 0);
    check_err("the option on a pipe is ENOTSOCK", r, ENOTSOCK);
    poly_sigaction(SIGPIPE, SIG_DFL, 0, 0, 0);
    sysret f = bsd0(SYS_fork);
    if (f.ret2 == 1) {
        sysret q = bsd0(SYS_pipe);
        bsd1(SYS_close, q.ret);
        bsd3(SYS_write, q.ret2, "x", 1);
        poly_exit(0);
    }
    int st = poly_wait(f.ret);
    check("by default SIGPIPE kills the writer (status 13)", !W_EXITED(st) && W_TERMSIG(st) == SIGPIPE, st);
    return finish();
}

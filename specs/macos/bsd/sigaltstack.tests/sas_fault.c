#include "poly_mtest.h"
/* sigaltstack, BSD 53: a fault handled on the alternate stack. A child with an alternate stack and a
 * SIGSEGV handler with SA_ONSTACK (and SA_SIGINFO) survives a stack overflow: it recurses until the
 * normal stack is exhausted, the handler runs on the alternate stack and the child exits with 77.
 * Without the alternate stack the same program is killed (by SIGSEGV or SIGBUS, here recorded as a
 * value; the handler cannot run). */
#define SYS_sigaltstack 53
#define SA_ONSTACK 1
struct ss { void *sp; size_t size; i32 flags; i32 pad; };
static char stk[65536] __attribute__((aligned(16)));
static void handler(int sig, void *info, void *uc) {
    (void)sig; (void)info; (void)uc;
    poly_exit(77);
}
static volatile int stop_recursing;
static __attribute__((noinline)) int recurse(volatile char *p) {
    if (stop_recursing) return 0;
    volatile char pad[512];
    pad[0] = p ? p[0] : 1;
    pad[511] = pad[0];
    return recurse(pad) + pad[3];
}
int test_main(void) {
    sysret f = bsd0(SYS_fork);
    if (f.ret2 == 1) {
        struct ss n = {stk, sizeof stk, 0, 0};
        bsd2(SYS_sigaltstack, &n, 0);
        poly_sigaction(SIGSEGV, (void *)handler, 0, SA_SIGINFO | SA_ONSTACK, 0);
        poly_sigaction(10, (void *)handler, 0, SA_SIGINFO | SA_ONSTACK, 0);
        poly_exit(recurse(0));
    }
    int st = poly_wait(f.ret);
    check("with an alternate stack the handler runs after the overflow: exit 77", W_EXITED(st) && W_EXITSTATUS(st) == 77, st);
    f = bsd0(SYS_fork);
    if (f.ret2 == 1) {
        poly_sigaction(SIGSEGV, (void *)handler, 0, SA_SIGINFO | SA_ONSTACK, 0);
        poly_sigaction(10, (void *)handler, 0, SA_SIGINFO | SA_ONSTACK, 0);
        poly_exit(recurse(0));
    }
    st = poly_wait(f.ret);
    check("without one the child is killed by a signal", !W_EXITED(st) && W_TERMSIG(st) != 0, st);
    value("the signal that kills it", W_TERMSIG(st));
    return finish();
}

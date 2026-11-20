#include "poly_mtest.h"
/* sigaltstack, BSD 53, with signal delivery: a handler installed with SA_ONSTACK (1) runs on the
 * alternate stack (its locals lie inside it) and sees SS_ONSTACK (1) in the stack's flags; a handler
 * without SA_ONSTACK runs on the normal stack and sees flags 0 even when a stack is set; changing the
 * stack from inside the handler that runs on it is EPERM (1); after the handler the flags are 0
 * again; with the stack disabled SA_ONSTACK handlers run on the normal stack. */
#define SYS_sigaltstack 53
#define SS_DISABLE 4
#define SA_ONSTACK 1
struct ss { void *sp; size_t size; i32 flags; i32 pad; };
static char stk[65536] __attribute__((aligned(16)));
static volatile i64 in_range, flags_in, change_err, runs;
static void h(int sig, void *info, void *uc) {
    (void)sig; (void)info; (void)uc;
    char here;
    in_range = (&here >= stk && &here < stk + sizeof stk);
    struct ss cur;
    bsd2(SYS_sigaltstack, 0, &cur);
    flags_in = cur.flags;
    struct ss nw = {stk, sizeof stk, 0, 0};
    change_err = bsd2(SYS_sigaltstack, &nw, 0).err;
    runs++;
}
int test_main(void) {
    i64 me = bsd0(SYS_getpid).ret;
    struct ss n = {stk, sizeof stk, 0, 0}, o;
    bsd2(SYS_sigaltstack, &n, 0);
    poly_sigaction(SIGUSR1, (void *)h, 0, SA_SIGINFO, 0);
    bsd2(SYS_kill, me, SIGUSR1);
    check("a handler without SA_ONSTACK runs on the normal stack", runs == 1 && in_range == 0, in_range);
    check("and sees flags 0", flags_in == 0, flags_in);
    check("changing the stack there is allowed (not on it)", change_err == 0, change_err);
    poly_sigaction(SIGUSR1, (void *)h, 0, SA_SIGINFO | SA_ONSTACK, 0);
    bsd2(SYS_kill, me, SIGUSR1);
    check("a handler with SA_ONSTACK runs on the alternate stack", runs == 2 && in_range == 1, in_range);
    check("and sees SS_ONSTACK", flags_in == 1, flags_in);
    check("changing the stack from inside it is EPERM", change_err == EPERM, change_err);
    memset(&o, 0x55, sizeof o);
    bsd2(SYS_sigaltstack, 0, &o);
    check("after the handler the flags are 0 again", o.flags == 0, o.flags);
    n.flags = SS_DISABLE;
    bsd2(SYS_sigaltstack, &n, 0);
    bsd2(SYS_kill, me, SIGUSR1);
    check("with the stack disabled, SA_ONSTACK handlers use the normal stack", runs == 3 && in_range == 0, in_range);
    return finish();
}

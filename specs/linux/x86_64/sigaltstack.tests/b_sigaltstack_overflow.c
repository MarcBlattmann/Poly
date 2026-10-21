/* The alternate stack's main use: a stack overflow raises SIGSEGV, and a
 * handler with SA_ONSTACK can run on the alternate stack (the child exits
 * from it with a code); without an alternate stack the overflow kills the
 * child with SIGSEGV. rt_sigreturn also restores the alternate stack from
 * uc_stack: a handler that edits it changes the stack afterwards. The
 * alternate stack is inherited across fork.
 * poly-diff: syscalls=off (child processes)
 */
#include "poly_ipc.h"

static char alt[65536] __attribute__((aligned(16)));
static char alt2[32768] __attribute__((aligned(16)));

static void on_segv(int sig) {
    (void)sig;
    exit_group(42);
}

static volatile int limit = 1 << 30; /* never reached: the stack runs out first */

static int recurse(volatile char *prev, int depth) {
    volatile char buf[4096];
    buf[0] = (char)depth;
    buf[4095] = prev ? prev[0] : 0;
    if (depth >= limit) return 0;
    return recurse(buf, depth + 1) + buf[100];
}

static void edit_stack(int sig, struct k_siginfo *info, void *ctx) {
    struct k_ucontext *uc = ctx;
    (void)sig;
    (void)info;
    uc->uc_stack.ss_sp = (u64)alt2;
    uc->uc_stack.ss_size = sizeof alt2;
    uc->uc_stack.ss_flags = 0;
}

int test_main(void) {
    i64 pid = fork();
    if (pid == 0) {
        struct k_stack ss = { (u64)alt, 0, 0, sizeof alt };
        syscall2(SYS_sigaltstack, &ss, 0);
        sig_handler(SIGSEGV, on_segv, SA_ONSTACK);
        exit_group(recurse(0, 0));
    }
    check("overflow_handled_on_alternate_stack", wait_exit(pid) == 42, 0);

    pid = fork();
    if (pid == 0) {
        sig_handler(SIGSEGV, on_segv, SA_ONSTACK); /* no alternate stack set */
        exit_group(recurse(0, 0));
    }
    int st = wait_status(pid);
    check("overflow_without_alternate_stack_kills", W_SIGNALED(st) && W_TERMSIG(st) == SIGSEGV, st);

    /* Inherited across fork. */
    struct k_stack ss = { (u64)alt, 0, 0, sizeof alt };
    syscall2(SYS_sigaltstack, &ss, 0);
    pid = fork();
    if (pid == 0) {
        struct k_stack q;
        syscall2(SYS_sigaltstack, 0, &q);
        exit_group(q.ss_sp == (u64)alt && q.ss_size == sizeof alt && q.ss_flags == 0 ? 0 : 1);
    }
    check("alternate_stack_inherited", wait_exit(pid) == 0, 0);

    /* rt_sigreturn takes the alternate stack from uc_stack. */
    sig_action(SIGUSR1, (u64)edit_stack, SA_SIGINFO, 0);
    raise_sig(SIGUSR1);
    struct k_stack q;
    syscall2(SYS_sigaltstack, 0, &q);
    check("sigreturn_restores_edited_uc_stack", q.ss_sp == (u64)alt2 && q.ss_size == sizeof alt2, (i64)q.ss_size);
    return finish();
}

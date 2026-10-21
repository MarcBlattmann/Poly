/* Handlers on the alternate stack: with SA_ONSTACK the handler runs inside
 * the alternate stack, sigaltstack there reports SS_ONSTACK and refuses a
 * change (EPERM), and uc_stack holds the alternate stack; a handler
 * without SA_ONSTACK runs on the normal stack; a nested SA_ONSTACK signal
 * stays on the alternate stack below the first frame. A stack overflow's
 * SIGSEGV can be handled there. */
#include "poly_ipc.h"

static char alt[65536] __attribute__((aligned(16)));

static volatile int on_alt, nested_on_alt, nested_below, query_flags, change_result, uc_ok, plain_on_alt;
static volatile u64 outer_sp;

static int in_alt(u64 p) { return p >= (u64)alt && p < (u64)alt + sizeof alt; }

static void nested(int sig) {
    volatile char here;
    (void)sig;
    nested_on_alt = in_alt((u64)&here);
    nested_below = (u64)&here < outer_sp;
}

static void on_usr1(int sig, struct k_siginfo *info, void *ctx) {
    struct k_ucontext *uc = ctx;
    volatile char here;
    (void)sig;
    (void)info;
    on_alt = in_alt((u64)&here);
    outer_sp = (u64)&here;
    struct k_stack q;
    syscall2(SYS_sigaltstack, 0, &q);
    query_flags = q.ss_flags;
    struct k_stack other = { (u64)alt, 0, 0, 8192 };
    change_result = (int)syscall2(SYS_sigaltstack, &other, 0);
    uc_ok = uc->uc_stack.ss_sp == (u64)alt && uc->uc_stack.ss_size == sizeof alt && uc->uc_stack.ss_flags == 0;
    raise_sig(SIGUSR2);
}

static void plain(int sig) {
    volatile char here;
    (void)sig;
    plain_on_alt = in_alt((u64)&here);
}

int test_main(void) {
    struct k_stack ss = { (u64)alt, 0, 0, sizeof alt };
    syscall2(SYS_sigaltstack, &ss, 0);
    sig_action(SIGUSR1, (u64)on_usr1, SA_SIGINFO | SA_ONSTACK, 0);
    sig_handler(SIGUSR2, nested, SA_ONSTACK);
    sig_handler(SIGTERM, plain, 0);

    raise_sig(SIGUSR1);
    check("handler_on_alternate_stack", on_alt, 0);
    check("query_inside_reports_onstack", query_flags == SS_ONSTACK, query_flags);
    check("change_inside_eperm", change_result == -EPERM, change_result);
    check("uc_stack_is_alternate_stack", uc_ok, 0);
    check("nested_handler_on_alternate_stack", nested_on_alt, 0);
    check("nested_frame_below_first", nested_below, 0);

    struct k_stack q;
    syscall2(SYS_sigaltstack, 0, &q);
    check("after_handler_flags_zero", q.ss_flags == 0, q.ss_flags);
    check("stack_unchanged_by_refused_change", q.ss_size == sizeof alt, (i64)q.ss_size);

    raise_sig(SIGTERM);
    check("handler_without_onstack_on_normal_stack", plain_on_alt == 0, plain_on_alt);
    return finish();
}

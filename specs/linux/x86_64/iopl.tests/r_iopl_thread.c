/* The I/O privilege level is per thread. A thread created after iopl(3)
 * inherits the level (it has port access); a thread that calls iopl(0)
 * loses its access while the main thread keeps it; a thread that calls
 * iopl(3) while the main thread is at 0 gets access, the main thread
 * still has none.
 * poly-diff: needs=root (raising the I/O privilege level needs CAP_SYS_RAWIO)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_iopl 172

static volatile int g_sig;
static volatile int g_done;
static char g_stack[65536] __attribute__((aligned(16)));
static volatile int g_result;

static void on_segv(int sig, struct k_siginfo *si, struct k_ucontext *uc) {
    (void)si;
    g_sig = sig;
    uc->gregs[UC_RIP] += 2;
}

static int faults(void) {
    g_sig = 0;
    __asm__ volatile("inb $0x80, %%al" ::: "rax", "memory");
    return g_sig == SIGSEGV;
}

/* arg 0: report access, then iopl(0) and report again.
 * arg 1: iopl(3), report access. */
static int thread_fn(void *arg) {
    int st = 0;
    if (arg == 0) {
        if (!faults()) st |= 1;
        if (syscall1(SYS_iopl, 0) == 0 && faults()) st |= 2;
    } else {
        if (syscall1(SYS_iopl, 3) == 0 && !faults()) st |= 4;
    }
    g_result = st;
    return 0;
}

static int run_thread(void *arg) {
    g_result = -1;
    g_done = -1;
    int ptid = 0;
    i64 tid = poly_clone(THREAD_FLAGS, g_stack + sizeof g_stack, &ptid, (int *)&g_done, thread_fn, arg);
    if (tid <= 0) return -1;
    while (g_done != 0) futex((volatile unsigned *)&g_done, FUTEX_WAIT, (unsigned)g_done, 0, 0, 0);
    return g_result;
}

int test_main(void) {
    i64 r = sig_action(SIGSEGV, (u64)on_segv, SA_SIGINFO, 0);
    check("sigaction", r == 0, r);
    r = syscall1(SYS_iopl, 3);
    check("iopl3", r == 0, r);
    if (r != 0) return finish();

    int st = run_thread((void *)0);
    check("thread_inherits_access", st >= 0 && (st & 1), st);
    check("thread_iopl0_drops_its_access", st >= 0 && (st & 2), st);
    check("main_keeps_access", !faults(), g_sig);

    r = syscall1(SYS_iopl, 0);
    check("main_iopl0", r == 0, r);
    st = run_thread((void *)1);
    check("thread_iopl3_gets_access", st == 4, st);
    check("main_still_no_access", faults(), g_sig);
    return finish();
}

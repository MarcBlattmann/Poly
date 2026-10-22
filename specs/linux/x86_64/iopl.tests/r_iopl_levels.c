/* Levels 1 and 2 are accepted (as root) but grant no port access; only
 * level 3 does. Moving from 3 down to 2 or 1 takes the access away.
 * Root also gets EINVAL for levels above 3, and the failed call leaves
 * the current level (and its access) unchanged.
 * poly-diff: needs=root (raising the I/O privilege level needs CAP_SYS_RAWIO)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_iopl 172
/* asm-generic/errno-base.h */
#define EINVAL 22

static volatile int g_sig;

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

int test_main(void) {
    i64 r = sig_action(SIGSEGV, (u64)on_segv, SA_SIGINFO, 0);
    check("sigaction", r == 0, r);
    r = syscall1(SYS_iopl, 1);
    check("iopl1_returns_0", r == 0, r);
    if (r != 0) return finish();
    check("level1_ports_fault", faults(), g_sig);
    r = syscall1(SYS_iopl, 2);
    check("iopl2_returns_0", r == 0, r);
    check("level2_ports_fault", faults(), g_sig);
    r = syscall1(SYS_iopl, 3);
    check("iopl3_returns_0", r == 0, r);
    check("level3_ports_allowed", !faults(), g_sig);
    r = syscall1(SYS_iopl, 4);
    check("root_level4_einval", r == -EINVAL, r);
    check("einval_keeps_level3", !faults(), g_sig);
    r = syscall1(SYS_iopl, -1);
    check("root_level_minus1_einval", r == -EINVAL, r);
    check("einval_still_level3", !faults(), g_sig);
    r = syscall1(SYS_iopl, 2);
    check("down_to_2_returns_0", r == 0, r);
    check("down_to_2_ports_fault", faults(), g_sig);
    r = syscall1(SYS_iopl, 3);
    check("back_to_3", r == 0, r);
    check("back_to_3_allowed", !faults(), g_sig);
    r = syscall1(SYS_iopl, 1);
    check("down_to_1_returns_0", r == 0, r);
    check("down_to_1_ports_fault", faults(), g_sig);
    r = syscall1(SYS_iopl, (1LL << 32) | 3);
    check("high_half_ignored_level3", r == 0, r);
    check("high_half_level3_allowed", !faults(), g_sig);
    r = syscall1(SYS_iopl, 0);
    check("down_to_0", r == 0, r);
    check("level0_ports_fault", faults(), g_sig);
    return finish();
}

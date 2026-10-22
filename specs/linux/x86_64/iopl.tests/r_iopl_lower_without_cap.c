/* Only raising the level needs CAP_SYS_RAWIO. A root process at level 3
 * that drops all capabilities can still call iopl(3) again (same level,
 * returns 0) and keeps its port access; lowering (to 2, then 0) works;
 * raising again (0 to 1, 2 to 3) is EPERM and leaves the level as it was.
 * poly-diff: needs=root (raising the I/O privilege level needs CAP_SYS_RAWIO)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_iopl 172
/* linux/capability.h */
#define LINUX_CAPABILITY_VERSION_3 0x20080522

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
    r = syscall1(SYS_iopl, 3);
    check("iopl3_with_cap", r == 0, r);
    if (r != 0) return finish();

    struct { unsigned version; int pid; } hdr = { LINUX_CAPABILITY_VERSION_3, 0 };
    unsigned data[6] = { 0 };
    r = syscall2(SYS_capset, &hdr, data);
    check("capset_drop_all", r == 0, r);
    check("access_kept_after_cap_drop", !faults(), g_sig);
    r = syscall1(SYS_iopl, 3);
    check("same_level3_without_cap_ok", r == 0, r);
    check("still_allowed", !faults(), g_sig);
    r = syscall1(SYS_iopl, 2);
    check("lower_to_2_without_cap_ok", r == 0, r);
    check("level2_faults", faults(), g_sig);
    r = syscall1(SYS_iopl, 2);
    check("same_level2_ok", r == 0, r);
    r = syscall1(SYS_iopl, 3);
    check("raise_2_to_3_eperm", r == -EPERM, r);
    check("eperm_keeps_level2_faults", faults(), g_sig);
    r = syscall1(SYS_iopl, 0);
    check("lower_to_0_ok", r == 0, r);
    r = syscall1(SYS_iopl, 1);
    check("raise_0_to_1_eperm", r == -EPERM, r);
    r = syscall1(SYS_iopl, 3);
    check("raise_0_to_3_eperm", r == -EPERM, r);
    check("level0_faults", faults(), g_sig);
    return finish();
}

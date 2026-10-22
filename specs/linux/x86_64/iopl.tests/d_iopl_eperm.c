/* Raising the I/O privilege level above the current one (0) needs
 * CAP_SYS_RAWIO: without capabilities iopl(1), iopl(2) and iopl(3) fail
 * with EPERM. The failed calls change nothing: iopl(0) still returns 0,
 * and port I/O still raises SIGSEGV.
 * (All capabilities are dropped first, so this behaves the same for root.)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_iopl 172
/* linux/capability.h */
#define LINUX_CAPABILITY_VERSION_3 0x20080522

static volatile int g_sig, g_code;

static void on_segv(int sig, struct k_siginfo *si, struct k_ucontext *uc) {
    g_sig = sig;
    g_code = si->si_code;
    uc->gregs[UC_RIP] += 2; /* inb $imm8, %al is 2 bytes */
}

int test_main(void) {
    struct { unsigned version; int pid; } hdr = { LINUX_CAPABILITY_VERSION_3, 0 };
    unsigned data[6] = { 0 };
    i64 r = syscall2(SYS_capset, &hdr, data);
    check("capset_drop_all", r == 0, r);

    r = syscall1(SYS_iopl, 1);
    check("level1_eperm", r == -EPERM, r);
    r = syscall1(SYS_iopl, 2);
    check("level2_eperm", r == -EPERM, r);
    r = syscall1(SYS_iopl, 3);
    check("level3_eperm", r == -EPERM, r);
    r = syscall1(SYS_iopl, 3);
    check("level3_again_eperm", r == -EPERM, r);
    r = syscall1(SYS_iopl, 0);
    check("level0_ok", r == 0, r);

    r = sig_action(SIGSEGV, (u64)on_segv, SA_SIGINFO, 0);
    check("sigaction", r == 0, r);
    __asm__ volatile("inb $0x80, %%al" ::: "rax", "memory");
    check("port_still_faults_sigsegv", g_sig == SIGSEGV, g_sig);
    check("port_fault_si_kernel", g_code == SI_KERNEL, g_code);
    return finish();
}

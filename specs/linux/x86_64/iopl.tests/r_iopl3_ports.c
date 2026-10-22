/* As root, iopl(3) returns 0 and grants access to all 65536 I/O ports:
 * in/out with an immediate port, ports in %dx up to 0xffff, 8/16/32-bit
 * widths and ins all run without a signal. It does not grant anything
 * else of ring-0 I/O privilege: EFLAGS IOPL still reads 0, cli and sti
 * still raise SIGSEGV (si_code SI_KERNEL). iopl(0) takes the access away
 * again: port I/O raises SIGSEGV.
 * poly-diff: needs=root (raising the I/O privilege level needs CAP_SYS_RAWIO)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_iopl 172

#define EFL_IOPL 0x3000UL

static volatile int g_sig, g_code, g_skip;

static void on_segv(int sig, struct k_siginfo *si, struct k_ucontext *uc) {
    g_sig = sig;
    g_code = si->si_code;
    uc->gregs[UC_RIP] += g_skip;
}

/* 1 when the instruction faulted. */
static int inb80(void) {
    g_sig = 0;
    g_skip = 2;
    __asm__ volatile("inb $0x80, %%al" ::: "rax", "memory");
    return g_sig == SIGSEGV;
}

int test_main(void) {
    i64 r = sig_action(SIGSEGV, (u64)on_segv, SA_SIGINFO, 0);
    check("sigaction", r == 0, r);
    r = syscall1(SYS_iopl, 3);
    check("iopl3_returns_0", r == 0, r);
    if (r != 0) return finish();

    check("inb_imm_allowed", !inb80(), g_sig);
    g_sig = 0;
    g_skip = 3;
    __asm__ volatile("inw $0x80, %%ax" ::: "rax", "memory");
    check("inw_imm_allowed", g_sig == 0, g_sig);
    g_skip = 2;
    __asm__ volatile("inl $0x80, %%eax" ::: "rax", "memory");
    check("inl_imm_allowed", g_sig == 0, g_sig);
    g_skip = 2;
    __asm__ volatile("outb %%al, $0x80" ::"a"(0) : "memory");
    check("outb_port80_allowed", g_sig == 0, g_sig);
    g_skip = 1;
    __asm__ volatile("inb %%dx, %%al" ::"d"(0xffff) : "rax", "memory");
    check("inb_port_ffff_allowed", g_sig == 0, g_sig);
    __asm__ volatile("inb %%dx, %%al" ::"d"(0x8000) : "rax", "memory");
    check("inb_port_8000_allowed", g_sig == 0, g_sig);
    unsigned char buf[4];
    __asm__ volatile("insb" ::"D"(buf), "d"(0x80) : "memory");
    check("insb_allowed", g_sig == 0, g_sig);

    u64 f;
    __asm__ volatile("pushfq; popq %0" : "=r"(f)::"memory");
    check("eflags_iopl_still_0", (f & EFL_IOPL) == 0, (i64)f);
    g_skip = 1;
    __asm__ volatile("cli" ::: "memory");
    check("cli_still_sigsegv", g_sig == SIGSEGV && g_code == SI_KERNEL, g_sig);
    g_sig = 0;
    g_code = 0;
    __asm__ volatile("sti" ::: "memory");
    check("sti_still_sigsegv", g_sig == SIGSEGV && g_code == SI_KERNEL, g_sig);

    r = syscall1(SYS_iopl, 3);
    check("iopl3_again_returns_0", r == 0, r);
    check("still_allowed", !inb80(), g_sig);
    r = syscall1(SYS_iopl, 0);
    check("iopl0_returns_0", r == 0, r);
    check("after_iopl0_faults", inb80(), g_sig);
    check("after_iopl0_si_kernel", g_code == SI_KERNEL, g_code);
    r = syscall1(SYS_iopl, 3);
    check("iopl3_once_more", r == 0, r);
    check("allowed_once_more", !inb80(), g_sig);
    syscall1(SYS_iopl, 0);
    return finish();
}

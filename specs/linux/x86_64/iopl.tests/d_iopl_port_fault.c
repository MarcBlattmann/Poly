/* At I/O privilege level 0 (where every process starts) every port I/O
 * instruction raises SIGSEGV: in and out with an immediate port or a port
 * in %dx, 8/16/32-bit widths, and the string forms ins/outs. The signal
 * has si_code SI_KERNEL (0x80) and si_addr 0; the signal frame's
 * sigcontext reports trapno 13 (general protection) and err 0. A handler
 * that steps over the instruction resumes normally.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_iopl 172

static volatile int g_sig, g_code, g_skip, g_count;
static volatile u64 g_addr, g_trap, g_err;

static void on_segv(int sig, struct k_siginfo *si, struct k_ucontext *uc) {
    g_sig = sig;
    g_code = si->si_code;
    g_addr = si->u.addr;
    g_trap = uc->trapno;
    g_err = uc->err;
    g_count++;
    uc->gregs[UC_RIP] += g_skip;
}

static void reset(int skip) {
    g_sig = 0;
    g_code = 0;
    g_addr = 1;
    g_trap = 0;
    g_err = 1;
    g_skip = skip;
}

static void report(const char *what) {
    char name[64];
    char *p = append_str(name, what);
    append_str(p, "_sigsegv");
    check(name, g_sig == SIGSEGV, g_sig);
    p = append_str(name, what);
    append_str(p, "_si_kernel");
    check(name, g_code == SI_KERNEL, g_code);
    p = append_str(name, what);
    append_str(p, "_si_addr_0");
    check(name, g_addr == 0, (i64)g_addr);
}

int test_main(void) {
    i64 r = syscall1(SYS_iopl, 0);
    check("iopl0", r == 0, r);
    r = sig_action(SIGSEGV, (u64)on_segv, SA_SIGINFO, 0);
    check("sigaction", r == 0, r);
    unsigned char buf[8] = { 0 };

    reset(2);
    __asm__ volatile("inb $0x80, %%al" ::: "rax", "memory");
    report("inb_imm");
    check("inb_imm_trapno_13", g_trap == 13, (i64)g_trap);
    check("inb_imm_err_0", g_err == 0, (i64)g_err);

    reset(3);
    __asm__ volatile("inw $0x80, %%ax" ::: "rax", "memory");
    report("inw_imm");
    reset(2);
    __asm__ volatile("inl $0x80, %%eax" ::: "rax", "memory");
    report("inl_imm");
    reset(2);
    __asm__ volatile("outb %%al, $0x80" ::"a"(0) : "memory");
    report("outb_imm");
    reset(1);
    __asm__ volatile("inb %%dx, %%al" ::"d"(0x3f8) : "rax", "memory");
    report("inb_dx");
    reset(1);
    __asm__ volatile("outb %%al, %%dx" ::"a"(0), "d"(0xffff) : "memory");
    report("outb_dx");
    reset(1);
    __asm__ volatile("insb" ::"D"(buf), "d"(0x80) : "memory");
    report("insb");
    check("insb_buffer_untouched", buf[0] == 0, buf[0]);
    reset(1);
    __asm__ volatile("outsb" ::"S"(buf), "d"(0x80) : "memory");
    report("outsb");
    check("eight_faults_one_per_instruction", g_count == 8, g_count);
    return finish();
}

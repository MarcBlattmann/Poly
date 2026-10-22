/* Without I/O permission a port instruction does not run: inb $0x80 raises
 * SIGSEGV (a general protection fault), seen by an SA_SIGINFO handler with
 * si_code SI_KERNEL (0x80), si_addr 0, sigcontext trapno 13 and err 0, and
 * the saved rip pointing at the faulting instruction itself.
 */
#include "poly_ipc.h"

static volatile int got_sig, got_code, hits;
static volatile u64 got_addr, got_trapno, got_err, got_rip;

extern char port_insn[], port_after[];
static __attribute__((noinline)) void probe(void) {
    __asm__ volatile(".global port_insn\n.global port_after\n"
                     "port_insn: inb $0x80, %%al\n"
                     "port_after:\n" ::: "rax", "memory");
}

static void handler(int sig, struct k_siginfo *si, void *ucv) {
    struct k_ucontext *uc = ucv;
    hits++;
    got_sig = sig;
    got_code = si->si_code;
    got_addr = si->u.addr;
    got_trapno = uc->trapno;
    got_err = uc->err;
    got_rip = uc->gregs[UC_RIP];
    uc->gregs[UC_RIP] = (u64)port_after;
}

int test_main(void) {
    check("sigaction", sig_action(SIGSEGV, (u64)handler, SA_SIGINFO, 0) == 0, 0);
    probe();
    check("one_signal", hits == 1, hits);
    check("signal_is_sigsegv", got_sig == SIGSEGV, got_sig);
    check("si_code_si_kernel", got_code == SI_KERNEL, got_code);
    check("si_addr_0", got_addr == 0, (i64)got_addr);
    check("trapno_13", got_trapno == 13, (i64)got_trapno);
    check("err_0", got_err == 0, (i64)got_err);
    check("rip_at_instruction", got_rip == (u64)port_insn, 0);
    probe();
    check("faults_again", hits == 2, hits);
    return finish();
}

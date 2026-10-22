/* The I/O privilege level is inherited by a fork child: after iopl(3) a
 * child has port access without calling iopl itself. The level belongs
 * to each process on its own: the child lowering to 0 does not take the
 * parent's access away, and a child forked after the parent went back to
 * 0 has no access.
 * poly-diff: needs=root (raising the I/O privilege level needs CAP_SYS_RAWIO)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_iopl 172

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

/* Exit status of a fork child: bit 0 = it had access at the start,
 * bit 1 = it had none after its own iopl(0) (when asked to lower). */
static int child_status(int lower) {
    /* fork with the parent suspended until the child ends (CLONE_VFORK, no CLONE_VM) */
    i64 pid = syscall5(SYS_clone, 0x4000 | SIGCHLD, 0, 0, 0, 0);
    if (pid == 0) {
        int st = faults() ? 0 : 1;
        if (lower) {
            syscall1(SYS_iopl, 0);
            if (faults()) st |= 2;
        }
        syscall1(SYS_exit_group, st);
    }
    int status = wait_status(pid);
    if (pid < 0 || status == -1 || !W_EXITED(status)) return -1;
    return W_EXITCODE(status);
}

int test_main(void) {
    u64 old;
    sig_mask(SIG_BLOCK, SIGBIT(SIGCHLD), &old);
    i64 r = sig_action(SIGSEGV, (u64)on_segv, SA_SIGINFO, 0);
    check("sigaction", r == 0, r);
    r = syscall1(SYS_iopl, 3);
    check("iopl3", r == 0, r);
    if (r != 0) return finish();

    int st = child_status(1);
    check("child_inherits_access", st >= 0 && (st & 1), st);
    check("child_lowering_takes_its_access", st >= 0 && (st & 2), st);
    check("parent_keeps_access", !faults(), g_sig);
    st = child_status(0);
    check("second_child_inherits_access", st == 1, st);

    r = syscall1(SYS_iopl, 0);
    check("parent_iopl0", r == 0, r);
    check("parent_no_access", faults(), g_sig);
    st = child_status(0);
    check("child_after_iopl0_has_no_access", st == 0, st);
    return finish();
}

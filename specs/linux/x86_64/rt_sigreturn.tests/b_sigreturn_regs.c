/* rt_sigreturn restores the interrupted context from the frame: registers
 * the handler leaves alone come back unchanged, registers it edits in the
 * ucontext (r12, and rax, the interrupted syscall's result) come back
 * edited, and an edited uc_sigmask becomes the signal mask. Nested
 * handlers return in order and each restores its own mask. */
#include "poly_ipc.h"

static volatile u64 seen_r12, seen_rbx, seen_rax;
static volatile int nested_ok;

static void edit_context(int sig, struct k_siginfo *info, void *ctx) {
    struct k_ucontext *uc = ctx;
    (void)sig;
    (void)info;
    seen_r12 = uc->gregs[UC_R12];
    seen_rbx = uc->gregs[UC_RBX];
    seen_rax = uc->gregs[UC_RAX];
    uc->gregs[UC_R12] = 0x2222;
    uc->gregs[UC_RAX] = 77;
    uc->uc_sigmask |= SIGBIT(SIGUSR2);
}

static volatile int inner_ran;
static volatile u64 inner_mask, outer_mask_after;

static void inner(int sig) {
    (void)sig;
    inner_ran++;
    inner_mask = sig_current_mask();
}

static void outer(int sig) {
    (void)sig;
    raise_sig(SIGUSR2); /* handled now, nested on top of this frame */
    outer_mask_after = sig_current_mask();
    nested_ok = inner_ran == 1;
}

int test_main(void) {
    i64 pid = getpid();
    sig_action(SIGUSR1, (u64)edit_context, SA_SIGINFO, 0);
    u64 r12_after, rbx_after, rax_after;
    __asm__ volatile("mov $0x1111, %%r12\n"
                     "mov $0x3333, %%rbx\n"
                     "mov %[pid], %%rdi\n"
                     "mov $10, %%esi\n"
                     "mov $62, %%eax\n"
                     "syscall\n"
                     "mov %%r12, %[r12]\n"
                     "mov %%rbx, %[rbx]\n"
                     "mov %%rax, %[rax]\n"
                     : [r12] "=m"(r12_after), [rbx] "=m"(rbx_after), [rax] "=m"(rax_after)
                     : [pid] "r"(pid)
                     : "rax", "rdi", "rsi", "rcx", "r11", "r12", "rbx", "memory");
    check("frame_has_interrupted_r12", seen_r12 == 0x1111, (i64)seen_r12);
    check("frame_has_interrupted_rbx", seen_rbx == 0x3333, (i64)seen_rbx);
    check("frame_has_syscall_result_in_rax", seen_rax == 0, (i64)seen_rax);
    check("edited_r12_restored", r12_after == 0x2222, (i64)r12_after);
    check("unedited_rbx_restored", rbx_after == 0x3333, (i64)rbx_after);
    check("edited_rax_restored", rax_after == 77, (i64)rax_after);
    check("edited_uc_sigmask_applied", sig_current_mask() == SIGBIT(SIGUSR2), (i64)sig_current_mask());
    sig_mask(SIG_SETMASK, 0, 0);

    /* Nested: SIGUSR2 raised inside the SIGUSR1 handler. */
    sig_handler(SIGUSR1, outer, 0);
    sig_handler(SIGUSR2, inner, 0);
    raise_sig(SIGUSR1);
    check("nested_inner_ran_inside_outer", nested_ok, inner_ran);
    check("inner_mask_has_both", inner_mask == (SIGBIT(SIGUSR1) | SIGBIT(SIGUSR2)), (i64)inner_mask);
    check("outer_mask_restored_after_inner", outer_mask_after == SIGBIT(SIGUSR1), (i64)outer_mask_after);
    check("mask_restored_after_outer", sig_current_mask() == 0, (i64)sig_current_mask());
    return finish();
}

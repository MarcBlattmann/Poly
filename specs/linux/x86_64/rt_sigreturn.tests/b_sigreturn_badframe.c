/* rt_sigreturn with no signal frame on the stack (a zeroed area, not one
 * the kernel built) does not return: the process is killed by SIGSEGV.
 * A handler that sets the saved rip to an unmapped address makes the
 * process fault there after rt_sigreturn.
 * poly-diff: syscalls=off (child processes)
 */
#include "poly_ipc.h"

static u64 fake_frame[256] __attribute__((aligned(16)));

static void bad_rip(int sig, struct k_siginfo *info, void *ctx) {
    struct k_ucontext *uc = ctx;
    (void)sig;
    (void)info;
    uc->gregs[UC_RIP] = 0x1000;
}

int test_main(void) {
    i64 pid = fork();
    if (pid == 0) {
        __asm__ volatile("mov %0, %%rsp\n"
                         "mov $15, %%eax\n"
                         "syscall\n"
                         "mov $60, %%eax\n"
                         "mov $9, %%edi\n"
                         "syscall\n" ::"r"(fake_frame + 128)
                         : "memory");
        exit_group(8);
    }
    int st = wait_status(pid);
    check("sigreturn_without_frame_sigsegv", W_SIGNALED(st) && W_TERMSIG(st) == SIGSEGV, st);

    pid = fork();
    if (pid == 0) {
        sig_action(SIGUSR1, (u64)bad_rip, SA_SIGINFO, 0);
        raise_sig(SIGUSR1);
        exit_group(9);
    }
    st = wait_status(pid);
    check("edited_rip_unmapped_sigsegv", W_SIGNALED(st) && W_TERMSIG(st) == SIGSEGV, st);
    (void)fake_frame;
    return finish();
}

/* ioperm as root (CAP_SYS_RAWIO): enabling a range returns 0 and the
 * ports in it can then be read with in instructions without a fault, while
 * the ports next to it still fault; a fork child inherits the permission;
 * disabling the range (or a range covering it) takes it away again. On a
 * kernel in lockdown (integrity or confidentiality) even root gets EPERM,
 * and then nothing is granted: the test accepts that, and checks that the
 * ports keep faulting. Only ports 0x80 and 0x81 are ever read.
 * poly-diff: needs=root (only CAP_SYS_RAWIO may enable ports)
 * poly-diff: syscalls=off (a child process)
 */
#include "poly_ipc.h"

#define SYS_ioperm 173
#define EPERM 1

static volatile int hits;

static void handler(int sig, struct k_siginfo *si, void *ucv) {
    (void)sig;
    (void)si;
    struct k_ucontext *uc = ucv;
    hits++;
    uc->gregs[UC_RIP] += 2; /* inb $imm8, %al is 2 bytes */
}

static int faults_0x80(void) {
    int h = hits;
    __asm__ volatile("inb $0x80, %%al" ::: "rax", "memory");
    return hits != h;
}

static int faults_0x81(void) {
    int h = hits;
    __asm__ volatile("inb $0x81, %%al" ::: "rax", "memory");
    return hits != h;
}

int test_main(void) {
    check("sigaction", sig_action(SIGSEGV, (u64)handler, SA_SIGINFO, 0) == 0, 0);
    check("faults_before", faults_0x80(), hits);

    i64 r = syscall3(SYS_ioperm, 0x80, 1, 1);
    int granted = r == 0;
    check("enable_0x80_ok_or_lockdown_eperm", r == 0 || r == -EPERM, r);
    check("port_0x80_readable_iff_granted", faults_0x80() == !granted, hits);
    check("port_0x81_still_faults", faults_0x81(), hits);

    i64 pid = fork();
    if (pid == 0) exit_group(faults_0x80() == !granted ? 0 : 1);
    check("fork_child_inherits", wait_exit(pid) == 0, pid);

    r = syscall3(SYS_ioperm, 0x80, 1, 0);
    check("disable_0x80_ok", r == 0, r);
    check("port_0x80_faults_after_disable", faults_0x80(), hits);

    r = syscall3(SYS_ioperm, 0, 0x81, 1);
    check("enable_0_to_0x80_same_result", (r == 0) == granted && (r == 0 || r == -EPERM), r);
    check("range_end_readable_iff_granted", faults_0x80() == !granted, hits);
    check("past_range_end_faults", faults_0x81(), hits);
    r = syscall3(SYS_ioperm, 0x40, 0x41, 0);
    check("disable_covering_range_ok", r == 0, r);
    check("faults_after_covering_disable", faults_0x80(), hits);
    return finish();
}

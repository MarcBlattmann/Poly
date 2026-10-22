/* A failed enable grants nothing and a disable takes nothing that was not
 * there: after ioperm(0x80, 1, 1) failed with EPERM (no capabilities) the
 * port still faults; after a successful ioperm(0, 65536, 0) as well.
 */
#include "poly_ipc.h"

#define SYS_ioperm 173
#define EPERM 1

struct cap_header {
    unsigned version;
    int pid;
};
struct cap_data {
    unsigned effective, permitted, inheritable;
};

static volatile int hits;

static void handler(int sig, struct k_siginfo *si, void *ucv) {
    (void)sig;
    (void)si;
    struct k_ucontext *uc = ucv;
    hits++;
    uc->gregs[UC_RIP] += 2; /* inb $imm8, %al is 2 bytes */
}

static int port_faults(void) {
    int h = hits;
    __asm__ volatile("inb $0x80, %%al" ::: "rax", "memory");
    return hits == h + 1;
}

int test_main(void) {
    check("sigaction", sig_action(SIGSEGV, (u64)handler, SA_SIGINFO, 0) == 0, 0);
    struct cap_header h = { 0x20080522, 0 };
    struct cap_data d[2];
    memset(d, 0, sizeof d);
    check("capset_drop_all", syscall2(SYS_capset, &h, d) == 0, 0);
    i64 r = syscall3(SYS_ioperm, 0x80, 1, 1);
    check("enable_eperm", r == -EPERM, r);
    check("faults_after_failed_enable", port_faults(), hits);
    r = syscall3(SYS_ioperm, 0, 65536, 0);
    check("disable_all_ok", r == 0, r);
    check("faults_after_disable", port_faults(), hits);
    r = syscall3(SYS_ioperm, 0x80, 0, 1);
    check("bad_range_einval", r == -EINVAL, r);
    check("faults_after_einval", port_faults(), hits);
    return finish();
}

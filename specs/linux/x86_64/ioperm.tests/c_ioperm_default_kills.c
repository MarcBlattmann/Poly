/* With the default SIGSEGV action, a port instruction without I/O
 * permission kills the process by SIGSEGV (the child below is made
 * non-dumpable, so no core is written and the wait status is exactly 11).
 * The child first tried to enable the port without capabilities.
 * poly-diff: syscalls=off (a child process)
 */
#include "poly_ipc.h"

#define SYS_ioperm 173
#define PR_SET_DUMPABLE 4

struct cap_header {
    unsigned version;
    int pid;
};
struct cap_data {
    unsigned effective, permitted, inheritable;
};

int test_main(void) {
    i64 pid = fork();
    if (pid == 0) {
        struct cap_header h = { 0x20080522, 0 };
        struct cap_data d[2];
        memset(d, 0, sizeof d);
        syscall2(SYS_prctl, PR_SET_DUMPABLE, 0);
        if (syscall2(SYS_capset, &h, d) != 0) exit_group(2);
        if (syscall3(SYS_ioperm, 0x80, 1, 1) != -1) exit_group(3);
        __asm__ volatile("inb $0x80, %%al" ::: "rax", "memory");
        exit_group(4);
    }
    check("fork", pid > 0, pid);
    int s = wait_status(pid);
    check("child_killed", W_SIGNALED(s), s);
    check("killed_by_sigsegv", W_TERMSIG(s) == SIGSEGV, s);
    check("status_exactly_11", s == SIGSEGV, s);
    return finish();
}

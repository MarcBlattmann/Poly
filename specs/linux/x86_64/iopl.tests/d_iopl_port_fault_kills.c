/* With the default SIGSEGV action, a port access at I/O privilege level 0
 * kills the process: the parent's wait4 sees it terminated by SIGSEGV
 * (signal 11; the core-dump bit is not checked). The parent is
 * unaffected and its own level is still 0.
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_iopl 172
/* asm-generic/resource.h */
#define RLIMIT_CORE 4

int test_main(void) {
    a_block_sigchld();
    long pid = a_fork_sync();
    if (pid == 0) {
        u64 lim[2] = { 0, 0 };
        syscall2(SYS_setrlimit, RLIMIT_CORE, lim);
        __asm__ volatile("inb $0x80, %%al" ::: "rax", "memory");
        syscall1(SYS_exit_group, 42);
    }
    check("fork", pid > 0, pid);
    i32 status = 0;
    i64 r = syscall4(SYS_wait4, pid, &status, 0, 0);
    check("wait4", r == pid, r);
    check("child_signaled", A_WIFSIGNALED(status), status);
    check("child_killed_by_sigsegv", A_WTERMSIG(status) == SIGSEGV, status);
    r = syscall1(SYS_iopl, 0);
    check("parent_iopl0", r == 0, r);
    return finish();
}

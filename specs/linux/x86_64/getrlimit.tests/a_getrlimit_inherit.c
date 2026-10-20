/* getrlimit: a child process starts with its parent's limits; its own
 * changes stay its own. */
#include "a_sys.h"

int test_main(void) {
    a_block_sigchld();
    volatile i32 *sh = a_shared();
    struct a_rlimit old, l = {50, 60};
    syscall2(SYS_getrlimit, RLIMIT_NOFILE, &old);
    l.rlim_max = old.rlim_max;
    syscall2(SYS_setrlimit, RLIMIT_NOFILE, &l);
    i64 pid = a_fork_sync();
    if (pid == 0) {
        struct a_rlimit c;
        syscall2(SYS_getrlimit, RLIMIT_NOFILE, &c);
        sh[0] = (i32)c.rlim_cur;
        c.rlim_cur = 40;
        syscall2(SYS_setrlimit, RLIMIT_NOFILE, &c);
        syscall1(SYS_exit_group, 0);
    }
    i32 st;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("child_inherited", sh[0], 50);
    struct a_rlimit now;
    syscall2(SYS_getrlimit, RLIMIT_NOFILE, &now);
    expect("parent_unchanged", now.rlim_cur, 50);
    syscall2(SYS_setrlimit, RLIMIT_NOFILE, &old);
    return finish();
}

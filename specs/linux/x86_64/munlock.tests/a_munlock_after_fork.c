/* munlock: locks are not inherited across fork; a child may lock and
 * unlock its own copy; the parent's locked memory is unaffected. */
#include "a_sys.h"

int test_main(void) {
    a_block_sigchld();
    i64 m = syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    volatile char *p = (volatile char *)m;
    p[0] = 'P';
    i64 r = syscall2(SYS_mlock, m, 2 * A_PAGE);
    /* fork semantics, the parent waiting until the child has exited */
    i64 pid = a_fork_sync();
    if (pid == 0) {
        int code = 0;
        code |= syscall2(SYS_munlock, m, 2 * A_PAGE) == 0 ? 1 : 0;
        code |= syscall2(SYS_mlock, m, A_PAGE) == 0 ? 2 : 0;
        p[0] = 'C';
        code |= syscall2(SYS_munlock, m, A_PAGE) == 0 ? 4 : 0;
        syscall1(SYS_exit_group, code);
    }
    i32 st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("parent_lock", r, 0);
    check("fork", pid > 0, pid);
    expect("child_lock_calls", A_WEXITSTATUS(st), 7);
    check("parent_copy_kept", p[0] == 'P', p[0]);
    expect("parent_unlock", syscall2(SYS_munlock, m, 2 * A_PAGE), 0);
    syscall2(SYS_munmap, m, 2 * A_PAGE);
    return finish();
}

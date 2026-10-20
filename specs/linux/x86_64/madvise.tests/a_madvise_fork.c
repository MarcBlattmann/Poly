/* madvise MADV_WIPEONFORK: a forked child sees zeros in the range while
 * the parent keeps its data; MADV_KEEPONFORK undoes it. The child reports
 * through its exit status. */
#include "a_sys.h"

static i32 reap(i64 pid) {
    i32 status = -1;
    syscall4(SYS_wait4, pid, &status, 0, 0);
    return status;
}

int test_main(void) {
    a_block_sigchld();
    i64 m = syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    volatile char *p = (volatile char *)m;
    p[0] = 'w';
    p[A_PAGE] = 'k';
    i64 r1 = syscall3(SYS_madvise, m, A_PAGE, MADV_WIPEONFORK);
    /* fork semantics, the parent waiting until the child has exited */
    i64 pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, (p[0] == 0 ? 1 : 0) | (p[A_PAGE] == 'k' ? 2 : 0));
    i32 st = reap(pid);
    expect("wipeonfork", r1, 0);
    check("fork", pid > 0, pid);
    check("child_exited", A_WIFEXITED(st), st);
    expect("child_saw_zero_and_kept_page", A_WEXITSTATUS(st), 3);
    check("parent_kept_data", p[0] == 'w', p[0]);
    i64 r2 = syscall3(SYS_madvise, m, A_PAGE, MADV_KEEPONFORK);
    pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, p[0] == 'w' ? 7 : 8);
    st = reap(pid);
    expect("keeponfork", r2, 0);
    expect("child_sees_data_again", A_WEXITSTATUS(st), 7);
    /* WIPEONFORK is only for private anonymous memory. */
    i64 s = syscall6(SYS_mmap, 0, A_PAGE, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_ANONYMOUS, -1, 0);
    expect("wipeonfork_shared_einval", syscall3(SYS_madvise, s, A_PAGE, MADV_WIPEONFORK), -EINVAL);
    syscall2(SYS_munmap, s, A_PAGE);
    syscall2(SYS_munmap, m, 2 * A_PAGE);
    return finish();
}

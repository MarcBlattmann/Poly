/* munlockall in a fork child succeeds and acts on the child alone: the
 * parent's pages stay locked and its MCL_FUTURE stays on. */
#include "a_sys.h"

static i64 resident(i64 addr, i64 pages) {
    unsigned char v[16];
    i64 r = syscall3(SYS_mincore, addr, pages * A_PAGE, v);
    if (r) return r;
    i64 n = 0;
    for (i64 i = 0; i < pages; i++) n += v[i] & 1;
    return n;
}

int test_main(void) {
    a_block_sigchld();
    i64 m = syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap", m > 0, m);
    expect("mlockall", syscall1(SYS_mlockall, MCL_CURRENT | MCL_FUTURE), 0);
    i64 pid = a_fork_sync();
    if (pid == 0) {
        int bad = syscall0(SYS_munlockall) != 0;
        bad |= syscall1(SYS_mlockall, MCL_CURRENT) != 0;
        bad |= syscall0(SYS_munlockall) != 0;
        bad |= syscall3(SYS_madvise, m, A_PAGE, MADV_DONTNEED) != 0;
        syscall1(SYS_exit_group, bad);
    }
    i32 st = -1;
    expect("wait", syscall4(SYS_wait4, pid, &st, 0, 0), pid);
    expect("child_ok", st, 0);
    expect("parent_still_locked", syscall3(SYS_madvise, m, A_PAGE, MADV_DONTNEED), -EINVAL);
    i64 n = syscall6(SYS_mmap, 0, 4 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    expect("parent_future_on", resident(n, 4), 4);
    expect("munlockall", syscall0(SYS_munlockall), 0);
    return finish();
}

/* munlockall acts on the whole address space: called by another task that
 * shares it (CLONE_VM), it unlocks this task's pages and turns MCL_FUTURE
 * off for it too. */
#include "a_sys.h"

static i64 resident(i64 addr, i64 pages) {
    unsigned char v[16];
    i64 r = syscall3(SYS_mincore, addr, pages * A_PAGE, v);
    if (r) return r;
    i64 n = 0;
    for (i64 i = 0; i < pages; i++) n += v[i] & 1;
    return n;
}

static volatile i64 result = -1;

static int child(void *arg) {
    (void)arg;
    result = syscall0(SYS_munlockall);
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    i64 m = syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap", m > 0, m);
    expect("mlockall", syscall1(SYS_mlockall, MCL_CURRENT | MCL_FUTURE), 0);
    expect("locked_before", syscall3(SYS_madvise, m, A_PAGE, MADV_DONTNEED), -EINVAL);
    volatile i32 done;
    i64 pid = a_spawn(0, &done, child, 0);
    check("spawn", pid > 0, pid);
    check("child_ended", a_spin_until(&done, 0, A_SPIN_LIMIT), done);
    syscall4(SYS_wait4, pid, 0, 0, 0);
    expect("child_munlockall", result, 0);
    expect("unlocked_for_parent", syscall3(SYS_madvise, m, A_PAGE, MADV_DONTNEED), 0);
    i64 n = syscall6(SYS_mmap, 0, 4 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    expect("future_off_for_parent", resident(n, 4), 0);
    return finish();
}

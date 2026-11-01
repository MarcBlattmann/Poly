/* munlockall gives back everything counted against RLIMIT_MEMLOCK, for a
 * caller without CAP_IPC_LOCK (run as root, the test first becomes
 * uid/gid 65534). Soft limit 2 pages: after locking 2 pages a third is
 * ENOMEM; after munlockall 2 other pages can be locked. It needs no
 * limit itself: with the limit 0 it returns 0. After mlockall(MCL_FUTURE)
 * made an mmap over the limit fail with EAGAIN, munlockall lets it work.
 */
#include "a_sys.h"

#define SYS_mlock2 325
#define SYS_setresuid 117
#define SYS_setresgid 119
#define SYS_setgroups 116
#define MCL_FUTURE 2

static void become_user(void) {
    if (syscall0(SYS_geteuid) != 0) return;
    syscall2(SYS_setgroups, 0, 0);
    syscall3(SYS_setresgid, 65534, 65534, 65534);
    syscall3(SYS_setresuid, 65534, 65534, 65534);
}

static i64 set_soft(u64 bytes) {
    struct a_rlimit rl;
    syscall2(SYS_getrlimit, RLIMIT_MEMLOCK, &rl);
    rl.rlim_cur = bytes;
    return syscall2(SYS_setrlimit, RLIMIT_MEMLOCK, &rl);
}

static i64 map(int pages) {
    return syscall6(SYS_mmap, 0, (u64)pages * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
}

int test_main(void) {
    become_user();
    check("not_root", syscall0(SYS_geteuid) != 0, 0);
    char *m = (char *)map(8);

    expect("limit_2_pages", set_soft(2 * A_PAGE), 0);
    expect("lock_2", syscall3(SYS_mlock2, m, 2 * A_PAGE, 0), 0);
    expect("third_no", syscall3(SYS_mlock2, m + 4 * A_PAGE, A_PAGE, 0), -ENOMEM);
    expect("munlockall", syscall0(SYS_munlockall), 0);
    expect("lock_2_others", syscall3(SYS_mlock2, m + 4 * A_PAGE, 2 * A_PAGE, 0), 0);
    expect("munlockall_2", syscall0(SYS_munlockall), 0);
    expect("lock_first_again", syscall3(SYS_mlock2, m, 2 * A_PAGE, 0), 0);
    expect("munlockall_3", syscall0(SYS_munlockall), 0);

    expect("future", syscall1(SYS_mlockall, MCL_FUTURE), 0);
    expect("future_mmap_over_limit", map(4), -EAGAIN);
    expect("munlockall_4", syscall0(SYS_munlockall), 0);
    check("mmap_works_again", map(4) > 0, 0);

    expect("limit_0", set_soft(0), 0);
    expect("munlockall_with_limit_0", syscall0(SYS_munlockall), 0);
    expect("mlock2_with_limit_0", syscall3(SYS_mlock2, m, A_PAGE, 0), -EPERM);
    return finish();
}

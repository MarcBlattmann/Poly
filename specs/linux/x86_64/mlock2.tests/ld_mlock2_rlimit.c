/* mlock2 and RLIMIT_MEMLOCK for a caller without CAP_IPC_LOCK (run as
 * root, the test first becomes uid/gid 65534, which drops every
 * capability; as any other user it runs as it is). With the soft limit
 * at 2 pages: locking 2 pages works, a third is ENOMEM (12); locking an
 * already locked range again is free; a range overlapping locked pages
 * counts only the new ones (3 pages over 2 locked: ENOMEM); munlock gives
 * the pages back. A huge len (-8192) is ENOMEM (over the limit). With the
 * limit 0 every mlock2 is EPERM (1), even len 0, but bad flags are still
 * EINVAL (checked first); munlock still works.
 */
#include "a_sys.h"

#define SYS_mlock2 325
#define SYS_setresuid 117
#define SYS_setresgid 119
#define SYS_setgroups 116
#define MLOCK_ONFAULT 0x01

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

int test_main(void) {
    become_user();
    check("not_root", syscall0(SYS_geteuid) != 0, 0);
    char *m = (char *)syscall6(SYS_mmap, 0, 8 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);

    expect("limit_2_pages", set_soft(2 * A_PAGE), 0);
    expect("lock_2", syscall3(SYS_mlock2, m, 2 * A_PAGE, 0), 0);
    expect("lock_third", syscall3(SYS_mlock2, m + 2 * A_PAGE, A_PAGE, 0), -ENOMEM);
    expect("lock_third_onfault", syscall3(SYS_mlock2, m + 2 * A_PAGE, A_PAGE, MLOCK_ONFAULT), -ENOMEM);
    expect("relock_same", syscall3(SYS_mlock2, m, 2 * A_PAGE, MLOCK_ONFAULT), 0);
    expect("relock_one", syscall3(SYS_mlock2, m + A_PAGE, A_PAGE, 0), 0);
    expect("overlap_3", syscall3(SYS_mlock2, m, 3 * A_PAGE, 0), -ENOMEM);
    expect("huge_len", syscall3(SYS_mlock2, m, -8192L, 0), -ENOMEM);
    expect("munlock_first", syscall2(SYS_munlock, m, A_PAGE), 0);
    expect("lock_other_after_munlock", syscall3(SYS_mlock2, m + 4 * A_PAGE, A_PAGE, 0), 0);
    expect("full_again", syscall3(SYS_mlock2, m + 5 * A_PAGE, A_PAGE, 0), -ENOMEM);
    expect("munlock_all", syscall2(SYS_munlock, m, 8 * A_PAGE), 0);
    expect("lock_2_elsewhere", syscall3(SYS_mlock2, m + 6 * A_PAGE, 2 * A_PAGE, 0), 0);
    expect("munlock_all2", syscall2(SYS_munlock, m, 8 * A_PAGE), 0);

    expect("limit_0", set_soft(0), 0);
    expect("zero_limit", syscall3(SYS_mlock2, m, A_PAGE, 0), -EPERM);
    expect("zero_limit_onfault", syscall3(SYS_mlock2, m, A_PAGE, MLOCK_ONFAULT), -EPERM);
    expect("zero_limit_len0", syscall3(SYS_mlock2, m, 0, 0), -EPERM);
    expect("zero_limit_unmapped", syscall3(SYS_mlock2, 0x10000, A_PAGE, 0), -EPERM);
    expect("zero_limit_bad_flags", syscall3(SYS_mlock2, m, A_PAGE, 2), -EINVAL);
    expect("zero_limit_munlock", syscall2(SYS_munlock, m, A_PAGE), 0);

    expect("limit_1_page", set_soft(A_PAGE), 0);
    expect("one_page_ok", syscall3(SYS_mlock2, m, A_PAGE, 0), 0);
    expect("two_pages_no", syscall3(SYS_mlock2, m + A_PAGE, A_PAGE, 0), -ENOMEM);
    expect("munlock_end", syscall2(SYS_munlock, m, 8 * A_PAGE), 0);
    return finish();
}

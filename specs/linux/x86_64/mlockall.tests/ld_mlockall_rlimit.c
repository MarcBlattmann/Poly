/* mlockall and RLIMIT_MEMLOCK for a caller without CAP_IPC_LOCK (run as
 * root, the test first becomes uid/gid 65534, which drops every
 * capability). Soft limit 0: MCL_CURRENT and MCL_FUTURE are EPERM (1);
 * bad flags still EINVAL (checked first). Soft limit 2 pages (far below
 * the process's size): MCL_CURRENT is ENOMEM (12) and locks nothing;
 * MCL_FUTURE alone succeeds, after which an mmap that would take the
 * locked total over the limit fails with EAGAIN (11) while a 1-page mmap
 * works and a second one fails; munlockall ends that (the 4-page mmap
 * works again).
 */
#include "a_sys.h"

#define SYS_setresuid 117
#define SYS_setresgid 119
#define SYS_setgroups 116
#define MCL_CURRENT 1
#define MCL_FUTURE 2
#define MCL_ONFAULT 4

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

    expect("limit_0", set_soft(0), 0);
    expect("zero_current", syscall1(SYS_mlockall, MCL_CURRENT), -EPERM);
    expect("zero_future", syscall1(SYS_mlockall, MCL_FUTURE), -EPERM);
    expect("zero_future_onfault", syscall1(SYS_mlockall, MCL_FUTURE | MCL_ONFAULT), -EPERM);
    expect("zero_bad_flags", syscall1(SYS_mlockall, 0), -EINVAL);
    expect("zero_onfault_alone", syscall1(SYS_mlockall, MCL_ONFAULT), -EINVAL);
    expect("zero_munlockall", syscall0(SYS_munlockall), 0);
    check("zero_mmap_still_works", map(4) > 0, 0);

    expect("limit_2_pages", set_soft(2 * A_PAGE), 0);
    expect("small_current", syscall1(SYS_mlockall, MCL_CURRENT), -ENOMEM);
    expect("small_current_future", syscall1(SYS_mlockall, MCL_CURRENT | MCL_FUTURE), -ENOMEM);
    check("small_failed_current_no_future", map(4) > 0, 0);
    expect("small_future", syscall1(SYS_mlockall, MCL_FUTURE), 0);
    expect("future_mmap_over_limit", map(4), -EAGAIN);
    check("future_mmap_1_page", map(1) > 0, 0);
    check("future_mmap_2nd_page", map(1) > 0, 0);
    expect("future_mmap_3rd_page", map(1), -EAGAIN);
    expect("munlockall", syscall0(SYS_munlockall), 0);
    check("mmap_after_munlockall", map(4) > 0, 0);
    return finish();
}

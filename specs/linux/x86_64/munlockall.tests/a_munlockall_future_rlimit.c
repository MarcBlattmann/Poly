/* munlockall after mlockall(MCL_FUTURE) under a small RLIMIT_MEMLOCK:
 * mappings over the limit were refused (EAGAIN) while MCL_FUTURE was on;
 * afterwards they are allowed again, and brk grows. */
#include "a_sys.h"

static i64 anon(i64 pages) {
    return syscall6(SYS_mmap, 0, pages * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
}

int test_main(void) {
    int root = syscall0(SYS_getuid) == 0;
    struct a_rlimit rl;
    syscall2(SYS_getrlimit, RLIMIT_MEMLOCK, &rl);
    rl.rlim_cur = 8 * A_PAGE;
    expect("setrlimit", syscall2(SYS_setrlimit, RLIMIT_MEMLOCK, &rl), 0);
    expect("future", syscall1(SYS_mlockall, MCL_FUTURE), 0);
    check("fits", anon(8) > 0, 0);
    i64 r = anon(1);
    if (root) check("root_over_ok", r > 0, r);
    else expect("over_limit_eagain", r, -EAGAIN);
    expect("munlockall", syscall0(SYS_munlockall), 0);
    i64 big = anon(64);
    check("big_after_munlockall", big > 0, big);
    i64 b0 = syscall1(SYS_brk, 0);
    i64 b1 = syscall1(SYS_brk, b0 + 16 * A_PAGE);
    expect("brk_grows", b1 - b0, 16 * A_PAGE);
    expect("mlock_within_limit", syscall2(SYS_mlock, big, 8 * A_PAGE), 0);
    expect("munlockall_2", syscall0(SYS_munlockall), 0);
    return finish();
}

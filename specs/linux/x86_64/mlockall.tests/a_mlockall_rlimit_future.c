/* mlockall(MCL_FUTURE) for an unprivileged caller: a later mapping that
 * would take the locked memory over RLIMIT_MEMLOCK fails with EAGAIN
 * (PROT_NONE and MCL_ONFAULT included) and brk does not grow; unlocking
 * frees room again. Root is not limited. */
#include "a_sys.h"

static i64 anon(i64 pages, i64 prot) {
    return syscall6(SYS_mmap, 0, pages * A_PAGE, prot, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
}

int test_main(void) {
    int root = syscall0(SYS_getuid) == 0;
    struct a_rlimit rl;
    syscall2(SYS_getrlimit, RLIMIT_MEMLOCK, &rl);
    rl.rlim_cur = 64 * A_PAGE;
    expect("setrlimit", syscall2(SYS_setrlimit, RLIMIT_MEMLOCK, &rl), 0);
    expect("future", syscall1(SYS_mlockall, MCL_FUTURE), 0);
    i64 a = anon(32, PROT_READ | PROT_WRITE);
    check("first_half_fits", a > 0, a);
    i64 b = anon(32, PROT_READ | PROT_WRITE);
    check("up_to_limit_fits", b > 0, b);
    i64 c = anon(1, PROT_READ | PROT_WRITE);
    if (root) check("root_over_limit_ok", c > 0, c);
    else expect("over_limit_eagain", c, -EAGAIN);
    c = anon(1, PROT_NONE);
    if (root) check("root_prot_none_ok", c > 0, c);
    else expect("prot_none_over_limit_eagain", c, -EAGAIN);
    i64 brk0 = syscall1(SYS_brk, 0);
    i64 brk1 = syscall1(SYS_brk, brk0 + 4 * A_PAGE);
    expect("brk_growth", brk1 - brk0, root ? 4 * A_PAGE : 0);
    expect("munlock", syscall2(SYS_munlock, a, 32 * A_PAGE), 0);
    i64 d = anon(16, PROT_READ | PROT_WRITE);
    check("room_after_munlock", d > 0, d);
    expect("future_onfault", syscall1(SYS_mlockall, MCL_FUTURE | MCL_ONFAULT), 0);
    i64 e = anon(32, PROT_READ | PROT_WRITE);
    if (root) check("root_onfault_over_limit_ok", e > 0, e);
    else expect("onfault_over_limit_eagain", e, -EAGAIN);
    return finish();
}

/* mlockall: each call replaces the MCL_FUTURE setting. MCL_CURRENT alone
 * turns an earlier MCL_FUTURE off; MCL_FUTURE turns it back on;
 * munlock of a range leaves it on. */
#include "a_sys.h"

static i64 resident(i64 addr, i64 pages) {
    unsigned char v[16];
    i64 r = syscall3(SYS_mincore, addr, pages * A_PAGE, v);
    if (r) return r;
    i64 n = 0;
    for (i64 i = 0; i < pages; i++) n += v[i] & 1;
    return n;
}

static i64 anon(i64 pages) {
    return syscall6(SYS_mmap, 0, pages * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
}

int test_main(void) {
    expect("future", syscall1(SYS_mlockall, MCL_FUTURE), 0);
    i64 a = anon(4);
    expect("future_on", resident(a, 4), 4);
    expect("munlock_range", syscall2(SYS_munlock, a, 4 * A_PAGE), 0);
    expect("future_still_on", resident(anon(4), 4), 4);
    expect("current_only", syscall1(SYS_mlockall, MCL_CURRENT), 0);
    expect("future_off", resident(anon(4), 4), 0);
    expect("future_again", syscall1(SYS_mlockall, MCL_FUTURE | MCL_CURRENT), 0);
    expect("future_on_again", resident(anon(4), 4), 4);
    expect("failed_call_keeps_setting", syscall1(SYS_mlockall, MCL_CURRENT | 8), -EINVAL);
    expect("future_kept", resident(anon(4), 4), 4);
    return finish();
}

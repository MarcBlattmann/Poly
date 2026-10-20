/* dup: with RLIMIT_NOFILE lowered, dup fails with EMFILE once every
 * number below the limit is taken. */
#include "a_sys.h"

int test_main(void) {
    struct a_rlimit old, low;
    expect("getrlimit", syscall2(SYS_getrlimit, RLIMIT_NOFILE, &old), 0);
    low.rlim_cur = 8;
    low.rlim_max = old.rlim_max;
    expect("lower_soft_limit", syscall2(SYS_setrlimit, RLIMIT_NOFILE, &low), 0);
    int seq = 1;
    for (int i = 3; i < 8; i++) seq &= syscall1(SYS_dup, 1) == i;
    check("dup_up_to_7", seq, 0);
    expect("dup_at_limit_emfile", syscall1(SYS_dup, 1), -EMFILE);
    a_close(5);
    expect("dup_after_close_5", syscall1(SYS_dup, 1), 5);
    expect("dup_full_again_emfile", syscall1(SYS_dup, 1), -EMFILE);
    for (int i = 3; i < 8; i++) a_close(i);
    expect("restore_limit", syscall2(SYS_setrlimit, RLIMIT_NOFILE, &old), 0);
    expect("dup_after_restore", syscall1(SYS_dup, 1), 3);
    a_close(3);
    return finish();
}

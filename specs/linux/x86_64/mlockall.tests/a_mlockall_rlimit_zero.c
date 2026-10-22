/* mlockall with RLIMIT_MEMLOCK 0: an unprivileged caller gets EPERM for
 * every valid flag combination (bad flags are still EINVAL first). A
 * privileged caller (root, CAP_IPC_LOCK) is not limited and succeeds. */
#include "a_sys.h"

int test_main(void) {
    int root = syscall0(SYS_getuid) == 0;
    struct a_rlimit rl;
    expect("getrlimit", syscall2(SYS_getrlimit, RLIMIT_MEMLOCK, &rl), 0);
    rl.rlim_cur = 0;
    expect("setrlimit_0", syscall2(SYS_setrlimit, RLIMIT_MEMLOCK, &rl), 0);
    i64 want = root ? 0 : -EPERM;
    expect("current", syscall1(SYS_mlockall, MCL_CURRENT), want);
    expect("future", syscall1(SYS_mlockall, MCL_FUTURE), want);
    expect("current_future", syscall1(SYS_mlockall, MCL_CURRENT | MCL_FUTURE), want);
    expect("current_onfault", syscall1(SYS_mlockall, MCL_CURRENT | MCL_ONFAULT), want);
    expect("future_onfault", syscall1(SYS_mlockall, MCL_FUTURE | MCL_ONFAULT), want);
    expect("zero_still_einval", syscall1(SYS_mlockall, 0), -EINVAL);
    expect("onfault_alone_still_einval", syscall1(SYS_mlockall, MCL_ONFAULT), -EINVAL);
    expect("unknown_bit_still_einval", syscall1(SYS_mlockall, MCL_CURRENT | 8), -EINVAL);
    return finish();
}

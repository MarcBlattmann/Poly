/* sync() is not interrupted by and does not consume signals: with
 * SIGUSR1 blocked and pending it returns 0 and the signal stays pending;
 * the signal mask is unchanged. */
#include "a_sys.h"

#define SYS_sync 162

int test_main(void) {
    u64 set = 1UL << (SIGUSR1 - 1), old = 0, now = 0, pend = 0;
    expect("block", syscall4(SYS_rt_sigprocmask, SIG_BLOCK, &set, &old, 8), 0);
    expect("raise", syscall3(SYS_tgkill, syscall0(SYS_getpid), syscall0(SYS_gettid), SIGUSR1), 0);
    expect("sync", syscall0(SYS_sync), 0);
    expect("sigpending", syscall2(SYS_rt_sigpending, &pend, 8), 0);
    check("still_pending", (pend & set) != 0, (i64)pend);
    expect("getmask", syscall4(SYS_rt_sigprocmask, SIG_BLOCK, 0, &now, 8), 0);
    check("mask_kept", now == (old | set), (i64)now);
    expect("sync_again", syscall0(SYS_sync), 0);
    struct a_timespec zero = {0, 0};
    expect("consume", syscall4(SYS_rt_sigtimedwait, &set, 0, &zero, 8), SIGUSR1);
    return finish();
}

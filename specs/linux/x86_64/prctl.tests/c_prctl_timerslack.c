/* PR_SET_TIMERSLACK / PR_GET_TIMERSLACK: the calling thread's timer
 * slack in nanoseconds. GET returns it as the result (a value above
 * LONG_MAX comes back negative), ignoring other arguments. A started
 * program has the default 50000 ns. SET accepts any non-zero unsigned
 * 64-bit value, including 1 and ULONG_MAX, and returns 0; SET 0 goes
 * back to the default.
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_prctl 157
/* linux/prctl.h */
#define PR_SET_TIMERSLACK 29
#define PR_GET_TIMERSLACK 30

static i64 pr(i64 o, i64 a, i64 b, i64 c, i64 d) { return syscall5(SYS_prctl, o, a, b, c, d); }

int test_main(void) {
    expect("default 50000", pr(PR_GET_TIMERSLACK, 0, 0, 0, 0), 50000);
    expect("set 1000", pr(PR_SET_TIMERSLACK, 1000, 0, 0, 0), 0);
    expect("get 1000", pr(PR_GET_TIMERSLACK, 0, 0, 0, 0), 1000);
    expect("get, extra args", pr(PR_GET_TIMERSLACK, 5, 5, 5, 5), 1000);
    expect("set 1", pr(PR_SET_TIMERSLACK, 1, 0, 0, 0), 0);
    expect("get 1", pr(PR_GET_TIMERSLACK, 0, 0, 0, 0), 1);
    expect("set 1<<40", pr(PR_SET_TIMERSLACK, 1L << 40, 0, 0, 0), 0);
    expect("get 1<<40", pr(PR_GET_TIMERSLACK, 0, 0, 0, 0), 1L << 40);
    expect("set ULONG_MAX", pr(PR_SET_TIMERSLACK, -1, 0, 0, 0), 0);
    expect("get ULONG_MAX", pr(PR_GET_TIMERSLACK, 0, 0, 0, 0), -1);
    expect("set 0 resets", pr(PR_SET_TIMERSLACK, 0, 0, 0, 0), 0);
    expect("back to 50000", pr(PR_GET_TIMERSLACK, 0, 0, 0, 0), 50000);
    expect("set 7, extra args", pr(PR_SET_TIMERSLACK, 7, 9, 9, 9), 0);
    expect("get 7", pr(PR_GET_TIMERSLACK, 0, 0, 0, 0), 7);
    /* A sleep still works with a large slack (no upper timing bound here). */
    pr(PR_SET_TIMERSLACK, 1000000, 0, 0, 0);
    struct a_timespec ts = {0, 1000000};
    expect("nanosleep with 1 ms slack", syscall2(SYS_nanosleep, &ts, 0), 0);
    pr(PR_SET_TIMERSLACK, 0, 0, 0, 0);
    return finish();
}

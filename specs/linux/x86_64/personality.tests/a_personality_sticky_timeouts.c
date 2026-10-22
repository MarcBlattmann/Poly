/* personality: with STICKY_TIMEOUTS (0x4000000) select, pselect6 and
 * ppoll leave the caller's timeout untouched; without it they write
 * back the time left (0 after a full timeout).
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_personality 135
#define SYS_select 23
#define SYS_pselect6 270
#define SYS_ppoll 271
/* linux/personality.h */
#define STICKY_TIMEOUTS 0x4000000
#define PER_SVR4 0x04100001

static i64 personality(u64 p) { return syscall1(SYS_personality, p); }

int test_main(void) {
    struct a_timeval tv;
    struct a_timespec ts;

    tv.tv_sec = 0, tv.tv_usec = 20000;
    expect("select_plain", syscall5(SYS_select, 0, 0, 0, 0, &tv), 0);
    expect("select_plain_updated", tv.tv_usec, 0);
    ts.tv_sec = 0, ts.tv_nsec = 20000000;
    expect("pselect_plain", syscall6(SYS_pselect6, 0, 0, 0, 0, (i64)&ts, 0), 0);
    expect("pselect_plain_updated", ts.tv_nsec, 0);
    ts.tv_sec = 0, ts.tv_nsec = 20000000;
    expect("ppoll_plain", syscall5(SYS_ppoll, 0, 0, &ts, 0, 8), 0);
    expect("ppoll_plain_updated", ts.tv_nsec, 0);

    personality(STICKY_TIMEOUTS);
    tv.tv_sec = 0, tv.tv_usec = 20000;
    expect("select_sticky", syscall5(SYS_select, 0, 0, 0, 0, &tv), 0);
    expect("select_sticky_usec", tv.tv_usec, 20000);
    expect("select_sticky_sec", tv.tv_sec, 0);
    ts.tv_sec = 0, ts.tv_nsec = 20000000;
    expect("pselect_sticky", syscall6(SYS_pselect6, 0, 0, 0, 0, (i64)&ts, 0), 0);
    expect("pselect_sticky_nsec", ts.tv_nsec, 20000000);
    ts.tv_sec = 0, ts.tv_nsec = 20000000;
    expect("ppoll_sticky", syscall5(SYS_ppoll, 0, 0, &ts, 0, 8), 0);
    expect("ppoll_sticky_nsec", ts.tv_nsec, 20000000);

    /* A domain that implies the flag (PER_SVR4 includes its bit). */
    personality(PER_SVR4);
    tv.tv_sec = 0, tv.tv_usec = 20000;
    expect("select_svr4", syscall5(SYS_select, 0, 0, 0, 0, &tv), 0);
    expect("select_svr4_usec", tv.tv_usec, 20000);

    personality(0);
    tv.tv_sec = 0, tv.tv_usec = 20000;
    expect("select_cleared", syscall5(SYS_select, 0, 0, 0, 0, &tv), 0);
    expect("select_cleared_updated", tv.tv_usec, 0);
    return finish();
}

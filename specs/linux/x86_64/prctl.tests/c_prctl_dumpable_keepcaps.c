/* PR_GET_DUMPABLE / PR_SET_DUMPABLE and PR_GET_KEEPCAPS /
 * PR_SET_KEEPCAPS: both flags are 0 or 1 and GET returns the value
 * itself. A normal process starts dumpable 1 and keepcaps 0. Setting any
 * other value (2, -1, 1<<32) is EINVAL and changes nothing. Arguments
 * past arg2 are ignored by all four. keepcaps is also bit 4
 * (SECBIT_KEEP_CAPS, 0x10) of PR_GET_SECUREBITS.
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_prctl 157
/* linux/prctl.h */
#define PR_GET_DUMPABLE 3
#define PR_SET_DUMPABLE 4
#define PR_GET_KEEPCAPS 7
#define PR_SET_KEEPCAPS 8
#define PR_GET_SECUREBITS 27
/* linux/securebits.h */
#define SECBIT_KEEP_CAPS 0x10

static i64 pr(i64 o, i64 a, i64 b, i64 c, i64 d) { return syscall5(SYS_prctl, o, a, b, c, d); }

int test_main(void) {
    expect("dumpable starts 1", pr(PR_GET_DUMPABLE, 0, 0, 0, 0), 1);
    expect("get dumpable, extra args", pr(PR_GET_DUMPABLE, -1, -1, -1, -1), 1);
    expect("set dumpable 2", pr(PR_SET_DUMPABLE, 2, 0, 0, 0), -EINVAL);
    expect("set dumpable -1", pr(PR_SET_DUMPABLE, -1, 0, 0, 0), -EINVAL);
    expect("set dumpable 1<<32", pr(PR_SET_DUMPABLE, 1L << 32, 0, 0, 0), -EINVAL);
    expect("still 1", pr(PR_GET_DUMPABLE, 0, 0, 0, 0), 1);
    expect("set dumpable 0, extra args", pr(PR_SET_DUMPABLE, 0, 7, 7, 7), 0);
    expect("now 0", pr(PR_GET_DUMPABLE, 0, 0, 0, 0), 0);
    expect("set dumpable 1", pr(PR_SET_DUMPABLE, 1, 0, 0, 0), 0);
    expect("back to 1", pr(PR_GET_DUMPABLE, 0, 0, 0, 0), 1);

    expect("keepcaps starts 0", pr(PR_GET_KEEPCAPS, 0, 0, 0, 0), 0);
    expect("securebits keep_caps clear", pr(PR_GET_SECUREBITS, 0, 0, 0, 0) & SECBIT_KEEP_CAPS, 0);
    expect("get keepcaps, extra args", pr(PR_GET_KEEPCAPS, 1, 2, 3, 4), 0);
    expect("set keepcaps 2", pr(PR_SET_KEEPCAPS, 2, 0, 0, 0), -EINVAL);
    expect("set keepcaps -1", pr(PR_SET_KEEPCAPS, -1, 0, 0, 0), -EINVAL);
    expect("still 0", pr(PR_GET_KEEPCAPS, 0, 0, 0, 0), 0);
    expect("set keepcaps 1", pr(PR_SET_KEEPCAPS, 1, 0, 0, 0), 0);
    expect("now 1", pr(PR_GET_KEEPCAPS, 0, 0, 0, 0), 1);
    expect("securebits keep_caps set", pr(PR_GET_SECUREBITS, 0, 0, 0, 0) & SECBIT_KEEP_CAPS, SECBIT_KEEP_CAPS);
    expect("set keepcaps 0, extra args", pr(PR_SET_KEEPCAPS, 0, 1, 1, 1), 0);
    expect("back to 0", pr(PR_GET_KEEPCAPS, 0, 0, 0, 0), 0);
    expect("securebits keep_caps clear again", pr(PR_GET_SECUREBITS, 0, 0, 0, 0) & SECBIT_KEEP_CAPS, 0);
    return finish();
}

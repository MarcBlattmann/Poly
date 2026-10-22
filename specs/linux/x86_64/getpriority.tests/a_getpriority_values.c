/* getpriority returns 20 - nice for every nice value the caller can
 * reach by raising it: nice n gives 20 - n, nice 19 gives 1; a nice
 * value above 19 given to setpriority is clamped to 19, so getpriority
 * then returns 1.
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_getpriority 140
#define SYS_setpriority 141
/* linux/resource.h */
#define PRIO_PROCESS 0

static i64 getprio(i64 which, i64 who) { return syscall2(SYS_getpriority, which, who); }
static i64 setprio(i64 which, i64 who, i64 nice) { return syscall3(SYS_setpriority, which, who, nice); }

int test_main(void) {
    i64 n0 = 20 - getprio(PRIO_PROCESS, 0);
    check("start_nice_at_most_10", n0 <= 10, n0);
    int all = 1;
    i64 bad = 0;
    for (i64 n = n0; n <= 19; n++) {
        if (setprio(PRIO_PROCESS, 0, n) != 0 || getprio(PRIO_PROCESS, 0) != 20 - n) {
            all = 0;
            bad = n;
        }
    }
    check("each_nice_is_20_minus_nice", all, bad);
    expect("nice_19_is_1", getprio(PRIO_PROCESS, 0), 1);
    expect("set_100", setprio(PRIO_PROCESS, 0, 100), 0);
    expect("clamped_to_1", getprio(PRIO_PROCESS, 0), 1);
    expect("set_20", setprio(PRIO_PROCESS, 0, 20), 0);
    expect("still_1", getprio(PRIO_PROCESS, 0), 1);
    expect("set_big", setprio(PRIO_PROCESS, 0, 0x7fffffff), 0);
    expect("still_1_big", getprio(PRIO_PROCESS, 0), 1);
    return finish();
}

/* time advances by about a second across a 1.1 s sleep. */
#include "a_sys.h"

int test_main(void) {
    i64 t0 = syscall1(SYS_time, 0);
    struct a_timespec req = {1, 100000000};
    expect("sleep", syscall2(SYS_nanosleep, &req, 0), 0);
    i64 t1 = syscall1(SYS_time, 0);
    check("advanced_1_or_2", t1 - t0 >= 1 && t1 - t0 <= 3, t1 - t0);
    return finish();
}

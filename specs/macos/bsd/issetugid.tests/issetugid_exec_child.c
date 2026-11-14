/* issetugid, BSD 327: a program started by execve from an ordinary file
 * also gets 0 (the test runs itself again). */
#include "poly_mtest.h"

int test_main(void) {
    if (is_child("child")) poly_exit((int)bsd0(SYS_issetugid).ret + 10);
    i64 pid = poly_start_self("child", 0, 0);
    int st = poly_wait(pid);
    check("the new program: 0", W_EXITED(st) && W_EXITSTATUS(st) == 10, st);
    return finish();
}

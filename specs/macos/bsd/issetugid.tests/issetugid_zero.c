/* issetugid, BSD 327: 0 for a process that was not started set-user-ID or
 * set-group-ID, whatever arguments are in the registers. */
#include "poly_mtest.h"

int test_main(void) {
    sysret r = bsd0(SYS_issetugid);
    check("returns 0", r.err == 0 && r.ret == 0, r.err ? r.err : r.ret);
    r = bsd6(SYS_issetugid, 1, 2, 3, 4, 5, 6);
    check("with arguments: still 0", r.err == 0 && r.ret == 0, r.ret);
    return finish();
}

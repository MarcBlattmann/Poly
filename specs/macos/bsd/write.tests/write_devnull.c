/* write(2), BSD 4, to /dev/null: every byte is accepted, also many at once,
 * and the result is the count. */
#include "poly_mtest.h"

static char big[100000];

int test_main(void) {
    sysret n = bsd3(SYS_open, "/dev/null", O_WRONLY, 0);
    check_ok("open /dev/null", n);
    sysret r = bsd3(SYS_write, n.ret, "abc", 3);
    check("3 bytes: 3", r.err == 0 && r.ret == 3, r.ret);
    r = bsd3(SYS_write, n.ret, big, sizeof big);
    check("100000 bytes: 100000", r.err == 0 && r.ret == 100000, r.ret);
    r = bsd3(SYS_write, n.ret, big, 0);
    check("0 bytes: 0", r.err == 0 && r.ret == 0, r.ret);
    return finish();
}

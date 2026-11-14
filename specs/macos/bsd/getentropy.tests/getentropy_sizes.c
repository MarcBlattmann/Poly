/* getentropy, BSD 500, lengths: 0, 1 and 256 work; more than 256 and a
 * negative length are EINVAL (libSystem's wrapper turns that into EIO). */
#include "poly_mtest.h"

int test_main(void) {
    unsigned char buf[512];
    check_ok("length 0", bsd2(SYS_getentropy, buf, 0));
    check_ok("length 1", bsd2(SYS_getentropy, buf, 1));
    check_ok("length 255", bsd2(SYS_getentropy, buf, 255));
    check_ok("length 256", bsd2(SYS_getentropy, buf, 256));
    check_err("length 257: EINVAL", bsd2(SYS_getentropy, buf, 257), EINVAL);
    check_err("length 512: EINVAL", bsd2(SYS_getentropy, buf, 512), EINVAL);
    check_err("length -1: EINVAL", bsd2(SYS_getentropy, buf, -1), EINVAL);
    return finish();
}

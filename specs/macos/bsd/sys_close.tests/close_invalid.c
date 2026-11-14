/* close(2), BSD 6: descriptors that were never open. */
#include "poly_mtest.h"

int test_main(void) {
    check_err("fd -1: EBADF", bsd1(SYS_close, -1), EBADF);
    check_err("fd 99: EBADF", bsd1(SYS_close, 99), EBADF);
    check_err("fd 100000: EBADF", bsd1(SYS_close, 100000), EBADF);
    return finish();
}

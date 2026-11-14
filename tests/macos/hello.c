/* The smallest macOS test: write(2) to fd 1 and exit(2) with status 0,
 * both as raw BSD system calls (ADR 0182). */
#include "poly_mtest.h"

int test_main(void) {
    sysret r = bsd3(SYS_write, 1, "hello from a macOS test\n", 24);
    check("write returns the byte count", r.err == 0 && r.ret == 24, r.err ? r.err : r.ret);
    return finish();
}

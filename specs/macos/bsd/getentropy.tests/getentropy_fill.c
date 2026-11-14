/* getentropy, BSD 500 (buffer, length): fills exactly length bytes with
 * random data, up to 256: two calls give different bytes, almost all of
 * them nonzero, and the bytes after the buffer's length are not touched. */
#include "poly_mtest.h"

int test_main(void) {
    unsigned char a[256], b[256];
    memset(a, 0, sizeof a);
    memset(b, 0, sizeof b);
    sysret r = bsd2(SYS_getentropy, a, 256);
    check("256 bytes: returns 0", r.err == 0 && r.ret == 0, r.err);
    bsd2(SYS_getentropy, b, 256);
    check("two calls differ", !mem_eq(a, b, 256), 0);
    int nonzero = 0;
    for (int i = 0; i < 256; i++) nonzero += a[i] != 0;
    check("almost all bytes are nonzero", nonzero > 200, nonzero);
    unsigned char c[64];
    memset(c, 0x55, sizeof c);
    bsd2(SYS_getentropy, c, 16);
    int past = 1;
    for (int i = 16; i < 64; i++) past &= c[i] == 0x55;
    check("only the first 16 bytes were written", past, 0);
    return finish();
}

/* arc4random_buf: fills exactly n bytes with random data; no limit on n;
 * never fails; bytes past n are not touched. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <stdlib.h>
#include <string.h>

int test_main(void) {
    unsigned char a[64], b[64];
    memset(a, 0, sizeof a);
    memset(b, 0, sizeof b);
    arc4random_buf(a, 32);
    arc4random_buf(b, 32);
    int zeros = 0;
    for (int i = 0; i < 32; i++) zeros += a[i] == 0;
    check("32 bytes: almost all nonzero", zeros <= 4, zeros);
    check("two calls differ", memcmp(a, b, 32) != 0, 0);
    int rest = 0;
    for (int i = 32; i < 64; i++) rest |= a[i] | b[i];
    check("bytes past n are untouched", rest == 0, rest);

    memset(a, 0x55, sizeof a);
    arc4random_buf(a, 0);
    check("n = 0 writes nothing", a[0] == 0x55, a[0]);
    arc4random_buf(NULL, 0);
    check("n = 0 with NULL is fine (still alive)", 1, 0);
    memset(a, 0x55, sizeof a);
    arc4random_buf(a + 1, 1);
    check("n = 1 writes one byte at most", a[0] == 0x55 && a[2] == 0x55, a[2]);
    return finish();
}

/* strlcpy: copies of every length 0..20 into a 10 byte buffer; a guard
 * byte after the buffer never changes. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <string.h>

int test_main(void) {
    char src[32];
    for (int len = 0; len <= 20; len++) {
        char buf[11];
        memset(buf, 'g', sizeof buf);
        for (int i = 0; i < len; i++) src[i] = (char)('a' + i);
        src[len] = 0;
        size_t r = strlcpy(buf, src, 10);
        int want = len < 10 ? len : 9;
        int ok = r == (size_t)len && buf[want] == 0 && buf[10] == 'g' && memcmp(buf, src, (size_t)want) == 0;
        check("length", ok, len);
    }
    char b[5];
    memset(b, 'x', sizeof b);
    /* Copy of a copy, same buffer start: src == dst is allowed for the
     * return value only when sizes match; use two distinct buffers. */
    strlcpy(b, "abcd", sizeof b);
    char c[5];
    check("copy of a copy", strlcpy(c, b, sizeof c) == 4, 0);
    check_str("equal", c, "abcd");
    return finish();
}

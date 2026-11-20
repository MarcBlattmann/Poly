/* strlcat when dst has no NUL within size, and size 0: dst is not touched
 * and the result is size + strlen(src). */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <string.h>

int test_main(void) {
    char b[8];
    memset(b, 'a', sizeof b); /* no NUL in 8 bytes */
    check("unterminated, size 8: returns 8 + 3", strlcat(b, "xyz", 8) == 11, 0);
    check("unterminated: dst untouched", b[0] == 'a' && b[7] == 'a', b[7]);

    char c[8] = "ab";
    memset(c + 3, 'q', 5);
    check("size 0: returns 0 + 3", strlcat(c, "xyz", 0) == 3, 0);
    check_str("size 0: dst untouched", c, "ab");

    char d[8] = "abcdef";
    check("size 2 smaller than strlen(dst): returns 2 + 3", strlcat(d, "xyz", 2) == 5, 0);
    check_str("size 2: dst untouched", d, "abcdef");

    char e[8] = "abc";
    check("size == strlen(dst): returns 3 + 3", strlcat(e, "xyz", 3) == 6, 0);
    check_str("size == strlen(dst): dst untouched", e, "abc");

    char f[4] = "ab";
    check("a long source, small room: returns 2 + 1000", strlcat(f, "1234567890", sizeof f) == 12, 0);
    check_str("a: ab1", f, "ab1");
    return finish();
}

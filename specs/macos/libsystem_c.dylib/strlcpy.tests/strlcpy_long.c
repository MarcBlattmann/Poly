/* strlcpy with a source much longer than the destination: the whole source
 * is measured, only size-1 bytes are copied. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <string.h>

int test_main(void) {
    static char src[1001];
    memset(src, 'a', 1000);
    src[1000] = 0;
    char b[8];
    memset(b, 'x', sizeof b);
    check("returns 1000", strlcpy(b, src, sizeof b) == 1000, 0);
    check_str("7 bytes and NUL", b, "aaaaaaa");

    /* A path-building loop as callers write it: detect truncation by
     * ret >= size. */
    char path[10];
    size_t n = strlcpy(path, "/usr/lib", sizeof path);
    check("no truncation: ret < size", n < sizeof path, (long long)n);
    n = strlcpy(path, "/usr/local/lib", sizeof path);
    check("truncation: ret >= size", n >= sizeof path, (long long)n);
    check_str("truncated to 9 bytes", path, "/usr/loca");

    /* Copying from the end of a page: strlcpy must not read past the NUL. */
    unsigned char *page = (unsigned char *)b; (void)page;
    static char buf[64];
    memcpy(buf + 60, "ab", 3);
    char d[4];
    check("short source at a buffer end", strlcpy(d, buf + 60, sizeof d) == 2, 0);
    check_str("copied", d, "ab");

    /* The result is a copy, not an alias. */
    char one[2] = {'q', 'q'};
    strlcpy(one, "z", 2);
    check_str("one byte", one, "z");
    return finish();
}

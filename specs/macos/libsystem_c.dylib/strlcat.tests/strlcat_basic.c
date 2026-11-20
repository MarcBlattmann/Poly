/* strlcat: appends src to the NUL-terminated string in dst, to at most
 * size-1 bytes in all; returns the length it tried to make. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <string.h>

int test_main(void) {
    char b[16];
    memset(b, 'x', sizeof b);
    strlcpy(b, "foo", sizeof b);
    check("append: returns 6", strlcat(b, "bar", sizeof b) == 6, 0);
    check_str("append: foobar", b, "foobar");

    strlcpy(b, "foo", sizeof b);
    check("empty src: returns 3", strlcat(b, "", sizeof b) == 3, 0);
    check_str("empty src: unchanged", b, "foo");

    memset(b, 'x', sizeof b);
    b[0] = 0;
    check("empty dst: returns strlen(src)", strlcat(b, "abc", sizeof b) == 3, 0);
    check_str("empty dst: abc", b, "abc");

    memset(b, 'x', sizeof b);
    strlcpy(b, "foo", sizeof b);
    check("exact fit (size 7): returns 6", strlcat(b, "bar", 7) == 6, 0);
    check_str("exact fit: foobar", b, "foobar");

    memset(b, 'x', sizeof b);
    strlcpy(b, "foo", sizeof b);
    check("truncated (size 6): returns 6", strlcat(b, "bar", 6) == 6, 0);
    check_str("truncated: fooba", b, "fooba");
    check("truncated: byte after the size untouched", b[6] == 'x', b[6]);

    memset(b, 'x', sizeof b);
    strlcpy(b, "foo", sizeof b);
    check("size 4, one byte of room is the NUL: returns 6", strlcat(b, "bar", 4) == 6, 0);
    check_str("size 4: foo", b, "foo");
    return finish();
}

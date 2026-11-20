/* memset_pattern16(b, pattern, len) fills len bytes with the 16 byte
 * pattern repeated, starting at b; the last copy is cut short. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <string.h>

int test_main(void) {
    const char pat[17] = "0123456789abcdef";
    char b[64];
    memset(b, '.', sizeof b);
    memset_pattern16(b, pat, 40);
    check("whole copies", memcmp(b, pat, 16) == 0 && memcmp(b + 16, pat, 16) == 0, 0);
    check("partial third copy: 8 bytes", memcmp(b + 32, pat, 8) == 0, 0);
    check("nothing past len", b[40] == '.' && b[63] == '.', b[40]);

    memset(b, '.', sizeof b);
    memset_pattern16(b, pat, 5);
    check("len 5: first 5 bytes of the pattern", memcmp(b, pat, 5) == 0, 0);
    check("len 5: byte 5 untouched", b[5] == '.', b[5]);

    memset(b, '.', sizeof b);
    memset_pattern16(b, pat, 0);
    check("len 0: nothing written", b[0] == '.', b[0]);

    memset(b, '.', sizeof b);
    memset_pattern16(b, pat, 16);
    check("len 16: exactly one copy", memcmp(b, pat, 16) == 0 && b[16] == '.', 0);

    memset(b, '.', sizeof b);
    memset_pattern16(b, pat, 17);
    check("len 17: one copy and one byte", memcmp(b, pat, 16) == 0 && b[16] == '0' && b[17] == '.', b[16]);
    return finish();
}

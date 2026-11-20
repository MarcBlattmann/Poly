/* snprintf: integer conversions, flags, widths, length modifiers and the
 * BSD long forms %D %O %U. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <stdint.h>
#include <stdio.h>
#include <string.h>

static int (*volatile sn)(char *, size_t, const char *, ...) = snprintf;
/* One call into a 128 byte buffer: the name, the return value and the text. */
#define CK(want_ret, want_str, ...) do { \
    char b_[128]; memset(b_, 0, sizeof b_); \
    int r_ = sn(b_, sizeof b_, __VA_ARGS__); \
    check_str("text of " #__VA_ARGS__, b_, want_str); \
    check("return of " #__VA_ARGS__, r_ == (want_ret), r_); \
} while (0)

int test_main(void) {
    CK(1, "5", "%d", 5);
    CK(2, "-5", "%i", -5);
    CK(10, "4294967295", "%u", -1);
    CK(8, "ffffffff", "%x", -1);
    CK(3, "100", "%o", 64);
    CK(4, "0xff", "%#x", 255);
    CK(4, "0XFF", "%#X", 255);
    CK(1, "0", "%#x", 0);
    CK(1, "0", "%#o", 0);
    CK(3, "010", "%#o", 8);
    CK(6, "  007|", "%5.3d|", 7);
    CK(6, "7    |", "%-5d|", 7);
    CK(2, "+7", "%+d", 7);
    CK(2, " 7", "% d", 7);
    CK(5, "-0007", "%05d", -7);
    CK(0, "", "%.0d", 0);
    CK(2, "5|", "%.0d|", 5);
    CK(3, "005", "%.3x", 5);
    CK(5, "0x005", "%#.3x", 5);
    CK(5, "7    ", "%*d", -5, 7);
    CK(6, "    7|", "%*d|", 5, 7);
    CK(1, "7", "%.*d", -1, 7);
    CK(2, "44", "%hhd", 300);
    CK(4, "4464", "%hd", 70000);
    CK(1, "5", "%zu", (size_t)5);
    CK(2, "-1", "%jd", (intmax_t)-1);
    CK(1, "3", "%td", (ptrdiff_t)3);
    CK(2, "-5", "%ld", -5L);
    CK(19, "9223372036854775807", "%lld", 9223372036854775807LL);
    CK(20, "-9223372036854775808", "%lld", (-9223372036854775807LL - 1));
    CK(20, "18446744073709551615", "%llu", ~0ULL);
    CK(16, "ffffffffffffffff", "%llx", ~0ULL);
    CK(7, "1234567", "%'d", 1234567);
    /* BSD long forms. */
    CK(1, "5", "%D", 5L);
    CK(2, "10", "%O", 8L);
    CK(1, "7", "%U", 7L);
    CK(20, "-9223372036854775808", "%D", (-9223372036854775807L - 1));
    CK(20, "18446744073709551615", "%U", ~0UL);
    CK(3, "5 6", "%d %d", 5, 6);
    CK(3, "2 1", "%2$d %1$d", 1, 2);
    CK(7, "rep rep", "%1$s %1$s", "rep");
    return finish();
}

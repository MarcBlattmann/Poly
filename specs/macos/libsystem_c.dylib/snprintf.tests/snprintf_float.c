/* snprintf: floating-point conversions: fixed, exponent, shortest, hex;
 * infinity and NaN; round-half-even. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
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
    double inf = 1.0 / 0.0, nan = 0.0 / 0.0;
    CK(8, "1.500000", "%f", 1.5);
    CK(9, "-0.000000", "%f", -0.0);
    CK(12, "1.234568e+04", "%e", 12345.678);
    CK(6, "0.0001", "%g", 0.0001);
    CK(5, "1e-05", "%g", 0.00001);
    CK(11, "1.23457e+08", "%g", 123456789.0);
    CK(3, "100", "%g", 100.0);
    CK(5, "1E-10", "%G", 1e-10);
    CK(22, "0.10000000000000000555", "%.20f", 0.1);
    CK(1, "0", "%.0f", 0.5);
    CK(1, "2", "%.0f", 1.5);
    CK(1, "2", "%.0f", 2.5);
    CK(2, "2.", "%#.0f", 2.0);
    CK(8, "0003.142", "%08.3f", 3.14159);
    CK(9, "3.142   |", "%-8.3f|", 3.14159);
    CK(8, "+3.14159", "%+g", 3.14159);
    CK(3, "inf", "%f", inf);
    CK(4, "-inf", "%f", -inf);
    CK(3, "nan", "%f", nan);
    CK(3, "inf", "%e", inf);
    CK(3, "INF", "%F", inf);
    CK(3, "NAN", "%E", nan);
    CK(3, "NAN", "%G", nan);
    CK(11, "       inf|", "%10f|", inf);
    CK(11, "       inf|", "%010f|", inf);
    CK(4, "+inf", "%+f", inf);
    CK(6, "0x1p+0", "%a", 1.0);
    CK(6, "0x1p-1", "%a", 0.5);
    CK(9, "0X1.FEP+7", "%A", 255.0);
    CK(9, "0x1.00p+0", "%.2a", 1.0);
    CK(6, "0x0p+0", "%a", 0.0);
    CK(8, "1.500000", "%Lf", (long double)1.5);
    CK(23, "10000000000000000000000", "%.0f", 1e22);
    CK(3, "1.5", "%.2g", 1.5);
    CK(5, "2e+02", "%.1g", 150.0);
    CK(1, "1", "%g", 1.0);
    CK(11, "-1.5000e+00", "%.4e", -1.5);
    return finish();
}

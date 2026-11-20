/* poly-mac: corefoundation */
/* CFNumberCompare orders by value across types; NaN is equal to NaN and
 * smaller than any other number. */
#include "poly_mcf.h"

static CFNumberRef I(int v) { return CFNumberCreate(kCFAllocatorDefault, kCFNumberIntType, &v); }
static CFNumberRef D(double v) { return CFNumberCreate(kCFAllocatorDefault, kCFNumberDoubleType, &v); }

int test_main(void) {
    CKV(CFNumberCompare(I(1), D(1.5), NULL) == -1, 0);
    CKV(CFNumberCompare(D(2.5), I(2), NULL) == 1, 0);
    CKV(CFNumberCompare(I(7), D(7.0), NULL) == 0, 0);
    CKV(CFNumberCompare(I(-1), I(1), NULL) == -1, 0);
    CKV(CFNumberCompare(kCFNumberNaN, kCFNumberNaN, NULL) == 0, 0);
    CKV(CFNumberCompare(kCFNumberNaN, I(1), NULL) == -1, 0);
    CKV(CFNumberCompare(I(1), kCFNumberNaN, NULL) == 1, 0);
    CKV(CFNumberCompare(kCFNumberNegativeInfinity, I(-1000), NULL) == -1, 0);
    CKV(CFNumberCompare(kCFNumberPositiveInfinity, D(1e300), NULL) == 1, 0);
    return finish();
}

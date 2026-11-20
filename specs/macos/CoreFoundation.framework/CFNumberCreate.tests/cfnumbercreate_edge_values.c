/* poly-mac: corefoundation */
/* The extremes read back as stored; the constants kCFNumberNaN and the
 * infinities are numbers of floating type, and NaN made by hand is the same
 * object as kCFNumberNaN. */
#include "poly_mcf.h"

int test_main(void) {
    long long out = 0;
    CFNumberRef big = CFNumberCreate(kCFAllocatorDefault, kCFNumberLongLongType, &(long long){0x7fffffffffffffffLL});
    CK(CFNumberGetValue(big, kCFNumberLongLongType, &out) && out == 0x7fffffffffffffffLL);
    CFNumberRef small = CFNumberCreate(kCFAllocatorDefault, kCFNumberLongLongType, &(long long){(long long)0x8000000000000000ULL});
    CK(CFNumberGetValue(small, kCFNumberLongLongType, &out) && out == (long long)0x8000000000000000ULL);
    CFNumberRef neg = CFNumberCreate(kCFAllocatorDefault, kCFNumberIntType, &(int){-12});
    CK(CFNumberGetValue(neg, kCFNumberLongLongType, &out) && out == -12);
    double d = 0;
    CK(CFNumberGetValue(CFNumberCreate(kCFAllocatorDefault, kCFNumberDoubleType, &(double){2.5}), kCFNumberDoubleType, &d) && d == 2.5);
    CK(CFNumberIsFloatType(kCFNumberNaN) && CFNumberIsFloatType(kCFNumberPositiveInfinity));
    CK(CFGetTypeID(kCFNumberNaN) == CFNumberGetTypeID());
    CK(CFNumberGetValue(kCFNumberNaN, kCFNumberDoubleType, &d) && d != d);
    CK(CFNumberGetValue(kCFNumberPositiveInfinity, kCFNumberDoubleType, &d) && d > 1e300);
    CK(CFNumberGetValue(kCFNumberNegativeInfinity, kCFNumberDoubleType, &d) && d < -1e300);
    CK(CFNumberCreate(kCFAllocatorDefault, kCFNumberDoubleType, &(double){__builtin_nan("")}) == kCFNumberNaN);
    return finish();
}
